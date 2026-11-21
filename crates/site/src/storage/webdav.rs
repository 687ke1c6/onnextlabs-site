//! Storage on a WebDAV server (e.g. `rclone serve webdav`). Only a small part of WebDAV is
//! used: see docs/adr/0001-file-storage-over-webdav.md for the method table.
//!
//! Uploads are two steps so a file only appears under its real name once complete:
//! `upload` writes a hidden temporary file in the target folder, then `commit` moves it
//! to its name, never overwriting. Callers `delete` the temporary file if they give up.

use std::time::Duration;

use axum::body::{Body, Bytes};
use futures_util::{stream, Stream, StreamExt};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use quick_xml::{events::Event, Reader};
use reqwest::{header, Client, Method, RequestBuilder, Response, StatusCode, Url};
use tokio::sync::mpsc;

use super::{
    next_chunk, temp_name, Blob, ByteRange, FileInfo, Listing, Meter, StorageError, StoragePath,
    UploadError, Uploaded,
};

/// Everything but RFC 3986 "unreserved" characters is percent-encoded in a path segment.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// How long to wait for a connection, and for each read once connected. There's no limit
/// on a whole request: a large upload or download can take as long as it needs.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Asks only for what `list` needs.
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:"><prop><resourcetype/><getcontentlength/></prop></propfind>"#;

pub struct Storage {
    client: Client,
    /// Always ends in `/`, so paths can be appended.
    base: Url,
    auth: Option<(String, String)>,
}

impl Storage {
    pub fn new(
        mut base: Url,
        user: Option<String>,
        password: Option<String>,
        proxy: Option<reqwest::Proxy>,
    ) -> Result<Self, String> {
        if !matches!(base.scheme(), "http" | "https") {
            return Err(format!("FILES_URL must be http(s), not {}", base.scheme()));
        }
        if base.query().is_some() || base.fragment().is_some() {
            return Err("FILES_URL can't have a query or fragment".into());
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err("put credentials in FILES_USER / FILES_PASSWORD, not FILES_URL".into());
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let mut client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            // Credentials go on every request; never send them somewhere else.
            .redirect(reqwest::redirect::Policy::none());
        if let Some(p) = proxy {
            client = client.proxy(p);
        }
        let client = client.build().map_err(|e| e.to_string())?;
        let auth = match (user, password) {
            (Some(u), p) => Some((u, p.unwrap_or_default())),
            (None, Some(_)) => return Err("FILES_PASSWORD is set without FILES_USER".into()),
            (None, None) => None,
        };
        Ok(Storage { client, base, auth })
    }

    /// The root URL, for logs (credentials are never part of it).
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// Folder URLs end in `/`, which saves the server a redirect.
    fn url(&self, path: &StoragePath, dir: bool) -> Url {
        url_for(&self.base, path, dir)
    }

    fn request(&self, method: Method, url: Url) -> RequestBuilder {
        let req = self.client.request(method, url);
        match &self.auth {
            Some((user, pass)) => req.basic_auth(user, Some(pass)),
            None => req,
        }
    }

    /// One `PROPFIND` of the root, to log at startup whether storage answers.
    pub async fn check(&self) -> Result<(), StorageError> {
        let resp = self
            .request(propfind(), self.base.clone())
            .header("Depth", "0")
            .header(header::CONTENT_TYPE, "application/xml")
            .body(PROPFIND_BODY)
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::MULTI_STATUS | StatusCode::OK => Ok(()),
            s => Err(status_error(s, "PROPFIND", &self.base)),
        }
    }

    pub async fn list(&self, dir: &StoragePath) -> Result<Listing, StorageError> {
        let url = self.url(dir, true);
        let resp = self
            .request(propfind(), url.clone())
            .header("Depth", "1")
            .header(header::CONTENT_TYPE, "application/xml")
            .body(PROPFIND_BODY)
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::MULTI_STATUS => {}
            s => return Err(status_error(s, "PROPFIND", &url)),
        }
        let xml = resp.text().await.map_err(transport)?;
        let entries = parse_multistatus(&xml).map_err(|e| {
            StorageError::Io(std::io::Error::other(format!("bad PROPFIND reply: {e}")))
        })?;
        Ok(children(&url, entries))
    }

    /// Is there a file (not a folder) at `path`? A `Depth: 0` `PROPFIND` says which it is;
    /// servers differ on `HEAD` for folders (rclone answers 405).
    pub async fn exists(&self, path: &StoragePath) -> Result<bool, StorageError> {
        let url = self.url(path, false);
        let resp = self
            .request(propfind(), url.clone())
            .header("Depth", "0")
            .header(header::CONTENT_TYPE, "application/xml")
            .body(PROPFIND_BODY)
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::MULTI_STATUS => {}
            s => {
                return match status_error(s, "PROPFIND", &url) {
                    StorageError::NotFound => Ok(false),
                    e => Err(e),
                }
            }
        }
        let xml = resp.text().await.map_err(transport)?;
        let entries = parse_multistatus(&xml).map_err(|e| {
            StorageError::Io(std::io::Error::other(format!("bad PROPFIND reply: {e}")))
        })?;
        Ok(entries.first().is_some_and(|e| !e.collection))
    }

    /// `MKCOL` one level at a time, parents first. "Already exists" (405) is fine.
    pub async fn make_dirs(&self, dir: &StoragePath) -> Result<(), StorageError> {
        let mut path = StoragePath::root();
        for segment in dir.segments() {
            path = path
                .join(segment)
                .expect("segments of a valid path are valid");
            let url = self.url(&path, true);
            let resp = self
                .request(mkcol(), url.clone())
                .send()
                .await
                .map_err(transport)?;
            match resp.status() {
                StatusCode::CREATED | StatusCode::METHOD_NOT_ALLOWED => {}
                s => return Err(status_error(s, "MKCOL", &url)),
            }
        }
        Ok(())
    }

    /// `PUT`s the body as it arrives. The size cap and hash are applied on the way through;
    /// on any failure the temporary file is deleted (an interrupted `PUT` may leave one).
    pub async fn upload<S, E>(
        &self,
        dir: &StoragePath,
        name: &str,
        mut source: S,
        max_bytes: u64,
    ) -> Result<Uploaded, UploadError<E>>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send,
    {
        let temp = dir
            .join(&temp_name(name))
            .map_err(|_| StorageError::Io(std::io::Error::other("bad temporary name")))?;
        // The request body must be `'static`, and `source` (a multipart field) isn't:
        // pass chunks through a small channel and drive both sides here.
        let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(4);
        let body = reqwest::Body::wrap_stream(stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        }));
        let url = self.url(&temp, false);
        let put = self
            .request(Method::PUT, url.clone())
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .send();

        // Owns `tx`, so the body ends when this finishes.
        let feed = async move {
            let mut meter = Meter::new(max_bytes);
            loop {
                match next_chunk(&mut source, &mut meter).await {
                    Ok(Some(chunk)) => {
                        if tx.send(Ok(chunk)).await.is_err() {
                            // The request ended early; its result says why.
                            return (Ok(()), meter);
                        }
                    }
                    Ok(None) => return (Ok(()), meter),
                    Err(e) => {
                        // Abort the request rather than finish a truncated file.
                        let _ = tx
                            .send(Err(std::io::Error::other("upload abandoned")))
                            .await;
                        return (Err(e), meter);
                    }
                }
            }
        };
        let ((fed, meter), sent) = tokio::join!(feed, put);

        let result: Result<(), UploadError<E>> = match (fed, sent) {
            (Err(e), _) => Err(e),
            (Ok(()), Err(e)) => Err(transport(e).into()),
            (Ok(()), Ok(resp)) => match resp.status() {
                StatusCode::CREATED | StatusCode::NO_CONTENT | StatusCode::OK => Ok(()),
                s => Err(status_error(s, "PUT", &url).into()),
            },
        };
        let result = result.and_then(|()| meter.finish(temp.clone()));
        if result.is_err() {
            if let Err(e) = self.delete(&temp).await {
                if !matches!(e, StorageError::NotFound) {
                    tracing::warn!(path = %temp, error = %e, "couldn't delete failed upload");
                }
            }
        }
        result
    }

    /// `MOVE` with `Overwrite: F`: the server refuses (412) if `target` exists.
    pub async fn commit(
        &self,
        temp: &StoragePath,
        target: &StoragePath,
    ) -> Result<(), StorageError> {
        let url = self.url(temp, false);
        let resp = self
            .request(Method::from_bytes(b"MOVE").unwrap(), url.clone())
            .header("Destination", self.url(target, false).as_str())
            .header("Overwrite", "F")
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::CREATED | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::PRECONDITION_FAILED => Err(StorageError::AlreadyExists),
            s => Err(status_error(s, "MOVE", &url)),
        }
    }

    /// `GET`, passing `range` on. A server that ignores it (200) gets the whole file sent.
    pub async fn get(
        &self,
        path: &StoragePath,
        range: Option<ByteRange>,
    ) -> Result<Blob, StorageError> {
        let url = self.url(path, false);
        let mut req = self.request(Method::GET, url.clone());
        if let Some(r) = range {
            req = req.header(header::RANGE, r.to_header());
        }
        let resp = req.send().await.map_err(transport)?;
        let bad = |why: &str| StorageError::Io(std::io::Error::other(format!("GET {url}: {why}")));
        let header = |name| resp.headers().get(name).and_then(|v| v.to_str().ok());
        let (total, range) = match resp.status() {
            StatusCode::OK => {
                let len = content_length(&resp).ok_or_else(|| bad("no Content-Length"))?;
                (len, None)
            }
            StatusCode::PARTIAL_CONTENT => {
                let (first, last, total) = header(header::CONTENT_RANGE)
                    .and_then(parse_content_range)
                    .ok_or_else(|| bad("bad Content-Range"))?;
                if content_length(&resp).is_some_and(|n| n != last - first + 1) {
                    return Err(bad("Content-Length doesn't match Content-Range"));
                }
                (total, Some((first, last)))
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let total = header(header::CONTENT_RANGE)
                    .and_then(|v| v.strip_prefix("bytes */"))
                    .and_then(|n| n.trim().parse().ok())
                    .ok_or_else(|| bad("416 without the length"))?;
                return Err(StorageError::RangeNotSatisfiable { total });
            }
            s => return Err(status_error(s, "GET", &url)),
        };
        let body = resp
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other));
        Ok(Blob {
            total,
            range,
            body: Body::from_stream(body),
        })
    }

    pub async fn delete(&self, path: &StoragePath) -> Result<(), StorageError> {
        let url = self.url(path, false);
        let resp = self
            .request(Method::DELETE, url.clone())
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::NO_CONTENT | StatusCode::OK => Ok(()),
            s => Err(status_error(s, "DELETE", &url)),
        }
    }
}

fn propfind() -> Method {
    Method::from_bytes(b"PROPFIND").unwrap()
}

fn mkcol() -> Method {
    Method::from_bytes(b"MKCOL").unwrap()
}

/// The `Content-Length` header itself (`Response::content_length` can be a body hint).
fn content_length(resp: &Response) -> Option<u64> {
    resp.headers()
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

/// `bytes a-b/total` → (a, b, total). An unknown total (`*`) isn't accepted.
fn parse_content_range(v: &str) -> Option<(u64, u64, u64)> {
    let (span, total) = v.trim().strip_prefix("bytes ")?.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    let (a, b, total) = (a.parse().ok()?, b.parse().ok()?, total.parse().ok()?);
    (a <= b && b < total).then_some((a, b, total))
}

fn url_for(base: &Url, path: &StoragePath, dir: bool) -> Url {
    let mut rel = path
        .segments()
        .iter()
        .map(|s| utf8_percent_encode(s, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/");
    if dir && !rel.is_empty() {
        rel.push('/');
    }
    // `rel` is all encoded segments, so joining can only append to `base`.
    base.join(&rel).expect("encoded relative path joins")
}

/// The server couldn't be reached, or stopped answering.
fn transport(e: reqwest::Error) -> StorageError {
    StorageError::Unavailable(e.to_string())
}

/// A reply we didn't expect. Missing things are `NotFound`; a struggling or misconfigured
/// server (5xx, bad credentials) is `Unavailable`; anything else is a plain error.
fn status_error(status: StatusCode, method: &str, url: &Url) -> StorageError {
    let what = format!("{method} {} answered {status}", url.path());
    match status {
        // 409 Conflict: a parent folder is missing.
        StatusCode::NOT_FOUND | StatusCode::CONFLICT => StorageError::NotFound,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => StorageError::Unavailable(what),
        s if s.is_server_error() => StorageError::Unavailable(what),
        _ => StorageError::Io(std::io::Error::other(what)),
    }
}

/// One `<response>` of a multistatus reply.
#[derive(Debug, PartialEq)]
struct Entry {
    href: String,
    collection: bool,
    /// `<getcontentlength>`, for files.
    size: Option<u64>,
}

/// Reads `<href>`, whether `<resourcetype>` holds `<collection/>`, and
/// `<getcontentlength>`, per `<response>`.
/// Namespace prefixes vary between servers, so elements are matched by local name.
fn parse_multistatus(xml: &str) -> Result<Vec<Entry>, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    let mut in_href = false;
    let mut in_resourcetype = false;
    let mut in_length = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"response" => {
                    current = Some(Entry {
                        href: String::new(),
                        collection: false,
                        size: None,
                    })
                }
                b"href" => in_href = true,
                b"getcontentlength" => in_length = true,
                b"resourcetype" => in_resourcetype = true,
                b"collection" if in_resourcetype => {
                    if let Some(c) = &mut current {
                        c.collection = true;
                    }
                }
                _ => {}
            },
            Event::Empty(e) => {
                if in_resourcetype && e.local_name().as_ref() == b"collection" {
                    if let Some(c) = &mut current {
                        c.collection = true;
                    }
                }
            }
            Event::Text(t) if in_href => {
                if let Some(c) = &mut current {
                    c.href.push_str(&t.unescape()?);
                }
            }
            Event::Text(t) if in_length => {
                if let Some(c) = &mut current {
                    c.size = t.unescape()?.trim().parse().ok();
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"response" => entries.extend(current.take()),
                b"href" => in_href = false,
                b"resourcetype" => in_resourcetype = false,
                b"getcontentlength" => in_length = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(entries)
}

/// The folders and files directly inside `dir_url`. Hrefs can be absolute or relative and
/// encoded differently from how we asked, so they're compared as decoded segments.
fn children(dir_url: &Url, entries: Vec<Entry>) -> Listing {
    let parent = decoded_segments(dir_url.path());
    let mut listing = Listing::default();
    for e in entries {
        let Ok(url) = dir_url.join(e.href.trim()) else {
            continue;
        };
        let mut segments = decoded_segments(url.path());
        if segments.len() != parent.len() + 1 || !segments.starts_with(&parent) {
            continue; // the folder itself, or something unexpected
        }
        let name = segments.pop().expect("one more than the parent");
        if e.collection {
            listing.dirs.push(name);
        } else {
            // Every server we know sends the length; 0 is a harmless stand-in.
            let size = e.size.unwrap_or(0);
            listing.files.push(FileInfo { name, size });
        }
    }
    listing
}

fn decoded_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(s: &str) -> Url {
        Storage::new(Url::parse(s).unwrap(), None, None, None)
            .unwrap()
            .base
            .clone()
    }

    #[test]
    fn urls_encode_each_segment() {
        let b = base("http://files.internal:8080/dav");
        assert_eq!(b.as_str(), "http://files.internal:8080/dav/");
        let p = StoragePath::root()
            .join_all("orgs/acme/documents/Q1 #2 100%/Ünïcode?.pdf")
            .unwrap();
        assert_eq!(
            url_for(&b, &p, false).as_str(),
            "http://files.internal:8080/dav/orgs/acme/documents/Q1%20%232%20100%25/%C3%9Cn%C3%AFcode%3F.pdf"
        );
        let dir = StoragePath::root().join_all("orgs/acme").unwrap();
        assert_eq!(
            url_for(&b, &dir, true).as_str(),
            "http://files.internal:8080/dav/orgs/acme/"
        );
        assert_eq!(url_for(&b, &StoragePath::root(), true), b);
        // A segment that looks like a scheme or a parent can't escape the base.
        let tricky = StoragePath::root().join("http:x").unwrap();
        assert_eq!(
            url_for(&b, &tricky, false).as_str(),
            "http://files.internal:8080/dav/http%3Ax"
        );
    }

    #[test]
    fn content_ranges_parse() {
        assert_eq!(parse_content_range("bytes 0-4/11"), Some((0, 4, 11)));
        assert_eq!(parse_content_range("bytes 0-4/*"), None);
        assert_eq!(parse_content_range("bytes 5-4/11"), None);
        assert_eq!(parse_content_range("bytes 0-11/11"), None);
        assert_eq!(parse_content_range("bytes */11"), None);
    }

    #[test]
    fn config_is_checked() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(Storage::new(url("ftp://x/"), None, None, None).is_err());
        assert!(Storage::new(url("http://u:p@x/"), None, None, None).is_err());
        assert!(Storage::new(url("http://x/"), None, Some("p".into()), None).is_err());
        assert!(Storage::new(url("http://x/"), Some("u".into()), Some("p".into()), None).is_ok());
    }

    /// As `rclone serve webdav` v1.71.2 answered `PROPFIND_BODY` with `Depth: 1` (served from `/`).
    const RCLONE: &str = include_str!("testdata/propfind-rclone.xml");

    /// Another common shape: `D:` prefix, absolute URLs, `<collection></collection>`,
    /// an entity in a name, and a file.
    const OTHER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>http://files.internal:8080/dav/orgs/acme/documents/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection></D:collection></D:resourcetype></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
  <D:response>
    <D:href>http://files.internal:8080/dav/orgs/acme/documents/R%26D/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection></D:collection></D:resourcetype></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/orgs/acme/documents/Tom &amp; Jerry/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/orgs/acme/documents/deck.pdf</D:href>
    <D:propstat><D:prop><D:resourcetype/><D:getcontentlength>1234</D:getcontentlength></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
  </D:response>
</D:multistatus>"#;

    #[test]
    fn propfind_lists_child_folders_only() {
        let b = base("http://files.internal:8080/dav/");
        let dir = StoragePath::root().join_all("orgs/acme/documents").unwrap();
        let url = url_for(&b, &dir, true);
        let entries = parse_multistatus(OTHER).unwrap();
        assert_eq!(entries.len(), 4);
        let listing = children(&url, entries);
        assert_eq!(listing.dirs, ["R&D", "Tom & Jerry"]);
        assert_eq!(
            listing.files,
            [FileInfo {
                name: "deck.pdf".into(),
                size: 1234
            }]
        );
    }

    #[test]
    fn propfind_reads_rclone_replies() {
        let b = base("http://127.0.0.1:8090/");
        let dir = StoragePath::root().join_all("orgs/acme/documents").unwrap();
        let mut listing = children(&url_for(&b, &dir, true), parse_multistatus(RCLONE).unwrap());
        listing.dirs.sort();
        assert_eq!(
            listing.dirs,
            [".hidden", "Brand & Logo", "Q1 #2 100%", "Ünïcode"]
        );
        assert_eq!(
            listing.files,
            [FileInfo {
                name: "deck one.pdf".into(),
                size: 3
            }]
        );
    }

    /// A real `rclone serve webdav` on a free port, serving a fresh temporary folder.
    /// Stopped and cleaned up on drop. rclone is the only backend, so these tests fail
    /// (rather than skip) without it: it's in the devcontainer.
    struct Rclone {
        child: std::process::Child,
        root: std::path::PathBuf,
        storage: Storage,
    }

    impl Rclone {
        async fn start() -> Rclone {
            let root =
                std::env::temp_dir().join(format!("onnextlabs-dav-{:016x}", rand::random::<u64>()));
            std::fs::create_dir_all(&root).unwrap();
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            let child = std::process::Command::new("rclone")
                .args(["serve", "webdav", "--addr", &format!("127.0.0.1:{port}")])
                .arg(&root)
                .env("RCLONE_USER", "test")
                .env("RCLONE_PASS", "test")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("rclone must be installed for these tests (it is in the devcontainer)");
            let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
            let storage =
                Storage::new(url, Some("test".into()), Some("test".into()), None).unwrap();
            // Built first, so `Drop` stops rclone even if it never answers.
            let dav = Rclone {
                child,
                root,
                storage,
            };
            for _ in 0..100 {
                if dav.storage.check().await.is_ok() {
                    return dav;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("rclone didn't start");
        }
    }

    impl Drop for Rclone {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn chunks(parts: &[&'static str]) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin {
        stream::iter(
            parts
                .iter()
                .map(|p| Ok(Bytes::from_static(p.as_bytes())))
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn rclone_upload_commit_get_and_no_overwrite() {
        let dav = Rclone::start().await;
        let storage = &dav.storage;
        let dir = StoragePath::root()
            .join_all("orgs/acme/documents/Q1 #2 100%/Ünï & co")
            .unwrap();
        storage.make_dirs(&dir).await.unwrap();
        storage.make_dirs(&dir).await.unwrap(); // already there: fine

        let up = storage
            .upload(&dir, "a b.txt", chunks(&["hello ", "world"]), 100)
            .await
            .unwrap();
        assert_eq!(up.size, 11);
        assert_eq!(
            up.sha256,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        // Hidden until committed.
        let target = dir.join("a b.txt").unwrap();
        assert!(matches!(
            storage.get(&target, None).await,
            Err(StorageError::NotFound)
        ));
        assert!(!storage.exists(&target).await.unwrap());
        storage.commit(&up.temp, &target).await.unwrap();
        let on_disk = dav
            .root
            .join("orgs/acme/documents/Q1 #2 100%/Ünï & co/a b.txt");
        assert_eq!(std::fs::read_to_string(&on_disk).unwrap(), "hello world");
        assert!(storage.exists(&target).await.unwrap());
        assert!(!storage.exists(&dir).await.unwrap()); // a folder isn't a file

        let whole = storage.get(&target, None).await.unwrap();
        assert_eq!((whole.total, whole.range, whole.len()), (11, None, 11));
        let part = storage
            .get(&target, Some(ByteRange::FromTo(6, 100)))
            .await
            .unwrap();
        assert_eq!((part.total, part.range, part.len()), (11, Some((6, 10)), 5));
        let bytes = axum::body::to_bytes(part.body, 100).await.unwrap();
        assert_eq!(&bytes[..], b"world");
        let last = storage
            .get(&target, Some(ByteRange::Last(5)))
            .await
            .unwrap();
        assert_eq!(last.range, Some((6, 10)));
        assert!(matches!(
            storage.get(&target, Some(ByteRange::From(11))).await,
            Err(StorageError::RangeNotSatisfiable { total: 11 })
        ));

        // A second upload can't take the same name, and is left for the caller.
        let second = storage
            .upload(&dir, "a b.txt", chunks(&["other"]), 100)
            .await
            .unwrap();
        assert!(matches!(
            storage.commit(&second.temp, &target).await,
            Err(StorageError::AlreadyExists)
        ));
        assert_eq!(std::fs::read_to_string(&on_disk).unwrap(), "hello world");
        storage.delete(&second.temp).await.unwrap();

        let docs = StoragePath::root().join_all("orgs/acme/documents").unwrap();
        let listing = storage.list(&docs).await.unwrap();
        assert_eq!(listing.dirs, ["Q1 #2 100%"]);
        assert!(listing.files.is_empty());
        let listing = storage.list(&dir).await.unwrap();
        assert!(listing.dirs.is_empty());
        assert_eq!(
            listing.files,
            [FileInfo {
                name: "a b.txt".into(),
                size: 11
            }]
        );
    }

    #[tokio::test]
    async fn rclone_failed_uploads_leave_nothing() {
        let dav = Rclone::start().await;
        let storage = &dav.storage;
        let dir = StoragePath::root().join("docs").unwrap();
        storage.make_dirs(&dir).await.unwrap();

        let too_big = storage
            .upload(&dir, "big.bin", chunks(&["12345", "678"]), 6)
            .await;
        assert!(matches!(too_big, Err(UploadError::TooLarge)));
        let empty = storage.upload(&dir, "empty.bin", chunks(&[]), 6).await;
        assert!(matches!(empty, Err(UploadError::Empty)));
        let broken = stream::iter(vec![
            Ok(Bytes::from_static(b"abc")),
            Err(std::io::Error::other("connection reset")),
        ]);
        let cut_off = storage.upload(&dir, "cut.bin", broken, 100).await;
        assert!(matches!(cut_off, Err(UploadError::Source(_))));

        assert_eq!(std::fs::read_dir(dav.root.join("docs")).unwrap().count(), 0);
        assert!(storage.list(&dir).await.unwrap().files.is_empty());
        let missing = StoragePath::root().join("missing").unwrap();
        assert!(matches!(
            storage.list(&missing).await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage.upload(&missing, "x", chunks(&["x"]), 6).await,
            Err(UploadError::Storage(StorageError::NotFound))
        ));
    }

    #[tokio::test]
    async fn storage_down_and_bad_credentials_are_unavailable() {
        let dav = Rclone::start().await;
        let wrong = Storage::new(
            dav.storage.base.clone(),
            Some("test".into()),
            Some("nope".into()),
            None,
        )
        .unwrap();
        assert!(matches!(
            wrong.list(&StoragePath::root()).await,
            Err(StorageError::Unavailable(_))
        ));
        let base = dav.storage.base.clone();
        drop(dav);
        let down = Storage::new(base, Some("test".into()), Some("test".into()), None).unwrap();
        assert!(matches!(
            down.check().await,
            Err(StorageError::Unavailable(_))
        ));
        assert!(matches!(
            down.get(&StoragePath::root().join("x").unwrap(), None)
                .await,
            Err(StorageError::Unavailable(_))
        ));
    }
}

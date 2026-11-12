//! Storage on a WebDAV server (e.g. `rclone serve webdav`). Only a small part of WebDAV is
//! used: see docs/adr/0001-file-storage-over-webdav.md for the method table.

use std::time::Duration;

use axum::body::{Body, Bytes};
use futures_util::{stream, Stream, StreamExt};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use quick_xml::{events::Event, Reader};
use reqwest::{header, Client, Method, RequestBuilder, Response, StatusCode, Url};
use tokio::sync::mpsc;

use super::{next_chunk, Blob, Meter, StorageError, StoragePath, UploadError, Uploaded};

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

/// Asks only for what `list_dirs` needs.
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:"><prop><resourcetype/></prop></propfind>"#;

pub struct WebDav {
    client: Client,
    /// Always ends in `/`, so paths can be appended.
    base: Url,
    auth: Option<(String, String)>,
}

impl WebDav {
    pub fn new(
        mut base: Url,
        user: Option<String>,
        password: Option<String>,
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
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            // Credentials go on every request; never send them somewhere else.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let auth = match (user, password) {
            (Some(u), p) => Some((u, p.unwrap_or_default())),
            (None, Some(_)) => return Err("FILES_PASSWORD is set without FILES_USER".into()),
            (None, None) => None,
        };
        Ok(WebDav { client, base, auth })
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

    pub async fn list_dirs(&self, dir: &StoragePath) -> Result<Vec<String>, StorageError> {
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
        Ok(child_dirs(&url, &entries))
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
        temp: StoragePath,
        mut source: S,
        max_bytes: u64,
    ) -> Result<Uploaded, UploadError<E>>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send,
    {
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

    pub async fn get(&self, path: &StoragePath) -> Result<Blob, StorageError> {
        let url = self.url(path, false);
        let resp = self
            .request(Method::GET, url.clone())
            .send()
            .await
            .map_err(transport)?;
        match resp.status() {
            StatusCode::OK => {}
            s => return Err(status_error(s, "GET", &url)),
        }
        let len = content_length(&resp).ok_or_else(|| {
            StorageError::Io(std::io::Error::other(format!(
                "GET {url}: no Content-Length"
            )))
        })?;
        let body = resp
            .bytes_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other));
        Ok(Blob {
            len,
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
}

/// Reads `<href>` and whether `<resourcetype>` holds `<collection/>`, per `<response>`.
/// Namespace prefixes vary between servers, so elements are matched by local name.
fn parse_multistatus(xml: &str) -> Result<Vec<Entry>, quick_xml::Error> {
    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    let mut in_href = false;
    let mut in_resourcetype = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"response" => {
                    current = Some(Entry {
                        href: String::new(),
                        collection: false,
                    })
                }
                b"href" => in_href = true,
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
            Event::End(e) => match e.local_name().as_ref() {
                b"response" => entries.extend(current.take()),
                b"href" => in_href = false,
                b"resourcetype" => in_resourcetype = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(entries)
}

/// Names of the folders directly inside `dir_url`. Hrefs can be absolute or relative and
/// encoded differently from how we asked, so they're compared as decoded segments.
fn child_dirs(dir_url: &Url, entries: &[Entry]) -> Vec<String> {
    let parent = decoded_segments(dir_url.path());
    entries
        .iter()
        .filter(|e| e.collection)
        .filter_map(|e| {
            let url = dir_url.join(e.href.trim()).ok()?;
            let mut segments = decoded_segments(url.path());
            if segments.len() != parent.len() + 1 || !segments.starts_with(&parent) {
                return None; // the folder itself, or something unexpected
            }
            segments.pop()
        })
        .collect()
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
        WebDav::new(Url::parse(s).unwrap(), None, None)
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
    fn config_is_checked() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(WebDav::new(url("ftp://x/"), None, None).is_err());
        assert!(WebDav::new(url("http://u:p@x/"), None, None).is_err());
        assert!(WebDav::new(url("http://x/"), None, Some("p".into())).is_err());
        assert!(WebDav::new(url("http://x/"), Some("u".into()), Some("p".into())).is_ok());
    }

    /// As `rclone serve webdav` v1.71.2 answered a `Depth: 1` PROPFIND (served from `/`).
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
    <D:propstat><D:prop><D:resourcetype/></D:prop>
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
        assert_eq!(child_dirs(&url, &entries), ["R&D", "Tom & Jerry"]);
    }

    #[test]
    fn propfind_reads_rclone_replies() {
        let b = base("http://127.0.0.1:8090/");
        let dir = StoragePath::root().join_all("orgs/acme/documents").unwrap();
        let mut names = child_dirs(
            &url_for(&b, &dir, true),
            &parse_multistatus(RCLONE).unwrap(),
        );
        names.sort();
        assert_eq!(names, [".hidden", "Brand & Logo", "Q1 #2 100%", "Ünïcode"]);
    }
}

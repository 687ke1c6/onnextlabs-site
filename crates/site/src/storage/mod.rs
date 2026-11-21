//! Where org files live: a WebDAV server at `FILES_URL` (docs/adr/0001-file-storage-over-webdav.md).
//! It's the source of truth for which files exist; the database only adds details to them.
//!
//! Paths are `StoragePath`s, relative to the storage root and checked segment by segment,
//! so nothing here can reach outside the root. What names are *allowed* (no dot-files,
//! depth limits, …) is the caller's policy, not this module's.
//!
//! Uploads are two steps so a file only appears under its real name once complete:
//! `upload` writes a hidden temporary file in the target folder, then `commit` moves it
//! to its name, never overwriting. Callers `delete` the temporary file if they give up.

mod webdav;

use std::{fmt, io};

use axum::body::{Body, Bytes};
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};

pub use webdav::Storage;

/// A file's bytes (or one range of them), ready to send.
pub struct Blob {
    /// The whole file's length.
    pub total: u64,
    /// First and last byte (inclusive) when only a range was read; `None` = all of it.
    pub range: Option<(u64, u64)>,
    pub body: Body,
}

impl Blob {
    /// How many bytes `body` holds.
    pub fn len(&self) -> u64 {
        match self.range {
            Some((first, last)) => last - first + 1,
            None => self.total,
        }
    }
}

/// One byte range from a `Range` header, before the file's length is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteRange {
    /// `bytes=a-b`, inclusive.
    FromTo(u64, u64),
    /// `bytes=a-`
    From(u64),
    /// `bytes=-n`: the last `n` bytes.
    Last(u64),
}

impl ByteRange {
    /// Parses a `Range` header. `None` for anything but a single, well-formed byte range:
    /// callers then send the whole file, which is always allowed (RFC 9110 §14.2).
    pub fn parse(header: &str) -> Option<Self> {
        let (unit, spec) = header.trim().split_once('=')?;
        if !unit.trim().eq_ignore_ascii_case("bytes") || spec.contains(',') {
            return None;
        }
        let (first, last) = spec.trim().split_once('-')?;
        let num = |s: &str| -> Option<u64> {
            (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .then(|| s.parse().ok())
                .flatten()
        };
        match (first.trim(), last.trim()) {
            ("", n) => Some(ByteRange::Last(num(n)?)),
            (a, "") => Some(ByteRange::From(num(a)?)),
            (a, b) => {
                let (a, b) = (num(a)?, num(b)?);
                (a <= b).then_some(ByteRange::FromTo(a, b))
            }
        }
    }

    /// As a `Range` header value, to pass on.
    fn to_header(self) -> String {
        match self {
            ByteRange::FromTo(a, b) => format!("bytes={a}-{b}"),
            ByteRange::From(a) => format!("bytes={a}-"),
            ByteRange::Last(n) => format!("bytes=-{n}"),
        }
    }
}

/// What's directly inside a folder. Names are as stored, hidden ones included.
#[derive(Debug, Default)]
pub struct Listing {
    pub dirs: Vec<String>,
    pub files: Vec<FileInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
}

/// A finished upload, still under its temporary name.
pub struct Uploaded {
    pub temp: StoragePath,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub enum StorageError {
    NotFound,
    /// `commit` target, or a folder in the way, already exists.
    AlreadyExists,
    /// The requested range starts past the end of the file (`total` bytes long).
    RangeNotSatisfiable {
        total: u64,
    },
    /// The storage host can't be reached or isn't answering properly.
    Unavailable(String),
    Io(io::Error),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::NotFound => f.write_str("not found"),
            StorageError::AlreadyExists => f.write_str("already exists"),
            StorageError::RangeNotSatisfiable { total } => {
                write!(f, "range not satisfiable ({total} bytes)")
            }
            StorageError::Unavailable(why) => write!(f, "storage unavailable: {why}"),
            StorageError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => StorageError::NotFound,
            io::ErrorKind::AlreadyExists => StorageError::AlreadyExists,
            _ => StorageError::Io(e),
        }
    }
}

/// Why an upload failed. `E` is the incoming body's error (e.g. a broken multipart stream).
#[derive(Debug)]
pub enum UploadError<E> {
    /// Over the size limit; nothing is kept.
    TooLarge,
    /// No bytes at all; nothing is kept.
    Empty,
    Source(E),
    Storage(StorageError),
}

impl<E> From<StorageError> for UploadError<E> {
    fn from(e: StorageError) -> Self {
        UploadError::Storage(e)
    }
}

/// `.<name>.partial-<random>`: hidden (dot-files are never listed as documents) and unique.
pub(crate) fn temp_name(name: &str) -> String {
    format!(".{name}.partial-{:016x}", rand::random::<u64>())
}

/// Counts and hashes an upload's bytes, enforcing the size limit.
struct Meter {
    size: u64,
    max: u64,
    hasher: Sha256,
}

impl Meter {
    fn new(max: u64) -> Self {
        Meter {
            size: 0,
            max,
            hasher: Sha256::new(),
        }
    }

    /// Accounts for one chunk; `false` once the total is over the limit.
    fn take(&mut self, chunk: &Bytes) -> bool {
        self.size += chunk.len() as u64;
        self.hasher.update(chunk);
        self.size <= self.max
    }

    fn finish<E>(self, temp: StoragePath) -> Result<Uploaded, UploadError<E>> {
        if self.size == 0 {
            return Err(UploadError::Empty);
        }
        Ok(Uploaded {
            temp,
            size: self.size,
            sha256: hex::encode(self.hasher.finalize()),
        })
    }
}

/// Reads the next chunk, metering it. `Ok(None)` at the end.
async fn next_chunk<S, E>(
    source: &mut S,
    meter: &mut Meter,
) -> Result<Option<Bytes>, UploadError<E>>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    match source.next().await {
        None => Ok(None),
        Some(Err(e)) => Err(UploadError::Source(e)),
        Some(Ok(chunk)) if meter.take(&chunk) => Ok(Some(chunk)),
        Some(Ok(_)) => Err(UploadError::TooLarge),
    }
}

/// A path relative to the storage root, as checked segments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoragePath(Vec<String>);

#[derive(Debug)]
pub struct InvalidPath;

impl StoragePath {
    pub fn root() -> Self {
        StoragePath(Vec::new())
    }

    /// One more segment. Rejects empty, `.`, `..`, separators and control characters.
    pub fn join(&self, segment: &str) -> Result<Self, InvalidPath> {
        let ok = !segment.is_empty()
            && segment != "."
            && segment != ".."
            && !segment
                .chars()
                .any(|c| c == '/' || c == '\\' || c.is_control());
        if !ok {
            return Err(InvalidPath);
        }
        let mut segments = self.0.clone();
        segments.push(segment.to_owned());
        Ok(StoragePath(segments))
    }

    /// Each `/`-separated part of `rel` in turn ("" adds nothing).
    pub fn join_all(&self, rel: &str) -> Result<Self, InvalidPath> {
        rel.split('/')
            .filter(|s| !s.is_empty())
            .try_fold(self.clone(), |p, s| p.join(s))
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for StoragePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}", self.0.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_reject_escapes() {
        let root = StoragePath::root();
        assert_eq!(
            root.join_all("orgs/acme/documents").unwrap().segments(),
            ["orgs", "acme", "documents"]
        );
        assert_eq!(root.join_all("").unwrap(), root);
        for bad in ["..", ".", "a\\b", "a\u{0}b", ""] {
            assert!(root.join(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(root.join_all("a/../b").is_err());
        // Dot-files are a caller policy, not a storage one (temporary uploads use them).
        assert!(root.join(".partial").is_ok());
        assert_eq!(root.join_all("a/b c").unwrap().to_string(), "/a/b c");
    }

    #[test]
    fn ranges_parse_like_rfc_9110() {
        use ByteRange::*;
        assert_eq!(ByteRange::parse("bytes=0-499"), Some(FromTo(0, 499)));
        assert_eq!(ByteRange::parse(" Bytes = 500- "), Some(From(500)));
        assert_eq!(ByteRange::parse("bytes=-500"), Some(Last(500)));
        // Ignored, so the whole file is sent: multiple ranges, other units, nonsense.
        for ignored in [
            "bytes=0-1,5-6",
            "items=0-1",
            "bytes=5-1",
            "bytes=-",
            "bytes=a-b",
            "bytes=+1-2",
            "bytes 0-1",
            "bytes=99999999999999999999-",
        ] {
            assert_eq!(ByteRange::parse(ignored), None, "{ignored:?}");
        }
    }

    #[test]
    fn ranges_pass_on_as_headers() {
        assert_eq!(ByteRange::FromTo(0, 499).to_header(), "bytes=0-499");
        assert_eq!(ByteRange::From(500).to_header(), "bytes=500-");
        assert_eq!(ByteRange::Last(3).to_header(), "bytes=-3");
    }

    #[test]
    fn temp_names_are_hidden_and_unique() {
        let a = temp_name("deck.pdf");
        assert!(a.starts_with(".deck.pdf.partial-"));
        assert_ne!(a, temp_name("deck.pdf"));
    }
}

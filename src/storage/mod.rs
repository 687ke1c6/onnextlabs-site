//! Where org files live. One interface over the backends chosen at startup:
//!
//! - `Local`: a directory on this machine (`$DATA_DIR`), the default.
//! - WebDAV (planned; see docs/adr/0001-file-storage-over-webdav.md).
//!
//! Paths are `StoragePath`s, relative to the storage root and checked segment by segment,
//! so nothing here can reach outside the root. What names are *allowed* (no dot-files,
//! depth limits, …) is the caller's policy, not this module's.
//!
//! Uploads are two steps so a file only appears under its real name once complete:
//! `upload` writes a hidden temporary file in the target folder, then `commit` moves it
//! to its name, never overwriting. Callers `delete` the temporary file if they give up.

mod local;

use std::{fmt, io, path::PathBuf};

use axum::body::{Body, Bytes};
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};

pub use local::Local;

pub enum Storage {
    Local(Local),
}

/// A file's bytes, ready to send.
pub struct Blob {
    pub len: u64,
    pub body: Body,
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
    /// The storage host can't be reached or isn't answering properly.
    #[allow(dead_code)] // raised by the WebDAV backend (plan step 2)
    Unavailable(String),
    Io(io::Error),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::NotFound => f.write_str("not found"),
            StorageError::AlreadyExists => f.write_str("already exists"),
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

impl Storage {
    /// Names of the folders directly inside `dir` (all of them, including hidden ones).
    pub async fn list_dirs(&self, dir: &StoragePath) -> Result<Vec<String>, StorageError> {
        match self {
            Storage::Local(s) => s.list_dirs(dir).await,
        }
    }

    /// Creates `dir` and any missing parents. Fine if it already exists.
    pub async fn make_dirs(&self, dir: &StoragePath) -> Result<(), StorageError> {
        match self {
            Storage::Local(s) => s.make_dirs(dir).await,
        }
    }

    /// Streams `source` to a hidden temporary file in `dir` (which must exist), enforcing
    /// `max_bytes` and hashing as it goes. On error nothing is left behind.
    pub async fn upload<S, E>(
        &self,
        dir: &StoragePath,
        name: &str,
        source: S,
        max_bytes: u64,
    ) -> Result<Uploaded, UploadError<E>>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin + Send,
    {
        let temp = dir
            .join(&temp_name(name))
            .map_err(|_| StorageError::Io(io::Error::other("bad temporary name")))?;
        match self {
            Storage::Local(s) => s.upload(temp, source, max_bytes).await,
        }
    }

    /// Moves a finished upload to `target`. `AlreadyExists` if the name is taken: nothing
    /// is overwritten, and the temporary file is left for the caller to retry or delete.
    pub async fn commit(
        &self,
        temp: &StoragePath,
        target: &StoragePath,
    ) -> Result<(), StorageError> {
        match self {
            Storage::Local(s) => s.commit(temp, target).await,
        }
    }

    pub async fn get(&self, path: &StoragePath) -> Result<Blob, StorageError> {
        match self {
            Storage::Local(s) => s.get(path).await,
        }
    }

    pub async fn delete(&self, path: &StoragePath) -> Result<(), StorageError> {
        match self {
            Storage::Local(s) => s.delete(path).await,
        }
    }

    /// For the startup log line.
    pub fn describe(&self) -> String {
        match self {
            Storage::Local(s) => format!("local directory {}", s.root().display()),
        }
    }
}

/// `.<name>.partial-<random>`: hidden (dot-files are never listed as documents) and unique.
fn temp_name(name: &str) -> String {
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

    #[cfg_attr(not(test), allow(dead_code))] // used by the WebDAV backend (plan step 2)
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// On a local filesystem, under `root`.
    fn to_local(&self, root: &std::path::Path) -> PathBuf {
        self.0.iter().fold(root.to_path_buf(), |p, s| p.join(s))
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
    fn temp_names_are_hidden_and_unique() {
        let a = temp_name("deck.pdf");
        assert!(a.starts_with(".deck.pdf.partial-"));
        assert_ne!(a, temp_name("deck.pdf"));
    }
}

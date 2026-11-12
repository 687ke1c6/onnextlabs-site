//! Storage in a directory on this machine.

use std::{
    io,
    path::{Path, PathBuf},
};

use axum::body::{Body, Bytes};
use futures_util::Stream;
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::io::ReaderStream;

use super::{next_chunk, Blob, Meter, StorageError, StoragePath, UploadError, Uploaded};

pub struct Local {
    root: PathBuf,
}

impl Local {
    /// Uses `root` (created if missing) as the storage root.
    pub fn new(root: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Local { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, p: &StoragePath) -> PathBuf {
        p.to_local(&self.root)
    }

    pub async fn list_dirs(&self, dir: &StoragePath) -> Result<Vec<String>, StorageError> {
        let mut entries = fs::read_dir(self.path(dir)).await?;
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            // Not followed: a symlink to a directory doesn't count as one.
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }

    pub async fn make_dirs(&self, dir: &StoragePath) -> Result<(), StorageError> {
        fs::create_dir_all(self.path(dir)).await?;
        Ok(())
    }

    pub async fn upload<S, E>(
        &self,
        temp: StoragePath,
        mut source: S,
        max_bytes: u64,
    ) -> Result<Uploaded, UploadError<E>>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
    {
        let path = self.path(&temp);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(StorageError::from)?;
        let mut meter = Meter::new(max_bytes);

        let written: Result<(), UploadError<E>> = async {
            while let Some(chunk) = next_chunk(&mut source, &mut meter).await? {
                file.write_all(&chunk).await.map_err(StorageError::from)?;
            }
            file.flush().await.map_err(StorageError::from)?;
            Ok(())
        }
        .await;
        drop(file);

        let result = written.and_then(|()| meter.finish(temp));
        if result.is_err() {
            let _ = fs::remove_file(&path).await;
        }
        result
    }

    /// Never overwrites: a hard link fails if `target` exists, so taking the name is
    /// atomic. Falls back to check-then-rename where hard links aren't supported.
    pub async fn commit(
        &self,
        temp: &StoragePath,
        target: &StoragePath,
    ) -> Result<(), StorageError> {
        let (from, to) = (self.path(temp), self.path(target));
        match fs::hard_link(&from, &to).await {
            Ok(()) => {
                fs::remove_file(&from).await?;
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(StorageError::AlreadyExists),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied
                ) =>
            {
                if fs::try_exists(&to).await? {
                    return Err(StorageError::AlreadyExists);
                }
                fs::rename(&from, &to).await?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    pub async fn get(&self, path: &StoragePath) -> Result<Blob, StorageError> {
        let file = fs::File::open(self.path(path)).await?;
        let meta = file.metadata().await?;
        if !meta.is_file() {
            return Err(StorageError::NotFound);
        }
        Ok(Blob {
            len: meta.len(),
            body: Body::from_stream(ReaderStream::new(file)),
        })
    }

    pub async fn delete(&self, path: &StoragePath) -> Result<(), StorageError> {
        fs::remove_file(self.path(path)).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use futures_util::stream;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "onnextlabs-storage-{tag}-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn chunks(parts: &[&'static str]) -> impl Stream<Item = Result<Bytes, io::Error>> + Unpin {
        stream::iter(
            parts
                .iter()
                .map(|p| Ok(Bytes::from_static(p.as_bytes())))
                .collect::<Vec<_>>(),
        )
    }

    #[tokio::test]
    async fn upload_commit_get_and_no_overwrite() {
        let root = temp_root("roundtrip");
        let storage = Storage::Local(Local::new(root.clone()).unwrap());
        let dir = StoragePath::root()
            .join_all("orgs/acme/documents/Brand")
            .unwrap();
        storage.make_dirs(&dir).await.unwrap();

        let up = storage
            .upload(&dir, "a.txt", chunks(&["hello ", "world"]), 100)
            .await
            .unwrap();
        assert_eq!(up.size, 11);
        assert_eq!(
            up.sha256,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        // Hidden until committed.
        let target = dir.join("a.txt").unwrap();
        assert!(matches!(
            storage.get(&target).await,
            Err(StorageError::NotFound)
        ));
        storage.commit(&up.temp, &target).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("orgs/acme/documents/Brand/a.txt")).unwrap(),
            "hello world"
        );
        assert_eq!(storage.get(&target).await.unwrap().len, 11);

        // A second upload can't take the same name, and is left for the caller.
        let second = storage
            .upload(&dir, "a.txt", chunks(&["other"]), 100)
            .await
            .unwrap();
        assert!(matches!(
            storage.commit(&second.temp, &target).await,
            Err(StorageError::AlreadyExists)
        ));
        assert_eq!(
            std::fs::read_to_string(root.join("orgs/acme/documents/Brand/a.txt")).unwrap(),
            "hello world"
        );
        storage.delete(&second.temp).await.unwrap();

        // Only the folder itself shows up as a subfolder of documents.
        let docs = StoragePath::root().join_all("orgs/acme/documents").unwrap();
        assert_eq!(storage.list_dirs(&docs).await.unwrap(), ["Brand"]);
        assert_eq!(
            std::fs::read_dir(root.join("orgs/acme/documents/Brand"))
                .unwrap()
                .count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn failed_uploads_leave_nothing() {
        let root = temp_root("failed");
        let storage = Storage::Local(Local::new(root.clone()).unwrap());
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
            Err(io::Error::other("connection reset")),
        ]);
        let cut_off = storage.upload(&dir, "cut.bin", broken, 100).await;
        assert!(matches!(cut_off, Err(UploadError::Source(_))));

        assert_eq!(std::fs::read_dir(root.join("docs")).unwrap().count(), 0);
        assert!(matches!(
            storage
                .list_dirs(&StoragePath::root().join("missing").unwrap())
                .await,
            Err(StorageError::NotFound)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}

//! Blob stores (`docs/SPEC.md` §6.4, §8.1).
//!
//! The crate reads and writes the disk store in the Reference's layout
//! (`disk-blobstore.ts` at the pin): a blob is at `<root>/<did>/<cid>`, an
//! upload waits at `<tmp>/<did>/<key>` until a record references it, and
//! `<tmp>` is `<root>/tempt` unless it is configured. The S3-compatible
//! store is in [`s3`]. [`BlobStore`] is the one that is configured.

pub mod s3;
pub mod sniff;

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};

use cid::Cid;
use sha2::{Digest as _, Sha256};

pub use s3::{Clock, S3BlobStore, S3Config};
pub use sniff::sniff;

/// Reading a blob failed.
#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    /// The store does not hold the blob.
    #[error("blob not found")]
    NotFound,
    /// The name is not one the store could have written.
    #[error("not a blob name: {0}")]
    BadName(String),
    /// The file system reported an error.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// What the file system said.
        source: io::Error,
    },
    /// The S3-compatible store refused a request, or did not answer.
    #[error("{object}: {message}")]
    Remote {
        /// The object, or the endpoint.
        object: String,
        /// What the store said.
        message: String,
    },
}

/// The disk blob store: `<root>/<did>/<cid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskBlobStore {
    root: PathBuf,
    tmp: PathBuf,
}

/// An upload that waits in the staging directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedBlob {
    /// The name of the staged file.
    pub key: String,
    /// The number of bytes.
    pub size: u64,
    /// The CID of the bytes: `raw` with SHA-256.
    pub cid: Cid,
    /// The media type the content shows, when [`sniff`] knows it.
    pub sniffed: Option<&'static str>,
}

/// Syncs a directory, so that a file created or renamed in it survives a
/// power loss (SPEC §8.3).
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Where the bytes of an upload go.
#[derive(Debug)]
enum Sink {
    Disk { path: PathBuf, file: Option<File> },
    S3(Box<s3::Upload>),
}

/// An upload in progress. The staged bytes are removed when the value is
/// dropped without [`Staging::finish`].
#[derive(Debug)]
pub struct Staging {
    key: String,
    sink: Sink,
    hasher: Sha256,
    size: u64,
    head: Vec<u8>,
}

impl Staging {
    fn new(key: &str, sink: Sink) -> Self {
        Self {
            key: key.to_owned(),
            sink,
            hasher: Sha256::new(),
            size: 0,
            head: Vec::with_capacity(sniff::SNIFF_LEN),
        }
    }

    /// Appends bytes of the upload.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), BlobError> {
        match &mut self.sink {
            Sink::Disk { file: None, .. } => return Ok(()),
            Sink::Disk {
                path,
                file: Some(file),
            } => file.write_all(bytes).map_err(|source| BlobError::Io {
                path: path.clone(),
                source,
            })?,
            Sink::S3(upload) => upload.write(bytes)?,
        }
        self.hasher.update(bytes);
        self.size += bytes.len() as u64;
        if self.head.len() < sniff::SNIFF_LEN {
            let room = sniff::SNIFF_LEN - self.head.len();
            self.head.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
        Ok(())
    }

    /// The number of bytes written.
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Makes the staged bytes durable and describes them.
    pub fn finish(mut self) -> Result<StagedBlob, BlobError> {
        match &mut self.sink {
            Sink::Disk { path, file } => {
                let io = |source| BlobError::Io {
                    path: path.clone(),
                    source,
                };
                if let Some(file) = file.take() {
                    file.sync_all().map_err(io)?;
                }
                if let Some(dir) = path.parent() {
                    sync_dir(dir).map_err(io)?;
                }
            }
            Sink::S3(upload) => upload.finish()?,
        }
        let digest: [u8; 32] = std::mem::take(&mut self.hasher).finalize().into();
        Ok(StagedBlob {
            key: std::mem::take(&mut self.key),
            size: self.size,
            cid: pds_types::content::cid_for_raw_digest(&digest),
            sniffed: sniff(&self.head),
        })
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // An upload to the S3-compatible store removes its own parts.
        if let Sink::Disk { path, file } = &mut self.sink
            && file.take().is_some()
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A DID and a CID are path components here, so neither may leave the
/// directory.
fn check_component(name: &str) -> Result<(), BlobError> {
    let plain =
        !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0']);
    if plain {
        Ok(())
    } else {
        Err(BlobError::BadName(name.to_owned()))
    }
}

impl DiskBlobStore {
    /// The store under `root` (`PDS_BLOBSTORE_DISK_LOCATION`), staging in
    /// `<root>/tempt`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let tmp = root.join("tempt");
        Self { root, tmp }
    }

    /// The same store, staging in `tmp`
    /// (`PDS_BLOBSTORE_DISK_TMP_LOCATION`).
    #[must_use]
    pub fn with_tmp(mut self, tmp: impl Into<PathBuf>) -> Self {
        self.tmp = tmp.into();
        self
    }

    fn tmp_path(&self, did: &str, key: &str) -> Result<PathBuf, BlobError> {
        check_component(did)?;
        check_component(key)?;
        Ok(self.tmp.join(did).join(key))
    }

    /// Starts an upload for `did`. `key` names the staged file: 32
    /// characters that the caller draws at random.
    pub fn stage(&self, did: &str, key: &str) -> Result<Staging, BlobError> {
        let path = self.tmp_path(did, key)?;
        let io = |source| BlobError::Io {
            path: path.clone(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io)?;
        Ok(Staging::new(
            key,
            Sink::Disk {
                path,
                file: Some(file),
            },
        ))
    }

    /// Moves a staged upload to its place as the blob `cid`. When the store
    /// already holds the blob, the staged file is removed. A staged file
    /// that is gone is no error when the blob is in place: an earlier call
    /// moved it.
    pub fn make_permanent(&self, did: &str, key: &str, cid: &str) -> Result<(), BlobError> {
        let staged = self.tmp_path(did, key)?;
        let stored = self.path(did, cid)?;
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| BlobError::Io { path, source }
        };
        if self.has(did, cid)? {
            match std::fs::remove_file(&staged) {
                Err(source) if source.kind() != io::ErrorKind::NotFound => {
                    return Err(io(&staged)(source));
                }
                _ => return Ok(()),
            }
        }
        let dir = self.root.join(did);
        std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        if std::fs::rename(&staged, &stored).is_err() {
            // The staging directory may be on another file system.
            let partial = dir.join(format!(".{key}.partial"));
            std::fs::copy(&staged, &partial).map_err(io(&staged))?;
            File::open(&partial)
                .and_then(|file| file.sync_all())
                .map_err(io(&partial))?;
            std::fs::rename(&partial, &stored).map_err(io(&stored))?;
            let _ = std::fs::remove_file(&staged);
        }
        sync_dir(&dir).map_err(io(&dir))
    }

    /// Removes a stored blob. A blob that is not there is no error.
    pub fn delete(&self, did: &str, cid: &str) -> Result<(), BlobError> {
        let path = self.path(did, cid)?;
        match std::fs::remove_file(&path) {
            Err(source) if source.kind() != io::ErrorKind::NotFound => {
                Err(BlobError::Io { path, source })
            }
            _ => Ok(()),
        }
    }

    /// Removes a staged upload. One that is not there is no error.
    pub fn delete_staged(&self, did: &str, key: &str) -> Result<(), BlobError> {
        let path = self.tmp_path(did, key)?;
        match std::fs::remove_file(&path) {
            Err(source) if source.kind() != io::ErrorKind::NotFound => {
                Err(BlobError::Io { path, source })
            }
            _ => Ok(()),
        }
    }

    /// Where the blob `cid` of `did` is stored.
    fn path(&self, did: &str, cid: &str) -> Result<PathBuf, BlobError> {
        check_component(did)?;
        check_component(cid)?;
        Ok(self.root.join(did).join(cid))
    }

    /// Opens a stored blob for reading.
    pub fn open(&self, did: &str, cid: &str) -> Result<File, BlobError> {
        let path = self.path(did, cid)?;
        File::open(&path).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => BlobError::NotFound,
            _ => BlobError::Io { path, source },
        })
    }

    /// Whether the store holds the blob.
    pub fn has(&self, did: &str, cid: &str) -> Result<bool, BlobError> {
        let path = self.path(did, cid)?;
        match std::fs::metadata(&path) {
            Ok(meta) => Ok(meta.is_file()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(BlobError::Io { path, source }),
        }
    }
}

/// The blob store that the configuration names.
///
/// Every call blocks. A call on the S3-compatible store waits for the
/// network and must run on the blocking pool.
#[derive(Debug, Clone)]
pub enum BlobStore {
    /// `PDS_BLOBSTORE_DISK_LOCATION`.
    Disk(DiskBlobStore),
    /// `PDS_BLOBSTORE_S3_BUCKET`.
    S3(S3BlobStore),
}

impl From<DiskBlobStore> for BlobStore {
    fn from(store: DiskBlobStore) -> Self {
        Self::Disk(store)
    }
}

impl From<S3BlobStore> for BlobStore {
    fn from(store: S3BlobStore) -> Self {
        Self::S3(store)
    }
}

impl BlobStore {
    /// Starts an upload for `did`. `key` names the staged bytes: 32
    /// characters that the caller draws at random.
    pub fn stage(&self, did: &str, key: &str) -> Result<Staging, BlobError> {
        match self {
            Self::Disk(store) => store.stage(did, key),
            Self::S3(store) => {
                check_component(did)?;
                check_component(key)?;
                Ok(Staging::new(key, Sink::S3(Box::new(store.stage(did, key)))))
            }
        }
    }

    /// Moves a staged upload to its place as the blob `cid`. The call is
    /// safe to repeat.
    pub fn make_permanent(&self, did: &str, key: &str, cid: &str) -> Result<(), BlobError> {
        match self {
            Self::Disk(store) => store.make_permanent(did, key, cid),
            Self::S3(store) => {
                check_component(did)?;
                check_component(key)?;
                check_component(cid)?;
                store.make_permanent(did, key, cid)
            }
        }
    }

    /// Removes a stored blob. A blob that is not there is no error.
    pub fn delete(&self, did: &str, cid: &str) -> Result<(), BlobError> {
        match self {
            Self::Disk(store) => store.delete(did, cid),
            Self::S3(store) => {
                check_component(did)?;
                check_component(cid)?;
                store.delete(did, cid)
            }
        }
    }

    /// Removes a staged upload. One that is not there is no error.
    pub fn delete_staged(&self, did: &str, key: &str) -> Result<(), BlobError> {
        match self {
            Self::Disk(store) => store.delete_staged(did, key),
            Self::S3(store) => {
                check_component(did)?;
                check_component(key)?;
                store.delete_staged(did, key)
            }
        }
    }

    /// Opens a stored blob for reading.
    pub fn open(&self, did: &str, cid: &str) -> Result<Box<dyn Read + Send>, BlobError> {
        match self {
            Self::Disk(store) => Ok(Box::new(store.open(did, cid)?)),
            Self::S3(store) => {
                check_component(did)?;
                check_component(cid)?;
                Ok(Box::new(store.open(did, cid)?))
            }
        }
    }

    /// Whether the store holds the blob.
    pub fn has(&self, did: &str, cid: &str) -> Result<bool, BlobError> {
        match self {
            Self::Disk(store) => store.has(did, cid),
            Self::S3(store) => {
                check_component(did)?;
                check_component(cid)?;
                store.has(did, cid)
            }
        }
    }

    /// The CID of every blob the store holds for `did`, when the store
    /// answers that in fewer requests than one [`BlobStore::has`] for each
    /// blob. The disk store answers `None`.
    pub fn stored(&self, did: &str) -> Result<Option<BTreeSet<String>>, BlobError> {
        match self {
            Self::Disk(_) => Ok(None),
            Self::S3(store) => {
                check_component(did)?;
                store.stored(did).map(Some)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn a_name_cannot_leave_the_store() {
        let store = DiskBlobStore::new("/blobs");
        assert_eq!(
            store.path("did:plc:abc", "bafkrei").unwrap(),
            Path::new("/blobs/did:plc:abc/bafkrei")
        );
        assert!(store.path("..", "bafkrei").is_err());
        assert!(store.path("did:plc:abc", "../x").is_err());
        assert!(store.path("did:plc:abc", "").is_err());
        assert!(store.stage("did:plc:abc", "../key").is_err());
    }

    #[test]
    fn an_upload_is_staged_hashed_and_made_permanent() {
        let root = std::env::temp_dir().join(format!("pds-blob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = DiskBlobStore::new(&root);
        let did = "did:plc:abc";

        let mut staging = store.stage(did, "key1").unwrap();
        staging.write(b"a text ").unwrap();
        staging.write(b"blob").unwrap();
        let staged = staging.finish().unwrap();
        assert_eq!(staged.size, 11);
        assert_eq!(
            staged.cid.to_string(),
            "bafkreib5njcqaneu6gbwsmfbqeswb654woqznfim3pjbn7klh3alxjsrhm"
        );
        assert_eq!(staged.sniffed, None);
        let cid = staged.cid.to_string();
        assert!(!store.has(did, &cid).unwrap());

        store.make_permanent(did, "key1", &cid).unwrap();
        assert!(store.has(did, &cid).unwrap());
        assert!(!root.join("tempt").join(did).join("key1").exists());
        // A second call finds the blob in place.
        store.make_permanent(did, "key1", &cid).unwrap();

        // An upload that is dropped leaves no file.
        let mut dropped = store.stage(did, "key2").unwrap();
        dropped.write(b"x").unwrap();
        drop(dropped);
        assert!(!root.join("tempt").join(did).join("key2").exists());

        store.delete(did, &cid).unwrap();
        store.delete(did, &cid).unwrap();
        assert!(!store.has(did, &cid).unwrap());
        std::fs::remove_dir_all(&root).unwrap();
    }
}

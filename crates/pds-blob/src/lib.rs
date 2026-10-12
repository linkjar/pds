//! Blob stores (`docs/SPEC.md` §6.4, §8.1).
//!
//! This unit reads the disk store a Reference server wrote. Uploads,
//! staging, quarantine and the S3-compatible store arrive with the write
//! path (`docs/plan.md`, unit 4).

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

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
}

/// The disk blob store: `<root>/<did>/<cid>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskBlobStore {
    root: PathBuf,
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
    /// The store under `root` (`PDS_BLOBSTORE_DISK_LOCATION`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the blob `cid` of `did` is stored.
    pub fn path(&self, did: &str, cid: &str) -> Result<PathBuf, BlobError> {
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

#[cfg(test)]
mod tests {
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
    }
}

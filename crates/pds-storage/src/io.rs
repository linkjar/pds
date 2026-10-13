//! The seam below SQLite (`docs/SPEC.md` §19.2).
//!
//! Every database of the data directory is opened here. Production opens
//! it with the operating system's files. The simulation registers a
//! virtual file system that fails and loses writes, and names it with
//! [`use_vfs`]; from then on every database of the process goes through
//! it.

use std::path::Path;
use std::sync::RwLock;

use rusqlite::{Connection, OpenFlags};

static VFS: RwLock<Option<String>> = RwLock::new(None);

/// Opens every database of this process through the SQLite VFS `name`, or
/// through the default one with `None`. The VFS must be registered.
pub fn use_vfs(name: Option<&str>) {
    let mut vfs = VFS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *vfs = name.map(str::to_owned);
}

/// Opens the database at `path`.
pub(crate) fn open(path: &Path, flags: OpenFlags) -> rusqlite::Result<Connection> {
    let vfs = VFS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match vfs.as_deref() {
        Some(name) => Connection::open_with_flags_and_vfs(path, flags, name),
        None => Connection::open_with_flags(path, flags),
    }
}

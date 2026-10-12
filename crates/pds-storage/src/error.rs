//! The error of every storage call.

use std::path::PathBuf;

/// A storage call failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// SQLite reported an error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The file system reported an error.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the file system said.
        source: std::io::Error,
    },
    /// A row holds a value that the schema does not allow.
    #[error("table {table}: {reason}")]
    Corrupt {
        /// The table.
        table: &'static str,
        /// What is wrong with the row.
        reason: String,
    },
}

impl StorageError {
    pub(crate) fn corrupt(table: &'static str, reason: impl Into<String>) -> Self {
        Self::Corrupt {
            table,
            reason: reason.into(),
        }
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

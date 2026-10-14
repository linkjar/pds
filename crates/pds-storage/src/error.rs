//! The error of every storage call.

use std::path::PathBuf;

/// A storage call failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// SQLite reported an error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Postgres reported an error.
    #[error("{0}")]
    Postgres(#[from] crate::pg::PgError),
    /// A column of a result holds another type than the code reads.
    #[error("column {index}: expected {expected}")]
    Column {
        /// The column, from 0.
        index: usize,
        /// What the code reads.
        expected: &'static str,
    },
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

impl From<crate::sql::Error> for StorageError {
    fn from(error: crate::sql::Error) -> Self {
        use crate::sql::Error;
        match error {
            Error::Sqlite(error) => Self::Sqlite(error),
            Error::Postgres(error) => Self::Postgres(error),
            Error::NoRows => Self::Sqlite(rusqlite::Error::QueryReturnedNoRows),
            Error::Type { index, expected } => Self::Column { index, expected },
        }
    }
}

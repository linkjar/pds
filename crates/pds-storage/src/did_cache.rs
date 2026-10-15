//! `did_cache.sqlite`: DID documents with the time they were stored.

use std::path::Path;

use crate::sql::{Connection, OptionalExtension, named_params};

use crate::error::StorageError;

/// A cached DID document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedDoc {
    /// The document as JSON text.
    pub doc: String,
    /// When it was stored, in milliseconds since the Unix epoch.
    pub updated_at_ms: i64,
}

/// A connection to `did_cache.sqlite`. The cache is the one database a
/// read-only server writes.
#[derive(Debug)]
pub struct DidCacheDb {
    conn: Connection,
}

impl DidCacheDb {
    /// The cache over a connection of the shared tier.
    pub(crate) const fn over(conn: Connection) -> Self {
        Self { conn }
    }

    /// Opens the cache the Reference created. It is not created here: the
    /// schema comes from the `migrate` command.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: crate::layout::open_read_write(path)?.into(),
        })
    }

    /// The cached document of a DID.
    pub fn get(&self, did: &str) -> Result<Option<CachedDoc>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "doc", "updatedAt" from "did_doc" where "did" = :did"#,
                named_params! { ":did": did },
                |row| {
                    Ok(CachedDoc {
                        doc: row.get(0)?,
                        updated_at_ms: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Stores a document, replacing an older one.
    pub fn put(&self, did: &str, doc: &str, now_ms: i64) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "did_doc" ("did", "doc", "updatedAt") values (:did, :doc, :now)
               on conflict ("did") do update set "doc" = excluded."doc", "updatedAt" = excluded."updatedAt""#,
            named_params! { ":did": did, ":doc": doc, ":now": now_ms },
        )?;
        Ok(())
    }

    /// Removes a document.
    pub fn clear(&self, did: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"delete from "did_doc" where "did" = :did"#,
            named_params! { ":did": did },
        )?;
        Ok(())
    }
}

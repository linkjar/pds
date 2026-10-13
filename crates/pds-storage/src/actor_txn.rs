//! The write side of one actor's `store.sqlite`.
//!
//! Every statement is the Reference's (`actor-store/` at the pin), so that
//! either server can serve what the other wrote (`docs/SPEC.md` §8.2).

use std::ops::Deref;
use std::path::Path;

use cid::Cid;
use pds_repo::BlockMap;
use rusqlite::{OptionalExtension, named_params, params_from_iter};

use crate::actor::ActorDb;
use crate::error::StorageError;
use crate::layout::open_read_write;

/// The most values one statement binds. SQLite allows 32,766.
const BIND_BATCH: usize = 500;

/// One row of `backlink`: a record that points at a subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    /// The record that holds the link.
    pub uri: String,
    /// Where in the record: `subject` or `subject.uri`.
    pub path: &'static str,
    /// The DID or AT URI the record points at.
    pub link_to: String,
}

/// A row of `blob` as the commit path reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    /// Where the bytes wait until a record references the blob. `None` once
    /// the blob is permanent.
    pub temp_key: Option<String>,
    /// The size in bytes.
    pub size: i64,
    /// The stored media type.
    pub mime_type: String,
}

/// An upload that no record has referenced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUpload {
    /// The blob's CID.
    pub cid: String,
    /// Where its bytes wait.
    pub temp_key: String,
}

/// A referenced blob that the index lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingBlob {
    /// The blob's CID.
    pub cid: String,
    /// A record that references it.
    pub record_uri: String,
}

/// An open write transaction on an actor store. It rolls back when dropped
/// without [`ActorTxn::commit`].
#[derive(Debug)]
pub struct ActorTxn<'a> {
    db: &'a ActorDb,
    open: bool,
}

impl Deref for ActorTxn<'_> {
    type Target = ActorDb;

    fn deref(&self) -> &ActorDb {
        self.db
    }
}

impl Drop for ActorTxn<'_> {
    fn drop(&mut self) {
        if self.open {
            // The connection rolls back by itself when it closes; this
            // frees the write lock sooner.
            let _ = self.db.conn().execute_batch("rollback");
        }
    }
}

fn placeholders(count: usize) -> String {
    let mut text = String::with_capacity(count * 2);
    for index in 0..count {
        if index > 0 {
            text.push(',');
        }
        text.push('?');
    }
    text
}

impl ActorDb {
    /// Opens the store for reading and writing.
    pub fn open_writable(path: &Path) -> Result<Self, StorageError> {
        Ok(Self::from_connection(open_read_write(path)?))
    }

    /// Starts a write transaction. It takes the write lock at once, so two
    /// writers never both read the same root.
    pub fn begin(&mut self) -> Result<ActorTxn<'_>, StorageError> {
        self.conn().execute_batch("begin immediate")?;
        Ok(ActorTxn {
            db: self,
            open: true,
        })
    }

    /// Takes a record down with the reference `takedown`, or lifts the
    /// takedown with `None`.
    pub fn set_record_takedown(
        &self,
        uri: &str,
        takedown: Option<&str>,
    ) -> Result<(), StorageError> {
        self.conn().execute(
            r#"update "record" set "takedownRef" = ?2 where "uri" = ?1"#,
            rusqlite::params![uri, takedown],
        )?;
        Ok(())
    }

    /// Takes a blob down with the reference `takedown`, or lifts the
    /// takedown with `None`.
    pub fn set_blob_takedown(&self, cid: &str, takedown: Option<&str>) -> Result<(), StorageError> {
        self.conn().execute(
            r#"update "blob" set "takedownRef" = ?2 where "cid" = ?1"#,
            rusqlite::params![cid, takedown],
        )?;
        Ok(())
    }

    /// The CID of the record at `uri`, taken down or not.
    pub fn record_cid(&self, uri: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn()
            .query_row(
                r#"select "cid" from "record" where "uri" = :uri"#,
                named_params! { ":uri": uri },
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The record keys of the records in `collection` that hold a link to
    /// `link_to` at `path`.
    pub fn backlink_rkeys(
        &self,
        collection: &str,
        path: &str,
        link_to: &str,
    ) -> Result<Vec<String>, StorageError> {
        let mut statement = self.conn().prepare(
            r#"select record."rkey" from "record" inner join "backlink" on backlink."uri" = record."uri" where backlink."path" = :path and backlink."linkTo" = :linkTo and record."collection" = :collection"#,
        )?;
        let rows = statement.query_map(
            named_params! { ":path": path, ":linkTo": link_to, ":collection": collection },
            |row| row.get(0),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The row of a blob that is not taken down.
    pub fn stored_blob(&self, cid: &str) -> Result<Option<StoredBlob>, StorageError> {
        Ok(self
            .conn()
            .query_row(
                r#"select "tempKey", "size", "mimeType" from "blob" where "cid" = :cid and "takedownRef" is null"#,
                named_params! { ":cid": cid },
                |row| {
                    Ok(StoredBlob {
                        temp_key: row.get(0)?,
                        size: row.get(1)?,
                        mime_type: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Whether a record references the blob.
    pub fn has_records_for_blob(&self, cid: &str) -> Result<bool, StorageError> {
        Ok(self
            .conn()
            .query_row(
                r#"select 1 from "record_blob" where "blobCid" = :cid limit 1"#,
                named_params! { ":cid": cid },
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// A page of the blobs that records reference and the index lacks, in
    /// CID order.
    pub fn list_missing_blobs(
        &self,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MissingBlob>, StorageError> {
        let mut sql = String::from(
            r#"select "blobCid", "recordUri" from "record_blob" where not exists (select 1 from "blob" where blob."cid" = record_blob."blobCid")"#,
        );
        if cursor.is_some() {
            sql.push_str(r#" and "blobCid" > :cursor"#);
        }
        sql.push_str(r#" group by "blobCid" order by "blobCid" asc limit :limit"#);
        let mut statement = self.conn().prepare(&sql)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(MissingBlob {
                cid: row.get(0)?,
                record_uri: row.get(1)?,
            })
        };
        let rows = match cursor {
            Some(cursor) => {
                statement.query_map(named_params! { ":cursor": cursor, ":limit": limit }, map)?
            }
            None => statement.query_map(named_params! { ":limit": limit }, map)?,
        };
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The uploads that arrived before `before` (an ISO 8601 time) and that
    /// no record has referenced.
    pub fn pending_uploads(&self, before: &str) -> Result<Vec<PendingUpload>, StorageError> {
        let mut statement = self.conn().prepare(
            r#"select "cid", "tempKey" from "blob" where "tempKey" is not null and "createdAt" < :before and not exists (select 1 from "record_blob" where record_blob."blobCid" = blob."cid")"#,
        )?;
        let rows = statement.query_map(named_params! { ":before": before }, |row| {
            Ok(PendingUpload {
                cid: row.get(0)?,
                temp_key: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

impl ActorTxn<'_> {
    /// Makes the writes of the transaction durable.
    pub fn commit(mut self) -> Result<(), StorageError> {
        self.db.conn().execute_batch("commit")?;
        self.open = false;
        Ok(())
    }

    /// Sets the latest commit. With `create` the store has none yet.
    pub fn update_root(
        &self,
        did: &str,
        cid: &Cid,
        rev: &str,
        now: &str,
        create: bool,
    ) -> Result<(), StorageError> {
        let cid = cid.to_string();
        if create {
            self.conn().execute(
                r#"insert into "repo_root" ("did", "cid", "rev", "indexedAt") values (:did, :cid, :rev, :now)"#,
                named_params! { ":did": did, ":cid": cid, ":rev": rev, ":now": now },
            )?;
        } else {
            self.conn().execute(
                r#"update "repo_root" set "cid" = :cid, "rev" = :rev, "indexedAt" = :now"#,
                named_params! { ":cid": cid, ":rev": rev, ":now": now },
            )?;
        }
        Ok(())
    }

    /// Stores blocks under revision `rev`. A block the store holds keeps
    /// its revision.
    pub fn put_blocks(&self, blocks: &BlockMap, rev: &str) -> Result<(), StorageError> {
        let mut statement = self.conn().prepare_cached(
            r#"insert into "repo_block" ("cid", "repoRev", "size", "content") values (?1, ?2, ?3, ?4) on conflict do nothing"#,
        )?;
        for (cid, bytes) in blocks.iter() {
            let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
            statement.execute(rusqlite::params![cid.to_string(), rev, size, bytes])?;
        }
        Ok(())
    }

    /// Removes blocks.
    pub fn delete_blocks(&self, cids: &[Cid]) -> Result<(), StorageError> {
        for batch in cids.chunks(BIND_BATCH) {
            let sql = format!(
                r#"delete from "repo_block" where "cid" in ({})"#,
                placeholders(batch.len())
            );
            self.conn()
                .execute(&sql, params_from_iter(batch.iter().map(Cid::to_string)))?;
        }
        Ok(())
    }

    /// Of `cids`, those that a record outside `touched_uris` still holds.
    pub fn duplicate_record_cids(
        &self,
        cids: &[Cid],
        touched_uris: &[String],
    ) -> Result<Vec<String>, StorageError> {
        if cids.is_empty() || touched_uris.is_empty() {
            return Ok(Vec::new());
        }
        let mut found = Vec::new();
        // The URIs of one commit are at most 200; the CIDs are batched.
        for batch in cids.chunks(BIND_BATCH) {
            let sql = format!(
                r#"select "cid" from "record" where "cid" in ({}) and "uri" not in ({})"#,
                placeholders(batch.len()),
                placeholders(touched_uris.len())
            );
            let mut statement = self.conn().prepare(&sql)?;
            let values = batch
                .iter()
                .map(Cid::to_string)
                .chain(touched_uris.iter().cloned());
            let rows = statement.query_map(params_from_iter(values), |row| row.get(0))?;
            for row in rows {
                found.push(row?);
            }
        }
        Ok(found)
    }

    /// Writes the index row of a record: the current version at `uri`.
    pub fn index_record(
        &self,
        uri: &str,
        cid: &str,
        collection: &str,
        rkey: &str,
        rev: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        self.conn().prepare_cached(
            r#"insert into "record" ("uri", "cid", "collection", "rkey", "repoRev", "indexedAt") values (:uri, :cid, :collection, :rkey, :rev, :now) on conflict ("uri") do update set "cid" = :cid, "repoRev" = :rev, "indexedAt" = :now"#,
        )?
        .execute(named_params! {
            ":uri": uri, ":cid": cid, ":collection": collection, ":rkey": rkey, ":rev": rev, ":now": now,
        })?;
        Ok(())
    }

    /// Removes the index row of a record and its backlinks.
    pub fn delete_record(&self, uri: &str) -> Result<(), StorageError> {
        self.conn().execute(
            r#"delete from "record" where "uri" = :uri"#,
            named_params! { ":uri": uri },
        )?;
        self.remove_backlinks(uri)
    }

    /// Removes the backlinks of a record.
    pub fn remove_backlinks(&self, uri: &str) -> Result<(), StorageError> {
        self.conn().execute(
            r#"delete from "backlink" where "uri" = :uri"#,
            named_params! { ":uri": uri },
        )?;
        Ok(())
    }

    /// Adds backlinks. One that exists stays.
    pub fn add_backlinks(&self, backlinks: &[Backlink]) -> Result<(), StorageError> {
        for link in backlinks {
            self.conn().execute(
                r#"insert into "backlink" ("uri", "path", "linkTo") values (:uri, :path, :linkTo) on conflict do nothing"#,
                named_params! { ":uri": link.uri, ":path": link.path, ":linkTo": link.link_to },
            )?;
        }
        Ok(())
    }

    /// Removes the blob references of the records at `uris` and returns the
    /// CIDs they held.
    pub fn delete_record_blobs(&self, uris: &[String]) -> Result<Vec<String>, StorageError> {
        let mut removed = Vec::new();
        for batch in uris.chunks(BIND_BATCH) {
            let sql = format!(
                r#"delete from "record_blob" where "recordUri" in ({}) returning "blobCid""#,
                placeholders(batch.len())
            );
            let mut statement = self.conn().prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(batch.iter()), |row| row.get(0))?;
            for row in rows {
                removed.push(row?);
            }
        }
        Ok(removed)
    }

    /// Of `cids`, those that a record still references.
    pub fn referenced_blobs(&self, cids: &[String]) -> Result<Vec<String>, StorageError> {
        let mut found = Vec::new();
        for batch in cids.chunks(BIND_BATCH) {
            let sql = format!(
                r#"select "blobCid" from "record_blob" where "blobCid" in ({})"#,
                placeholders(batch.len())
            );
            let mut statement = self.conn().prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(batch.iter()), |row| row.get(0))?;
            for row in rows {
                found.push(row?);
            }
        }
        Ok(found)
    }

    /// Removes rows of the blob index.
    pub fn delete_blobs(&self, cids: &[String]) -> Result<(), StorageError> {
        for batch in cids.chunks(BIND_BATCH) {
            let sql = format!(
                r#"delete from "blob" where "cid" in ({})"#,
                placeholders(batch.len())
            );
            self.conn().execute(&sql, params_from_iter(batch.iter()))?;
        }
        Ok(())
    }

    /// Records that the record at `uri` references a blob.
    pub fn associate_blob(&self, cid: &str, uri: &str) -> Result<(), StorageError> {
        self.conn().prepare_cached(
            r#"insert into "record_blob" ("blobCid", "recordUri") values (:cid, :uri) on conflict do nothing"#,
        )?
        .execute(named_params! { ":cid": cid, ":uri": uri })?;
        Ok(())
    }

    /// Marks the blob with `temp_key` as permanent.
    pub fn clear_temp_key(&self, temp_key: &str) -> Result<(), StorageError> {
        self.conn().execute(
            r#"update "blob" set "tempKey" = null where "tempKey" = :key"#,
            named_params! { ":key": temp_key },
        )?;
        Ok(())
    }

    /// Whether the index holds the blob as taken down.
    pub fn blob_is_taken_down(&self, cid: &str) -> Result<bool, StorageError> {
        Ok(self.blob_takedown(cid)? == Some(true))
    }

    /// Writes the index row of an upload. When the index holds the blob and
    /// it still waits for a record, the new bytes replace the old ones; a
    /// permanent blob keeps its row.
    pub fn track_upload(
        &self,
        cid: &str,
        mime_type: &str,
        size: i64,
        temp_key: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        self.conn().execute(
            r#"insert into "blob" ("cid", "mimeType", "size", "tempKey", "createdAt") values (:cid, :mime, :size, :key, :now) on conflict ("cid") do update set "tempKey" = :key where blob."tempKey" is not null"#,
            named_params! { ":cid": cid, ":mime": mime_type, ":size": size, ":key": temp_key, ":now": now },
        )?;
        Ok(())
    }
}

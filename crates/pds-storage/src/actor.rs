//! One actor's `store.sqlite`: the repository blocks, the record index and
//! the blob index.

use std::collections::HashSet;
use std::path::Path;

use cid::Cid;
use pds_repo::{BlockStore, StoreError};
use rusqlite::{Connection, OptionalExtension, Row, named_params, params_from_iter};

use crate::error::StorageError;
use crate::layout::open_read_only;

/// The most blocks one query asks for, as in the Reference.
pub const BLOCK_BATCH: usize = 500;

/// The latest commit of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRoot {
    /// The commit's CID.
    pub cid: Cid,
    /// The commit's revision.
    pub rev: String,
}

/// A record with its block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRow {
    /// The record's AT URI.
    pub uri: String,
    /// The CID of the record block.
    pub cid: String,
    /// The record block: DAG-CBOR.
    pub content: Vec<u8>,
    /// Present while the record is taken down.
    pub takedown_ref: Option<String>,
}

/// A record with the time the server indexed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentRecord {
    /// The record's AT URI.
    pub uri: String,
    /// The CID of the record block.
    pub cid: String,
    /// When the server indexed the record (ISO 8601).
    pub indexed_at: String,
    /// The record block: DAG-CBOR.
    pub content: Vec<u8>,
}

/// What the blob index knows about a blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    /// The length in bytes.
    pub size: u64,
    /// The stored MIME type.
    pub mime_type: String,
}

/// A block of the repository with the revision that wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRow {
    /// The block's CID, as stored.
    pub cid: String,
    /// The revision of the commit that added the block.
    pub rev: String,
    /// The block.
    pub content: Vec<u8>,
}

/// The answer to a request for several blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundBlocks {
    /// The blocks the store holds, in the order it returned them.
    pub found: Vec<(Cid, Vec<u8>)>,
    /// The CIDs it does not hold, in the order they were asked for.
    pub missing: Vec<Cid>,
}

/// A connection to one actor's `store.sqlite`.
#[derive(Debug)]
pub struct ActorDb {
    conn: Connection,
}

fn parse_cid(table: &'static str, text: &str) -> Result<Cid, StorageError> {
    Cid::try_from(text).map_err(|e| StorageError::corrupt(table, format!("CID {text}: {e}")))
}

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<RecordRow> {
    Ok(RecordRow {
        uri: row.get(0)?,
        cid: row.get(1)?,
        content: row.get(2)?,
        takedown_ref: row.get(3)?,
    })
}

const RECORD_SELECT: &str = r#"select record."uri", record."cid", repo_block."content", record."takedownRef" from "record" inner join "repo_block" on repo_block."cid" = record."cid""#;

impl ActorDb {
    /// Opens the store for reading.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: open_read_only(path)?,
        })
    }

    /// Opens the store for reading inside one read transaction. The first
    /// read fixes what every later read sees, so the reads agree with each
    /// other while a writer commits. The transaction ends when the value is
    /// dropped.
    pub fn open_snapshot(path: &Path) -> Result<Self, StorageError> {
        let conn = open_read_only(path)?;
        conn.execute_batch("begin")?;
        Ok(Self { conn })
    }

    pub(crate) const fn from_connection(conn: Connection) -> Self {
        Self { conn }
    }

    pub(crate) const fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The latest commit, or `None` for a store without one.
    pub fn root(&self) -> Result<Option<RepoRoot>, StorageError> {
        let row: Option<(String, String)> = self
            .conn
            .prepare_cached(r#"select "cid", "rev" from "repo_root" limit 1"#)?
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        row.map(|(cid, rev)| {
            Ok(RepoRoot {
                cid: parse_cid("repo_root", &cid)?,
                rev,
            })
        })
        .transpose()
    }

    /// The bytes of one block.
    pub fn block(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self
            .conn
            .prepare_cached(r#"select "content" from "repo_block" where "cid" = :cid"#)?
            .query_row(named_params! { ":cid": cid.to_string() }, |row| row.get(0))
            .optional()?)
    }

    /// The blocks with these CIDs, in the order the store returns them, and
    /// the CIDs it does not hold, in the order they were asked for.
    pub fn blocks(&self, cids: &[Cid]) -> Result<FoundBlocks, StorageError> {
        let mut found: Vec<(Cid, Vec<u8>)> = Vec::with_capacity(cids.len());
        let mut have: HashSet<Cid> = HashSet::with_capacity(cids.len());
        for batch in cids.chunks(BLOCK_BATCH) {
            let marks = vec!["?"; batch.len()].join(", ");
            let sql =
                format!(r#"select "cid", "content" from "repo_block" where "cid" in ({marks})"#);
            let mut statement = self.conn.prepare(&sql)?;
            let rows = statement
                .query_map(params_from_iter(batch.iter().map(Cid::to_string)), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
                })?;
            for row in rows {
                let (cid, content) = row?;
                let cid = parse_cid("repo_block", &cid)?;
                have.insert(cid);
                found.push((cid, content));
            }
        }
        let missing = cids
            .iter()
            .filter(|cid| !have.contains(cid))
            .copied()
            .collect();
        Ok(FoundBlocks { found, missing })
    }

    /// The CIDs among these that the store does not hold, in the order they
    /// were asked for. The contents of the blocks are not read.
    pub fn missing_blocks(&self, cids: &[Cid]) -> Result<Vec<Cid>, StorageError> {
        let mut have: HashSet<Cid> = HashSet::with_capacity(cids.len());
        for batch in cids.chunks(BLOCK_BATCH) {
            let marks = vec!["?"; batch.len()].join(", ");
            let sql = format!(r#"select "cid" from "repo_block" where "cid" in ({marks})"#);
            let mut statement = self.conn.prepare(&sql)?;
            let rows = statement
                .query_map(params_from_iter(batch.iter().map(Cid::to_string)), |row| {
                    row.get::<_, String>(0)
                })?;
            for row in rows {
                have.insert(parse_cid("repo_block", &row?)?);
            }
        }
        Ok(cids
            .iter()
            .filter(|cid| !have.contains(cid))
            .copied()
            .collect())
    }

    /// A page of blocks from the newest revision down, then by CID down:
    /// the order of the Reference's repository export. `since` keeps the
    /// blocks of later revisions only. `after` is the revision and CID of the
    /// last block of the previous page.
    pub fn block_page(
        &self,
        since: Option<&str>,
        after: Option<(&str, &str)>,
    ) -> Result<Vec<BlockRow>, StorageError> {
        let mut sql = String::from(r#"select "cid", "repoRev", "content" from "repo_block""#);
        let mut clauses = Vec::new();
        if after.is_some() {
            clauses.push(r#"(("repoRev", "cid") < (:rev, :cid))"#);
        }
        if since.is_some() {
            clauses.push(r#""repoRev" > :since"#);
        }
        if !clauses.is_empty() {
            sql.push_str(" where ");
            sql.push_str(&clauses.join(" and "));
        }
        sql.push_str(r#" order by "repoRev" desc, "cid" desc limit :limit"#);
        let mut statement = self.conn.prepare(&sql)?;
        let limit = i64::try_from(BLOCK_BATCH).unwrap_or(i64::MAX);
        let mut params: Vec<(&str, &dyn rusqlite::ToSql)> = vec![(":limit", &limit)];
        if let Some((rev, cid)) = &after {
            params.push((":rev", rev));
            params.push((":cid", cid));
        }
        if let Some(since) = &since {
            params.push((":since", since));
        }
        let rows = statement.query_map(params.as_slice(), |row| {
            Ok(BlockRow {
                cid: row.get(0)?,
                rev: row.get(1)?,
                content: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The record at `uri` that is not taken down. With `cid`, only when it
    /// is that version.
    pub fn record(&self, uri: &str, cid: Option<&str>) -> Result<Option<RecordRow>, StorageError> {
        let mut sql = format!(
            r#"{RECORD_SELECT} where record."uri" = :uri and record."takedownRef" is null"#
        );
        if cid.is_some() {
            sql.push_str(r#" and record."cid" = :cid"#);
        }
        let mut statement = self.conn.prepare_cached(&sql)?;
        let row = match cid {
            Some(cid) => {
                statement.query_row(named_params! { ":uri": uri, ":cid": cid }, record_from_row)
            }
            None => statement.query_row(named_params! { ":uri": uri }, record_from_row),
        };
        Ok(row.optional()?)
    }

    /// Up to ten records written after the revision `rev`, oldest first:
    /// the URI, the CID, the time of indexing and the block. Empty when the
    /// store has no record at or before `rev`: a revision from before the
    /// account came here says nothing of what this store wrote since.
    pub fn records_since_rev(&self, rev: &str) -> Result<Vec<RecentRecord>, StorageError> {
        let mut statement = self.conn.prepare_cached(
            r#"select record."uri", record."cid", record."indexedAt", repo_block."content" from "record" inner join "repo_block" on repo_block."cid" = record."cid" where record."repoRev" > ?1 order by record."repoRev" asc limit 10"#,
        )?;
        let rows: Vec<_> = statement
            .query_map([rev], |row| {
                Ok(RecentRecord {
                    uri: row.get(0)?,
                    cid: row.get(1)?,
                    indexed_at: row.get(2)?,
                    content: row.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        if rows.is_empty() {
            return Ok(rows);
        }
        let known: bool = self
            .conn
            .prepare_cached(
                r#"select count(*) > 0 from (select 1 from "record" where "repoRev" <= ?1 limit 1)"#,
            )?
            .query_row([rev], |row| row.get(0))?;
        Ok(if known { rows } else { Vec::new() })
    }

    /// A page of the records of a collection that are not taken down, by
    /// record key: newest first, or oldest first with `reverse`. `cursor` is
    /// the record key of the last entry of the previous page. `did` is the
    /// account of the store.
    ///
    /// The query reads a range of the primary key, `uri`. Every URI of the
    /// collection is `at://<did>/<collection>/<rkey>`, so the order of the
    /// URIs is the order of the record keys. The schema of the Reference
    /// indexes `collection` alone, and a query by that index sorts every
    /// record of the collection for each page: 100 ms at 90,000 records,
    /// against 0.1 ms for the range.
    pub fn list_records(
        &self,
        did: &str,
        collection: &str,
        limit: u32,
        reverse: bool,
        cursor: Option<&str>,
    ) -> Result<Vec<RecordRow>, StorageError> {
        let prefix = format!("at://{did}/{collection}/");
        // The first string after every string that starts with the prefix:
        // '0' follows '/'.
        let end = format!("at://{did}/{collection}0");
        let (low, high) = match (cursor, reverse) {
            (Some(cursor), true) => (format!("{prefix}{cursor}"), end),
            (Some(cursor), false) => (prefix.clone(), format!("{prefix}{cursor}")),
            (None, _) => (prefix, end),
        };
        // The `+` keeps SQLite from the index of `collection`; the
        // condition stays, for a URI that does not match its row.
        let order = if reverse { "asc" } else { "desc" };
        let sql = format!(
            r#"{RECORD_SELECT} where record."uri" > :low and record."uri" < :high and +record."collection" = :collection and record."takedownRef" is null order by record."uri" {order} limit :limit"#
        );
        let mut statement = self.conn.prepare_cached(&sql)?;
        let rows = statement.query_map(
            named_params! { ":low": low, ":high": high, ":collection": collection, ":limit": limit },
            record_from_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The collections that hold at least one record, in order.
    ///
    /// Each step of the query takes the next name from the index of
    /// `collection`, so it reads one entry for each collection and not one
    /// for each record: 0.03 ms at 100,000 records, against 6.6 ms for a
    /// `group by`.
    pub fn list_collections(&self) -> Result<Vec<String>, StorageError> {
        let mut statement = self.conn.prepare_cached(
            r#"with recursive "next"("name") as (select min("collection") from "record" union all select (select min("collection") from "record" where "collection" > "next"."name") from "next" where "next"."name" is not null) select "name" from "next" where "name" is not null"#,
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every record of the index as `(uri, cid)`, in URI order.
    pub fn all_records(&self) -> Result<Vec<(String, String)>, StorageError> {
        let mut statement = self
            .conn
            .prepare_cached(r#"select "uri", "cid" from "record" order by "uri" asc"#)?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The index entry of a blob that is permanent and not taken down.
    ///
    /// An upload that still waits for a record is not one, even when its
    /// file is already in place: a commit that stopped after the file moved
    /// leaves that state, and no record references such a blob
    /// (SPEC §6.4). The Reference reads the row whatever its `tempKey`.
    pub fn blob(&self, cid: &str) -> Result<Option<BlobMeta>, StorageError> {
        let row: Option<(i64, String)> = self
            .conn
            .prepare_cached(
                r#"select "size", "mimeType" from "blob" where "cid" = :cid and "takedownRef" is null and "tempKey" is null"#,
            )?
            .query_row(named_params! { ":cid": cid }, |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        row.map(|(size, mime_type)| {
            let size = u64::try_from(size)
                .map_err(|_| StorageError::corrupt("blob", format!("size {size} of {cid}")))?;
            Ok(BlobMeta { size, mime_type })
        })
        .transpose()
    }

    /// Whether the index holds a blob, and whether it is taken down:
    /// `None` when there is no row, `Some(true)` for a taken-down blob.
    pub fn blob_takedown(&self, cid: &str) -> Result<Option<bool>, StorageError> {
        Ok(self
            .conn
            .prepare_cached(r#"select "takedownRef" is not null from "blob" where "cid" = :cid"#)?
            .query_row(named_params! { ":cid": cid }, |row| row.get(0))
            .optional()?)
    }

    /// A page of the CIDs of blobs that records reference, in CID order.
    /// `since` keeps the blobs of records written after that revision.
    pub fn list_blobs(
        &self,
        since: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<Vec<String>, StorageError> {
        let mut sql = String::from(r#"select "blobCid" from "record_blob""#);
        let mut clauses = Vec::new();
        if since.is_some() {
            sql.push_str(r#" inner join "record" on record."uri" = record_blob."recordUri""#);
            clauses.push(r#"record."repoRev" > :since"#);
        }
        if cursor.is_some() {
            clauses.push(r#""blobCid" > :cursor"#);
        }
        if !clauses.is_empty() {
            sql.push_str(" where ");
            sql.push_str(&clauses.join(" and "));
        }
        sql.push_str(r#" group by "blobCid" order by "blobCid" asc limit :limit"#);
        let mut statement = self.conn.prepare(&sql)?;
        let mut params: Vec<(&str, &dyn rusqlite::ToSql)> = vec![(":limit", &limit)];
        if let Some(since) = &since {
            params.push((":since", since));
        }
        if let Some(cursor) = &cursor {
            params.push((":cursor", cursor));
        }
        let rows = statement.query_map(params.as_slice(), |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every blob reference of the index as `(blob CID, record URI)`.
    pub fn all_record_blobs(&self) -> Result<Vec<(String, String)>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "blobCid", "recordUri" from "record_blob" order by "blobCid" asc, "recordUri" asc"#,
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every row of the blob index: the CID and, for an upload that
    /// waits, the name of its staged bytes.
    pub fn all_blob_rows(&self) -> Result<Vec<(String, Option<String>)>, StorageError> {
        let mut statement = self
            .conn
            .prepare(r#"select "cid", "tempKey" from "blob""#)?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The stored preferences in the order they were written: row id,
    /// name and JSON value.
    pub fn preferences(&self) -> Result<Vec<(i64, String, String)>, StorageError> {
        let mut statement = self
            .conn
            .prepare(r#"select "id", "name", "valueJson" from "account_pref" order by "id""#)?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The takedown reference of a blob: `None` when the store has no row
    /// for it, `Some(None)` when it is not taken down.
    pub fn blob_takedown_ref(&self, cid: &str) -> Result<Option<Option<String>>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "takedownRef" from "blob" where "cid" = ?1"#,
                [cid],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The CID and the takedown reference of a record, taken down or not.
    pub fn record_takedown_ref(
        &self,
        uri: &str,
    ) -> Result<Option<(String, Option<String>)>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "cid", "takedownRef" from "record" where "uri" = ?1"#,
                [uri],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// The number of indexed records, of blob rows, and of distinct blobs
    /// that records reference.
    pub fn counts(&self) -> Result<(u64, u64, u64), StorageError> {
        let count = |sql: &str| -> Result<u64, StorageError> {
            let value: i64 = self.conn.query_row(sql, [], |row| row.get(0))?;
            Ok(u64::try_from(value).unwrap_or(0))
        };
        Ok((
            count(r#"select count(*) from "record""#)?,
            count(r#"select count(*) from "blob""#)?,
            count(r#"select count(distinct "blobCid") from "record_blob""#)?,
        ))
    }

    /// The number of blocks in the store.
    pub fn block_count(&self) -> Result<u64, StorageError> {
        let count: i64 =
            self.conn
                .query_row(r#"select count(*) from "repo_block""#, [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}

impl BlockStore for ActorDb {
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, StoreError> {
        self.block(cid).map_err(|e| StoreError(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DID: &str = "did:plc:a";

    fn keys_of(store: &ActorDb, collection: &str) -> usize {
        store
            .list_records(DID, collection, 10, false, None)
            .unwrap()
            .len()
    }

    #[test]
    fn a_page_of_records_follows_the_record_keys_of_its_collection() {
        let dir = std::env::temp_dir().join(format!("pds-actor-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("store.sqlite");
        let writer = crate::schema::create_actor_store(&path, "2026-10-14T00:00:00.000Z").unwrap();
        let put = |collection: &str, rkey: &str, takedown: Option<&str>| {
            let cid = format!("{collection}-{rkey}");
            writer
                .execute(
                    r#"insert into "repo_block" values (?1, '1', 1, x'00')"#,
                    [&cid],
                )
                .unwrap();
            writer
                .execute(
                    r#"insert into "record" values (?1, ?2, ?3, ?4, '1', 'now', ?5)"#,
                    rusqlite::params![
                        format!("at://{DID}/{collection}/{rkey}"),
                        cid,
                        collection,
                        rkey,
                        takedown
                    ],
                )
                .unwrap();
        };
        for rkey in ["3", "1", "5", "2", "4"] {
            put("a.b.note", rkey, None);
        }
        put("a.b.note", "6", Some("taken"));
        // Collections whose URIs sort beside the ones of `a.b.note`.
        put("a.b.notes", "0", None);
        put("a.b.note.x", "9", None);
        put("a.b.nota", "9", None);

        let store = ActorDb::open(&path).unwrap();
        let keys = |limit, reverse, cursor| -> Vec<String> {
            store
                .list_records(DID, "a.b.note", limit, reverse, cursor)
                .unwrap()
                .into_iter()
                .map(|row| row.uri.rsplit('/').next().unwrap().to_owned())
                .collect()
        };
        assert_eq!(keys(10, false, None), ["5", "4", "3", "2", "1"]);
        assert_eq!(keys(10, true, None), ["1", "2", "3", "4", "5"]);
        assert_eq!(keys(2, false, None), ["5", "4"]);
        assert_eq!(keys(2, false, Some("4")), ["3", "2"]);
        assert_eq!(keys(2, true, Some("2")), ["3", "4"]);
        assert_eq!(keys(10, false, Some("1")).len(), 0);
        assert_eq!(keys(10, true, Some("")), ["1", "2", "3", "4", "5"]);
        assert_eq!(
            store.list_collections().unwrap(),
            ["a.b.nota", "a.b.note", "a.b.note.x", "a.b.notes"]
        );
        assert_eq!(keys_of(&store, "a.b.none"), 0);
        drop((store, writer));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

//! `sequencer.sqlite`: the event log (`docs/SPEC.md` §7.1).

use std::path::Path;

use crate::sql::{Connection, OptionalExtension, named_params};

use crate::error::StorageError;
use crate::layout::{open_read_only, open_read_write};

/// A break in the sequence numbers of `repo_seq`: no row lies between the
/// two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqGap {
    /// The last sequence number before the break.
    pub after: i64,
    /// The first sequence number after it.
    pub before: i64,
}

/// An event to append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqInsert {
    /// The account the event is about.
    pub did: String,
    /// `append`, `sync`, `identity` or `account`.
    pub event_type: &'static str,
    /// The DAG-CBOR body, without `seq` and `time`.
    pub event: Vec<u8>,
    /// When the event was sequenced, ISO 8601.
    pub sequenced_at: String,
}

/// A stored event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqRow {
    /// The sequence number.
    pub seq: i64,
    /// The account the event is about.
    pub did: String,
    /// `append`, `sync`, `identity` or `account`.
    pub event_type: String,
    /// The DAG-CBOR body, without `seq` and `time`.
    pub event: Vec<u8>,
    /// When the event was sequenced, ISO 8601.
    pub sequenced_at: String,
}

/// A connection to `sequencer.sqlite`.
#[derive(Debug)]
pub struct SequencerDb {
    conn: Connection,
}

impl SequencerDb {
    /// The log over a connection of the shared tier.
    pub(crate) const fn over(conn: Connection) -> Self {
        Self { conn }
    }

    /// Opens the database for reading.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: open_read_only(path)?.into(),
        })
    }

    /// Opens the database for reading and writing.
    pub fn open_writable(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: open_read_write(path)?.into(),
        })
    }

    /// Appends events in one transaction and returns their sequence
    /// numbers, in order.
    pub fn append(&mut self, events: &[SeqInsert]) -> Result<Vec<i64>, StorageError> {
        // One writer at a time: the order of the sequence numbers is the
        // order of the commits, also when several processes append.
        let txn = self
            .conn
            .transaction_with_behavior(crate::sql::TransactionBehavior::Immediate)?;
        let mut seqs = Vec::with_capacity(events.len());
        {
            let mut statement = txn.prepare_cached(
                r#"insert into "repo_seq" ("did", "eventType", "event", "sequencedAt") values (:did, :type, :event, :at) returning "seq""#,
            )?;
            for event in events {
                seqs.push(statement.query_row(
                    named_params! {
                        ":did": event.did, ":type": event.event_type,
                        ":event": event.event, ":at": event.sequenced_at,
                    },
                    |row| row.get(0),
                )?);
            }
        }
        txn.commit()?;
        Ok(seqs)
    }

    /// Removes every event of `did` except the one at `keep`.
    pub fn delete_for_did_except(&self, did: &str, keep: i64) -> Result<(), StorageError> {
        self.conn.execute(
            r#"delete from "repo_seq" where "did" = :did and "seq" != :keep"#,
            named_params! { ":did": did, ":keep": keep },
        )?;
        Ok(())
    }

    /// The highest sequence number, or `None` for an empty log.
    pub fn current(&self) -> Result<Option<i64>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "seq" from "repo_seq" order by "seq" desc limit 1"#,
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// When the first event after `cursor` was sequenced, or `None` when
    /// there is none.
    pub fn next_time(&self, cursor: i64) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "sequencedAt" from "repo_seq" where "seq" > :cursor order by "seq" asc limit 1"#,
                named_params! { ":cursor": cursor },
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The sequence number of the earliest event sequenced at or after
    /// `time`.
    pub fn earliest_after_time(&self, time: &str) -> Result<Option<i64>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "seq" from "repo_seq" where "sequencedAt" >= :time order by "sequencedAt" asc limit 1"#,
                named_params! { ":time": time },
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Up to `limit` events after `cursor`, in order. Invalidated rows are
    /// left out, as the Reference leaves them out.
    pub fn range(&self, cursor: i64, limit: u32) -> Result<Vec<SeqRow>, StorageError> {
        Ok(self.page(cursor, limit, usize::MAX)?.0)
    }

    /// One page of the events after `cursor`, in order: at most `max_rows`
    /// events, and no more after the page holds `max_bytes` of event
    /// bodies. A page has at least one event when one follows the cursor.
    ///
    /// The flag is true when the page ends at the head of the log.
    pub fn page(
        &self,
        cursor: i64,
        max_rows: u32,
        max_bytes: usize,
    ) -> Result<(Vec<SeqRow>, bool), StorageError> {
        const BATCH: usize = 64;
        if max_rows == 0 {
            return Ok((Vec::new(), false));
        }
        // Read lengths first: the shared SQL adapter materializes its
        // result, so fetching a batch of full events defeats the byte cap.
        let mut metadata = self.conn.prepare_cached(
            r#"select "seq", length("event") from "repo_seq" where "invalidated" = 0 and "seq" > :cursor order by "seq" asc limit :limit"#,
        )?;
        let mut count = 0_usize;
        let mut bytes = 0_usize;
        let mut after = cursor;
        let at_head = 'pages: loop {
            let rows: Vec<(i64, usize)> = metadata
                .query_map(named_params! { ":cursor": after, ":limit": BATCH }, |row| {
                    Ok((
                        row.get(0)?,
                        usize::try_from(row.get::<i64>(1)?).unwrap_or(usize::MAX),
                    ))
                })?
                .collect::<Result<_, _>>()?;
            let read = rows.len();
            for (seq, size) in rows {
                if count >= max_rows as usize
                    || (count > 0 && bytes.saturating_add(size) > max_bytes)
                {
                    break 'pages false;
                }
                bytes = bytes.saturating_add(size);
                after = seq;
                count += 1;
            }
            if read < BATCH {
                break true;
            }
        };
        let page = self.conn.prepare_cached(
            r#"select "seq", "did", "eventType", "event", "sequencedAt" from "repo_seq" where "invalidated" = 0 and "seq" > :cursor and "seq" <= :last order by "seq" asc"#,
        )?
        .query_map(named_params! { ":cursor": cursor, ":last": after }, |row| {
            Ok(SeqRow {
                seq: row.get(0)?,
                did: row.get(1)?,
                event_type: row.get(2)?,
                event: row.get(3)?,
                sequenced_at: row.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
        Ok((page, at_head))
    }

    /// How many events follow `cursor`, counted up to `cap`.
    pub fn pending(&self, cursor: i64, cap: u32) -> Result<u32, StorageError> {
        Ok(self.conn.prepare_cached(
            r#"select count(*) from (select 1 from "repo_seq" where "invalidated" = 0 and "seq" > :cursor limit :cap) as "pending""#,
        )?
        .query_row(named_params! { ":cursor": cursor, ":cap": cap }, |row| row.get(0))?)
    }

    /// The newest event of `did` among `types`, or `None`.
    pub fn latest_of(&self, did: &str, types: &[&str]) -> Result<Option<SeqRow>, StorageError> {
        if types.is_empty() {
            return Ok(None);
        }
        let mut values = vec![crate::sql::Value::Text(did.to_owned())];
        let mut places = Vec::new();
        for kind in types {
            values.push(crate::sql::Value::Text((*kind).to_owned()));
            places.push(format!("?{}", values.len()));
        }
        let sql = format!(
            r#"select "seq", "did", "eventType", "event", "sequencedAt" from "repo_seq" where "did" = ?1 and "invalidated" = 0 and "eventType" in ({}) order by "seq" desc limit 1"#,
            places.join(", ")
        );
        Ok(self
            .conn
            .query_row(&sql, crate::sql::params_from_iter(values), |row| {
                Ok(SeqRow {
                    seq: row.get(0)?,
                    did: row.get(1)?,
                    event_type: row.get(2)?,
                    event: row.get(3)?,
                    sequenced_at: row.get(4)?,
                })
            })
            .optional()?)
    }

    /// The number of events.
    pub fn event_count(&self) -> Result<u64, StorageError> {
        let count: i64 = self
            .conn
            .query_row(r#"select count(*) from "repo_seq""#, [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// Every break in the sequence numbers, in order.
    pub fn gaps(&self) -> Result<Vec<SeqGap>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "prev", "seq" from (select "seq", lag("seq") over (order by "seq") as "prev" from "repo_seq") as "ordered" where "prev" is not null and "seq" <> "prev" + 1"#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SeqGap {
                after: row.get(0)?,
                before: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

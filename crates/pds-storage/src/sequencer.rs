//! `sequencer.sqlite`: the event log. This unit reads it for `verify` only.

use std::path::Path;

use rusqlite::Connection;

use crate::error::StorageError;
use crate::layout::open_read_only;

/// A break in the sequence numbers of `repo_seq`: no row lies between the
/// two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqGap {
    /// The last sequence number before the break.
    pub after: i64,
    /// The first sequence number after it.
    pub before: i64,
}

/// A read connection to `sequencer.sqlite`.
#[derive(Debug)]
pub struct SequencerDb {
    conn: Connection,
}

impl SequencerDb {
    /// Opens the database for reading.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            conn: open_read_only(path)?,
        })
    }

    /// The number of events and the lowest and highest sequence numbers.
    pub fn span(&self) -> Result<(u64, Option<i64>, Option<i64>), StorageError> {
        let (count, min, max): (i64, Option<i64>, Option<i64>) = self.conn.query_row(
            r#"select count(*), min("seq"), max("seq") from "repo_seq""#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok((u64::try_from(count).unwrap_or(0), min, max))
    }

    /// Every break in the sequence numbers, in order.
    pub fn gaps(&self) -> Result<Vec<SeqGap>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select "prev", "seq" from (select "seq", lag("seq") over (order by "seq") as "prev" from "repo_seq") where "prev" is not null and "seq" <> "prev" + 1"#,
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

//! The copy of the shared tier from one backend to the other
//! (`docs/SPEC.md` §8.4): SQLite to Postgres, and back.
//!
//! The target gets the tables as this server creates them, and then every
//! row of the source. The copy ends with a comparison: the number of rows
//! of each table, the head of the event log and the last checkpoint of
//! the audit log.

use crate::error::StorageError;
use crate::layout::{DataLayout, SharedDb};
use crate::sql::{Connection, Value, params_from_iter};

/// How many rows one statement reads.
const BATCH: usize = 500;

/// Tables that others refer to. Their rows go first, so that a foreign
/// key of Postgres finds its row.
const PARENTS: &[&str] = &["actor", "account", "device", "token"];

/// What was copied for one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// The database.
    pub database: &'static str,
    /// The table.
    pub table: String,
    /// The rows of the source, which the target has too.
    pub rows: u64,
}

/// Why a copy did not happen or did not verify.
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    /// A storage call failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The source lacks a database of the tier.
    #[error("the source has no {0} database")]
    NoSource(&'static str),
    /// The target has tables already.
    #[error("the target has a {0} database already; a copy needs an empty target")]
    TargetExists(&'static str),
    /// The source has a table that this server does not create.
    #[error("the {database} database of the source has the table {table}, which the target lacks")]
    UnknownTable {
        /// The database.
        database: &'static str,
        /// The table.
        table: String,
    },
    /// The target differs from the source after the copy.
    #[error("{0}")]
    Differs(String),
}

impl From<crate::sql::Error> for MigrateError {
    fn from(error: crate::sql::Error) -> Self {
        Self::Storage(error.into())
    }
}

fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', ""))
}

fn count(conn: &Connection, table: &str) -> Result<u64, MigrateError> {
    let rows: i64 = conn.query_row(
        &format!("select count(*) from {}", quoted(table)),
        [],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(rows).unwrap_or(0))
}

/// Copies the rows of one table. The rows are read in the order of every
/// column, a batch at a time.
fn copy_table(source: &Connection, target: &Connection, table: &str) -> Result<u64, MigrateError> {
    let columns = source.columns(table)?;
    let names: Vec<String> = columns.iter().map(|column| quoted(column)).collect();
    let order: Vec<String> = (1..=columns.len()).map(|index| index.to_string()).collect();
    let places: Vec<String> = (1..=columns.len())
        .map(|index| format!("?{index}"))
        .collect();
    let select = format!(
        "select {} from {} order by {} limit ?1 offset ?2",
        names.join(", "),
        quoted(table),
        order.join(", ")
    );
    let insert = format!(
        "insert into {} ({}) values ({})",
        quoted(table),
        names.join(", "),
        places.join(", ")
    );
    let mut copied = 0_u64;
    loop {
        let rows = source.query_values(&select, crate::params![BATCH, copied])?;
        let read = rows.len();
        for row in rows {
            target.execute(&insert, params_from_iter(row))?;
            copied += 1;
        }
        if read < BATCH {
            return Ok(copied);
        }
    }
}

fn copy_database(
    source: &DataLayout,
    target: &DataLayout,
    db: SharedDb,
) -> Result<Vec<Copied>, MigrateError> {
    let from = source.connect(db, false)?;
    let mut to = target.connect(db, true)?;
    let mut tables = from.tables()?;
    tables.sort_by_key(|table| {
        let rank = PARENTS.iter().position(|parent| parent == table);
        (rank.unwrap_or(PARENTS.len()), table.clone())
    });
    let known = to.tables()?;
    let mut copied = Vec::new();
    let txn = to.transaction()?;
    for table in tables {
        if !known.contains(&table) {
            return Err(MigrateError::UnknownTable {
                database: db.name(),
                table,
            });
        }
        // The creation of the target wrote the rows of the migrator. The
        // audit log takes no delete, and it is empty.
        if db != SharedDb::Audit {
            txn.execute(&format!("delete from {}", quoted(&table)), [])?;
        }
        let rows = copy_table(&from, &txn, &table)?;
        txn.align_sequences(&table)?;
        copied.push(Copied {
            database: db.name(),
            table,
            rows,
        });
    }
    txn.commit()?;
    Ok(copied)
}

/// One value of a database, for the comparison after the copy.
fn probe(layout: &DataLayout, db: SharedDb, sql: &str) -> Result<Vec<Vec<Value>>, MigrateError> {
    Ok(layout.connect(db, false)?.query_values(sql, [])?)
}

fn verify(source: &DataLayout, target: &DataLayout, copied: &[Copied]) -> Result<(), MigrateError> {
    for db in SharedDb::ALL {
        let conn = target.connect(db, false)?;
        for table in copied.iter().filter(|table| table.database == db.name()) {
            let rows = count(&conn, &table.table)?;
            if rows != table.rows {
                return Err(MigrateError::Differs(format!(
                    "the table {} of the {} database has {rows} rows in the target and {} in the source",
                    table.table,
                    db.name(),
                    table.rows
                )));
            }
        }
    }
    for (db, what, sql) in [
        (
            SharedDb::Sequencer,
            "the head of the event log",
            r#"select max("seq") from "repo_seq""#,
        ),
        (
            SharedDb::Audit,
            "the last checkpoint of the audit log",
            r#"select "last", "root", "signature" from "checkpoint" order by "last" desc limit 1"#,
        ),
        (
            SharedDb::Audit,
            "the last entry of the audit log",
            r#"select "seq", "hash" from "entry" order by "seq" desc limit 1"#,
        ),
    ] {
        // A source without an audit log gave the target none.
        if db == SharedDb::Audit && !source.exists(db)? {
            continue;
        }
        if probe(source, db, sql)? != probe(target, db, sql)? {
            return Err(MigrateError::Differs(format!(
                "{what} differs in the target"
            )));
        }
    }
    Ok(())
}

/// Copies the shared tier of `source` into `target` and compares the two.
/// The target must have no database of the tier. `now` is the time the
/// target records for its migrations.
///
/// No server may run on either side during the copy: the caller holds
/// the lock of the data directory and the lease of the event log.
pub fn copy_shared(
    source: &DataLayout,
    target: &DataLayout,
    now: &str,
) -> Result<Vec<Copied>, MigrateError> {
    if !source.has_shared_databases()? {
        return Err(MigrateError::NoSource("account, sequencer or did_cache"));
    }
    for db in SharedDb::ALL {
        if target.exists(db)? {
            return Err(MigrateError::TargetExists(db.name()));
        }
    }
    target.create_missing(now)?;
    // The tables of the profile, when the source has them.
    let linkjar = source
        .connect(SharedDb::Account, false)?
        .table_exists("linkjar_handle_policy")?;
    if linkjar {
        target.accounts_writable()?.apply_linkjar_migrations(now)?;
    }
    let mut copied = Vec::new();
    for db in [SharedDb::Account, SharedDb::Sequencer, SharedDb::DidCache] {
        copied.extend(copy_database(source, target, db)?);
    }
    // A data directory that never ran this server has no audit log.
    if source.exists(SharedDb::Audit)? {
        drop(target.audit()?);
        copied.extend(copy_database(source, target, SharedDb::Audit)?);
    }
    verify(source, target, &copied)?;
    Ok(copied)
}

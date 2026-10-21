//! Reproducible bundled-SQLite comparison for blob-reference cleanup.
//!
//! Run `cargo run --locked --release -p pds-storage --example record_blob_index`.
//! It creates and removes its own temporary database; it accepts no database path.

#![allow(clippy::print_stdout)] // the benchmark's result is its output

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pds_storage::{ActorDb, schema};
use rusqlite::{Connection, params};

const ROWS: usize = 100_000;
const WARMUP: usize = 20;
const TRIALS: usize = 201;
const URI: &str = "at://did:plc:aaaaaaaaaaaaaaaaaaaaaaaa/com.example.note/00050000";
const DELETE: &str = r#"delete from "record_blob" where "recordUri" in (?1) returning "blobCid""#;
const INSERT: &str =
    r#"insert into "record_blob" ("blobCid", "recordUri") values (?1, ?2) on conflict do nothing"#;

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn samples<F>(conn: &Connection, mut operation: F) -> Result<Vec<Duration>, Box<dyn Error>>
where
    F: FnMut(&Connection) -> rusqlite::Result<()>,
{
    let mut elapsed = Vec::with_capacity(TRIALS);
    for trial in 0..(WARMUP + TRIALS) {
        conn.execute_batch("begin immediate")?;
        let start = Instant::now();
        operation(conn)?;
        let time = start.elapsed();
        conn.execute_batch("rollback")?;
        if trial >= WARMUP {
            elapsed.push(time);
        }
    }
    elapsed.sort_unstable();
    Ok(elapsed)
}

fn report(name: &str, times: &[Duration]) {
    println!(
        "{name},{:.3},{:.3}",
        times[(times.len() * 50).div_ceil(100) - 1].as_secs_f64() * 1e6,
        times[(times.len() * 95).div_ceil(100) - 1].as_secs_f64() * 1e6,
    );
}

fn measure(conn: &Connection, label: &str) -> Result<(), Box<dyn Error>> {
    let plan: String = conn.query_row(&format!("explain query plan {DELETE}"), [URI], |row| {
        row.get(3)
    })?;
    println!("plan_{label}={plan}");
    report(
        &format!("delete_one_{label}"),
        &samples(conn, |conn| {
            // Match ActorTxn::delete_record_blobs: prepare, step RETURNING,
            // and collect every returned CID inside the timed section.
            let removed = conn
                .prepare(DELETE)?
                .query_map([URI], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            assert_eq!(removed.len(), 1);
            Ok(())
        })?,
    );
    let extra: Vec<_> = (ROWS..ROWS + 100)
        .map(|index| {
            (
                format!("bafkrei{index:053}"),
                format!("at://did:plc:aaaaaaaaaaaaaaaaaaaaaaaa/com.example.note/{index:08}"),
            )
        })
        .collect();
    report(
        &format!("insert_100_{label}"),
        &samples(conn, |conn| {
            // Match associate_blob's statement and per-row execution.
            for (cid, uri) in &extra {
                conn.execute(INSERT, params![cid, uri])?;
            }
            Ok(())
        })?,
    );
    Ok(())
}

fn database_bytes(conn: &Connection) -> rusqlite::Result<i64> {
    let pages: i64 = conn.query_row("pragma page_count", [], |row| row.get(0))?;
    let page_size: i64 = conn.query_row("pragma page_size", [], |row| row.get(0))?;
    Ok(pages * page_size)
}

fn main() -> Result<(), Box<dyn Error>> {
    let directory =
        std::env::temp_dir().join(format!("pds-record-blob-benchmark-{}", std::process::id()));
    std::fs::create_dir(&directory)?;
    let scratch = Scratch(directory);
    let path = scratch.0.join("store.sqlite");
    // The exact Reference schema deliberately has no Candidate index.
    let conn = schema::create(&path, schema::ACTOR_STORE)?;
    let initial = Instant::now();
    conn.execute_batch("begin immediate")?;
    {
        let mut insert = conn.prepare(INSERT)?;
        for index in 0..ROWS {
            insert.execute(params![
                format!("bafkrei{index:053}"),
                format!("at://did:plc:aaaaaaaaaaaaaaaaaaaaaaaa/com.example.note/{index:08}"),
            ])?;
        }
    }
    conn.execute_batch("commit; pragma wal_checkpoint(truncate)")?;
    let fill_ms = initial.elapsed().as_secs_f64() * 1e3;
    let cache_size: i64 = conn.query_row("pragma cache_size", [], |row| row.get(0))?;
    let synchronous: i64 = conn.query_row("pragma synchronous", [], |row| row.get(0))?;
    println!("sqlite={}", rusqlite::version());
    println!(
        "rows={ROWS},warmup={WARMUP},trials={TRIALS},journal=wal,cache_size={cache_size},synchronous={synchronous}"
    );
    println!("initial_insert_commit_checkpoint_ms={fill_ms:.3}");
    println!("operation,p50_us,p95_us");
    measure(&conn, "before")?;
    let bytes_before = database_bytes(&conn)?;
    drop(conn);
    let start = Instant::now();
    // Exercise the production upgrade path against this temporary store.
    drop(ActorDb::open_writable(&path)?);
    let index_ms = start.elapsed().as_secs_f64() * 1e3;
    let conn = Connection::open(&path)?;
    conn.execute_batch("pragma wal_checkpoint(truncate)")?;
    let bytes_after = database_bytes(&conn)?;
    println!("index_open_build_commit_ms={index_ms:.3}");
    println!(
        "database_bytes_before={bytes_before},database_bytes_after={bytes_after},index_growth_bytes={}",
        bytes_after - bytes_before
    );
    measure(&conn, "after")?;
    let rows: i64 = conn.query_row("select count(*) from record_blob", [], |row| row.get(0))?;
    assert_eq!(rows, i64::try_from(ROWS)?);
    Ok(())
}

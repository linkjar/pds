//! Compare blob-reference existence checks with the previous fanout query.
//!
//! Run `cargo run --locked --release -p pds-storage --example blob_references`.
//! Only a fresh temporary database is opened; no database path is accepted.

#![allow(clippy::print_stdout)] // the benchmark's result is its output

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pds_storage::{ActorDb, schema};
use rusqlite::{Connection, OpenFlags, params, params_from_iter};

const SHARED_ROWS: usize = 100_000;
const DISTINCT_ROWS: usize = 500;
const WARMUP: usize = 20;
const TRIALS: usize = 201;
const SHARED_CID: &str = "bafkreigh2akiscaildcj5cxvzxqc4dq2kkxio5i4h3a7tdn7p7wdqj4dj4";
const RECORD_URI: &str = "at://did:plc:aaaaaaaaaaaaaaaaaaaaaaaa/com.example.note/one";

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn previous(conn: &Connection, candidates: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut found = Vec::new();
    for batch in candidates.chunks(500) {
        let marks = vec!["?"; batch.len()].join(", ");
        let sql = format!(r#"select "blobCid" from "record_blob" where "blobCid" in ({marks})"#);
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(batch), |row| row.get(0))?;
        for row in rows {
            found.push(row?);
        }
    }
    Ok(found)
}

fn report(label: &str, mut samples: Vec<Duration>, rows: usize) {
    samples.sort_unstable();
    println!(
        "{label},{:.3},{:.3},{rows}",
        samples[(samples.len() * 50).div_ceil(100) - 1].as_secs_f64() * 1e6,
        samples[(samples.len() * 95).div_ceil(100) - 1].as_secs_f64() * 1e6,
    );
}

fn compare(
    conn: &Connection,
    db: &mut ActorDb,
    label: &str,
    candidates: &[String],
    before_rows: usize,
    after_rows: usize,
    before: bool,
) -> Result<(), Box<dyn Error>> {
    let mut samples = Vec::with_capacity(TRIALS);
    for trial in 0..WARMUP + TRIALS {
        let (found, elapsed) = if before {
            conn.execute_batch("begin immediate")?;
            let start = Instant::now();
            let found = previous(conn, candidates)?;
            let elapsed = start.elapsed();
            conn.execute_batch("rollback")?;
            (found, elapsed)
        } else {
            let txn = db.begin()?;
            let start = Instant::now();
            let found = txn.referenced_blobs(candidates)?;
            let elapsed = start.elapsed();
            drop(txn);
            (found, elapsed)
        };
        assert_eq!(found.len(), if before { before_rows } else { after_rows });
        drop(found);
        if trial >= WARMUP {
            samples.push(elapsed);
        }
    }
    let variant = if before { "before" } else { "after" };
    report(
        &format!("{label}_{variant}"),
        samples,
        if before { before_rows } else { after_rows },
    );
    Ok(())
}

fn compare_record_lookup(
    conn: &Connection,
    db: &mut ActorDb,
    before: bool,
) -> Result<(), Box<dyn Error>> {
    let mut samples = Vec::with_capacity(TRIALS);
    for trial in 0..WARMUP + TRIALS {
        let elapsed = if before {
            conn.execute_batch("begin immediate")?;
            let start = Instant::now();
            for _ in 0..200 {
                let cid: String = conn.query_row(
                    r#"select "cid" from "record" where "uri" = :uri"#,
                    &[(":uri", RECORD_URI)],
                    |row| row.get(0),
                )?;
                assert_eq!(std::hint::black_box(cid), "record-cid");
            }
            let elapsed = start.elapsed();
            conn.execute_batch("rollback")?;
            elapsed
        } else {
            let txn = db.begin()?;
            let start = Instant::now();
            for _ in 0..200 {
                assert_eq!(
                    std::hint::black_box(txn.record_cid(RECORD_URI)?).as_deref(),
                    Some("record-cid"),
                );
            }
            let elapsed = start.elapsed();
            drop(txn);
            elapsed
        };
        if trial >= WARMUP {
            samples.push(elapsed);
        }
    }
    let variant = if before { "before" } else { "after" };
    report(&format!("record_lookup_200_{variant}"), samples, 200);
    Ok(())
}

fn run_case(case: &str, before: bool) -> Result<(), Box<dyn Error>> {
    let directory = std::env::temp_dir().join(format!(
        "pds-blob-reference-benchmark-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory)?;
    let scratch = Scratch(directory);
    let path = scratch.0.join("store.sqlite");
    let conn = schema::create_actor_store(&path, "2026-10-21T00:00:00Z")?;
    conn.execute_batch("begin immediate")?;
    {
        let mut insert = conn.prepare(r#"insert into "record_blob" values (?1, ?2)"#)?;
        for row in 0..SHARED_ROWS {
            insert.execute(params![
                SHARED_CID,
                format!("at://did:plc:alice/com.example.note/r{row}")
            ])?;
        }
        for row in 0..DISTINCT_ROWS {
            insert.execute(params![
                format!("distinct-{row}"),
                format!("at://did:plc:alice/com.example.note/d{row}")
            ])?;
        }
    }
    conn.execute(
        r#"insert into "record" values (?1, 'record-cid', 'com.example.note', 'one', '1', 'now', null)"#,
        [RECORD_URI],
    )?;
    conn.execute_batch("commit; pragma wal_checkpoint(truncate)")?;
    drop(conn);
    // Match the production writer's connection settings, instead of retaining
    // the connection that constructed and populated the schema.
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.set_prepared_statement_cache_capacity(128);
    let mut db = ActorDb::open_writable(&path)?;
    match case {
        "shared_one" => compare(
            &conn,
            &mut db,
            case,
            &[SHARED_CID.to_owned()],
            SHARED_ROWS,
            1,
            before,
        )?,
        "distinct_500" => compare(
            &conn,
            &mut db,
            case,
            &(0..DISTINCT_ROWS)
                .map(|n| format!("distinct-{n}"))
                .collect::<Vec<_>>(),
            DISTINCT_ROWS,
            DISTINCT_ROWS,
            before,
        )?,
        "missing_500" => compare(
            &conn,
            &mut db,
            case,
            &(0..DISTINCT_ROWS)
                .map(|n| format!("missing-{n}"))
                .collect::<Vec<_>>(),
            0,
            0,
            before,
        )?,
        "record_lookup_200" => compare_record_lookup(&conn, &mut db, before)?,
        _ => return Err("unknown benchmark case".into()),
    }
    let rows: i64 = conn.query_row(r#"select count(*) from "record_blob""#, [], |row| {
        row.get(0)
    })?;
    assert_eq!(rows, i64::try_from(SHARED_ROWS + DISTINCT_ROWS)?);
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let [variant, case] = args.as_slice() {
        let before = match variant.as_str() {
            "before" => true,
            "after" => false,
            _ => return Err("expected before or after".into()),
        };
        return run_case(case, before);
    }
    if !args.is_empty() {
        return Err(
            "expected no arguments, or a variant and case; no data paths are accepted".into(),
        );
    }
    println!("sqlite={}", rusqlite::version());
    println!(
        "shared_rows={SHARED_ROWS},distinct_rows={DISTINCT_ROWS},warmup={WARMUP},trials={TRIALS},journal=wal,process_per_case_and_variant=true"
    );
    println!("operation,p50_us,p95_us,returned_rows");
    // The old fanout query allocates millions of duplicate strings. Isolate
    // each variant and case so that process state cannot contaminate others.
    for case in [
        "shared_one",
        "distinct_500",
        "missing_500",
        "record_lookup_200",
    ] {
        for variant in ["before", "after"] {
            let status = std::process::Command::new(std::env::current_exe()?)
                .args([variant, case])
                .status()?;
            if !status.success() {
                return Err(format!("{variant} {case} failed: {status}").into());
            }
        }
    }
    Ok(())
}

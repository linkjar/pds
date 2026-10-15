//! The pool of kept read connections (`docs/SPEC.md` §8.3): what it keeps,
//! what it closes, and what it does to the WAL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{ACCOUNT_SCHEMA, Scratch};
use pds_storage::reads::ReadPool;
use pds_storage::{ActorDb, Availability, DataLayout};
use rusqlite::{Connection, params};

const NOW: &str = "2026-10-15T00:00:00.000Z";
const DID: &str = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";

/// A data directory with an `account.sqlite` that holds `did`, and the
/// pool over it.
fn data_dir(scratch: &Scratch, capacity: usize) -> (DataLayout, Arc<ReadPool>) {
    let layout = DataLayout::new(&scratch.0);
    let (_, accounts) = scratch.database("account.sqlite", ACCOUNT_SCHEMA);
    accounts
        .execute(
            r#"insert into "actor" values (?1, 'alice.test', ?2, null, null, null)"#,
            params![DID, NOW],
        )
        .unwrap();
    let pool = ReadPool::new(layout.clone(), capacity);
    (layout, pool)
}

/// Creates the actor store of `did` with a root at revision `rev`, and
/// gives a write connection to it.
fn actor_store(layout: &DataLayout, did: &str, rev: &str) -> (PathBuf, Connection) {
    let path = layout.actor_db(did);
    let writer = pds_storage::schema::create_actor_store(&path, NOW).unwrap();
    writer
        .execute(
            r#"insert into "repo_root" values (?1, 'bafyreigtzf3ocq5sgcm6xl3txb6uzn36k2uqzcbm7vkroowjzhn3ivx7o4', ?2, ?3)"#,
            params![did, rev, NOW],
        )
        .unwrap();
    (path, writer)
}

fn beside(database: &Path, suffix: &str) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Whether the pool holds a connection to the store of `did`: with its
/// file removed, only a kept connection can still read it.
fn is_kept(layout: &DataLayout, pool: &Arc<ReadPool>, did: &str) -> bool {
    let path = layout.actor_db(did);
    let moved = beside(&path, ".moved");
    std::fs::rename(&path, &moved).unwrap();
    let kept = pool.store(did).is_ok();
    std::fs::rename(&moved, &path).unwrap();
    kept
}

#[test]
fn a_returned_connection_is_lent_again() {
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 4);
    let (_, _writer) = actor_store(&layout, DID, "r1");

    let accounts = pool.accounts().unwrap();
    assert!(accounts.actor(DID, Availability::ANY).unwrap().is_some());
    let store = pool.store(DID).unwrap();
    assert_eq!(store.root().unwrap().unwrap().rev, "r1");
    assert_eq!(pool.kept(), (0, 0));
    drop((accounts, store));
    assert_eq!(pool.kept(), (1, 1));

    // The kept connections go out again, and the pool is empty meanwhile.
    let (accounts, store) = (pool.accounts().unwrap(), pool.store(DID).unwrap());
    assert_eq!(pool.kept(), (0, 0));
    assert!(accounts.actor(DID, Availability::ANY).unwrap().is_some());
    assert_eq!(store.root().unwrap().unwrap().rev, "r1");
    drop((accounts, store));
    assert_eq!(pool.kept(), (1, 1));

    // A store that is not there is an error, as for a fresh open.
    assert!(pool.store("did:plc:nobody").is_err());
    assert_eq!(pool.kept(), (1, 1));
}

#[test]
fn the_pool_keeps_the_stores_used_last() {
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 2);
    let dids = ["did:plc:a", "did:plc:b", "did:plc:c"];
    let writers: Vec<_> = dids
        .iter()
        .map(|did| actor_store(&layout, did, "r1").1)
        .collect();

    for did in dids {
        drop(pool.store(did).unwrap());
    }
    assert_eq!(pool.kept(), (0, 2));
    assert!(!is_kept(&layout, &pool, "did:plc:a"));
    assert!(is_kept(&layout, &pool, "did:plc:b"));
    assert!(is_kept(&layout, &pool, "did:plc:c"));

    // A use moves a store to the back: `b` was used last, so `c` goes
    // when `a` comes.
    drop(pool.store("did:plc:b").unwrap());
    drop(pool.store("did:plc:a").unwrap());
    assert_eq!(pool.kept(), (0, 2));
    assert!(is_kept(&layout, &pool, "did:plc:a"));
    assert!(is_kept(&layout, &pool, "did:plc:b"));
    assert!(!is_kept(&layout, &pool, "did:plc:c"));
    drop(writers);
}

#[test]
fn two_loans_of_one_store_are_two_connections() {
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 4);
    let (_, _writer) = actor_store(&layout, DID, "r1");

    let first = pool.store(DID).unwrap();
    let second = pool.store(DID).unwrap();
    assert_eq!(first.root().unwrap().unwrap().rev, "r1");
    assert_eq!(second.root().unwrap().unwrap().rev, "r1");
    drop((first, second));
    assert_eq!(pool.kept(), (0, 2));
}

#[test]
fn a_capacity_of_zero_keeps_no_store() {
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 0);
    let (_, _writer) = actor_store(&layout, DID, "r1");
    drop(pool.store(DID).unwrap());
    assert_eq!(pool.kept(), (0, 0));
    assert!(!is_kept(&layout, &pool, DID));
}

#[test]
fn the_free_list_of_the_accounts_is_bounded() {
    let scratch = Scratch::new();
    let (_, pool) = data_dir(&scratch, 4);
    let loans: Vec<_> = (0..12).map(|_| pool.accounts().unwrap()).collect();
    assert_eq!(pool.kept(), (0, 0));
    drop(loans);
    assert_eq!(pool.kept(), (8, 0));
}

#[test]
fn a_forgotten_store_is_closed_whether_kept_or_out() {
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 4);
    let (_, _a) = actor_store(&layout, "did:plc:a", "r1");
    let (_, _b) = actor_store(&layout, "did:plc:b", "r1");

    // Kept: closed at once.
    drop(pool.store("did:plc:a").unwrap());
    drop(pool.store("did:plc:b").unwrap());
    pool.forget_store("did:plc:a");
    assert_eq!(pool.kept(), (0, 1));
    assert!(!is_kept(&layout, &pool, "did:plc:a"));
    assert!(is_kept(&layout, &pool, "did:plc:b"));

    // Out: closed when it comes back, not kept.
    let out = pool.store("did:plc:a").unwrap();
    pool.forget_store("did:plc:a");
    assert_eq!(out.root().unwrap().unwrap().rev, "r1");
    drop(out);
    assert_eq!(pool.kept(), (0, 1));
    assert!(!is_kept(&layout, &pool, "did:plc:a"));

    // A loan taken after the forgetting is kept again.
    drop(pool.store("did:plc:a").unwrap());
    assert!(is_kept(&layout, &pool, "did:plc:a"));
}

#[test]
fn a_kept_connection_sees_the_next_commit() {
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 4);
    let (_, writer) = actor_store(&layout, DID, "r1");
    drop(pool.store(DID).unwrap());
    writer
        .execute(r#"update "repo_root" set "rev" = 'r2'"#, [])
        .unwrap();
    assert_eq!(pool.store(DID).unwrap().root().unwrap().unwrap().rev, "r2");
}

/// What the WAL needs: a kept connection that waits, or that is out and
/// runs nothing, holds no read transaction, so the writer's checkpoint
/// resets the `-wal` file. A snapshot holds one, and is never kept.
#[test]
fn an_idle_kept_connection_does_not_block_a_checkpoint() {
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 4);
    let (path, writer) = actor_store(&layout, DID, "r1");
    writer
        .execute_batch("pragma wal_autocheckpoint = 0")
        .unwrap();
    let wal = beside(&path, "-wal");
    let checkpoint = || -> (i64, i64, i64) {
        writer
            .query_row("pragma wal_checkpoint(truncate)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
    };
    let mut next = 0;
    let mut grow = || {
        for _ in 0..100 {
            writer
                .execute(
                    r#"insert into "repo_block" values (?1, 'r2', 1, x'00')"#,
                    [format!("b{next}")],
                )
                .unwrap();
            next += 1;
        }
        assert!(std::fs::metadata(&wal).unwrap().len() > 0);
    };

    // Kept and waiting.
    let loan = pool.store(DID).unwrap();
    assert_eq!(loan.block_count().unwrap(), 0);
    drop(loan);
    grow();
    assert_eq!(checkpoint().0, 0, "the checkpoint was blocked");
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);

    // Out, between two statements.
    let loan = pool.store(DID).unwrap();
    assert_eq!(loan.block_count().unwrap(), 100);
    grow();
    assert_eq!(checkpoint().0, 0, "the checkpoint was blocked");
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
    assert_eq!(loan.block_count().unwrap(), 200);
    drop(loan);

    // A snapshot pins what it read: the file cannot be truncated below it.
    let snapshot = ActorDb::open_snapshot(&path).unwrap();
    assert_eq!(snapshot.block_count().unwrap(), 200);
    grow();
    assert_eq!(checkpoint().0, 1, "the snapshot did not hold the WAL");
    assert!(std::fs::metadata(&wal).unwrap().len() > 0);
    drop(snapshot);
    assert_eq!(checkpoint().0, 0);
}

/// The open file descriptors of this process.
#[cfg(unix)]
fn open_files() -> usize {
    std::fs::read_dir("/dev/fd").unwrap().count()
}

/// The pool holds at most `capacity` stores open, whatever the number of
/// actors read: three descriptors each on a WAL database, for the file,
/// its `-wal` and its `-shm`.
#[cfg(unix)]
#[test]
fn the_open_files_are_bounded_by_the_capacity() {
    const ACTORS: usize = 120;
    const CAPACITY: usize = 40;
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, CAPACITY);
    let dids: Vec<String> = (0..ACTORS).map(|n| format!("did:plc:{n:024}")).collect();
    for did in &dids {
        drop(actor_store(&layout, did, "r1"));
    }
    let before = open_files();
    for did in &dids {
        assert_eq!(pool.store(did).unwrap().root().unwrap().unwrap().rev, "r1");
    }
    let after = open_files();
    assert_eq!(pool.kept(), (0, CAPACITY));
    assert!(
        after - before <= 3 * CAPACITY,
        "{} descriptors for {CAPACITY} kept stores",
        after - before
    );
    assert!(after - before >= CAPACITY, "{} descriptors", after - before);
    drop(pool);
    assert!(open_files() <= before);
}

/// The cost the pool saves, on a store of 20,000 blocks: a fresh
/// connection with its first statement against a kept one. Ignored: a
/// measurement, run by hand.
///
/// ```sh
/// cargo test --release -p pds-storage --test pool -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, run by hand in release mode"]
fn a_kept_connection_against_a_fresh_one() {
    const ROUNDS: u32 = 2000;
    let scratch = Scratch::new();
    let (layout, pool) = data_dir(&scratch, 4);
    let (path, writer) = actor_store(&layout, DID, "r1");
    writer.execute_batch("begin").unwrap();
    for n in 0..20_000 {
        writer
            .execute(
                r#"insert into "repo_block" values (?1, 'r1', 8, x'0000000000000000')"#,
                [format!("bafy{n:08}")],
            )
            .unwrap();
        writer
            .execute(
                r#"insert into "record" values (?1, ?2, 'app.bsky.feed.post', ?3, 'r1', ?4, null)"#,
                params![
                    format!("at://{DID}/app.bsky.feed.post/r{n:08}"),
                    format!("bafy{n:08}"),
                    format!("r{n:08}"),
                    NOW
                ],
            )
            .unwrap();
    }
    writer.execute_batch("commit").unwrap();
    let uri = format!("at://{DID}/app.bsky.feed.post/r00012345");

    let fresh = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let store = ActorDb::open(&path).unwrap();
        assert!(store.record(&uri, None).unwrap().is_some());
    }
    let fresh = fresh.elapsed() / ROUNDS;

    let kept = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let store = pool.store(DID).unwrap();
        assert!(store.record(&uri, None).unwrap().is_some());
    }
    let kept = kept.elapsed() / ROUNDS;
    println!(
        "record on a fresh connection: {} µs; on a kept one: {} µs",
        fresh.as_micros(),
        kept.as_micros()
    );
}

//! Read connections on databases in WAL journal mode, the mode of the
//! Reference's databases (`docs/SPEC.md` §8.3).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use std::path::{Path, PathBuf};

use common::{ACCOUNT_SCHEMA, Scratch};
use pds_storage::{AccountDb, Availability};
use rusqlite::{Connection, OpenFlags, params};

const DID: &str = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";

fn beside(database: &Path, suffix: &str) -> PathBuf {
    let mut name = database.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// An `account.sqlite` in WAL mode whose one actor is in the `-wal` file
/// only: the writer has not written it into the database file.
fn wal_database(scratch: &Scratch) -> (PathBuf, Connection) {
    let (path, writer) = scratch.database("account.sqlite", ACCOUNT_SCHEMA);
    let mode: String = writer
        .query_row("pragma journal_mode = wal", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    writer
        .execute_batch("pragma wal_autocheckpoint = 0; pragma wal_checkpoint(truncate);")
        .unwrap();
    writer
        .execute(
            r#"insert into "actor" values (?1, 'alice.test', 't', null, null, null)"#,
            params![DID],
        )
        .unwrap();
    assert!(std::fs::metadata(beside(&path, "-wal")).unwrap().len() > 0);
    (path, writer)
}

#[test]
fn a_read_connection_sees_what_an_open_writer_committed() {
    let scratch = Scratch::new();
    let (path, _writer) = wal_database(&scratch);
    let db = AccountDb::open(&path).unwrap();
    assert!(db.actor(DID, Availability::ANY).unwrap().is_some());
}

#[test]
fn a_read_connection_reads_the_wal_file_of_a_copy_and_creates_the_shm_file() {
    let scratch = Scratch::new();
    let (path, _writer) = wal_database(&scratch);
    // A copy of a live database: the database file and its `-wal` file,
    // without the `-shm` file.
    let copy_dir = scratch.0.join("copy");
    std::fs::create_dir(&copy_dir).unwrap();
    let copy = copy_dir.join("account.sqlite");
    std::fs::copy(&path, &copy).unwrap();
    std::fs::copy(beside(&path, "-wal"), beside(&copy, "-wal")).unwrap();
    assert!(!beside(&copy, "-shm").exists());

    // An `immutable` connection writes no file and does not read the
    // `-wal` file, so it does not find the actor.
    let uri = format!("file:{}?immutable=1", copy.display());
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    let immutable = Connection::open_with_flags(uri, flags).unwrap();
    let actors: i64 = immutable
        .query_row(r#"select count(*) from "actor""#, [], |row| row.get(0))
        .unwrap();
    assert_eq!(actors, 0);
    drop(immutable);
    assert!(!beside(&copy, "-shm").exists());

    let db = AccountDb::open(&copy).unwrap();
    // The actor is in the `-wal` file only, and the read finds it.
    assert!(db.actor(DID, Availability::ANY).unwrap().is_some());
    // The read wrote a file beside the database.
    assert!(beside(&copy, "-shm").exists());
    drop(db);

    // The database file itself did not change.
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(&copy).unwrap());
}

#[test]
fn a_read_connection_reads_a_wal_database_after_its_writer_closed() {
    let scratch = Scratch::new();
    let (path, writer) = wal_database(&scratch);
    drop(writer);
    // The last connection to close removed both files.
    assert!(!beside(&path, "-wal").exists());
    assert!(!beside(&path, "-shm").exists());
    let db = AccountDb::open(&path).unwrap();
    assert!(db.actor(DID, Availability::ANY).unwrap().is_some());
    // The read created both files again, and closing leaves them.
    assert!(beside(&path, "-wal").exists());
    assert!(beside(&path, "-shm").exists());
    drop(db);
    assert!(beside(&path, "-wal").exists());
    assert!(beside(&path, "-shm").exists());
}

#[cfg(unix)]
#[test]
fn a_read_connection_needs_a_writable_directory() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = Scratch::new();
    let (path, writer) = wal_database(&scratch);
    drop(writer);
    let dir = path.parent().unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    // A user that ignores permissions, such as root, cannot show this.
    let probe = dir.join("probe");
    let ignores_permissions = std::fs::write(&probe, b"").is_ok();
    let result = AccountDb::open(&path).and_then(|db| db.actor(DID, Availability::ANY));
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    if ignores_permissions {
        return;
    }
    assert!(result.is_err(), "{result:?}");
}

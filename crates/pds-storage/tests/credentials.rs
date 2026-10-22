//! Shared recovery state and account-scoped credential transactions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

mod common;

use common::Scratch;
use pds_storage::{AccountDb, DataLayout, SharedDb};

const DID: &str = "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa";

fn account(layout: &DataLayout) {
    layout.create_missing("2026-10-19T00:00:00Z").unwrap();
    let conn = layout.connect(SharedDb::Account, true).unwrap();
    conn.execute(
        r#"insert into "actor" ("did", "handle", "createdAt") values (?1, 'alice.test', 't')"#,
        [DID],
    )
    .unwrap();
    conn.execute(r#"insert into "account" ("did", "email", "passwordScrypt") values (?1, 'alice@example.com', 'old-hash')"#, [DID]).unwrap();
}

#[test]
fn a_writable_reference_account_store_adds_shared_recovery_state() {
    let scratch = Scratch::new();
    let path = scratch.0.join("account.sqlite");
    let reference = pds_storage::schema::create_shared(
        &path,
        &pds_storage::schema_shared::ACCOUNT,
        "2024-01-01T00:00:00Z",
    )
    .unwrap();
    let before: String = reference
        .query_row(
            "select group_concat(name || timestamp) from kysely_migration",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let reader = AccountDb::open(&path).unwrap();
    assert_eq!(reader.credential_epoch(DID).unwrap(), None);
    let absent: i64 = reference
        .query_row(
            "select count(*) from sqlite_master where name='credential_epoch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(absent, 0);
    drop(AccountDb::open_writable(&path).unwrap());
    let present: i64 = reference
        .query_row(
            "select count(*) from sqlite_master where name='credential_epoch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(present, 1);
    // Keep the read handle across another connection's schema upgrade.
    // Only table presence may be cached; absence and row values stay live.
    reference
        .execute("insert into credential_epoch values (?1, 'first')", [DID])
        .unwrap();
    assert_eq!(
        reader.credential_epoch(DID).unwrap().as_deref(),
        Some("first")
    );
    reference
        .execute(
            "update credential_epoch set epoch = 'second' where did = ?1",
            [DID],
        )
        .unwrap();
    assert_eq!(
        reader.credential_epoch(DID).unwrap().as_deref(),
        Some("second")
    );
    let after: String = reference
        .query_row(
            "select group_concat(name || timestamp) from kysely_migration",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, after);

    let layout = DataLayout::new(&scratch.0.join("new"));
    layout.create_missing("2026-10-19T00:00:00Z").unwrap();
    assert!(
        layout
            .connect(SharedDb::Account, false)
            .unwrap()
            .table_exists("credential_epoch")
            .unwrap()
    );
}

fn reads_keep_one_account_snapshot(layout: &DataLayout) {
    let reader = layout.accounts().unwrap();
    let before = (
        reader.password_hash(DID).unwrap(),
        reader.credential_epoch(DID).unwrap(),
    );
    let mut writer = layout.accounts_writable().unwrap();
    reader
        .with_snapshot(|db| {
            assert_eq!(db.password_hash(DID)?, before.0);
            writer.update_password(DID, "after-snapshot", "after-snapshot", None)?;
            assert_eq!(db.password_hash(DID)?, before.0);
            assert_eq!(db.credential_epoch(DID)?, before.1);
            Ok::<_, pds_storage::StorageError>(())
        })
        .unwrap();
    assert_eq!(
        reader.password_hash(DID).unwrap().as_deref(),
        Some("after-snapshot")
    );
    assert_eq!(
        reader.credential_epoch(DID).unwrap().as_deref(),
        Some("after-snapshot")
    );
    // A failed read must also release its snapshot before pooling the handle.
    let failed = reader.with_snapshot(|db| {
        db.password_hash(DID)?;
        writer.update_password(DID, "after-error", "after-error", None)?;
        Err::<(), _>(pds_storage::StorageError::Sqlite(
            rusqlite::Error::InvalidQuery,
        ))
    });
    assert!(failed.is_err());
    assert_eq!(
        reader.password_hash(DID).unwrap().as_deref(),
        Some("after-error")
    );
}

#[test]
fn sqlite_account_reads_keep_a_snapshot_and_release_it_afterward() {
    let scratch = Scratch::new();
    let layout = DataLayout::new(&scratch.0);
    account(&layout);
    reads_keep_one_account_snapshot(&layout);
}

#[test]
fn recovery_atomically_changes_the_shared_epoch_password_and_sessions() {
    let scratch = Scratch::new();
    let layout = DataLayout::new(&scratch.0);
    account(&layout);
    let mut accounts = layout.accounts_writable().unwrap();
    accounts
        .store_refresh_token("old-refresh", DID, "2030-01-01T00:00:00Z", None)
        .unwrap();
    accounts
        .update_password(DID, "new-hash", "new-epoch", None)
        .unwrap();
    assert_eq!(
        accounts.credential_epoch(DID).unwrap().as_deref(),
        Some("new-epoch")
    );
    assert_eq!(
        accounts.password_hash(DID).unwrap().as_deref(),
        Some("new-hash")
    );
    assert!(accounts.refresh_token("old-refresh").unwrap().is_none());
}

#[test]
fn recovery_unlinks_the_identities_of_providers() {
    let scratch = Scratch::new();
    let layout = DataLayout::new(&scratch.0);
    account(&layout);
    let mut accounts = layout.accounts_writable().unwrap();
    accounts
        .apply_linkjar_migrations("2026-10-19T00:00:00Z")
        .unwrap();
    accounts
        .link_identity(
            DID,
            &pds_storage::linkjar::NewIdentity {
                provider: "google",
                subject: "planted",
                email: None,
                email_verified: false,
                display_name: None,
            },
            "notice-1",
            "2026-10-19T00:00:00Z",
            0,
        )
        .unwrap();
    assert_eq!(accounts.external_identities(DID).unwrap().len(), 1);
    accounts
        .update_password(DID, "new-hash", "new-epoch", None)
        .unwrap();
    assert_eq!(accounts.external_identities(DID).unwrap(), Vec::new());
    assert!(
        accounts
            .external_identity("google", "planted", "t")
            .unwrap()
            .is_none()
    );
}

fn recovery_waits_for_a_grant(layout: &DataLayout) {
    use std::sync::mpsc;
    use std::time::Duration;

    let (checked, wait_checked) = mpsc::channel();
    let (release, wait_release) = mpsc::channel();
    let grant_layout = layout.clone();
    let grant = std::thread::spawn(move || {
        grant_layout
            .accounts_writable()
            .unwrap()
            .with_account_lock(DID, |db| {
                assert_eq!(db.password_hash(DID)?.as_deref(), Some("old-hash"));
                checked.send(()).unwrap();
                wait_release.recv().unwrap();
                db.store_refresh_token("racing-grant", DID, "2030-01-01T00:00:00Z", None)?;
                Ok::<_, pds_storage::StorageError>(())
            })
            .unwrap();
    });
    wait_checked.recv().unwrap();
    let (done, wait_done) = mpsc::channel();
    let recovery_layout = layout.clone();
    let recovery = std::thread::spawn(move || {
        recovery_layout
            .accounts_writable()
            .unwrap()
            .update_password(DID, "new-hash", "new-epoch", None)
            .unwrap();
        done.send(()).unwrap();
    });
    let completed_before_grant = wait_done.recv_timeout(Duration::from_millis(250)).is_ok();
    release.send(()).unwrap();
    grant.join().unwrap();
    recovery.join().unwrap();
    assert!(
        !completed_before_grant,
        "recovery must wait for the credential check and grant to finish"
    );
    let db = layout.accounts().unwrap();
    assert!(db.refresh_token("racing-grant").unwrap().is_none());
    assert_eq!(
        db.credential_epoch(DID).unwrap().as_deref(),
        Some("new-epoch")
    );
}

fn nested_grants_roll_back_with_the_account_transaction(layout: &DataLayout) {
    let mut db = layout.accounts_writable().unwrap();
    db.store_refresh_token("rollback-old", DID, "2030-01-01T00:00:00Z", None)
        .unwrap();
    let result = db.with_account_lock(DID, |db| {
        assert!(db.rotate_refresh_token(
            "rollback-old",
            "2029-01-01T00:00:00Z",
            "rollback-new",
            "2031-01-01T00:00:00Z"
        )?);
        db.update_password(DID, "rolled-back-hash", "rolled-back-epoch", None)?;
        Err::<(), _>(pds_storage::StorageError::Sqlite(
            rusqlite::Error::InvalidQuery,
        ))
    });
    assert!(result.is_err());
    assert_eq!(db.password_hash(DID).unwrap().as_deref(), Some("new-hash"));
    assert_eq!(
        db.credential_epoch(DID).unwrap().as_deref(),
        Some("new-epoch")
    );
    assert_eq!(
        db.refresh_token("rollback-old").unwrap().unwrap().next_id,
        None
    );
    assert!(db.refresh_token("rollback-new").unwrap().is_none());
    db.with_account_lock(DID, |db| {
        assert!(db.rotate_refresh_token(
            "rollback-old",
            "2029-01-01T00:00:00Z",
            "committed-next",
            "2031-01-01T00:00:00Z"
        )?);
        Ok::<_, pds_storage::StorageError>(())
    })
    .unwrap();
    assert!(db.refresh_token("committed-next").unwrap().is_some());

    let device = pds_storage::oauth::DeviceRow {
        id: "duplicate-device".to_owned(),
        session_id: "duplicate-session".to_owned(),
        user_agent: None,
        ip_address: "127.0.0.1".to_owned(),
        last_seen_at: "2026-10-19T00:00:00Z".to_owned(),
    };
    db.create_device(&device).unwrap();
    db.with_account_lock(DID, |db| {
        // A real SQL error aborts a Postgres subtransaction. Rolling that
        // savepoint back must leave the outer account transaction usable.
        let failed = db.with_account_lock(DID, |db| db.create_device(&device));
        assert!(failed.is_err());
        db.store_refresh_token("after-sql-error", DID, "2030-01-01T00:00:00Z", None)
    })
    .unwrap();
    assert!(db.refresh_token("after-sql-error").unwrap().is_some());
}

#[test]
fn sqlite_recovery_serializes_with_credential_grants() {
    let scratch = Scratch::new();
    let layout = DataLayout::new(&scratch.0);
    account(&layout);
    recovery_waits_for_a_grant(&layout);
    nested_grants_roll_back_with_the_account_transaction(&layout);
}

#[test]
fn postgres_recovery_serializes_with_credential_grants() {
    let Ok(url) = std::env::var("PDS_TEST_POSTGRES_URL") else {
        return;
    };
    let scratch = Scratch::new();
    let mut layout = DataLayout::new(&scratch.0);
    let tier = pds_storage::SharedTier::new(
        pds_storage::PgTarget::parse(&url).unwrap(),
        &format!("t{}_credential_lock", std::process::id()),
    );
    layout.shared = Some(tier.clone());
    account(&layout);
    let result = std::panic::catch_unwind(|| {
        recovery_waits_for_a_grant(&layout);
        nested_grants_roll_back_with_the_account_transaction(&layout);
        reads_keep_one_account_snapshot(&layout);
    });
    tier.drop_schemas().unwrap();
    result.unwrap();
}

#[test]
fn recovery_markers_survive_a_sqlite_postgres_roundtrip() {
    let Ok(url) = std::env::var("PDS_TEST_POSTGRES_URL") else {
        return;
    };
    let scratch = Scratch::new();
    let source = DataLayout::new(&scratch.0.join("source"));
    account(&source);
    source
        .accounts_writable()
        .unwrap()
        .update_password(DID, "new-hash", "shared-roundtrip-epoch", None)
        .unwrap();
    let tier = pds_storage::SharedTier::new(
        pds_storage::PgTarget::parse(&url).unwrap(),
        &format!("t{}_credential_copy", std::process::id()),
    );
    let mut postgres = DataLayout::new(&scratch.0.join("postgres"));
    postgres.shared = Some(tier.clone());
    let result = std::panic::catch_unwind(|| {
        let copied =
            pds_storage::migrate::copy_shared(&source, &postgres, "2026-10-19T00:00:00Z").unwrap();
        assert!(
            copied
                .iter()
                .any(|table| table.table == "credential_epoch" && table.rows == 1)
        );
        assert_eq!(
            postgres
                .accounts()
                .unwrap()
                .credential_epoch(DID)
                .unwrap()
                .as_deref(),
            Some("shared-roundtrip-epoch")
        );
        let returned = DataLayout::new(&scratch.0.join("returned"));
        pds_storage::migrate::copy_shared(&postgres, &returned, "2026-10-19T00:00:00Z").unwrap();
        assert_eq!(
            returned
                .accounts()
                .unwrap()
                .credential_epoch(DID)
                .unwrap()
                .as_deref(),
            Some("shared-roundtrip-epoch")
        );
        let migrations = |layout: &DataLayout| {
            layout
                .connect(SharedDb::Account, false)
                .unwrap()
                .query_values(
                    r#"select "name", "timestamp" from "kysely_migration" order by "name""#,
                    [],
                )
                .unwrap()
        };
        assert_eq!(migrations(&source), migrations(&postgres));
        assert_eq!(migrations(&source), migrations(&returned));
    });
    tier.drop_schemas().unwrap();
    result.unwrap();
}

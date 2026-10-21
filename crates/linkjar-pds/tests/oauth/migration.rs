use super::*;

#[tokio::test]
#[allow(clippy::too_many_lines)] // one data directory, there and back
async fn a_data_directory_moves_to_postgres_and_back_with_its_sessions() {
    let Ok(url) = std::env::var("PDS_TEST_POSTGRES_URL") else {
        return;
    };
    let prefix = format!("t{}_oauth", std::process::id());
    let dir = DataDir::new_with(
        1,
        &[
            ("PDS_POSTGRES_URL", &url),
            ("PDS_POSTGRES_SCHEMA_PREFIX", &prefix),
        ],
    );
    let postgres = dir.config.layout.clone();
    let tier = postgres.shared.clone().unwrap();
    let files = pds_storage::DataLayout {
        shared: None,
        ..postgres.clone()
    };
    let now = "2026-10-13T00:00:00.000Z";

    // The fixture is in the files. The copy puts it in Postgres.
    let copied = pds_storage::migrate::copy_shared(&files, &postgres, now).unwrap();
    let rows = |copied: &[pds_storage::migrate::Copied], table: &str| {
        copied
            .iter()
            .find(|entry| entry.table == table)
            .map(|entry| entry.rows)
    };
    assert_eq!(rows(&copied, "actor"), Some(3));
    assert_eq!(rows(&copied, "account"), Some(3));
    // A target that exists is refused.
    assert!(matches!(
        pds_storage::migrate::copy_shared(&files, &postgres, now),
        Err(pds_storage::migrate::MigrateError::TargetExists(_))
    ));

    // The server on Postgres knows the account of the fixture: a client
    // is authorized, and its session is used, refreshed and listed.
    let mut server = Server::start_in(dir).await;
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(true).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let key = client_key();
    let session = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(session.status, 200, "{}", session.text);
    assert_eq!(session.json()["did"], ALICE);
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let next = server
        .endpoint(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh)],
        )
        .await;
    assert_eq!(next.status, 200, "{}", next.text);
    // The refresh token that was rotated away ends the session.
    let replay = server
        .endpoint(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh)],
        )
        .await;
    assert_eq!(replay.status, 400, "{}", replay.text);
    let (_, tokens) = server.sign_in(true).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let record = json!({"repo": ALICE, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let written = server
        .resource(
            &key,
            reqwest::Method::POST,
            "/xrpc/com.atproto.repo.createRecord",
            &access,
            Some(&record),
        )
        .await;
    assert_eq!(written.status, 200, "{}", written.text);

    // Back to files, in a directory of their own.
    let back = std::env::temp_dir().join(format!("linkjar-pds-back-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&back);
    std::fs::create_dir_all(&back).unwrap();
    let files_again = pds_storage::DataLayout::new(&back);
    let returned = pds_storage::migrate::copy_shared(&postgres, &files_again, now).unwrap();
    assert_eq!(rows(&returned, "actor"), Some(3));
    assert_eq!(rows(&returned, "token"), Some(1));
    assert!(rows(&returned, "repo_seq").unwrap() >= 1);
    assert!(
        rows(&returned, "entry").is_some(),
        "the audit log moves too"
    );
    let accounts = common::open(&files_again.account_db);
    let client: String = accounts
        .query_row(
            r#"select "clientId" from "token" where "did" = ?1"#,
            [ALICE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(client, client_id());
    let device: i64 = accounts
        .query_row(
            r#"select count(*) from "account_device" where "did" = ?1"#,
            [ALICE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(device, 1);
    // The next session of the files gets the number after the last one.
    let next_id: i64 = accounts
        .query_row(
            r#"select "seq" from "sqlite_sequence" where "name" = 'token'"#,
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(next_id >= 2, "{next_id}");

    tier.drop_schemas().unwrap();
    std::fs::remove_dir_all(&back).unwrap();
}

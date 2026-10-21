use super::*;

#[tokio::test]
async fn an_account_is_created_with_its_identity_repository_session_and_events() {
    let server = Server::start(&[]).await;
    let (status, made) = server
        .create(&json!({"handle": "Bob.pds-users.org", "email": "Bob@Example.com", "password": PASSWORD}))
        .await;
    assert_eq!(status, 200, "{made}");
    assert_eq!(
        made.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["handle", "did", "accessJwt", "refreshJwt"]
    );
    assert_eq!(made["handle"], "bob.pds-users.org");
    let did = text(&made, "did").to_owned();
    assert!(did.starts_with("did:plc:") && did.len() == 32);

    // The directory got one genesis operation for that DID.
    let ops = server.ops.lock().unwrap().clone();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].0, did);
    let op = &ops[0].1;
    assert_eq!(op["alsoKnownAs"], json!(["at://bob.pds-users.org"]));
    assert_eq!(
        op["services"]["atproto_pds"]["endpoint"],
        "https://pds.test"
    );
    assert_eq!(op["rotationKeys"].as_array().unwrap().len(), 1);

    // The data directory verifies, and the log holds the four events.
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");
    let events: Vec<String> = common::open(&server.dir.root.join("sequencer.sqlite"))
        .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
        .unwrap()
        .query_map([&did], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(events, ["identity", "account", "append", "sync"]);

    // The session works, the password opens another, and the repository
    // takes a write.
    let access = text(&made, "accessJwt");
    let (status, session) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": "bob@example.com", "password": PASSWORD}),
        )
        .await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["did"], did.as_str());
    assert_eq!(session["emailConfirmed"], false);
    let (status, refreshed) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!(status, 200, "{refreshed}");
    let record = json!({"$type": "com.example.test.note", "name": "first"});
    let (status, written) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(access),
            &json!({"repo": did, "collection": "com.example.test.note", "record": record}),
        )
        .await;
    assert_eq!(status, 200, "{written}");
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");
}

#[tokio::test]
async fn a_creation_is_refused_as_the_reference_refuses_it() {
    let server = Server::start(&[]).await;
    let base = json!({"handle": "carol.pds-users.org", "email": "carol@example.com", "password": PASSWORD});
    let with = |key: &str, value: Value| {
        let mut body = base.clone();
        body[key] = value;
        body
    };
    for (body, error, message) in [
        (
            with("plcOp", json!({})),
            "InvalidRequest",
            "Unsupported input: \"plcOp\"",
        ),
        (
            with("password", json!("x".repeat(257))),
            "InvalidRequest",
            "Password too long. Maximum length is 256 characters.",
        ),
        (
            json!({"handle": "carol.pds-users.org"}),
            "InvalidRequest",
            "Email is required",
        ),
        (
            with("email", json!("not an address")),
            "InvalidRequest",
            "This email address is not supported, please use a different email.",
        ),
        (
            with("handle", json!("carol.elsewhere.org")),
            "UnsupportedDomain",
            "Not a supported handle domain",
        ),
        (
            with("handle", json!("a.b.pds-users.org")),
            "InvalidHandle",
            "Invalid characters in handle",
        ),
        (
            with("handle", json!("ab.pds-users.org")),
            "InvalidHandle",
            "Handle too short",
        ),
        (
            with("handle", json!("abcdefghijklmnopqrs.pds-users.org")),
            "InvalidHandle",
            "Handle too long",
        ),
        (
            with("handle", json!("admin.pds-users.org")),
            "HandleNotAvailable",
            "Reserved handle",
        ),
        (
            with("handle", json!("carol.pds-users.invalid")),
            "InvalidHandle",
            "Handle TLD is invalid or disallowed",
        ),
        (
            with("email", json!("Alice@example.com")),
            "InvalidRequest",
            "Email already taken: Alice@example.com",
        ),
    ] {
        let (status, answer) = server.create(&body).await;
        assert_eq!(
            (status, text(&answer, "error"), text(&answer, "message")),
            (400, error, message),
            "{body}"
        );
    }
    let (status, answer) = server
        .create(&with("did", json!("did:plc:bbbbbbbbbbbbbbbbbbbbbbbb")))
        .await;
    assert_eq!(
        (status, text(&answer, "error")),
        (401, "AuthenticationRequired")
    );
    // Nothing was sent to the directory and nothing was stored.
    assert_eq!(server.ops.lock().unwrap().len(), 0);

    // A handle is taken once, whatever the case.
    assert_eq!(server.create(&base).await.0, 200);
    let (status, answer) = server
        .create(&with("email", json!("other@example.com")))
        .await;
    assert_eq!(
        (status, text(&answer, "message")),
        (400, "Handle already taken: carol.pds-users.org")
    );
}

#[tokio::test]
async fn an_invite_code_is_required_and_used_once() {
    let server = Server::start(&[("PDS_INVITE_REQUIRED", "true")]).await;
    common::open(&server.dir.root.join("account.sqlite"))
        .execute(
            r#"insert into "invite_code" values ('pds-users-org-abcde', 1, 0, 'admin', 'admin', '2026-01-01T00:00:00.000Z')"#,
            [],
        )
        .unwrap();
    let body = |name: &str, code: Option<&str>| {
        let mut body = json!({"handle": format!("{name}.pds-users.org"),
                              "email": format!("{name}@example.com"), "password": PASSWORD});
        if let Some(code) = code {
            body["inviteCode"] = json!(code);
        }
        body
    };
    let (status, answer) = server.create(&body("dave", None)).await;
    assert_eq!(
        (status, text(&answer, "error"), text(&answer, "message")),
        (400, "InvalidInviteCode", "No invite code provided")
    );
    let (status, answer) = server.create(&body("dave", Some("unknown"))).await;
    assert_eq!(
        (status, text(&answer, "error"), text(&answer, "message")),
        (
            400,
            "InvalidInviteCode",
            "Provided invite code not available"
        )
    );
    assert_eq!(
        server
            .create(&body("dave", Some("pds-users-org-abcde")))
            .await
            .0,
        200
    );
    let (status, answer) = server
        .create(&body("erin", Some("pds-users-org-abcde")))
        .await;
    assert_eq!((status, text(&answer, "error")), (400, "InvalidInviteCode"));
    assert_eq!(server.ops.lock().unwrap().len(), 1);
}

/// A server on an empty directory creates the databases of the Reference,
/// with its migrations recorded, and takes an account.
#[tokio::test]
async fn a_new_data_directory_gets_the_databases_of_the_reference() {
    let (plc, _ops) = plc().await;
    let root = std::env::temp_dir().join(format!("linkjar-pds-fresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("blocks")).unwrap();
    let vars = [
        ("PDS_HOSTNAME", "pds.test"),
        ("PDS_DATA_DIRECTORY", root.to_str().unwrap()),
        (
            "PDS_BLOBSTORE_DISK_LOCATION",
            root.join("blocks").to_str().unwrap(),
        ),
        ("PDS_JWT_SECRET", common::JWT_SECRET),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_INVITE_REQUIRED", "false"),
        ("PDS_DID_PLC_URL", plc.as_str()),
        ("PDS_SERVICE_HANDLE_DOMAINS", ".pds-users.org"),
        (
            "PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX",
            "0707070707070707070707070707070707070707070707070707070707070707",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let config = linkjar_pds::config::Config::from_vars(&vars).unwrap();
    linkjar_pds::serve::create_missing_databases(&config).unwrap();
    // A second start changes nothing.
    linkjar_pds::serve::create_missing_databases(&config).unwrap();

    for (file, tables, migrations) in [
        ("account.sqlite", 18, 7),
        ("sequencer.sqlite", 3, 1),
        ("did_cache.sqlite", 3, 1),
    ] {
        let db = common::open(&root.join(file));
        let count = |sql: &str| -> i64 { db.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(
            count(
                "select count(*) from sqlite_master where type = 'table' and name not like 'sqlite_%' and name != 'credential_epoch'"
            ),
            tables,
            "{file}"
        );
        if file == "account.sqlite" {
            assert_eq!(
                count(
                    "select count(*) from sqlite_master where type = 'table' and name = 'credential_epoch'"
                ),
                1
            );
        }
        assert_eq!(
            count("select count(*) from kysely_migration"),
            migrations,
            "{file}"
        );
        assert_eq!(
            count("select is_locked from kysely_migration_lock"),
            0,
            "{file}"
        );
        let mode: String = db
            .query_row("pragma journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal", "{file}");
    }

    let listener = common::http::bind().await;
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = pds_xrpc::router(app_state(&config).unwrap());
    let _listener = common::http::serve(listener, router);
    let response = reqwest::Client::new()
        .post(format!("{origin}/xrpc/com.atproto.server.createAccount"))
        .header("content-type", "application/json")
        .body(json!({"handle": "zoe-one.pds-users.org", "email": "zoe@example.com", "password": PASSWORD}).to_string())
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let made: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(status, 200, "{made}");
    let report = linkjar_pds::verify::verify(&config).unwrap();
    assert_eq!((report.actors, report.findings.len()), (1, 0), "{report}");
    // The actor store records its migration too.
    let store = common::open(&config.layout.actor_db(text(&made, "did")));
    let recorded: String = store
        .query_row("select name from kysely_migration", [], |row| row.get(0))
        .unwrap();
    assert_eq!(recorded, "001");
    std::fs::remove_dir_all(&root).unwrap();
}

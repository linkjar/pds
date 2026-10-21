use super::*;

#[tokio::test]
async fn an_account_moves_here_with_its_did_and_starts_deactivated() {
    const MOVER: &str = "did:plc:mmmmmmmmmmmmmmmmmmmmmmmm";
    let server = Server::start(&[]).await;
    // The DID document of an account that another server hosts.
    let multikey = common::key(9).public_key().to_multikey();
    let doc = format!(
        r#"{{"@context":["https://www.w3.org/ns/did/v1"],"id":"{MOVER}","alsoKnownAs":["at://mover.elsewhere.org"],"verificationMethod":[{{"id":"{MOVER}#atproto","type":"Multikey","controller":"{MOVER}","publicKeyMultibase":"{multikey}"}}]}}"#
    );
    common::open(&server.dir.config.layout.did_cache_db)
        .execute(
            r#"insert into "did_doc" values (?1, ?2, ?3)"#,
            rusqlite::params![MOVER, doc, i64::MAX / 2],
        )
        .unwrap();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let token = |key: u8| {
        pds_xrpc::api::proxy::service_jwt(
            &common::key(key),
            MOVER,
            "did:web:pds.test",
            Some("com.atproto.server.createAccount"),
            now,
            now + 60,
            &[key; 16],
        )
    };
    let account = json!({
        "did": MOVER,
        "handle": "mover.pds-users.org",
        "email": "mover@example.com",
        "password": PASSWORD,
    });
    let create = "com.atproto.server.createAccount";
    let before = server.ops.lock().unwrap().len();

    // Without a token.
    let (status, body) = server.create(&account).await;
    assert_eq!(
        (status, text(&body, "message")),
        (
            401,
            format!("Missing auth to create account with did: {MOVER}").as_str()
        )
    );
    let (status, made) = server.post(create, Some(&token(9)), &account).await;
    assert_eq!(status, 200, "{made}");
    assert_eq!(text(&made, "did"), MOVER);
    assert_eq!(text(&made, "handle"), "mover.pds-users.org");
    let access = text(&made, "accessJwt").to_owned();

    // The account is here, deactivated. The server wrote nothing to the
    // PLC directory and nothing to the firehose.
    assert_eq!(server.ops.lock().unwrap().len(), before);
    let deactivated: Option<String> = common::open(&server.dir.config.layout.account_db)
        .query_row(
            r#"select "deactivatedAt" from "actor" where "did" = ?1"#,
            [MOVER],
            |row| row.get(0),
        )
        .unwrap();
    assert!(deactivated.is_some());
    let events: i64 = common::open(&server.dir.config.layout.sequencer_db)
        .query_row(
            r#"select count(*) from "repo_seq" where "did" = ?1"#,
            [MOVER],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 0);
    let response = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.server.checkAccountStatus",
            server.origin
        ))
        .bearer_auth(&access)
        .send()
        .await
        .unwrap();
    let status: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(status["activated"], false, "{status}");
    assert_eq!(status["repoBlocks"], 2);

    // A token that another key signed brings no DID. The check comes
    // last: a bad signature makes the server ask the directory again, and
    // this directory does not know the DID.
    let other = json!({
        "did": MOVER,
        "handle": "mover2.pds-users.org",
        "email": "mover2@example.com",
        "password": PASSWORD,
    });
    let (status, _) = server.post(create, Some(&token(8)), &other).await;
    assert_eq!(status, 401);
}

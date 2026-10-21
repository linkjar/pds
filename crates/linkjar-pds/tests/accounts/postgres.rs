use super::*;

/// The variables of a server whose shared tier is in the Postgres that
/// `PDS_TEST_POSTGRES_URL` names, in schemas of its own. `None` without
/// the variable: the test then does nothing.
fn postgres(name: &str) -> Option<(String, String)> {
    let url = std::env::var("PDS_TEST_POSTGRES_URL").ok()?;
    Some((url, format!("t{}_{name}", std::process::id())))
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn the_shared_tier_runs_on_postgres() {
    use futures_util::StreamExt as _;
    use tokio_tungstenite::tungstenite::Message;
    let Some((url, prefix)) = postgres("accounts") else {
        return;
    };
    let server = Server::start(&[
        ("PDS_POSTGRES_URL", &url),
        ("PDS_POSTGRES_SCHEMA_PREFIX", &prefix),
        ("PDS_PROFILE", "linkjar"),
        ("PDS_SERVICE_HANDLE_DOMAINS", ".linkjar.social"),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
    ])
    .await;
    let tier = server.dir.config.layout.shared.clone().unwrap();

    // An account, with its rows in Postgres and none in the SQLite files.
    let account = json!({"handle": "petra.linkjar.social", "email": "petra@example.com", "password": PASSWORD});
    let (status, made) = server.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();
    let access = text(&made, "accessJwt").to_owned();
    let in_files: i64 = common::open(&server.dir.config.layout.account_db)
        .query_row(
            r#"select count(*) from "actor" where "did" = ?1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(in_files, 0);
    assert_eq!(server.create(&account).await.0, 400);

    // Sessions.
    let login = json!({"identifier": "petra.linkjar.social", "password": PASSWORD});
    let (status, session) = server
        .post("com.atproto.server.createSession", None, &login)
        .await;
    assert_eq!(status, 200, "{session}");
    let (status, refreshed) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&session, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_eq!(status, 200, "{refreshed}");
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&session, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_ne!(status, 500);
    // A token that the server never sent is refused, not an error of the
    // server: the statement compares a parameter with NULL.
    let wrong = json!({"email": "petra@example.com", "token": "aaaaa-bbbbb"});
    let (status, refused) = server
        .post("com.atproto.server.confirmEmail", Some(&access), &wrong)
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "InvalidToken"),
        "{refused}"
    );
    let reset = json!({"token": "aaaaa-bbbbb", "password": "another-password"});
    let (status, refused) = server
        .post("com.atproto.server.resetPassword", None, &reset)
        .await;
    assert_eq!(status, 400, "{refused}");
    let app = json!({"name": "phone"});
    let (status, body) = server
        .post("com.atproto.server.createAppPassword", Some(&access), &app)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        server
            .post("com.atproto.server.createAppPassword", Some(&access), &app)
            .await
            .0,
        400
    );

    // Records and the event log.
    for n in 0..3 {
        let record = json!({"repo": did, "collection": "com.example.note", "record": {"$type": "com.example.note", "n": n}});
        let (status, body) = server
            .post("com.atproto.repo.createRecord", Some(&access), &record)
            .await;
        assert_eq!(status, 200, "{body}");
    }
    let listed = server
        .http
        .get(format!("{}/xrpc/com.atproto.sync.listRepos", server.origin))
        .send()
        .await
        .unwrap();
    let repos: Value = serde_json::from_slice(&listed.bytes().await.unwrap()).unwrap();
    assert!(
        repos["repos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|repo| repo["did"] == did.as_str()),
        "{repos}"
    );
    let mut firehose = tokio_tungstenite::connect_async(format!(
        "{}/xrpc/com.atproto.sync.subscribeRepos?cursor=0",
        server.origin.replace("http://", "ws://")
    ))
    .await
    .unwrap()
    .0;
    // The identity, the account, and four commits: the first of the
    // repository and one for each record.
    for _ in 0..6 {
        assert!(matches!(
            firehose.next().await,
            Some(Ok(Message::Binary(_)))
        ));
    }

    // The admin, the operator's state, and the audit log.
    let (status, codes) = server
        .operate(
            "com.atproto.server.createInviteCodes",
            Some("a test"),
            Some(&json!({"codeCount": 2, "useCount": 1})),
        )
        .await;
    assert_eq!(status, 200, "{codes}");
    let takedown = json!({
        "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
        "takedown": {"applied": true, "ref": "pg-1"},
    });
    let (status, body) = server
        .operate(
            "com.atproto.admin.updateSubjectStatus",
            Some("a test"),
            Some(&takedown),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, card) = server
        .operate(&format!("internal.ops.getAccount?did={did}"), None, None)
        .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["status"], "takendown");
    assert_eq!(card["email"], "petra@example.com");
    let (status, overview) = server.operate("internal.ops.getOverview", None, None).await;
    assert_eq!(status, 200, "{overview}");
    assert_eq!(overview["accounts"], 1);
    assert!(overview["sequencer"]["head"].as_i64().unwrap() >= 6);
    let (status, audit) = server
        .operate("internal.ops.listAudit?limit=3", None, None)
        .await;
    assert_eq!(status, 200, "{audit}");
    assert_eq!(
        audit["entries"][0]["action"],
        "com.atproto.admin.updateSubjectStatus"
    );
    let (status, _) = server
        .operate("internal.ops.checkpoint", Some("a test"), Some(&json!({})))
        .await;
    assert_eq!(status, 200);

    // A second server on the tier does not get the lease of the event log.
    assert!(tier.sequencer_lease().unwrap().is_some());
    let held = tier.sequencer_lease().unwrap().unwrap();
    assert!(tier.sequencer_lease().unwrap().is_none());
    drop(held);
    tier.drop_schemas().unwrap();
}

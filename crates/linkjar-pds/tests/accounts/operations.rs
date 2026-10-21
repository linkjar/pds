use super::*;

#[tokio::test]
async fn the_metrics_count_requests_by_route_and_read_the_audit_log() {
    let dir = DataDir::new(1);
    let state = app_state(&dir.config).unwrap();
    let listener = common::http::bind().await;
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = pds_xrpc::router(state.clone());
    let _listener = common::http::serve(listener, router);
    let http = reqwest::Client::new();
    for path in [
        "/xrpc/_health",
        "/xrpc/_health",
        "/xrpc/app.bsky.feed.getTimeline",
        "/wp-login.php",
        "/xrpc/com.atproto.admin.getAccountInfo?did=did:plc:aaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        http.get(format!("{origin}{path}")).send().await.unwrap();
    }
    let text = tokio::task::spawn_blocking(move || pds_xrpc::metrics::render(&state))
        .await
        .unwrap();
    for line in [
        "pds_build_info{version=",
        "pds_http_requests_total{route=\"/xrpc/_health\",status=\"200\"} 2",
        // A method of another service and a path nobody serves have one label each.
        "pds_http_requests_total{route=\"proxied\",status=\"401\"} 1",
        "pds_http_requests_total{route=\"other\",status=\"404\"} 1",
        "pds_http_request_duration_seconds_count{route=\"/xrpc/_health\"} 2",
        // The admin method was refused and recorded.
        "pds_audit_entries 1",
        "pds_audit_unsigned_entries 1",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert!(!text.contains("wp-login"));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn the_admin_reads_and_acts_on_the_ops_listener_with_a_reason() {
    let server = Server::start(&[("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD)]).await;
    let account = json!({"handle": "olive.pds-users.org", "email": "olive@example.com", "password": PASSWORD});
    let (status, made) = server.create(&account).await;
    assert_eq!(status, 200, "{made}");
    let did = text(&made, "did").to_owned();

    // No credential, and the credential of an account.
    let anonymous = server
        .http
        .get(format!(
            "{}/xrpc/internal.ops.getOverview",
            server.ops_origin
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    let session = server
        .http
        .get(format!(
            "{}/xrpc/internal.ops.getOverview",
            server.ops_origin
        ))
        .bearer_auth(text(&made, "accessJwt"))
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), 401);

    // Reads.
    let (status, operator) = server.operate("internal.ops.getOperator", None, None).await;
    assert_eq!(status, 200);
    assert_eq!(operator, json!({"actor": "admin", "role": "admin"}));
    let (status, overview) = server.operate("internal.ops.getOverview", None, None).await;
    assert_eq!(status, 200, "{overview}");
    assert_eq!(overview["accounts"], 4);
    assert_eq!(overview["subscribers"], 0);
    assert_eq!(overview["profile"], "stock");
    // The fixture has three accounts.
    let (_, first) = server
        .operate("internal.ops.listAccounts?limit=3", None, None)
        .await;
    assert_eq!(first["accounts"].as_array().unwrap().len(), 3);
    let cursor = text(&first, "cursor").replace(':', "%3A");
    let (_, second) = server
        .operate(
            &format!("internal.ops.listAccounts?limit=3&cursor={cursor}"),
            None,
            None,
        )
        .await;
    assert_eq!(second["accounts"].as_array().unwrap().len(), 1, "{second}");
    assert!(second["cursor"].is_null());
    let listed = [&first, &second]
        .iter()
        .flat_map(|page| page["accounts"].as_array().unwrap())
        .filter(|account| account["did"] == did.as_str())
        .count();
    assert_eq!(listed, 1);
    let (status, card) = server
        .operate(&format!("internal.ops.getAccount?did={did}"), None, None)
        .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["email"], "olive@example.com");
    assert_eq!(card["status"], "active");
    assert_eq!(card["repo"]["records"], 0);
    assert_eq!(card["legacySessions"].as_array().unwrap().len(), 1);
    let (status, report) = server.operate("internal.ops.verify", None, None).await;
    assert_eq!(status, 200, "{report}");

    // An action needs a reason, and a method the listener does not have
    // writes no entry.
    let block = json!({"domain": "Spam.Example"});
    let (status, refused) = server
        .operate("internal.ops.blockEmailDomain", None, Some(&block))
        .await;
    assert_eq!(status, 400, "{refused}");
    let (status, _) = server
        .operate(
            "internal.ops.noSuchAction",
            Some("a test"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 501);
    let (status, _) = server
        .operate(
            "com.atproto.repo.createRecord",
            Some("a test"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 501);

    // A blocked email domain closes the sign-up to its addresses.
    let (status, _) = server
        .operate(
            "internal.ops.blockEmailDomain",
            Some("a wave of sign-ups"),
            Some(&block),
        )
        .await;
    assert_eq!(status, 200);
    let spam = json!({"handle": "spam.pds-users.org", "email": "a@mail.spam.example", "password": PASSWORD});
    let (status, refused) = server.create(&spam).await;
    assert_eq!(status, 400, "{refused}");
    let (_, domains) = server
        .operate("internal.ops.listBlockedEmailDomains", None, None)
        .await;
    assert_eq!(domains["domains"][0]["domain"], "spam.example");
    assert_eq!(domains["domains"][0]["reason"], "a wave of sign-ups");
    let lift = json!({"domain": "spam.example"});
    let (status, _) = server
        .operate(
            "internal.ops.unblockEmailDomain",
            Some("the wave ended"),
            Some(&lift),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(server.create(&spam).await.0, 200);

    // A role, for an account of the server only.
    let grant = json!({"did": did, "role": "moderator"});
    let (status, _) = server
        .operate(
            "internal.ops.grantRole",
            Some("joins the team"),
            Some(&grant),
        )
        .await;
    assert_eq!(status, 200);
    let stranger = json!({"did": "did:plc:zzzzzzzzzzzzzzzzzzzzzzzz", "role": "operator"});
    let (status, refused) = server
        .operate("internal.ops.grantRole", Some("a test"), Some(&stranger))
        .await;
    assert_eq!((status, text(&refused, "error")), (400, "AccountNotFound"));
    let (_, roles) = server.operate("internal.ops.listRoles", None, None).await;
    assert_eq!(roles["roles"][0]["role"], "moderator");
    assert_eq!(roles["roles"][0]["grantedBy"], "admin");

    // `#sync` for a repository.
    let events = |did: &str| -> i64 {
        common::open(&server.dir.config.layout.sequencer_db)
            .query_row(
                r#"select count(*) from "repo_seq" where "did" = ?1 and "eventType" = 'sync'"#,
                [did],
                |row| row.get(0),
            )
            .unwrap()
    };
    let before = events(&did);
    let (status, synced) = server
        .operate(
            "internal.ops.emitSync",
            Some("a relay lost the repository"),
            Some(&json!({"did": did})),
        )
        .await;
    assert_eq!(status, 200, "{synced}");
    assert!(synced["seq"].as_i64().unwrap() > 0);
    assert_eq!(events(&did), before + 1);

    // The sessions of an account end.
    let (status, ended) = server
        .operate(
            "internal.ops.revokeSessions",
            Some("a stolen phone"),
            Some(&json!({"did": did})),
        )
        .await;
    assert_eq!(status, 200, "{ended}");
    assert_eq!(ended["legacySessions"], 1);
    let (status, _) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &json!({}),
        )
        .await;
    assert_eq!(status, 400);

    // An admin method through the listener, and the housekeeping.
    let takedown = json!({
        "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": did},
        "takedown": {"applied": true, "ref": "ops-1"},
    });
    let (status, body) = server
        .operate(
            "com.atproto.admin.updateSubjectStatus",
            Some("spam"),
            Some(&takedown),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, info) = server
        .operate(
            &format!("com.atproto.admin.getSubjectStatus?did={did}"),
            None,
            None,
        )
        .await;
    assert_eq!(status, 200, "{info}");
    assert_eq!(info["takedown"]["applied"], true);
    let (status, _) = server
        .operate(
            "internal.ops.purgeExpired",
            Some("housekeeping"),
            Some(&json!({})),
        )
        .await;
    assert_eq!(status, 200);
    let (status, refused) = server
        .operate(
            "internal.ops.disconnectSubscriber",
            Some("a test"),
            Some(&json!({"id": 99})),
        )
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (400, "SubscriberNotFound")
    );

    // Each action is an entry with the operator, the reason and the
    // console. The newest is first. A read of the admin is an entry too.
    let (status, audit) = server
        .operate("internal.ops.listAudit?limit=5", None, None)
        .await;
    assert_eq!(status, 200, "{audit}");
    let entries = audit["entries"].as_array().unwrap();
    let actions: Vec<&str> = entries.iter().map(|entry| text(entry, "action")).collect();
    assert_eq!(
        actions,
        [
            "internal.ops.disconnectSubscriber",
            "internal.ops.purgeExpired",
            "com.atproto.admin.getSubjectStatus",
            "com.atproto.admin.updateSubjectStatus",
            "internal.ops.revokeSessions",
        ]
    );
    assert_eq!(entries[0]["outcome"], "SubscriberNotFound");
    let takedown = &entries[3];
    assert_eq!(takedown["actor"], "admin");
    assert_eq!(takedown["credential"], "admin");
    assert_eq!(takedown["reason"], "spam");
    assert_eq!(takedown["sidecar"], "console");
    assert_eq!(takedown["subject"], did.as_str());
    assert_eq!(entries[4]["subject"], did.as_str());
    assert_eq!(entries[2]["reason"], Value::Null);
    assert!(audit["before"].as_u64().unwrap() > 1);
    let all: Vec<String> = server
        .operate("internal.ops.listAudit?limit=500", None, None)
        .await
        .1["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| text(entry, "action").to_owned())
        .collect();
    assert!(!all.iter().any(|action| action.contains("noSuchAction")));
    assert!(
        !all.iter()
            .any(|action| action == "com.atproto.repo.createRecord")
    );
}

#[tokio::test]
async fn a_subscriber_is_seen_live_and_disconnected_by_the_operator() {
    use base64::Engine as _;
    use futures_util::StreamExt as _;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

    let server = Server::start(&[("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD)]).await;
    let account =
        json!({"handle": "piet.pds-users.org", "email": "piet@example.com", "password": PASSWORD});
    assert_eq!(server.create(&account).await.0, 200);

    let mut request = format!(
        "{}/xrpc/com.atproto.sync.subscribeRepos?cursor=0",
        server.origin.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("user-agent", "test-relay/1".parse().unwrap());
    let mut firehose = tokio_tungstenite::connect_async(request).await.unwrap().0;
    assert!(matches!(
        firehose.next().await,
        Some(Ok(Message::Binary(_)))
    ));

    // The live stream of the ops listener shows the subscriber.
    let mut request = format!(
        "{}/xrpc/internal.ops.subscribeState",
        server.ops_origin.replace("http://", "ws://")
    )
    .into_client_request()
    .unwrap();
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("admin:{}", common::ADMIN_PASSWORD));
    request
        .headers_mut()
        .insert("authorization", format!("Basic {basic}").parse().unwrap());
    let mut live = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let Some(Ok(Message::Text(frame))) = live.next().await else {
        panic!("the live stream sent no state");
    };
    let frame: Value = serde_json::from_str(frame.as_str()).unwrap();
    assert_eq!(frame["overview"]["subscribers"], 1, "{frame}");
    let subscriber = &frame["subscribers"][0];
    assert_eq!(subscriber["userAgent"], "test-relay/1");
    assert!(subscriber["frames"].as_u64().unwrap() >= 1);
    assert!(subscriber["bytes"].as_u64().unwrap() > 0);

    // The operator disconnects it: the stream ends with a close frame.
    let id = json!({"id": subscriber["id"]});
    let (status, body) = server
        .operate(
            "internal.ops.disconnectSubscriber",
            Some("a runaway consumer"),
            Some(&id),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let closed = loop {
        match firehose.next().await {
            Some(Ok(Message::Binary(_))) => {}
            other => break other,
        }
    };
    let Some(Ok(Message::Close(Some(close)))) = closed else {
        panic!("the firehose did not close: {closed:?}");
    };
    assert_eq!(close.reason.as_str(), "disconnected by the operator");
    let (_, listed) = server
        .operate("internal.ops.listSubscribers", None, None)
        .await;
    assert_eq!(listed["subscribers"], json!([]));
}

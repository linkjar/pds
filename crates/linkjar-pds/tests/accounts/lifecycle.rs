use super::*;

#[tokio::test]
async fn an_account_is_deactivated_checked_and_activated() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "frank.pds-users.org", "email": "frank@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let get = |nsid: &'static str| {
        let (http, origin, access) = (server.http.clone(), server.origin.clone(), access.clone());
        async move {
            let response = http
                .get(format!("{origin}/xrpc/{nsid}"))
                .bearer_auth(access)
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let events = || -> Vec<String> {
        common::open(&server.dir.root.join("sequencer.sqlite"))
            .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
            .unwrap()
            .query_map([&did], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    let (status, checked) = get("com.atproto.server.checkAccountStatus").await;
    assert_eq!(status, 200, "{checked}");
    assert_eq!(
        (
            &checked["activated"],
            &checked["validDid"],
            &checked["repoBlocks"],
            &checked["indexedRecords"]
        ),
        // The commit and the empty root of the tree.
        (&json!(true), &json!(true), &json!(2), &json!(0))
    );
    assert_eq!(checked.as_object().unwrap().len(), 9);

    let deactivate = "com.atproto.server.deactivateAccount";
    assert_eq!(
        server.post(deactivate, Some(&access), &json!({})).await.0,
        200
    );
    let (_, session) = get("com.atproto.server.getSession").await;
    assert_eq!(
        (&session["active"], &session["status"]),
        (&json!(false), &json!("deactivated"))
    );
    let (_, checked) = get("com.atproto.server.checkAccountStatus").await;
    assert_eq!(checked["activated"], false);
    // A deactivated repository takes no write.
    let (status, refused) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(&access),
            &json!({"repo": did, "collection": "com.example.test.note", "record": {"$type": "com.example.test.note"}}),
        )
        .await;
    assert_eq!(
        (status, text(&refused, "error")),
        (401, "AccountDeactivated")
    );

    let (status, body) = server
        .post(
            "com.atproto.server.activateAccount",
            Some(&access),
            &Value::Null,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, session) = get("com.atproto.server.getSession").await;
    assert_eq!(session["active"], true);
    assert_eq!(
        events(),
        [
            "identity", "account", "append", "sync", "account", "account", "identity", "sync"
        ]
    );
    let report = linkjar_pds::verify::verify(&server.dir.config).unwrap();
    assert_eq!(report.findings.len(), 0, "{report}");
}

#[tokio::test]
async fn a_handle_is_changed_in_the_directory_the_account_and_the_log() {
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "gina.pds-users.org", "email": "gina@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let update = "com.atproto.identity.updateHandle";

    let (status, body) = server
        .post(
            update,
            Some(&access),
            &json!({"handle": "Georgina.pds-users.org"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let ops = server.ops.lock().unwrap().clone();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[1].0, did);
    assert_eq!(
        ops[1].1["alsoKnownAs"],
        json!(["at://georgina.pds-users.org"])
    );
    assert!(ops[1].1["prev"].as_str().unwrap().starts_with("bafyrei"));
    let (status, session) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": "georgina.pds-users.org", "password": PASSWORD}),
        )
        .await;
    assert_eq!((status, &session["did"]), (200, &json!(did)));
    let last: String = common::open(&server.dir.root.join("sequencer.sqlite"))
        .query_row(
            r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq" desc limit 1"#,
            [&did],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(last, "identity");

    for (handle, error, message) in [
        (
            "carol.pds.example",
            "InvalidHandle",
            "Handle TLD is invalid or disallowed",
        ),
        ("ab.pds-users.org", "InvalidHandle", "Handle too short"),
        (
            "admin.pds-users.org",
            "HandleNotAvailable",
            "Reserved handle",
        ),
        (
            "unreachable.elsewhere.org",
            "InvalidRequest",
            "External handle did not resolve to DID",
        ),
    ] {
        let (status, body) = server
            .post(update, Some(&access), &json!({"handle": handle}))
            .await;
        assert_eq!(
            (status, text(&body, "error"), text(&body, "message")),
            (400, error, message),
            "{handle}"
        );
    }
    // Another account's handle is not available.
    server
        .create(&json!({"handle": "henry.pds-users.org", "email": "henry@example.com", "password": PASSWORD}))
        .await;
    let (status, body) = server
        .post(
            update,
            Some(&access),
            &json!({"handle": "henry.pds-users.org"}),
        )
        .await;
    assert_eq!(
        (status, text(&body, "error"), text(&body, "message")),
        (
            400,
            "HandleNotAvailable",
            "Handle already taken: henry.pds-users.org"
        )
    );
    assert_eq!(server.ops.lock().unwrap().len(), 3);
}

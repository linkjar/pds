use super::*;

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn the_admin_reads_and_changes_the_status_of_accounts_records_and_blobs() {
    use base64::Engine as _;
    let server = Server::start(&[]).await;
    let (_, made) = server
        .create(&json!({"handle": "irene.pds-users.org", "email": "irene@example.com", "password": PASSWORD}))
        .await;
    let (did, access) = (
        text(&made, "did").to_owned(),
        text(&made, "accessJwt").to_owned(),
    );
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let admin_get = |path: String| {
        let (http, origin, basic) = (server.http.clone(), server.origin.clone(), basic.clone());
        async move {
            let response = http
                .get(format!("{origin}/xrpc/{path}"))
                .header("authorization", basic)
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let admin_post = |nsid: &'static str, body: Value| {
        let (http, origin, basic) = (server.http.clone(), server.origin.clone(), basic.clone());
        async move {
            let response = http
                .post(format!("{origin}/xrpc/{nsid}"))
                .header("authorization", basic)
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let info = format!("com.atproto.admin.getAccountInfo?did={did}");
    let subject = format!("com.atproto.admin.getSubjectStatus?did={did}");
    let update = "com.atproto.admin.updateSubjectStatus";

    // Only the admin.
    let (status, _) = server.post(update, Some(&access), &json!({})).await;
    assert_eq!(status, 401);
    let (status, account) = admin_get(info.clone()).await;
    assert_eq!(status, 200, "{account}");
    assert_eq!(
        (
            &account["handle"],
            &account["email"],
            &account["invitesDisabled"]
        ),
        (
            &json!("irene.pds-users.org"),
            &json!("irene@example.com"),
            &json!(false)
        )
    );
    let (status, body) = admin_get(
        "com.atproto.admin.getAccountInfo?did=did:plc:zzzzzzzzzzzzzzzzzzzzzzzz".to_owned(),
    )
    .await;
    assert_eq!((status, text(&body, "error")), (400, "NotFound"));

    // A takedown ends the sessions and is in the log; lifting it restores
    // the account.
    let repo = json!({"$type": "com.atproto.admin.defs#repoRef", "did": did});
    let (status, body) = admin_post(
        update,
        json!({"subject": repo, "takedown": {"applied": true, "ref": "case-1"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["takedown"], json!({"applied": true, "ref": "case-1"}));
    let (_, state) = admin_get(subject.clone()).await;
    assert_eq!(state["takedown"], json!({"applied": true, "ref": "case-1"}));
    assert_eq!(state["deactivated"], json!({"applied": false}));
    let (status, body) = server
        .post(
            "com.atproto.server.refreshSession",
            Some(text(&made, "refreshJwt")),
            &Value::Null,
        )
        .await;
    assert_eq!((status, text(&body, "error")), (401, "AccountTakedown"));
    let (status, _) = admin_post(
        update,
        json!({"subject": repo, "takedown": {"applied": false}}),
    )
    .await;
    assert_eq!(status, 200);
    let (_, state) = admin_get(subject).await;
    assert_eq!(state["takedown"], json!({"applied": false}));
    let events: Vec<String> = common::open(&server.dir.root.join("sequencer.sqlite"))
        .prepare(r#"select "eventType" from "repo_seq" where "did" = ?1 order by "seq""#)
        .unwrap()
        .query_map([&did], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        events,
        [
            "identity", "account", "append", "sync", "account", "account"
        ]
    );

    // A record is taken down and no longer read.
    let (_, session) = server
        .post(
            "com.atproto.server.createSession",
            None,
            &json!({"identifier": did, "password": PASSWORD}),
        )
        .await;
    let access = text(&session, "accessJwt");
    let note = "com.example.test.note";
    let (_, written) = server
        .post(
            "com.atproto.repo.createRecord",
            Some(access),
            &json!({"repo": did, "collection": note, "rkey": "one", "record": {"$type": note}}),
        )
        .await;
    let uri = text(&written, "uri").to_owned();
    let strong = json!({"$type": "com.atproto.repo.strongRef", "uri": uri, "cid": written["cid"]});
    let (status, _) = admin_post(
        update,
        json!({"subject": strong, "takedown": {"applied": true}}),
    )
    .await;
    assert_eq!(status, 200);
    let (_, state) = admin_get(format!("com.atproto.admin.getSubjectStatus?uri={uri}")).await;
    assert_eq!(state["takedown"]["applied"], true);
    assert!(state["takedown"]["ref"].is_string());
    let read = server
        .http
        .get(format!(
            "{}/xrpc/com.atproto.repo.getRecord?repo={did}&collection={note}&rkey=one",
            server.origin
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status().as_u16(), 400);
    let (status, body) = admin_post(
        update,
        json!({"subject": {"$type": "other"}, "takedown": {"applied": true}}),
    )
    .await;
    assert_eq!(
        (status, text(&body, "message")),
        (400, "Invalid subject (other)")
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one end-to-end protocol scenario
async fn the_admin_makes_invite_codes_and_sees_who_used_them() {
    use base64::Engine as _;
    let server = Server::start(&[("PDS_INVITE_REQUIRED", "true")]).await;
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD
            .encode(format!("admin:{}", common::ADMIN_PASSWORD))
    );
    let call = |method: reqwest::Method, path: String, body: Option<Value>| {
        let mut request = server
            .http
            .request(method, format!("{}/xrpc/{path}", server.origin))
            .header("authorization", &basic);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let body: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            (status, body)
        }
    };
    let post = reqwest::Method::POST;
    let get = reqwest::Method::GET;

    let (status, _) = server
        .post(
            "com.atproto.server.createInviteCode",
            None,
            &json!({"useCount": 1}),
        )
        .await;
    assert_eq!(status, 401);
    let (status, made) = call(
        post.clone(),
        "com.atproto.server.createInviteCode".to_owned(),
        Some(json!({"useCount": 2})),
    )
    .await;
    assert_eq!(status, 200, "{made}");
    let code = text(&made, "code").to_owned();
    assert!(
        code.starts_with("pds-test-") && code.len() == "pds-test-".len() + 11,
        "{code}"
    );
    let (_, batch) = call(
        post,
        "com.atproto.server.createInviteCodes".to_owned(),
        Some(json!({"codeCount": 3, "useCount": 1})),
    )
    .await;
    assert_eq!(batch["codes"][0]["account"], "admin");
    assert_eq!(batch["codes"][0]["codes"].as_array().unwrap().len(), 3);

    let (status, joined) = server
        .create(&json!({"handle": "jules.pds-users.org", "email": "jules@example.com", "password": PASSWORD, "inviteCode": code}))
        .await;
    assert_eq!(status, 200, "{joined}");
    let did = text(&joined, "did");

    let (_, info) = call(
        get.clone(),
        format!("com.atproto.admin.getAccountInfo?did={did}"),
        None,
    )
    .await;
    assert_eq!(info["invitedBy"]["code"], code.as_str());
    assert_eq!(info["invitedBy"]["uses"][0]["usedBy"], did);
    assert_eq!(info["invites"], json!([]));
    let (_, infos) = call(
        get.clone(),
        format!(
            "com.atproto.admin.getAccountInfos?dids={did}&dids={}",
            common::ALICE
        ),
        None,
    )
    .await;
    assert_eq!(infos["infos"].as_array().unwrap().len(), 2);

    let (status, listed) = call(
        get.clone(),
        "com.atproto.admin.getInviteCodes?sort=usage&limit=2".to_owned(),
        None,
    )
    .await;
    assert_eq!(status, 200, "{listed}");
    let codes = listed["codes"].as_array().unwrap();
    assert_eq!(codes.len(), 2);
    assert_eq!(
        (&codes[0]["code"], &codes[0]["available"]),
        (&json!(code), &json!(2))
    );
    let cursor = text(&listed, "cursor").to_owned();
    let (_, rest) = call(
        get.clone(),
        format!("com.atproto.admin.getInviteCodes?sort=usage&limit=10&cursor={cursor}"),
        None,
    )
    .await;
    assert_eq!(rest["codes"].as_array().unwrap().len(), 2);
    let (_, recent) = call(
        get.clone(),
        "com.atproto.admin.getInviteCodes".to_owned(),
        None,
    )
    .await;
    assert_eq!(recent["codes"].as_array().unwrap().len(), 4);
    let (status, body) = call(
        get,
        "com.atproto.admin.getInviteCodes?sort=other".to_owned(),
        None,
    )
    .await;
    assert_eq!(
        (status, text(&body, "message")),
        (400, "unknown sort method: other")
    );
}

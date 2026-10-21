use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_device_remembers_an_account_and_a_token_is_revoked() {
    let mut server = Server::start().await;
    server.push(&[]).await;

    let (_, tokens) = server.sign_in(true).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let key = client_key();

    // The device remembers the account: the next flow starts on the consent page.
    let pushed = server.push(&[]).await;
    let request_uri = pushed.json()["request_uri"].as_str().unwrap().to_owned();
    let id = client_id();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &id)
        .append_pair("request_uri", &request_uri)
        .finish();
    let page = server
        .send(
            reqwest::Method::GET,
            &format!("/oauth/authorize?{query}"),
            None,
            &[],
        )
        .await;
    assert!(page.text.contains(">Authorize<"), "{}", page.text);
    assert!(!page.text.contains("name=\"password\""));
    // "Deny access" returns an error to the client.
    let csrf = page.field("csrf").unwrap();
    let form = [
        ("client_id", id.as_str()),
        ("request_uri", request_uri.as_str()),
        ("csrf", csrf.as_str()),
        ("did", ALICE),
    ];
    let denied = server
        .send(
            reqwest::Method::POST,
            "/oauth/authorize/reject",
            Some(&form),
            &[],
        )
        .await;
    assert_eq!(denied.status, 303);
    assert!(
        denied
            .header("location")
            .unwrap()
            .contains("error=access_denied")
    );

    // Revocation ends the session. A token nobody holds is revoked already.
    let revoked = server
        .endpoint("/oauth/revoke", &[("token", &access)])
        .await;
    assert_eq!(revoked.status, 200, "{}", revoked.text);
    let after = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(after.status, 401);
    let unknown = format!("ref-{}", "0".repeat(64));
    let nobody = server
        .endpoint("/oauth/revoke", &[("token", &unknown)])
        .await;
    assert_eq!(nobody.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_loads_only_by_navigation_as_at_the_reference() {
    let mut server = Server::start().await;
    let bare = |path: &str| {
        let request = server.http.get(format!("{}{path}", server.origin));
        async move {
            let response = request.send().await.unwrap();
            (response.status().as_u16(), response.text().await.unwrap())
        }
    };
    // Without any fetch metadata: the first header the Reference checks.
    let (status, text) = bare("/oauth/authorize").await;
    assert_eq!(status, 400);
    assert!(text.contains("Missing sec-fetch-site header"), "{text}");
    let (status, text) = bare("/account").await;
    assert_eq!(status, 400);
    assert!(text.contains("Missing sec-fetch-mode header"), "{text}");

    // What fetch() sends from a script.
    let script = [
        ("sec-fetch-mode", "cors".to_owned()),
        ("sec-fetch-dest", "empty".to_owned()),
    ];
    for path in [
        "/oauth/authorize",
        "/account",
        &format!("/account/{ALICE}/manage"),
    ] {
        let page = server.send(reqwest::Method::GET, path, None, &script).await;
        assert_eq!(page.status, 400, "{path}");
        assert!(
            page.text.contains("Forbidden sec-fetch-mode header"),
            "{path}: {}",
            page.text
        );
    }
    let frame = [("sec-fetch-dest", "iframe".to_owned())];
    let page = server
        .send(reqwest::Method::GET, "/account", None, &frame)
        .await;
    assert!(
        page.text.contains("Forbidden sec-fetch-dest header"),
        "{}",
        page.text
    );
    let other = [("origin", "https://elsewhere.example".to_owned())];
    let page = server
        .send(reqwest::Method::GET, "/account", None, &other)
        .await;
    assert_eq!(page.status, 400);
    assert!(
        page.text
            .contains("Invalid origin https://elsewhere.example"),
        "{}",
        page.text
    );

    // A navigation passes the checks and reaches the page itself, which
    // wants a client.
    let page = server
        .send(reqwest::Method::GET, "/oauth/authorize", None, &[])
        .await;
    assert_eq!(page.status, 400);
    assert!(
        page.text.contains("The request names no client"),
        "{}",
        page.text
    );
    let page = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert_eq!(page.status, 200, "{}", page.text);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_account_page_lists_and_ends_sessions_and_devices() {
    let mut server = Server::start().await;
    server.push(&[]).await;
    let key = client_key();
    let (_, tokens) = server.sign_in(true).await;
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    // The flow remembered the account on this device, so the first page
    // lists it.
    let list = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert_eq!(list.status, 200);
    let manage = format!("/account/{ALICE}/manage");
    assert!(
        list.text.contains(&format!("href=\"{manage}\"")),
        "{}",
        list.text
    );
    let page = server.send(reqwest::Method::GET, &manage, None, &[]).await;
    assert_eq!(page.status, 200);
    assert!(
        page.text.contains("<h2>Sign-in methods</h2>"),
        "{}",
        page.text
    );
    assert!(page.text.contains("A password is set."));
    assert!(
        page.text.contains("<code>http://localhost?"),
        "{}",
        page.text
    );
    assert!(page.text.contains("This device"));
    let csrf = page.field("csrf").unwrap();
    let token_id = page.field("token_id").unwrap();
    let device_id = page.field("device_id").unwrap();
    let form = [
        ("csrf", csrf.as_str()),
        ("did", ALICE),
        ("token_id", token_id.as_str()),
    ];
    let forged = [
        ("csrf", "0000"),
        ("did", ALICE),
        ("token_id", token_id.as_str()),
    ];
    let refused = server
        .send(reqwest::Method::POST, "/account/revoke", Some(&forged), &[])
        .await;
    assert_eq!(refused.status, 403);
    let revoked = server
        .send(reqwest::Method::POST, "/account/revoke", Some(&form), &[])
        .await;
    assert_eq!(
        (revoked.status, revoked.header("location")),
        (303, Some(manage.as_str()))
    );
    let after = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(after.status, 401);
    let form = [
        ("csrf", csrf.as_str()),
        ("did", ALICE),
        ("device_id", device_id.as_str()),
    ];
    server
        .send(reqwest::Method::POST, "/account/sign-out", Some(&form), &[])
        .await;
    // Signed out: the page of the account is closed, and the first page
    // offers to sign in.
    let closed = server.send(reqwest::Method::GET, &manage, None, &[]).await;
    assert_eq!(
        (closed.status, closed.header("location")),
        (303, Some("/account"))
    );
    let welcome = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(welcome.text.contains(">Sign in<"), "{}", welcome.text);
    let signed_out = server
        .send(reqwest::Method::GET, "/account?view=sign-in", None, &[])
        .await;
    assert!(
        signed_out.text.contains("name=\"password\""),
        "{}",
        signed_out.text
    );
    // A sign-in on the page brings the account back.
    let form = [
        ("csrf", csrf.as_str()),
        ("username", ALICE),
        ("password", PASSWORD),
    ];
    let signed_in = server
        .send(reqwest::Method::POST, "/account/sign-in", Some(&form), &[])
        .await;
    assert_eq!(signed_in.status, 303);
    let page = server.send(reqwest::Method::GET, &manage, None, &[]).await;
    assert!(page.text.contains("<h2>Devices</h2>"), "{}", page.text);
}

#[tokio::test]
async fn an_account_with_a_role_operates_through_its_console_session() {
    let console = client_id();
    let mut server = Server::start_with(&[
        ("PDS_PROFILE", "linkjar"),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_OPS_CLIENT_IDS", &console),
    ])
    .await;
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(false).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let who = "/xrpc/internal.ops.getOperator";

    // Without a role the session is refused.
    let refused = server.operate(who, &access, None, None).await;
    assert_eq!(refused.status, 403, "{}", refused.text);
    assert!(refused.header("dpop-nonce").is_some());

    // The admin grants the role of a moderator.
    let grant = json!({"did": ALICE, "role": "moderator"});
    assert_eq!(
        server
            .admin("internal.ops.grantRole", "joins the team", &grant)
            .await,
        200
    );
    let operator = server.operate(who, &access, None, None).await;
    assert_eq!(operator.status, 200, "{}", operator.text);
    assert_eq!(
        operator.json(),
        json!({"actor": ALICE, "role": "moderator"})
    );

    // A moderator acts on one account and grants no role.
    let sync = "/xrpc/internal.ops.emitSync";
    let body = json!({"did": common::DANA});
    let refused = server
        .operate(sync, &access, Some("a test"), Some(&body))
        .await;
    assert_eq!(refused.status, 403, "{}", refused.text);
    let grant = json!({"did": common::DANA, "role": "operator"});
    let refused = server
        .operate(
            "/xrpc/internal.ops.grantRole",
            &access,
            Some("a test"),
            Some(&grant),
        )
        .await;
    assert_eq!(refused.status, 403, "{}", refused.text);
    let takedown = json!({
        "subject": {"$type": "com.atproto.admin.defs#repoRef", "did": common::DANA},
        "takedown": {"applied": true, "ref": "ops-7"},
    });
    let status = "/xrpc/com.atproto.admin.updateSubjectStatus";
    let no_reason = server.operate(status, &access, None, Some(&takedown)).await;
    assert_eq!(no_reason.status, 400, "{}", no_reason.text);
    let done = server
        .operate(status, &access, Some("spam links"), Some(&takedown))
        .await;
    assert_eq!(done.status, 200, "{}", done.text);

    // The entry names the account, its client, the reason and the console.
    let audit = server
        .operate("/xrpc/internal.ops.listAudit?limit=1", &access, None, None)
        .await;
    assert_eq!(audit.status, 200, "{}", audit.text);
    let entry = &audit.json()["entries"][0];
    assert_eq!(entry["action"], "com.atproto.admin.updateSubjectStatus");
    assert_eq!(entry["actor"], ALICE);
    assert_eq!(entry["credential"], "oauth");
    assert_eq!(entry["clientId"], console.as_str());
    assert_eq!(entry["subject"], common::DANA);
    assert_eq!(entry["reason"], "spam links");
    assert_eq!(entry["sidecar"], "console");
    assert_eq!(entry["outcome"], "ok");

    // The admin takes the role away, and the session is refused again.
    let gone = json!({"did": ALICE});
    assert_eq!(
        server
            .admin("internal.ops.revokeRole", "leaves the team", &gone)
            .await,
        200
    );
    let refused = server.operate(who, &access, None, None).await;
    assert_eq!(refused.status, 403, "{}", refused.text);
}

#[tokio::test]
async fn the_ops_listener_refuses_a_session_of_another_client_and_of_the_stock_profile() {
    // The linkjar profile, and a console that is another client.
    let mut server = Server::start_with(&[
        ("PDS_PROFILE", "linkjar"),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_OPS_CLIENT_IDS", "https://console.example/client.json"),
    ])
    .await;
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(false).await;
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let grant = json!({"did": ALICE, "role": "operator"});
    assert_eq!(
        server
            .admin("internal.ops.grantRole", "a test", &grant)
            .await,
        200
    );
    let who = "/xrpc/internal.ops.getOperator";
    let refused = server.operate(who, &access, None, None).await;
    assert_eq!(refused.status, 403, "{}", refused.text);
    assert!(refused.text.contains("not one of the console"));

    // The stock profile grants no role: the admin is the only operator.
    let console = client_id();
    let mut server = Server::start_with(&[
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_OPS_CLIENT_IDS", &console),
    ])
    .await;
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(false).await;
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    assert_eq!(
        server
            .admin("internal.ops.grantRole", "a test", &grant)
            .await,
        200
    );
    let refused = server.operate(who, &access, None, None).await;
    assert_eq!(refused.status, 403, "{}", refused.text);
    assert!(refused.text.contains("grants no operator role"));
}

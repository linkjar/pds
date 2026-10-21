use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn an_unrelated_session_cookie_cannot_take_over_or_delete_a_remembered_device() {
    let mut server = Server::start().await;
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(true).await;
    assert_eq!(tokens["sub"], ALICE);
    let legitimate = server.cookies.clone();
    let device = legitimate
        .split("; ")
        .find(|cookie| cookie.starts_with("dev-id="))
        .unwrap();
    server.cookies = format!("{device}; ses-id=ses-{}", "0".repeat(32));
    let refused = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(
        !refused.text.contains(ALICE),
        "an unrelated session inherited the account"
    );
    assert_ne!(
        server.cookies, legitimate,
        "the response disclosed the genuine cookie"
    );
    server.cookies = legitimate;
    let kept = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(
        kept.text.contains(ALICE),
        "an unauthenticated request deleted the genuine session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_previous_session_cookie_gets_a_fixed_rotation_grace_period() {
    let mut server = Server::start().await;
    server.push(&[]).await;
    server.sign_in(true).await;
    let previous = server.cookies.clone();
    // A stored device idle since yesterday rotates on its next request.
    common::open(&server.dir.root.join("account.sqlite"))
        .execute(
            r#"update "device" set "lastSeenAt" = '2000-01-01T00:00:00.000Z'"#,
            [],
        )
        .unwrap();
    let rotated = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(rotated.text.contains(ALICE));
    let current = server.cookies.clone();
    assert_ne!(previous, current);
    server.cookies = previous.clone();
    let overlap = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(
        overlap.text.contains(ALICE),
        "the exact previous cookie must survive crossed requests"
    );
    assert_eq!(server.cookies, current);
    tokio::time::sleep(std::time::Duration::from_millis(5_100)).await;
    // Current-cookie traffic must not extend the previous cookie's deadline.
    server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    server.cookies = previous;
    let expired = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(!expired.text.contains(ALICE));
    server.cookies = current;
    let kept = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(kept.text.contains(ALICE));
}

#[tokio::test(flavor = "multi_thread")]
async fn password_recovery_ends_oauth_devices_codes_and_app_passwords() {
    let mut server = Server::start().await;
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(true).await;
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let cookies = server.cookies.clone();
    let pushed = server.push(&[]).await;
    let request = pushed.json()["request_uri"].as_str().unwrap().to_owned();
    let back = server.authorize(&request, true).await;
    let code = back
        .query_pairs()
        .find(|(name, _)| name == "code")
        .unwrap()
        .1
        .into_owned();
    let verifier = server.verifier.clone();
    let recovery = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.server.requestPasswordReset",
            server.origin
        ))
        .header("content-type", "application/json")
        .body(json!({"email": "alice@example.com"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(recovery.status(), 200);
    // Read the test mail credential, as the existing account HTTP harness does.
    let reset: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#, [ALICE], |row| row.get(0)).unwrap();
    let reset = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.server.resetPassword",
            server.origin
        ))
        .header("content-type", "application/json")
        .body(json!({"token": reset, "password": "recovered-password"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(reset.status(), 200);
    let denied = server
        .resource(
            &client_key(),
            reqwest::Method::GET,
            GET_SESSION,
            &access,
            None,
        )
        .await;
    assert_eq!(
        denied.status, 401,
        "the old OAuth access token survived password recovery"
    );
    let denied = server
        .endpoint(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh)],
        )
        .await;
    assert_eq!(denied.json()["error"], "invalid_grant");
    let denied = server
        .endpoint(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", &verifier),
                ("redirect_uri", REDIRECT),
            ],
        )
        .await;
    assert_eq!(denied.json()["error"], "invalid_grant");
    server.cookies = cookies;
    let page = server
        .send(reqwest::Method::GET, "/account", None, &[])
        .await;
    assert!(
        !page.text.contains(ALICE),
        "the remembered browser survived password recovery"
    );
    for (password, status) in [
        (PASSWORD, 401),
        (common::APP_PASSWORD, 401),
        ("recovered-password", 200),
    ] {
        let login = server
            .http
            .post(format!(
                "{}/xrpc/com.atproto.server.createSession",
                server.origin
            ))
            .header("content-type", "application/json")
            .body(json!({"identifier": ALICE, "password": password}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), status);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn password_recovery_ends_an_unremembered_sign_in_awaiting_consent() {
    let mut server = Server::start().await;
    server.push(&[]).await;
    let pushed = server.push(&[("login_hint", "")]).await;
    let request = pushed.json()["request_uri"].as_str().unwrap().to_owned();
    let client = client_id();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &client)
        .append_pair("request_uri", &request)
        .finish();
    let page = server
        .send(
            reqwest::Method::GET,
            &format!("/oauth/authorize?{query}&view=sign-in"),
            None,
            &[],
        )
        .await;
    let csrf = page.field("csrf").unwrap();
    let consent = server
        .send(
            reqwest::Method::POST,
            "/oauth/authorize/sign-in",
            Some(&[
                ("client_id", &client),
                ("request_uri", &request),
                ("csrf", &csrf),
                ("username", ALICE),
                ("password", PASSWORD),
            ]),
            &[],
        )
        .await;
    let ephemeral = consent.field("ephemeral").unwrap();
    // Another account signs in on this same, unremembered request. Its
    // association must not keep Alice's earlier proof usable after reset.
    let another = server
        .send(
            reqwest::Method::POST,
            "/oauth/authorize/sign-in",
            Some(&[
                ("client_id", &client),
                ("request_uri", &request),
                ("csrf", &csrf),
                ("username", common::DANA),
                ("password", PASSWORD),
            ]),
            &[],
        )
        .await;
    assert_eq!(another.status, 200, "{}", another.text);
    assert!(another.field("ephemeral").is_some());
    let recovery = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.server.requestPasswordReset",
            server.origin
        ))
        .header("content-type", "application/json")
        .body(json!({"email": "alice@example.com"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(recovery.status(), 200);
    let reset: String = common::open(&server.dir.root.join("account.sqlite"))
        .query_row(r#"select "token" from "email_token" where "did" = ?1 and "purpose" = 'reset_password'"#, [ALICE], |row| row.get(0)).unwrap();
    let reset = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.server.resetPassword",
            server.origin
        ))
        .header("content-type", "application/json")
        .body(json!({"token": reset, "password": "recovered-password"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(reset.status(), 200);
    let accepted = server
        .send(
            reqwest::Method::POST,
            "/oauth/authorize/accept",
            Some(&[
                ("client_id", &client),
                ("request_uri", &request),
                ("csrf", &csrf),
                ("did", ALICE),
                ("ephemeral", &ephemeral),
            ]),
            &[],
        )
        .await;
    assert!(
        accepted
            .header("location")
            .is_none_or(|location| !location.contains("code=")),
        "an old sign-in proof authorized a fresh session after password recovery"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn login_capitalization_shares_the_limit_across_legacy_and_browser_routes() {
    let mut server = Server::start_with(&[("PDS_RATE_LIMITS_ENABLED", "true")]).await;
    let too_long = "x".repeat(513);
    for _ in 0..30 {
        let refused = server
            .http
            .post(format!(
                "{}/xrpc/com.atproto.server.createSession",
                server.origin
            ))
            .header("content-type", "application/json")
            .body(json!({"identifier": "alice@example.com", "password": too_long}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 401);
    }
    let legacy = server
        .http
        .post(format!(
            "{}/xrpc/com.atproto.server.createSession",
            server.origin
        ))
        .header("content-type", "application/json")
        .body(json!({"identifier": "ALICE@EXAMPLE.COM", "password": PASSWORD}).to_string())
        .send()
        .await
        .unwrap();
    let page = server
        .send(reqwest::Method::GET, "/account?view=sign-in", None, &[])
        .await;
    let csrf = page.field("csrf").unwrap();
    let account = server
        .send(
            reqwest::Method::POST,
            "/account/sign-in",
            Some(&[
                ("csrf", &csrf),
                ("username", "Alice@example.com"),
                ("password", PASSWORD),
            ]),
            &[],
        )
        .await;
    server.push(&[]).await;
    let pushed = server.push(&[]).await;
    let request = pushed.json()["request_uri"].as_str().unwrap().to_owned();
    let client = client_id();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id", &client)
        .append_pair("request_uri", &request)
        .finish();
    let page = server
        .send(
            reqwest::Method::GET,
            &format!("/oauth/authorize?{query}"),
            None,
            &[],
        )
        .await;
    let csrf = page.field("csrf").unwrap();
    let oauth = server
        .send(
            reqwest::Method::POST,
            "/oauth/authorize/sign-in",
            Some(&[
                ("client_id", &client),
                ("request_uri", &request),
                ("csrf", &csrf),
                ("username", "alice@EXAMPLE.com"),
                ("password", PASSWORD),
            ]),
            &[],
        )
        .await;
    assert_eq!(
        [legacy.status().as_u16(), account.status, oauth.status],
        [429, 429, 429]
    );
}

// One flow, in the order a client goes through it.

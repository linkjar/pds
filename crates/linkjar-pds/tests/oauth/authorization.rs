use super::*;

// A browser client reads the nonce only when the answer names it in
// `Access-Control-Expose-Headers`. The rate limits run in production, and
// their headers join the list instead of taking its place.
#[tokio::test(flavor = "multi_thread")]
async fn a_browser_can_read_the_nonce_while_the_rate_limits_run() {
    let mut server = Server::start_with(&[("PDS_RATE_LIMITS_ENABLED", "true")]).await;
    // The first request has no nonce and is asked to use one.
    let first = server.push(&[]).await;
    assert_eq!(first.json()["error"], "use_dpop_nonce", "{}", first.text);
    let pushed = server.push(&[]).await;
    assert_eq!(pushed.status, 201, "{}", pushed.text);
    for answer in [&first, &pushed] {
        assert!(answer.header("dpop-nonce").is_some());
        assert!(answer.header("ratelimit-limit").is_some());
        let exposed: Vec<String> = answer
            .header("access-control-expose-headers")
            .unwrap()
            .split(',')
            .map(|name| name.trim().to_ascii_lowercase())
            .collect();
        for name in ["dpop-nonce", "www-authenticate", "ratelimit-limit"] {
            assert!(exposed.iter().any(|own| own == name), "{name}: {exposed:?}");
        }
    }
}

#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread")]
async fn a_client_is_authorized_and_its_session_is_used_refreshed_and_revoked() {
    let mut server = Server::start().await;
    let resource = server
        .send(
            reqwest::Method::GET,
            "/.well-known/oauth-protected-resource",
            None,
            &[],
        )
        .await;
    assert_eq!(
        resource.json()["authorization_servers"],
        json!([server.issuer])
    );

    // A browser on another origin may call the token endpoint.
    let preflight = server
        .send(reqwest::Method::OPTIONS, "/oauth/token", None, &[])
        .await;
    assert_eq!(preflight.status, 200);
    for (name, value) in [
        ("access-control-allow-origin", "*"),
        ("access-control-allow-methods", "*"),
        ("access-control-allow-headers", "Content-Type,DPoP"),
        ("access-control-max-age", "86400"),
    ] {
        assert_eq!(preflight.header(name), Some(value), "{name}");
    }

    // The first request has no nonce. The server gives one.
    let first = server.push(&[]).await;
    assert_eq!(
        (first.status, first.json()["error"].as_str()),
        (400, Some("use_dpop_nonce"))
    );
    assert_ne!(server.nonce, "");

    // What a pushed request is refused for.
    for (change, error) in [
        (("code_challenge", ""), "invalid_request"),
        (("code_challenge_method", "plain"), "invalid_request"),
        (
            ("redirect_uri", "http://127.0.0.1/elsewhere"),
            "invalid_request",
        ),
        (("scope", "atproto transition:chat.bsky"), "invalid_scope"),
        (("scope", "transition:generic"), "invalid_scope"),
        (("response_type", "token"), "unsupported_response_type"),
    ] {
        let refused = server.push(&[change]).await;
        assert_eq!(
            (refused.status, refused.json()["error"].as_str()),
            (400, Some(error)),
            "{change:?}: {}",
            refused.text
        );
    }

    // The endpoint does not take the parameters themselves.
    let direct = server
        .send(
            reqwest::Method::GET,
            &format!(
                "/oauth/authorize?client_id={}&response_type=code",
                "http%3A%2F%2Flocalhost"
            ),
            None,
            &[],
        )
        .await;
    assert_eq!(direct.status, 400);

    // A wrong password stays on the sign-in page.
    let pushed = server.push(&[]).await;
    assert_eq!(pushed.status, 201, "{}", pushed.text);
    assert_eq!(pushed.json()["expires_in"], 300);
    let request_uri = pushed.json()["request_uri"].as_str().unwrap().to_owned();
    assert!(request_uri.starts_with("urn:ietf:params:oauth:request_uri:req-"));
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
    assert!(page.text.contains("name=\"password\""), "{}", page.text);
    let csp = page.header("content-security-policy").unwrap();
    assert!(
        csp.contains("frame-ancestors 'none'") && csp.contains("font-src 'self'"),
        "{csp}"
    );
    // The typeface and the mark come from the server itself, kept for a year.
    assert!(
        page.text.contains("src=\"/oauth/assets/stock-mark.svg\""),
        "{}",
        page.text
    );
    let font = server
        .send(
            reqwest::Method::GET,
            "/oauth/assets/geist-latin.woff2",
            None,
            &[],
        )
        .await;
    assert_eq!(font.status, 200);
    assert_eq!(font.header("content-type"), Some("font/woff2"));
    assert_eq!(
        font.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    let missing = server
        .send(
            reqwest::Method::GET,
            "/oauth/assets/missing.woff2",
            None,
            &[],
        )
        .await;
    assert_eq!(missing.status, 404);
    let csrf = page.field("csrf").unwrap();
    let form = [
        ("client_id", id.as_str()),
        ("request_uri", request_uri.as_str()),
        ("csrf", csrf.as_str()),
        ("username", ALICE),
        ("password", "not-the-password"),
    ];
    let wrong = server
        .send(
            reqwest::Method::POST,
            "/oauth/authorize/sign-in",
            Some(&form),
            &[],
        )
        .await;
    assert_eq!(wrong.status, 400);
    assert!(
        wrong
            .text
            .contains("<div role=\"alert\">Invalid identifier or password</div>")
    );
    // A form without the token of the page is refused.
    let forged = [
        ("client_id", id.as_str()),
        ("request_uri", request_uri.as_str()),
        ("csrf", "0000"),
        ("username", ALICE),
        ("password", PASSWORD),
    ];
    let forged = server
        .send(
            reqwest::Method::POST,
            "/oauth/authorize/sign-in",
            Some(&forged),
            &[],
        )
        .await;
    assert_eq!(forged.status, 403);

    // The flow.
    let (code, tokens) = server.sign_in(false).await;
    assert_eq!(tokens["token_type"], "DPoP", "{tokens}");
    assert_eq!(tokens["sub"], ALICE);
    assert_eq!(tokens["scope"], SCOPE);
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let key = client_key();

    let session = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(session.status, 200, "{}", session.text);
    assert_eq!(session.json()["did"], ALICE);
    assert_eq!(
        session.json().get("email"),
        None,
        "the email needs transition:email"
    );
    assert!(session.header("dpop-nonce").is_some());

    let record = json!({
        "repo": ALICE,
        "collection": "com.example.note",
        "record": { "$type": "com.example.note", "via": "oauth" },
    });
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
    let managed = server
        .resource(
            &key,
            reqwest::Method::POST,
            "/xrpc/com.atproto.server.createAppPassword",
            &access,
            Some(&json!({ "name": "x" })),
        )
        .await;
    assert_eq!(
        (managed.status, managed.json()["error"].as_str()),
        (403, Some("Forbidden"))
    );

    // The token is bound to its key.
    let other = Keypair::generate(Algorithm::P256, |bytes| bytes.fill(9));
    let stolen = server
        .resource(&other, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(
        (stolen.status, stolen.json()["error"].as_str()),
        (401, Some("invalid_token"))
    );
    assert_eq!(stolen.json()["message"], "Invalid DPoP key binding");
    let bearer = server
        .http
        .get(format!("{}{GET_SESSION}", server.origin))
        .bearer_auth(&access)
        .send()
        .await
        .unwrap();
    assert_eq!(bearer.status().as_u16(), 400);
    let bare = server
        .http
        .get(format!("{}{GET_SESSION}", server.origin))
        .header("authorization", format!("DPoP {access}"))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status().as_u16(), 401);
    assert!(
        bare.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .starts_with("DPoP error=\"invalid_dpop_proof\"")
    );

    // A refresh rotates the token. The old one ends the session when it comes again.
    let rotated = server
        .endpoint(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh)],
        )
        .await;
    assert_eq!(rotated.status, 200, "{}", rotated.text);
    let next = rotated.json();
    assert_ne!(next["refresh_token"], tokens["refresh_token"]);
    let next_access = next["access_token"].as_str().unwrap().to_owned();
    let old_access = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(
        old_access.status, 401,
        "the token before the rotation is gone"
    );
    let again = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &next_access, None)
        .await;
    assert_eq!(again.status, 200, "{}", again.text);
    let reused = server
        .endpoint(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh)],
        )
        .await;
    assert_eq!(reused.json()["error"], "invalid_grant", "{}", reused.text);
    let successor = server
        .endpoint(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", next["refresh_token"].as_str().unwrap()),
            ],
        )
        .await;
    assert_eq!(successor.json()["error"], "invalid_grant");
    let ended = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &next_access, None)
        .await;
    assert_eq!(ended.status, 401);

    // A code is good once.
    let verifier = server.verifier.clone();
    let replay = server
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
    assert_eq!(replay.json()["error"], "invalid_grant");
}

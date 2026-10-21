use super::*;

const CHAT_METHODS: &[&str] = &[
    "chat.bsky.convo.acceptConvo",
    "chat.bsky.convo.futureMethod",
    "CHAT.BSKY.CONVO.FUTUREMETHOD",
    "chat.bsky.convo.getConvo",
];

async fn login(server: &Server, password: &str) -> String {
    let (status, _, body) = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.server.createSession",
                    server.origin
                ))
                .header("content-type", "application/json")
                .body(json!({"identifier": ALICE, "password": password}).to_string()),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body["accessJwt"].as_str().unwrap().to_owned()
}

async fn privileged_access(server: &Server, full: &str) -> String {
    let (status, _, body) = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.server.createAppPassword",
                    server.origin
                ))
                .bearer_auth(full)
                .header("content-type", "application/json")
                .body(json!({"name": "chat", "privileged": true}).to_string()),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    login(server, body["password"].as_str().unwrap()).await
}

fn service_token(server: &Server, access: &str, method: &str) -> reqwest::RequestBuilder {
    server
        .get(&format!(
            "com.atproto.server.getServiceAuth?aud=did:web:chat.test&lxm={method}"
        ))
        .bearer_auth(access)
}

#[tokio::test]
async fn ordinary_app_passwords_cannot_request_new_chat_service_tokens() {
    let (appview, _) = appview().await;
    let server = Server::start(&appview).await;
    let limited = login(&server, common::APP_PASSWORD).await;
    for method in CHAT_METHODS {
        let (status, _, body) = server.send(service_token(&server, &limited, method)).await;
        assert_eq!(
            (status, &body["error"]),
            (400, &json!("InvalidRequest")),
            "{method}"
        );
        assert_eq!(
            body["message"],
            format!(
                "insufficient access to request a service auth token for the following method: {method}"
            )
        );
    }
}

#[tokio::test]
async fn full_and_privileged_sessions_can_request_new_chat_service_tokens() {
    let (appview, _) = appview().await;
    let server = Server::start(&appview).await;
    let full = login(&server, PASSWORD).await;
    let privileged = privileged_access(&server, &full).await;
    for access in [&full, &privileged] {
        for method in CHAT_METHODS {
            let (status, _, body) = server.send(service_token(&server, access, method)).await;
            assert_eq!(status, 200, "{method}: {body}");
            let token = body["token"].as_str().unwrap();
            let payload = claims(token);
            assert_eq!(payload["lxm"], *method);
            assert_eq!(payload["iss"], ALICE);
            assert_eq!(payload["aud"], "did:web:chat.test");
            assert!(signed_by(token, &common::key(0)));
        }
    }
}

#[tokio::test]
async fn new_chat_proxy_methods_require_full_or_privileged_sessions() {
    let (appview, seen) = appview().await;
    let server = Server::start(&appview).await;
    let limited = login(&server, common::APP_PASSWORD).await;
    let full = login(&server, PASSWORD).await;
    let privileged = privileged_access(&server, &full).await;
    let proxy = |access: &str, method: &str| {
        // acceptConvo is a procedure; exercise POST as well as query routing.
        let request = if method == "chat.bsky.convo.acceptConvo" {
            server
                .http
                .post(format!("{}/xrpc/{method}", server.origin))
                .header("content-type", "application/json")
                .body(json!({"convoId": "conversation"}).to_string())
        } else {
            server.get(method)
        };
        request.bearer_auth(access)
    };
    for method in CHAT_METHODS {
        let (status, _, body) = server.send(proxy(&limited, method)).await;
        assert_eq!(
            (status, &body["error"]),
            (400, &json!("InvalidToken")),
            "{method}"
        );
        assert_eq!(body["message"], "Bad token method");
    }
    assert!(
        seen.lock().unwrap().is_empty(),
        "an unauthorized request reached the service"
    );
    for access in [&full, &privileged] {
        for method in CHAT_METHODS {
            let (status, _, body) = server.send(proxy(access, method)).await;
            assert_eq!(status, 200, "{method}: {body}");
        }
    }
    let paths: Vec<_> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.1.clone())
        .collect();
    assert_eq!(
        paths,
        [
            "/xrpc/chat.bsky.convo.acceptConvo",
            "/xrpc/chat.bsky.convo.futureMethod",
            "/xrpc/CHAT.BSKY.CONVO.FUTUREMETHOD",
            "/xrpc/chat.bsky.convo.getConvo",
            "/xrpc/chat.bsky.convo.acceptConvo",
            "/xrpc/chat.bsky.convo.futureMethod",
            "/xrpc/CHAT.BSKY.CONVO.FUTUREMETHOD",
            "/xrpc/chat.bsky.convo.getConvo",
        ]
    );
}

#[tokio::test]
async fn chat_namespace_guard_preserves_other_methods_and_create_account_case_policy() {
    let (appview, _) = appview().await;
    let server = Server::start(&appview).await;
    let limited = login(&server, common::APP_PASSWORD).await;
    let full = login(&server, PASSWORD).await;
    let privileged = privileged_access(&server, &full).await;
    for method in [
        "com.atproto.server.createAccount",
        "COM.ATPROTO.SERVER.CREATEACCOUNT",
    ] {
        let (status, _, body) = server.send(service_token(&server, &limited, method)).await;
        assert_eq!(
            (status, &body["error"]),
            (400, &json!("InvalidRequest")),
            "{method}"
        );
        for access in [&full, &privileged] {
            let (status, _, body) = server.send(service_token(&server, access, method)).await;
            assert_eq!(status, 200, "{method}: {body}");
            assert_eq!(claims(body["token"].as_str().unwrap())["lxm"], method);
        }
    }
    for method in [
        "chat.bskyish.futureMethod",
        "com.atproto.server.createAccountExtra",
    ] {
        let (status, _, body) = server.send(service_token(&server, &limited, method)).await;
        assert_eq!(status, 200, "{method}: {body}");
        assert_eq!(claims(body["token"].as_str().unwrap())["lxm"], method);
        let (status, _, body) = server.send(server.get(method).bearer_auth(&limited)).await;
        assert_eq!(status, 200, "{method}: {body}");
    }
}

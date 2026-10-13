//! The proxy and service tokens over HTTP (`docs/SPEC.md` §5.4).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    missing_docs
)]

mod common;

use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{ALICE, DataDir, PASSWORD};
use linkjar_pds::serve::app_state;
use serde_json::{Value, json};

/// What the stand-in AppView received: method, path with query, headers.
type Seen = Arc<Mutex<Vec<(String, String, Vec<(String, String)>, Vec<u8>)>>>;

async fn appview() -> (String, Seen) {
    let seen = Seen::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .fallback(|State(seen): State<Seen>, request: Request| async move {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, usize::MAX)
                .await
                .unwrap()
                .to_vec();
            let headers = parts
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.to_string(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
            let path = parts.uri.path_and_query().unwrap().to_string();
            seen.lock()
                .unwrap()
                .push((parts.method.to_string(), path.clone(), headers, bytes));
            if path.contains("app.bsky.feed.getTimeline") {
                return (
                    [
                        ("content-type", "application/json"),
                        ("atproto-repo-rev", "3abc"),
                        ("x-private", "no"),
                    ],
                    r#"{"feed":[]}"#,
                )
                    .into_response();
            }
            if path.contains("app.bsky.actor.getProfile") {
                return (
                    StatusCode::BAD_REQUEST,
                    [("content-type", "application/json")],
                    r#"{"error":"InvalidRequest","message":"Profile not found"}"#,
                )
                    .into_response();
            }
            if path.contains("app.bsky.unspecced.broken") {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            (StatusCode::OK, [("content-type", "application/json")], "{}").into_response()
        })
        .with_state(Arc::clone(&seen));
    tokio::spawn(async move { axum::serve(listener, app).await });
    (origin, seen)
}

struct Server {
    origin: String,
    http: reqwest::Client,
    _dir: DataDir,
}

impl Server {
    async fn start(appview: &str) -> Self {
        let dir = DataDir::new_with(
            1,
            &[
                ("PDS_BSKY_APP_VIEW_URL", appview),
                ("PDS_BSKY_APP_VIEW_DID", "did:web:appview.test"),
            ],
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = pds_xrpc::router(app_state(&dir.config).unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self {
            origin,
            http: reqwest::Client::new(),
            _dir: dir,
        }
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> (u16, reqwest::header::HeaderMap, Value) {
        let response = request.send().await.unwrap();
        let (status, headers) = (response.status().as_u16(), response.headers().clone());
        let bytes = response.bytes().await.unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.get(format!("{}/xrpc/{path}", self.origin))
    }
}

fn claims(token: &str) -> Value {
    let body = token.split('.').nth(1).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body).unwrap()).unwrap()
}

#[tokio::test]
async fn a_method_without_a_handler_goes_to_the_appview_with_a_service_token() {
    let (appview, seen) = appview().await;
    let server = Server::start(&appview).await;
    let (_, _, session) = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.server.createSession",
                    server.origin
                ))
                .header("content-type", "application/json")
                .body(json!({"identifier": ALICE, "password": PASSWORD}).to_string()),
        )
        .await;
    let access = session["accessJwt"].as_str().unwrap().to_owned();

    // Without a session nothing is sent.
    let timeline = "app.bsky.feed.getTimeline?limit=3&cursor=a%20b";
    let (status, _, body) = server.send(server.get(timeline)).await;
    assert_eq!((status, &body["error"]), (401, &json!("AuthMissing")));
    assert!(seen.lock().unwrap().is_empty());

    let (status, headers, body) = server
        .send(
            server
                .get(timeline)
                .bearer_auth(&access)
                .header("accept-language", "sr")
                .header("x-atproto-test", "1")
                .header("cookie", "secret=1"),
        )
        .await;
    assert_eq!((status, &body), (200, &json!({"feed": []})));
    assert_eq!(headers["atproto-repo-rev"], "3abc");
    assert!(!headers.contains_key("x-private"));
    {
        let seen = seen.lock().unwrap();
        let (method, path, sent, _) = &seen[0];
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("GET", format!("/xrpc/{timeline}").as_str())
        );
        let header = |name: &str| {
            sent.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(header("accept-language"), Some("sr"));
        assert_eq!(header("x-atproto-test"), Some("1"));
        assert_eq!(header("cookie"), None);
        // The token is the account's: its key signed it for this method.
        let token = header("authorization")
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap();
        let claims = claims(token);
        assert_eq!(
            (&claims["iss"], &claims["aud"], &claims["lxm"]),
            (
                &json!(ALICE),
                &json!("did:web:appview.test"),
                &json!("app.bsky.feed.getTimeline")
            )
        );
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            60
        );
        let (signed, signature) = token.rsplit_once('.').unwrap();
        common::key(0)
            .public_key()
            .verify(
                signed.as_bytes(),
                &URL_SAFE_NO_PAD.decode(signature).unwrap(),
            )
            .unwrap();
    }

    // A refusal of the other service is passed on; its failure is 502.
    let (status, _, body) = server
        .send(
            server
                .get("app.bsky.actor.getProfile?actor=nobody")
                .bearer_auth(&access),
        )
        .await;
    assert_eq!(
        (status, &body),
        (
            400,
            &json!({"error": "InvalidRequest", "message": "Profile not found"})
        )
    );
    let (status, _, body) = server
        .send(server.get("app.bsky.unspecced.broken").bearer_auth(&access))
        .await;
    assert_eq!((status, &body["error"]), (502, &json!("UpstreamFailure")));

    // A procedure is sent with its body.
    let (status, _, _) = server
        .send(
            server
                .http
                .post(format!("{}/xrpc/app.bsky.graph.muteActor", server.origin))
                .bearer_auth(&access)
                .header("content-type", "application/json")
                .body(r#"{"actor":"did:plc:x"}"#),
        )
        .await;
    assert_eq!(status, 200);
    {
        let seen = seen.lock().unwrap();
        let (method, _, sent, bytes) = seen.last().unwrap();
        assert_eq!(method, "POST");
        assert_eq!(bytes, br#"{"actor":"did:plc:x"}"#);
        assert!(
            sent.iter()
                .any(|(key, value)| key == "content-type" && value == "application/json")
        );
    }

    // Account management is never proxied, and the header is checked.
    let (status, _, body) = server
        .send(
            server
                .get("com.atproto.admin.sendEmail")
                .bearer_auth(&access),
        )
        .await;
    assert_eq!(
        (status, &body["message"]),
        (400, &json!("Bad token method"))
    );
    for (value, message) in [
        (
            "did:web:other.test",
            "no service id specified in proxy header",
        ),
        ("#bsky_appview", "no did specified in proxy header"),
        ("did:web:a.test#one#two", "invalid proxy header format"),
        ("not-a-did#service", "could not resolve proxy did"),
    ] {
        let (status, _, body) = server
            .send(
                server
                    .get(timeline)
                    .bearer_auth(&access)
                    .header("atproto-proxy", value),
            )
            .await;
        assert_eq!(
            (status, &body["message"]),
            (400, &json!(message)),
            "{value}"
        );
    }
    let (status, _, _) = server
        .send(
            server
                .get(timeline)
                .bearer_auth(&access)
                .header("atproto-proxy", "did:web:appview.test#bsky_appview"),
        )
        .await;
    assert_eq!(status, 200);

    // `getServiceAuth` gives the same kind of token for a method the
    // caller names.
    let (status, _, body) = server
        .send(
            server
                .get("com.atproto.server.getServiceAuth?aud=did:web:other.test&lxm=com.example.method")
                .bearer_auth(&access),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let claims = claims(body["token"].as_str().unwrap());
    assert_eq!(
        (&claims["iss"], &claims["aud"], &claims["lxm"]),
        (
            &json!(ALICE),
            &json!("did:web:other.test"),
            &json!("com.example.method")
        )
    );
    for (query, error) in [
        ("aud=nothing", "InvalidRequest"),
        ("aud=did:web:a.test&exp=1", "BadExpiration"),
        ("aud=did:web:a.test&exp=99999999999", "BadExpiration"),
        (
            "aud=did:web:a.test&lxm=com.atproto.server.getSession",
            "InvalidRequest",
        ),
    ] {
        let (status, _, body) = server
            .send(
                server
                    .get(&format!("com.atproto.server.getServiceAuth?{query}"))
                    .bearer_auth(&access),
            )
            .await;
        assert_eq!((status, &body["error"]), (400, &json!(error)), "{query}");
    }
}

#[tokio::test]
async fn preferences_stay_on_this_server() {
    let (appview, seen) = appview().await;
    let server = Server::start(&appview).await;
    let login = |password: &'static str| {
        let request = server
            .http
            .post(format!(
                "{}/xrpc/com.atproto.server.createSession",
                server.origin
            ))
            .header("content-type", "application/json")
            .body(json!({"identifier": ALICE, "password": password}).to_string());
        async {
            server.send(request).await.2["accessJwt"]
                .as_str()
                .unwrap()
                .to_owned()
        }
    };
    let (full, limited) = (login(PASSWORD).await, login(common::APP_PASSWORD).await);
    let get = "app.bsky.actor.getPreferences";
    let put = |token: &str, preferences: Value| {
        server
            .http
            .post(format!(
                "{}/xrpc/app.bsky.actor.putPreferences",
                server.origin
            ))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(json!({ "preferences": preferences }).to_string())
    };

    let (status, _, body) = server.send(server.get(get).bearer_auth(&full)).await;
    assert_eq!((status, &body), (200, &json!({"preferences": []})));
    let (status, _, body) = server.send(server.get(get)).await;
    assert_eq!((status, &body["error"]), (401, &json!("AuthMissing")));

    let adult = json!({"$type": "app.bsky.actor.defs#adultContentPref", "enabled": false});
    let personal = json!({"$type": "app.bsky.actor.defs#personalDetailsPref", "birthDate": "1990-01-01T00:00:00.000Z"});
    assert_eq!(
        server.send(put(&full, json!([adult, personal]))).await.0,
        200
    );
    let (_, _, body) = server.send(server.get(get).bearer_auth(&full)).await;
    let stored = body["preferences"].as_array().unwrap();
    assert_eq!(stored.len(), 3);
    assert_eq!((&stored[0], &stored[1]), (&adult, &personal));
    assert_eq!(
        stored[2],
        json!({"$type": "app.bsky.actor.defs#declaredAgePref", "isOverAge13": true, "isOverAge16": true, "isOverAge18": true})
    );
    // An app password reads and writes everything but the personal details.
    let (_, _, body) = server.send(server.get(get).bearer_auth(&limited)).await;
    let types: Vec<&str> = body["preferences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pref| pref["$type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "app.bsky.actor.defs#adultContentPref",
            "app.bsky.actor.defs#declaredAgePref"
        ]
    );
    let (status, _, body) = server.send(put(&limited, json!([personal]))).await;
    assert_eq!(status, 400);
    assert_eq!(
        body["message"],
        "Do not have authorization to set preferences: app.bsky.actor.defs#personalDetailsPref"
    );
    let muted = json!({"$type": "app.bsky.actor.defs#mutedWordsPref", "items": []});
    assert_eq!(server.send(put(&limited, json!([muted]))).await.0, 200);
    let (_, _, body) = server.send(server.get(get).bearer_auth(&full)).await;
    let types: Vec<&str> = body["preferences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pref| pref["$type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "app.bsky.actor.defs#personalDetailsPref",
            "app.bsky.actor.defs#mutedWordsPref",
            "app.bsky.actor.defs#declaredAgePref"
        ]
    );

    for (preferences, message) in [
        (
            json!([{"$type": "com.example.pref", "on": true}]),
            "Some preferences are not in the app.bsky namespace",
        ),
        (json!([{"enabled": true}]), "Preference is missing a $type"),
    ] {
        let (status, _, body) = server.send(put(&full, preferences)).await;
        assert_eq!((status, &body["message"]), (400, &json!(message)));
    }
    // Nothing of this reached the AppView.
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_service_token_of_the_account_uploads_a_blob() {
    let (appview, _seen) = appview().await;
    let server = Server::start(&appview).await;
    let (_, _, session) = server
        .send(
            server
                .http
                .post(format!(
                    "{}/xrpc/com.atproto.server.createSession",
                    server.origin
                ))
                .header("content-type", "application/json")
                .body(json!({"identifier": ALICE, "password": PASSWORD}).to_string()),
        )
        .await;
    let access = session["accessJwt"].as_str().unwrap().to_owned();
    let token_for = |aud: &'static str, lxm: &'static str| {
        let request = server
            .get(&format!(
                "com.atproto.server.getServiceAuth?aud={aud}&lxm={lxm}"
            ))
            .bearer_auth(&access);
        async {
            server.send(request).await.2["token"]
                .as_str()
                .unwrap()
                .to_owned()
        }
    };
    let upload = |token: String| {
        let request = server
            .http
            .post(format!(
                "{}/xrpc/com.atproto.repo.uploadBlob",
                server.origin
            ))
            .bearer_auth(token)
            .header("content-type", "text/plain")
            .body("service auth upload");
        async { server.send(request).await }
    };
    let own = "did:web:pds.test";
    let method = "com.atproto.repo.uploadBlob";

    let (status, _, body) = upload(token_for(own, method).await).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["blob"]["size"], 19);

    let (status, _, body) = upload(token_for(own, "com.atproto.repo.createRecord").await).await;
    assert_eq!(
        (status, &body["error"]),
        (401, &json!("BadJwtLexiconMethod"))
    );
    let (status, _, body) = upload(token_for("did:web:other.test", method).await).await;
    assert_eq!((status, &body["error"]), (401, &json!("BadJwtAudience")));
    // A token that another key signed.
    let forged = {
        let token = token_for(own, method).await;
        let (signed, _) = token.rsplit_once('.').unwrap();
        let signature = common::key(9).sign(signed.as_bytes());
        format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature))
    };
    // The server asks the directory for a newer key before it refuses. No
    // directory answers in this test, so the refusal names that.
    let (status, _, body) = upload(forged).await;
    assert_eq!(status, 401, "{body}");
}

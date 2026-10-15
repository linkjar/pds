//! The OAuth authorization server over HTTP (`docs/SPEC.md` §11): one flow
//! of a loopback client from the pushed request to revocation, with the
//! refusals on the way.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

mod common;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{ALICE, DataDir, PASSWORD};
use linkjar_pds::serve::app_state;
use pds_identity::{Algorithm, Keypair};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const REDIRECT: &str = "http://127.0.0.1/callback";
const SCOPE: &str = "atproto transition:generic";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
/// The fetch metadata a browser sends when it follows a link to a page.
const NAVIGATION: [(&str, &str); 3] = [
    ("sec-fetch-site", "cross-site"),
    ("sec-fetch-mode", "navigate"),
    ("sec-fetch-dest", "document"),
];

/// The DPoP key of the client.
fn client_key() -> Keypair {
    Keypair::generate(Algorithm::P256, |bytes| bytes.fill(7))
}

fn client_id() -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("scope", SCOPE)
        .finish();
    format!("http://localhost?{query}")
}

struct Answer {
    status: u16,
    headers: reqwest::header::HeaderMap,
    text: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    /// The value of a hidden field of a page.
    fn field(&self, name: &str) -> Option<String> {
        let marker = format!("name=\"{name}\" value=\"");
        let start = self.text.find(&marker)? + marker.len();
        let end = self.text[start..].find('"')? + start;
        Some(self.text[start..end].replace("&amp;", "&"))
    }
}

struct Server {
    origin: String,
    ops_origin: String,
    issuer: String,
    http: reqwest::Client,
    nonce: String,
    cookies: String,
    serial: u32,
    /// The PKCE verifier of the last pushed request.
    verifier: String,
    dir: DataDir,
}

impl Server {
    async fn start() -> Self {
        Self::start_with(&[]).await
    }

    /// A server with more variables of the configuration, and its ops
    /// listener on an address of its own.
    async fn start_with(extra: &[(&str, &str)]) -> Self {
        Self::start_in(DataDir::new_with(1, extra)).await
    }

    /// A server on a data directory that exists.
    async fn start_in(dir: DataDir) -> Self {
        linkjar_pds::serve::create_missing_databases(&dir.config).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = app_state(&dir.config).unwrap();
        let router = pds_xrpc::router(state.clone());
        tokio::spawn(async move { axum::serve(listener, router).await });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ops_origin = format!("http://{}", listener.local_addr().unwrap());
        let router = linkjar_pds::serve::ops_router(state, &dir.config);
        tokio::spawn(async move { axum::serve(listener, router).await });
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut server = Self {
            origin,
            ops_origin,
            issuer: String::new(),
            http,
            nonce: String::new(),
            cookies: String::new(),
            serial: 0,
            verifier: String::new(),
            dir,
        };
        let metadata = server
            .send(
                reqwest::Method::GET,
                "/.well-known/oauth-protected-resource",
                None,
                &[],
            )
            .await;
        assert_eq!(metadata.status, 200);
        // The address the server knows itself by. A member host of an
        // entryway has no authorization server metadata to read it from.
        metadata.json()["resource"]
            .as_str()
            .unwrap()
            .clone_into(&mut server.issuer);
        server
    }

    fn proof(&mut self, key: &Keypair, method: &str, path: &str, token: Option<&str>) -> String {
        let point = key.public_key().to_uncompressed();
        let header = json!({
            "typ": "dpop+jwt",
            "alg": "ES256",
            "jwk": {
                "kty": "EC", "crv": "P-256",
                "x": URL_SAFE_NO_PAD.encode(&point[1..33]),
                "y": URL_SAFE_NO_PAD.encode(&point[33..65]),
            },
        });
        self.serial += 1;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut claims = json!({
            "jti": format!("proof-{}", self.serial),
            "htm": method,
            "htu": format!("{}{}", self.issuer, path.split('?').next().unwrap()),
            "iat": now,
        });
        if !self.nonce.is_empty() {
            claims["nonce"] = json!(self.nonce);
        }
        if let Some(token) = token {
            claims["ath"] = json!(URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes())));
        }
        let signed = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(header.to_string()),
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(key.sign(signed.as_bytes()))
        )
    }

    async fn send(
        &mut self,
        method: reqwest::Method,
        path: &str,
        form: Option<&[(&str, &str)]>,
        headers: &[(&str, String)],
    ) -> Answer {
        let navigates = method == reqwest::Method::GET;
        let mut request = self.http.request(method, format!("{}{path}", self.origin));
        if navigates {
            // What a browser sends when it loads a page (SPEC §11.5). A test
            // that sends other values names them in `headers`.
            for (name, value) in NAVIGATION {
                if !headers
                    .iter()
                    .any(|(given, _)| given.eq_ignore_ascii_case(name))
                {
                    request = request.header(name, value);
                }
            }
        }
        if !self.cookies.is_empty() {
            request = request.header("cookie", &self.cookies);
        }
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        if let Some(form) = form {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(form)
                .finish();
            request = request
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let cookies: Vec<&str> = headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.split(';').next())
            .collect();
        if !cookies.is_empty() {
            self.cookies = cookies.join("; ");
        }
        if let Some(nonce) = headers.get("dpop-nonce").and_then(|v| v.to_str().ok()) {
            nonce.clone_into(&mut self.nonce);
        }
        Answer {
            status,
            headers,
            text: response.text().await.unwrap(),
        }
    }

    /// A request to an endpoint of the authorization server, with a proof.
    async fn endpoint(&mut self, path: &str, form: &[(&str, &str)]) -> Answer {
        let key = client_key();
        let proof = self.proof(&key, "POST", path, None);
        let id = client_id();
        let mut all = vec![("client_id", id.as_str())];
        all.extend_from_slice(form);
        self.send(reqwest::Method::POST, path, Some(&all), &[("dpop", proof)])
            .await
    }

    /// A request to the resource server with an OAuth access token.
    async fn resource(
        &mut self,
        key: &Keypair,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<&Value>,
    ) -> Answer {
        let proof = self.proof(key, method.as_str(), path, Some(token));
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.origin))
            .header("authorization", format!("DPoP {token}"))
            .header("dpop", proof);
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.unwrap();
        Answer {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            text: response.text().await.unwrap(),
        }
    }

    /// A request to the ops listener with an OAuth access token. The
    /// proof names the public address of the server.
    async fn operate(
        &mut self,
        path: &str,
        token: &str,
        reason: Option<&str>,
        body: Option<&Value>,
    ) -> Answer {
        let method = if body.is_some() {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };
        let proof = self.proof(&client_key(), method.as_str(), path, Some(token));
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.ops_origin))
            .header("authorization", format!("DPoP {token}"))
            .header("dpop", proof);
        if let Some(reason) = reason {
            request = request.header("x-ops-reason", reason);
        }
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = request.send().await.unwrap();
        Answer {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            text: response.text().await.unwrap(),
        }
    }

    /// An action of the admin on the ops listener.
    async fn admin(&self, nsid: &str, reason: &str, body: &Value) -> u16 {
        self.http
            .post(format!("{}/xrpc/{nsid}", self.ops_origin))
            .basic_auth("admin", Some(common::ADMIN_PASSWORD))
            .header("x-ops-reason", reason)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn push(&mut self, change: &[(&str, &str)]) -> Answer {
        // A challenge is good once in a day, so each request has its own.
        self.verifier = format!("{VERIFIER}{}", self.serial);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        let mut form = vec![
            ("response_type", "code"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", "the-state"),
            ("redirect_uri", REDIRECT),
            ("scope", SCOPE),
            ("login_hint", ALICE),
        ];
        for (name, value) in change {
            form.retain(|(own, _)| own != name);
            if !value.is_empty() {
                form.push((name, value));
            }
        }
        self.endpoint("/oauth/par", &form).await
    }

    /// The pages of one authorization, up to the redirect to the client.
    async fn authorize(&mut self, request_uri: &str, remember: bool) -> reqwest::Url {
        self.authorize_as(&client_id(), request_uri, remember).await
    }

    /// [`Server::authorize`] for the client `id`.
    async fn authorize_as(&mut self, id: &str, request_uri: &str, remember: bool) -> reqwest::Url {
        let id = id.to_owned();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &id)
            .append_pair("request_uri", request_uri)
            .finish();
        let page = self
            .send(
                reqwest::Method::GET,
                &format!("/oauth/authorize?{query}"),
                None,
                &[],
            )
            .await;
        assert_eq!(page.status, 200, "{}", page.text);
        // A request that names no account starts on the welcome page.
        let page = if page.field("csrf").is_some() {
            page
        } else {
            self.send(
                reqwest::Method::GET,
                &format!("/oauth/authorize?{query}&view=sign-in"),
                None,
                &[],
            )
            .await
        };
        let csrf = page.field("csrf").unwrap();
        let consent = if page.text.contains(">Authorize</button>") {
            page
        } else {
            let mut form = vec![
                ("client_id", id.as_str()),
                ("request_uri", request_uri),
                ("csrf", csrf.as_str()),
                ("username", ALICE),
                ("password", PASSWORD),
            ];
            if remember {
                form.push(("remember", "true"));
            }
            self.send(
                reqwest::Method::POST,
                "/oauth/authorize/sign-in",
                Some(&form),
                &[],
            )
            .await
        };
        // The PDS asks once for a client it authorized for the account.
        if consent.status == 303 {
            return reqwest::Url::parse(consent.header("location").unwrap()).unwrap();
        }
        assert_eq!(consent.status, 200, "{}", consent.text);
        assert!(
            consent.text.contains(">Authorize</button>"),
            "{}",
            consent.text
        );
        // A loopback client is not trusted: the page shows its identifier.
        if id.starts_with("http://localhost") {
            assert!(consent.text.contains("<code>http://localhost?"));
        }
        let did = consent.field("did").unwrap();
        let mut form = vec![
            ("client_id", id.as_str()),
            ("request_uri", request_uri),
            ("csrf", csrf.as_str()),
            ("did", did.as_str()),
        ];
        let ephemeral = consent.field("ephemeral");
        assert_eq!(ephemeral.is_some(), !remember);
        if let Some(ephemeral) = &ephemeral {
            form.push(("ephemeral", ephemeral));
        }
        let accepted = self
            .send(
                reqwest::Method::POST,
                "/oauth/authorize/accept",
                Some(&form),
                &[],
            )
            .await;
        assert_eq!(accepted.status, 303, "{}", accepted.text);
        reqwest::Url::parse(accepted.header("location").unwrap()).unwrap()
    }

    async fn sign_in(&mut self, remember: bool) -> (String, Value) {
        let pushed = self.push(&[]).await;
        assert_eq!(pushed.status, 201, "{}", pushed.text);
        let request_uri = pushed.json()["request_uri"].as_str().unwrap().to_owned();
        let back = self.authorize(&request_uri, remember).await;
        let parameter = |name: &str| {
            back.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        assert!(back.as_str().starts_with(REDIRECT));
        assert_eq!(parameter("iss").as_deref(), Some(self.issuer.as_str()));
        assert_eq!(parameter("state").as_deref(), Some("the-state"));
        let code = parameter("code").unwrap();
        let verifier = self.verifier.clone();
        let tokens = self
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
        (code, tokens.json())
    }
}

const GET_SESSION: &str = "/xrpc/com.atproto.server.getSession";

// One flow, in the order a client goes through it.
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
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
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
    assert!(page.text.contains(">Authorize</button>"), "{}", page.text);
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
    assert!(page.text.contains("(this device)"));
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
    assert!(
        welcome.text.contains(">Sign in</button>"),
        "{}",
        welcome.text
    );
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

/// A browser at the console: it keeps the cookie and follows no redirect.
struct Browser {
    origin: String,
    http: reqwest::Client,
    cookie: Option<String>,
}

impl Browser {
    async fn send(&mut self, request: reqwest::RequestBuilder) -> Answer {
        let request = match &self.cookie {
            Some(cookie) => request.header("cookie", cookie),
            None => request,
        };
        let response = request.send().await.unwrap();
        let headers = response.headers().clone();
        if let Some(set) = headers.get("set-cookie").and_then(|v| v.to_str().ok()) {
            let pair = set.split(';').next().unwrap().to_owned();
            self.cookie = (!pair.ends_with('=')).then_some(pair);
        }
        Answer {
            status: response.status().as_u16(),
            headers,
            text: response.text().await.unwrap(),
        }
    }

    async fn get(&mut self, path: &str) -> Answer {
        let request = self.http.get(format!("{}{path}", self.origin));
        self.send(request).await
    }

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> Answer {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let request = self
            .http
            .post(format!("{}{path}", self.origin))
            .header("content-type", "application/x-www-form-urlencoded")
            .header("origin", &self.origin)
            .body(body);
        self.send(request).await
    }

    /// Signs in through the pages of the PDS. Gives the answer of the
    /// console to the redirect.
    async fn sign_in(&mut self, server: &mut Server, console_id: &str) -> Answer {
        let start = self.get("/login").await;
        assert_eq!(start.status, 303, "{}", start.text);
        let to = reqwest::Url::parse(start.header("location").unwrap()).unwrap();
        assert!(
            to.as_str()
                .starts_with(&format!("{}/oauth/authorize?", server.issuer))
        );
        let request_uri = to
            .query_pairs()
            .find(|(key, _)| key == "request_uri")
            .unwrap()
            .1
            .into_owned();
        let back = server.authorize_as(console_id, &request_uri, false).await;
        assert!(back.as_str().starts_with(&self.origin), "{back}");
        self.get(&format!("{}?{}", back.path(), back.query().unwrap()))
            .await
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one operator, from the sign-in to the sign-out
async fn an_operator_signs_in_to_the_console_and_acts_through_it() {
    // The console listens first: its address is in its client id, which
    // the PDS must know.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let console_origin = format!("http://{}", listener.local_addr().unwrap());
    let vars = |pds: &str, internal: &str, ops: &str| {
        [
            ("CONSOLE_PUBLIC_URL", console_origin.as_str()),
            ("CONSOLE_PDS_PUBLIC_URL", pds),
            ("CONSOLE_PDS_INTERNAL_URL", internal),
            ("CONSOLE_OPS_URL", ops),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
    };
    let draft = pds_console::config::Config::from_vars(&vars(
        "https://pds.test",
        "http://127.0.0.1:1",
        "http://127.0.0.1:1",
    ))
    .unwrap();
    let console_id = draft.client_id();
    let mut server = Server::start_with(&[
        ("PDS_PROFILE", "linkjar"),
        ("PDS_ADMIN_PASSWORD", common::ADMIN_PASSWORD),
        ("PDS_OPS_CLIENT_IDS", &console_id),
    ])
    .await;
    let config = pds_console::config::Config::from_vars(&vars(
        &server.issuer,
        &server.origin,
        &server.ops_origin,
    ))
    .unwrap();
    assert_eq!(config.client_id(), console_id);
    let console = pds_console::Console::new(config).unwrap();
    tokio::spawn(async move { axum::serve(listener, pds_console::router(console)).await });
    let mut browser = Browser {
        origin: console_origin.clone(),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cookie: None,
    };

    // Without a session every page sends the browser to the sign-in page.
    let page = browser.get("/").await;
    assert_eq!(
        (page.status, page.header("location")),
        (303, Some("/sign-in"))
    );
    assert_eq!(browser.get("/live.json").await.status, 401);
    let page = browser.get("/sign-in").await;
    assert!(page.text.contains(r#"href="/login""#));
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains("default-src 'none'")
    );

    // An account without a role gets no session.
    let refused = browser.sign_in(&mut server, &console_id).await;
    assert_eq!(refused.status, 403, "{}", refused.text);
    assert!(refused.text.contains("holds no operator role"));
    assert!(browser.cookie.is_none());

    // With the role of an operator the console opens.
    let grant = json!({"did": ALICE, "role": "operator"});
    assert_eq!(
        server
            .admin("internal.ops.grantRole", "joins the team", &grant)
            .await,
        200
    );
    let signed = browser.sign_in(&mut server, &console_id).await;
    assert_eq!(
        (signed.status, signed.header("location")),
        (303, Some("/")),
        "{}",
        signed.text
    );
    assert!(signed.header("set-cookie").unwrap().contains("HttpOnly"));
    let home = browser.get("/").await;
    assert_eq!(home.status, 200, "{}", home.text);
    assert!(
        home.text
            .contains(&format!("<code>{ALICE}</code> · operator"))
    );
    assert!(
        home.text.contains(r#"data-live="accounts">3<"#),
        "{}",
        home.text
    );
    assert!(home.text.contains("No subscriber is connected."));
    let live = browser.get("/live.json").await;
    assert_eq!(live.json()["overview"]["accounts"], 3, "{}", live.text);
    let csrf = home.field("csrf").unwrap();

    // The pages.
    let list = browser.get("/accounts").await;
    assert!(
        list.text
            .contains(&format!("/accounts/{}", common::DANA.replace(':', "%3A")))
    );
    let dana = format!("/accounts/{}", common::DANA.replace(':', "%3A"));
    let card = browser.get(&dana).await;
    assert_eq!(card.status, 200, "{}", card.text);
    assert!(card.text.contains(r#"value="takedown""#));
    assert!(browser.get("/roles").await.text.contains(ALICE));
    assert!(
        browser
            .get("/domains")
            .await
            .text
            .contains("No email domain is blocked.")
    );

    // An action needs the token of the session, a reason, and for a
    // takedown the DID typed again.
    let takedown = |csrf: &'static str, reason: &'static str, confirm: &'static str| {
        [
            ("csrf", csrf),
            ("action", "takedown"),
            ("did", common::DANA),
            ("reason", reason),
            ("confirm", confirm),
        ]
    };
    assert_eq!(
        browser
            .post("/act", &takedown("wrong", "spam", common::DANA))
            .await
            .status,
        403
    );
    let csrf: &'static str = Box::leak(csrf.into_boxed_str());
    assert_eq!(
        browser
            .post("/act", &takedown(csrf, "", common::DANA))
            .await
            .status,
        400
    );
    assert_eq!(
        browser
            .post("/act", &takedown(csrf, "spam", "did:plc:other"))
            .await
            .status,
        400
    );
    let done = browser
        .post("/act", &takedown(csrf, "spam links", common::DANA))
        .await;
    assert_eq!(done.status, 303, "{}", done.text);
    assert_eq!(
        done.header("location"),
        Some(format!("{dana}?done=takedown").as_str())
    );
    let card = browser.get(&format!("{dana}?done=takedown")).await;
    assert!(card.text.contains("The account is taken down."));
    assert!(card.text.contains(r#"value="reinstate""#));

    // The PDS wrote the entry: the operator, the client, the reason.
    let log = browser.get("/audit").await;
    assert_eq!(log.status, 200, "{}", log.text);
    assert!(log.text.contains("com.atproto.admin.updateSubjectStatus"));
    assert!(log.text.contains("spam links"));
    let entries = pds_xrpc::audit::AuditLog::open(&server.dir.config.layout.audit_db, None)
        .unwrap()
        .page(None, 1)
        .unwrap();
    assert_eq!(entries[0]["actor"], ALICE);
    assert_eq!(entries[0]["clientId"], console_id.as_str());
    assert_eq!(entries[0]["sidecar"], "console");
    assert_eq!(entries[0]["reason"], "spam links");

    // A blocked domain, through the form.
    let block = [
        ("csrf", csrf),
        ("action", "block-domain"),
        ("domain", "spam.example"),
        ("reason", "a wave"),
    ];
    let done = browser.post("/act", &block).await;
    assert_eq!(
        done.header("location"),
        Some("/domains?done=block-domain"),
        "{}",
        done.text
    );
    assert!(
        browser
            .get("/domains")
            .await
            .text
            .contains("<code>spam.example</code>")
    );

    // The sign-out ends the session at the console.
    let out = browser.post("/sign-out", &[("csrf", csrf)]).await;
    assert_eq!(
        (out.status, out.header("location")),
        (303, Some("/sign-in"))
    );
    assert!(browser.cookie.is_none());
    assert_eq!(browser.get("/").await.status, 303);
}

/// An AI client of the MCP server, with the browser of its person.
struct AiClient {
    /// Where the MCP server listens.
    origin: String,
    http: reqwest::Client,
    cookie: Option<String>,
    serial: u32,
}

impl AiClient {
    async fn send(&mut self, request: reqwest::RequestBuilder) -> Answer {
        let request = match &self.cookie {
            Some(cookie) => request.header("cookie", cookie),
            None => request,
        };
        let response = request.send().await.unwrap();
        let headers = response.headers().clone();
        if let Some(set) = headers.get("set-cookie").and_then(|v| v.to_str().ok()) {
            self.cookie = Some(set.split(';').next().unwrap().to_owned());
        }
        Answer {
            status: response.status().as_u16(),
            headers,
            text: response.text().await.unwrap(),
        }
    }

    async fn form(&mut self, path: &str, form: &[(&str, &str)]) -> Answer {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let request = self
            .http
            .post(format!("{}{path}", self.origin))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body);
        self.send(request).await
    }

    /// One JSON-RPC request to the MCP endpoint.
    async fn rpc(&mut self, token: Option<&str>, method: &str, params: Value) -> Answer {
        self.serial += 1;
        let message =
            json!({"jsonrpc": "2.0", "id": self.serial, "method": method, "params": params});
        let mut request = self
            .http
            .post(format!("{}/mcp", self.origin))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(message.to_string());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        self.send(request).await
    }

    /// The result of a tool call.
    async fn tool(&mut self, token: &str, name: &str, arguments: Value) -> Value {
        let answer = self
            .rpc(
                Some(token),
                "tools/call",
                json!({"name": name, "arguments": arguments}),
            )
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
        answer.json()["result"].clone()
    }
}

const NOTE: &str = "com.example.note";
const AI_REDIRECT: &str = "http://127.0.0.1/ai-callback";
const MCP_PUBLIC: &str = "https://mcp.example.com";

/// The authorization of an AI client up to its code: the request, the
/// pages of the PDS, and the consent page of the MCP server.
async fn mcp_authorize(
    ai: &mut AiClient,
    server: &mut Server,
    sidecar_id: &str,
    client_id: &str,
    scope: &str,
    decision: &str,
) -> reqwest::Url {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", AI_REDIRECT)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", "ai-state")
        .append_pair("scope", scope)
        .append_pair("resource", &format!("{MCP_PUBLIC}/mcp"))
        .finish();
    let url = format!("{}/authorize?{query}", ai.origin);
    let back = mcp_browser(
        ai, server, sidecar_id, &url, "Test AI", MCP_PUBLIC, decision,
    )
    .await;
    assert!(back.as_str().starts_with(AI_REDIRECT), "{back}");
    back
}

/// The browser of the person from the authorization URL of an AI client
/// to the redirect back to it: the pages of the PDS, and the consent page
/// of the MCP server, which names the client `name`. `public` is the
/// public address of the MCP server.
async fn mcp_browser(
    ai: &mut AiClient,
    server: &mut Server,
    sidecar_id: &str,
    url: &str,
    name: &str,
    public: &str,
    decision: &str,
) -> reqwest::Url {
    let request = ai.http.get(url);
    let start = ai.send(request).await;
    assert_eq!(start.status, 303, "{}", start.text);
    let to = reqwest::Url::parse(start.header("location").unwrap()).unwrap();
    assert!(
        to.as_str()
            .starts_with(&format!("{}/oauth/authorize?", server.issuer)),
        "{to}"
    );
    let request_uri = to
        .query_pairs()
        .find(|(key, _)| key == "request_uri")
        .unwrap()
        .1
        .into_owned();
    let back = server.authorize_as(sidecar_id, &request_uri, false).await;
    assert!(
        back.as_str()
            .starts_with(&format!("{public}/oauth/pds-callback?")),
        "{back}"
    );
    let path = format!("{}?{}", back.path(), back.query().unwrap());

    // Another browser cannot finish the sign-in.
    let stranger = ai
        .http
        .get(format!("{}{path}", ai.origin))
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status(), 400);

    // The MCP server asks about the AI client, which the PDS did not name.
    let request = ai.http.get(format!("{}{path}", ai.origin));
    let consent = ai.send(request).await;
    assert_eq!(consent.status, 200, "{}", consent.text);
    assert!(
        consent.text.contains(&format!("<strong>{name}</strong>")),
        "{}",
        consent.text
    );
    assert!(consent.text.contains(&format!("<code>{ALICE}</code>")));
    assert!(consent.text.contains("<code>127.0.0.1</code>"));
    assert!(
        consent
            .header("content-security-policy")
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    let request_id = consent.field("request").unwrap();
    let csrf = consent.field("csrf").unwrap();

    // The form needs its token.
    let forged = [
        ("request", request_id.as_str()),
        ("csrf", "wrong"),
        ("decision", "allow"),
    ];
    assert_eq!(ai.form("/authorize/decision", &forged).await.status, 403);
    let form = [
        ("request", request_id.as_str()),
        ("csrf", csrf.as_str()),
        ("decision", decision),
    ];
    let decided = ai.form("/authorize/decision", &form).await;
    assert_eq!(decided.status, 303, "{}", decided.text);
    reqwest::Url::parse(decided.header("location").unwrap()).unwrap()
}

fn parameter(url: &reqwest::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one AI client, from its registration to the end of its session
async fn an_ai_client_reaches_a_repository_through_the_mcp_server() {
    // The MCP server is a client that the PDS knows without a fetch.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mcp_origin = format!("http://{}", listener.local_addr().unwrap());
    let vars = |pds: &str, internal: &str| -> std::collections::BTreeMap<String, String> {
        [
            ("MCP_PUBLIC_URL", MCP_PUBLIC),
            ("MCP_PDS_PUBLIC_URL", pds),
            ("MCP_PDS_INTERNAL_URL", internal),
            ("MCP_WRITE_COLLECTIONS", NOTE),
            (
                "MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX",
                "0606060606060606060606060606060606060606060606060606060606060606",
            ),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
    };
    let draft = pds_mcp::config::Config::from_vars(&vars("https://pds.test", "http://127.0.0.1:1"))
        .unwrap();
    let sidecar_id = draft.client_id();
    let known = std::env::temp_dir().join(format!(
        "linkjar-pds-known-clients-{}.json",
        std::process::id()
    ));
    let mut clients = serde_json::Map::new();
    clients.insert(sidecar_id.clone(), draft.client_metadata());
    std::fs::write(&known, Value::Object(clients).to_string()).unwrap();
    let mut server = Server::start_with(&[
        ("PDS_OAUTH_KNOWN_CLIENTS_FILE", known.to_str().unwrap()),
        ("PDS_MCP_CLIENT_IDS", &sidecar_id),
    ])
    .await;
    let config = pds_mcp::config::Config::from_vars(&vars(&server.issuer, &server.origin)).unwrap();
    let mcp = pds_mcp::Mcp::new(config).unwrap();
    tokio::spawn(async move { axum::serve(listener, pds_mcp::router(mcp)).await });
    let mut ai = AiClient {
        origin: mcp_origin,
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cookie: None,
        serial: 0,
    };

    // Discovery: no token, the challenge, the two metadata documents.
    let challenge = ai.rpc(None, "initialize", json!({})).await;
    assert_eq!(challenge.status, 401);
    let metadata_url = format!("{MCP_PUBLIC}/.well-known/oauth-protected-resource");
    assert_eq!(
        challenge.header("www-authenticate"),
        Some(format!(r#"Bearer resource_metadata="{metadata_url}""#).as_str())
    );
    let request = ai.http.get(format!(
        "{}/.well-known/oauth-protected-resource",
        ai.origin
    ));
    let resource = ai.send(request).await.json();
    assert_eq!(resource["resource"], format!("{MCP_PUBLIC}/mcp"));
    assert_eq!(resource["authorization_servers"], json!([MCP_PUBLIC]));
    let request = ai.http.get(format!(
        "{}/.well-known/oauth-authorization-server",
        ai.origin
    ));
    let issuer = ai.send(request).await.json();
    assert_eq!(
        issuer["registration_endpoint"],
        format!("{MCP_PUBLIC}/register")
    );
    assert_eq!(issuer["code_challenge_methods_supported"], json!(["S256"]));
    assert!(
        issuer["scopes_supported"]
            .as_array()
            .unwrap()
            .contains(&json!("repo:write:com.example.note"))
    );

    // Registration.
    let request = ai
        .http
        .post(format!("{}/register", ai.origin))
        .header("content-type", "application/json")
        .body(json!({"client_name": "Test AI", "redirect_uris": [AI_REDIRECT]}).to_string());
    let registered = ai.send(request).await;
    assert_eq!(registered.status, 201, "{}", registered.text);
    let client_id = registered.json()["client_id"].as_str().unwrap().to_owned();

    // A person who says no gives the client nothing.
    let scope = "account:read repo:read repo:write:com.example.note";
    let denied = mcp_authorize(&mut ai, &mut server, &sidecar_id, &client_id, scope, "deny").await;
    assert_eq!(
        parameter(&denied, "error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(parameter(&denied, "state").as_deref(), Some("ai-state"));

    // A person who allows it gives a code, which is good once with the verifier.
    let back = mcp_authorize(
        &mut ai,
        &mut server,
        &sidecar_id,
        &client_id,
        scope,
        "allow",
    )
    .await;
    assert_eq!(parameter(&back, "iss").as_deref(), Some(MCP_PUBLIC));
    let code = parameter(&back, "code").unwrap();
    let exchange = |verifier: &'static str, code: String, client_id: String| {
        vec![
            ("grant_type", "authorization_code".to_owned()),
            ("code", code),
            ("code_verifier", verifier.to_owned()),
            ("redirect_uri", AI_REDIRECT.to_owned()),
            ("client_id", client_id),
        ]
    };
    let borrowed =
        |form: &[(&'static str, String)]| -> Vec<(&'static str, String)> { form.to_vec() };
    let form = borrowed(&exchange(VERIFIER, code.clone(), client_id.clone()));
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let tokens = ai.form("/token", &form).await;
    assert_eq!(tokens.status, 200, "{}", tokens.text);
    let tokens = tokens.json();
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["scope"], scope);
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    assert_eq!(ai.form("/token", &form).await.status, 400);

    // The protocol.
    let hello = ai
        .rpc(Some(&access), "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}))
        .await;
    assert_eq!(hello.status, 200, "{}", hello.text);
    assert_eq!(hello.json()["result"]["protocolVersion"], "2025-06-18");
    let listed = ai.rpc(Some(&access), "tools/list", json!({})).await.json();
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 8);
    let unknown = ai.rpc(Some(&access), "no/such", json!({})).await.json();
    assert_eq!(unknown["error"]["code"], -32601);

    // Reads.
    let who = ai.tool(&access, "whoami", json!({})).await;
    assert_eq!(who["structuredContent"]["did"], ALICE, "{who}");
    let collections = ai.tool(&access, "list_collections", json!({})).await;
    assert!(
        collections["structuredContent"]["collections"]
            .as_array()
            .unwrap()
            .contains(&json!("app.bsky.feed.post"))
    );
    let posts = ai
        .tool(
            &access,
            "list_records",
            json!({"collection": "app.bsky.feed.post", "limit": 1}),
        )
        .await;
    assert_eq!(
        posts["structuredContent"]["records"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{posts}"
    );

    // A write in the collection of the scope, and none in another.
    let made = ai
        .tool(&access, "create_record", json!({"collection": NOTE, "rkey": "first", "record": {"$type": NOTE, "text": "from an AI client"}}))
        .await;
    assert_eq!(made["isError"], false, "{made}");
    let uri = format!("at://{ALICE}/{NOTE}/first");
    assert_eq!(made["structuredContent"]["uri"], uri.as_str());
    let other = ai
        .rpc(Some(&access), "tools/call", json!({"name": "create_record", "arguments": {"collection": "com.example.other", "record": {"$type": "com.example.other"}}}))
        .await;
    assert_eq!(other.status, 403, "{}", other.text);
    let header = other.header("www-authenticate").unwrap();
    assert!(
        header.contains(r#"error="insufficient_scope""#)
            && header.contains(r#"scope="repo:write:com.example.other""#),
        "{header}"
    );
    let read = ai
        .rpc(Some(&access), "resources/read", json!({"uri": uri}))
        .await
        .json();
    assert!(
        read["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("from an AI client"),
        "{read}"
    );
    let foreign = ai
        .rpc(
            Some(&access),
            "resources/read",
            json!({"uri": format!("at://{}/{NOTE}/first", common::DANA)}),
        )
        .await
        .json();
    assert_eq!(foreign["error"]["code"], -32002);
    // A refusal of the PDS is a result that the model reads.
    let missing = ai
        .tool(
            &access,
            "get_record",
            json!({"collection": NOTE, "rkey": "nope"}),
        )
        .await;
    assert_eq!(missing["isError"], true, "{missing}");

    // The PDS wrote an entry for the write: the sidecar and the AI client.
    let entries = pds_xrpc::audit::AuditLog::open(&server.dir.config.layout.audit_db, None)
        .unwrap()
        .page(None, 1)
        .unwrap();
    assert_eq!(
        entries[0]["action"], "com.atproto.repo.createRecord",
        "{entries:?}"
    );
    assert_eq!(entries[0]["actor"], ALICE);
    assert_eq!(entries[0]["clientId"], sidecar_id.as_str());
    assert_eq!(entries[0]["sidecar"], format!("mcp:{client_id}").as_str());

    // A refresh rotates both tokens. The old refresh token ends the session.
    let rotate = |refresh: &str| {
        vec![
            ("grant_type", "refresh_token".to_owned()),
            ("refresh_token", refresh.to_owned()),
            ("client_id", client_id.clone()),
        ]
    };
    let form = rotate(&refresh);
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let next = ai.form("/token", &form).await;
    assert_eq!(next.status, 200, "{}", next.text);
    let next_access = next.json()["access_token"].as_str().unwrap().to_owned();
    assert_eq!(ai.rpc(Some(&access), "ping", json!({})).await.status, 401);
    assert_eq!(
        ai.rpc(Some(&next_access), "ping", json!({})).await.status,
        200
    );
    assert_eq!(ai.form("/token", &form).await.status, 400);
    assert_eq!(
        ai.rpc(Some(&next_access), "ping", json!({})).await.status,
        401
    );

    // A revocation ends a session on both sides.
    let back = mcp_authorize(
        &mut ai,
        &mut server,
        &sidecar_id,
        &client_id,
        "repo:read",
        "allow",
    )
    .await;
    let form = exchange(
        VERIFIER,
        parameter(&back, "code").unwrap(),
        client_id.clone(),
    );
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let tokens = ai.form("/token", &form).await.json();
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let sessions = |server: &Server| -> i64 {
        common::open(&server.dir.config.layout.account_db)
            .query_row(
                r#"select count(*) from "token" where "did" = ?1"#,
                [ALICE],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(sessions(&server), 1);
    // Without the scope of the account the tool is refused.
    let who = ai
        .rpc(Some(&access), "tools/call", json!({"name": "whoami"}))
        .await;
    assert_eq!(who.status, 403, "{}", who.text);
    assert_eq!(ai.form("/revoke", &[("token", &access)]).await.status, 200);
    assert_eq!(ai.rpc(Some(&access), "ping", json!({})).await.status, 401);
    assert_eq!(sessions(&server), 0);
    let _ = std::fs::remove_file(&known);
}

/// One line of JSON from the official client, read without blocking the
/// servers that the client talks to.
async fn client_line(
    out: std::sync::Arc<std::sync::Mutex<std::io::BufReader<std::process::ChildStdout>>>,
) -> Value {
    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        let mut line = String::new();
        out.lock().unwrap().read_line(&mut line).unwrap();
        let value = serde_json::from_str(&line);
        assert!(value.is_ok(), "not a line of the client: {line:?}");
        value.unwrap()
    })
    .await
    .unwrap()
}

/// The gate of unit 10: the official MCP TypeScript client signs in
/// through the pages of the PDS, uses the tools, refreshes and revokes.
/// It runs when `PDS_TEST_MCP_CLIENT` names a Node binary and the
/// dependencies of `parity/` are installed. With `MCP_INSPECTOR` set to
/// the binary of the MCP Inspector, the Inspector lists the tools too.
#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // one client, from the challenge to the revocation
async fn the_official_mcp_client_signs_in_through_the_pds() {
    let Ok(node) = std::env::var("PDS_TEST_MCP_CLIENT") else {
        return;
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mcp_origin = format!("http://{}", listener.local_addr().unwrap());
    let vars = |pds: &str, internal: &str| -> std::collections::BTreeMap<String, String> {
        [
            ("MCP_PUBLIC_URL", MCP_PUBLIC),
            ("MCP_PDS_PUBLIC_URL", pds),
            ("MCP_PDS_INTERNAL_URL", internal),
            ("MCP_WRITE_COLLECTIONS", NOTE),
            (
                "MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX",
                "0707070707070707070707070707070707070707070707070707070707070707",
            ),
        ]
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
    };
    let draft = pds_mcp::config::Config::from_vars(&vars("https://pds.test", "http://127.0.0.1:1"))
        .unwrap();
    let sidecar_id = draft.client_id();
    let known = std::env::temp_dir().join(format!(
        "linkjar-pds-official-client-{}.json",
        std::process::id()
    ));
    let mut clients = serde_json::Map::new();
    clients.insert(sidecar_id.clone(), draft.client_metadata());
    std::fs::write(&known, Value::Object(clients).to_string()).unwrap();
    let mut server = Server::start_with(&[
        ("PDS_OAUTH_KNOWN_CLIENTS_FILE", known.to_str().unwrap()),
        ("PDS_MCP_CLIENT_IDS", &sidecar_id),
    ])
    .await;
    let config = pds_mcp::config::Config::from_vars(&vars(&server.issuer, &server.origin)).unwrap();
    let mcp = pds_mcp::Mcp::new(config).unwrap();
    tokio::spawn(async move { axum::serve(listener, pds_mcp::router(mcp)).await });
    let mut ai = AiClient {
        origin: mcp_origin.clone(),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        cookie: None,
        serial: 0,
    };

    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../parity/mcp/official-client.ts"
    );
    let mut child = std::process::Command::new(node)
        .arg(script)
        .arg(format!("{MCP_PUBLIC}/mcp"))
        .arg(&mcp_origin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let out = std::sync::Arc::new(std::sync::Mutex::new(std::io::BufReader::new(
        child.stdout.take().unwrap(),
    )));
    let mut input = child.stdin.take().unwrap();

    // The SDK found the server, registered, and sends the person to sign in.
    let step = client_line(out.clone()).await;
    assert!(step["authorize"].is_string(), "{step}");
    let url = step["authorize"].as_str().unwrap();
    let url = url.replace(MCP_PUBLIC, &mcp_origin);
    let back = mcp_browser(
        &mut ai,
        &mut server,
        &sidecar_id,
        &url,
        "Official SDK",
        MCP_PUBLIC,
        "allow",
    )
    .await;
    assert!(
        back.as_str()
            .starts_with("http://127.0.0.1/official-callback?"),
        "{back}"
    );
    std::io::Write::write_all(&mut input, format!("{back}\n").as_bytes()).unwrap();

    let done = client_line(out).await;
    let results = &done["done"];
    assert_eq!(results["registered"], true, "{done}");
    assert_eq!(
        results["authorizeResource"],
        format!("{MCP_PUBLIC}/mcp"),
        "{done}"
    );
    assert_eq!(results["state"], true, "{done}");
    // The SDK asks for every scope of `scopes_supported` (SEP-835) when the
    // challenge names none, and the person allows them on the consent page.
    let every = "account:read repo:read repo:write repo:write:com.example.note";
    assert_eq!(results["authorizeScope"], every, "{done}");
    assert_eq!(results["scope"], every, "{done}");
    assert_eq!(results["tools"].as_array().unwrap().len(), 8, "{done}");
    assert_eq!(results["did"], ALICE, "{done}");
    let uri = format!("at://{ALICE}/{NOTE}/official");
    assert_eq!(results["created"], uri.as_str(), "{done}");
    assert_eq!(results["read"], true, "{done}");
    assert_eq!(results["prompts"], 2, "{done}");
    if std::env::var_os("MCP_INSPECTOR").is_some() {
        assert_eq!(results["inspectorTools"], 8, "{done}");
    }
    assert_eq!(results["refresh"], "AUTHORIZED", "{done}");
    assert_eq!(results["rotated"], true, "{done}");
    assert_eq!(results["oldAccess"], 401, "{done}");
    assert_eq!(results["newAccess"], 200, "{done}");
    assert_eq!(results["revoked"], 200, "{done}");
    assert_eq!(results["afterRevocation"], 401, "{done}");
    assert!(child.wait().unwrap().success());
    let _ = std::fs::remove_file(&known);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one data directory, there and back
async fn a_data_directory_moves_to_postgres_and_back_with_its_sessions() {
    let Ok(url) = std::env::var("PDS_TEST_POSTGRES_URL") else {
        return;
    };
    let prefix = format!("t{}_oauth", std::process::id());
    let dir = DataDir::new_with(
        1,
        &[
            ("PDS_POSTGRES_URL", &url),
            ("PDS_POSTGRES_SCHEMA_PREFIX", &prefix),
        ],
    );
    let postgres = dir.config.layout.clone();
    let tier = postgres.shared.clone().unwrap();
    let files = pds_storage::DataLayout {
        shared: None,
        ..postgres.clone()
    };
    let now = "2026-10-13T00:00:00.000Z";

    // The fixture is in the files. The copy puts it in Postgres.
    let copied = pds_storage::migrate::copy_shared(&files, &postgres, now).unwrap();
    let rows = |copied: &[pds_storage::migrate::Copied], table: &str| {
        copied
            .iter()
            .find(|entry| entry.table == table)
            .map(|entry| entry.rows)
    };
    assert_eq!(rows(&copied, "actor"), Some(3));
    assert_eq!(rows(&copied, "account"), Some(3));
    // A target that exists is refused.
    assert!(matches!(
        pds_storage::migrate::copy_shared(&files, &postgres, now),
        Err(pds_storage::migrate::MigrateError::TargetExists(_))
    ));

    // The server on Postgres knows the account of the fixture: a client
    // is authorized, and its session is used, refreshed and listed.
    let mut server = Server::start_in(dir).await;
    server.push(&[]).await;
    let (_, tokens) = server.sign_in(true).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let key = client_key();
    let session = server
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(session.status, 200, "{}", session.text);
    assert_eq!(session.json()["did"], ALICE);
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let next = server
        .endpoint(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh)],
        )
        .await;
    assert_eq!(next.status, 200, "{}", next.text);
    // The refresh token that was rotated away ends the session.
    let replay = server
        .endpoint(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", &refresh)],
        )
        .await;
    assert_eq!(replay.status, 400, "{}", replay.text);
    let (_, tokens) = server.sign_in(true).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let record = json!({"repo": ALICE, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let access = tokens["access_token"].as_str().unwrap().to_owned();
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

    // Back to files, in a directory of their own.
    let back = std::env::temp_dir().join(format!("linkjar-pds-back-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&back);
    std::fs::create_dir_all(&back).unwrap();
    let files_again = pds_storage::DataLayout::new(&back);
    let returned = pds_storage::migrate::copy_shared(&postgres, &files_again, now).unwrap();
    assert_eq!(rows(&returned, "actor"), Some(3));
    assert_eq!(rows(&returned, "token"), Some(1));
    assert!(rows(&returned, "repo_seq").unwrap() >= 1);
    assert!(
        rows(&returned, "entry").is_some(),
        "the audit log moves too"
    );
    let accounts = common::open(&files_again.account_db);
    let client: String = accounts
        .query_row(
            r#"select "clientId" from "token" where "did" = ?1"#,
            [ALICE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(client, client_id());
    let device: i64 = accounts
        .query_row(
            r#"select count(*) from "account_device" where "did" = ?1"#,
            [ALICE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(device, 1);
    // The next session of the files gets the number after the last one.
    let next_id: i64 = accounts
        .query_row(
            r#"select "seq" from "sqlite_sequence" where "name" = 'token'"#,
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(next_id >= 2, "{next_id}");

    tier.drop_schemas().unwrap();
    std::fs::remove_dir_all(&back).unwrap();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // one client, at the entryway and at its member host
async fn a_client_authorized_at_the_entryway_writes_at_the_member_host() {
    let signing_hex = "0808080808080808080808080808080808080808080808080808080808080808";
    let signing = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(signing_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    let public = data_encoding::HEXLOWER.encode(&signing.public_key().to_compressed());
    let rotation_hex = "0707070707070707070707070707070707070707070707070707070707070707";
    let rotation = Keypair::from_secret_bytes(
        Algorithm::Secp256k1,
        &data_encoding::HEXLOWER
            .decode(rotation_hex.as_bytes())
            .unwrap(),
    )
    .unwrap();
    // The two servers have the account of the fixture: the entryway its
    // credentials, the member host its repository.
    let mut member = Server::start_with(&[
        ("PDS_ENTRYWAY_URL", "https://entryway.test"),
        ("PDS_ENTRYWAY_DID", "did:web:entryway.test"),
        ("PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX", &public),
        ("PDS_ENTRYWAY_PLC_ROTATION_KEY", &rotation.did_key()),
    ])
    .await;
    let mut entryway = Server::start_with(&[
        ("PDS_HOSTNAME", "entryway.test"),
        ("PDS_MEMBER_URL", "https://pds.test"),
        ("PDS_MEMBER_ADMIN_PASSWORD", "member-admin"),
        (
            "PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX",
            signing_hex,
        ),
        ("PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX", rotation_hex),
    ])
    .await;
    assert_eq!(entryway.issuer, "https://entryway.test");
    assert_eq!(member.issuer, "https://pds.test");

    // The client learns from the member host where to be authorized.
    let resource = member
        .send(
            reqwest::Method::GET,
            "/.well-known/oauth-protected-resource",
            None,
            &[],
        )
        .await;
    assert_eq!(
        resource.json()["authorization_servers"],
        json!(["https://entryway.test"])
    );

    // The authorization at the entryway.
    entryway.push(&[]).await;
    let (_, tokens) = entryway.sign_in(false).await;
    assert_eq!(tokens["sub"], ALICE, "{tokens}");
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let key = client_key();
    // The entryway takes its own token.
    let own = entryway
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(own.status, 200, "{}", own.text);

    // The member host takes it too, with a proof for its own address and
    // a nonce of its own.
    let record = json!({"repo": ALICE, "collection": "com.example.note", "record": {"$type": "com.example.note"}});
    let write = "/xrpc/com.atproto.repo.createRecord";
    let first = member
        .resource(&key, reqwest::Method::POST, write, &access, Some(&record))
        .await;
    assert_eq!(first.status, 401, "{}", first.text);
    first
        .header("dpop-nonce")
        .unwrap()
        .clone_into(&mut member.nonce);
    let written = member
        .resource(&key, reqwest::Method::POST, write, &access, Some(&record))
        .await;
    assert_eq!(written.status, 200, "{}", written.text);
    assert!(
        written.json()["uri"]
            .as_str()
            .unwrap()
            .starts_with(&format!("at://{ALICE}/com.example.note/"))
    );

    // A token with another signature, and a proof of another key.
    let mut forged = access.clone();
    let last = forged.pop().unwrap();
    forged.push(if last == 'A' { 'B' } else { 'A' });
    let refused = member
        .resource(&key, reqwest::Method::POST, write, &forged, Some(&record))
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
    let other = Keypair::generate(Algorithm::P256, |bytes| bytes.fill(8));
    let refused = member
        .resource(&other, reqwest::Method::POST, write, &access, Some(&record))
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
    // A session method goes on to the entryway with a service token of
    // the account, as a proof names the member host only. The entryway of
    // this test has no address, so the member host reports that it did not
    // answer: it checked the proof and passed the call on. The Harness runs
    // the whole way (parity, PARITY_CANDIDATE_ENTRYWAY=1).
    let passed = member
        .resource(&key, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(
        (passed.status, passed.json()["error"].as_str()),
        (502, Some("UpstreamFailure")),
        "{}",
        passed.text
    );
    // A proof of another key is refused at the member host first.
    let refused = member
        .resource(&other, reqwest::Method::GET, GET_SESSION, &access, None)
        .await;
    assert_eq!(refused.status, 401, "{}", refused.text);
}

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
    issuer: String,
    http: reqwest::Client,
    nonce: String,
    cookies: String,
    serial: u32,
    /// The PKCE verifier of the last pushed request.
    verifier: String,
    _dir: DataDir,
}

impl Server {
    async fn start() -> Self {
        let dir = DataDir::new(1);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = pds_xrpc::router(app_state(&dir.config).unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut server = Self {
            origin,
            issuer: String::new(),
            http,
            nonce: String::new(),
            cookies: String::new(),
            serial: 0,
            verifier: String::new(),
            _dir: dir,
        };
        let metadata = server
            .send(
                reqwest::Method::GET,
                "/.well-known/oauth-authorization-server",
                None,
                &[],
            )
            .await;
        assert_eq!(metadata.status, 200);
        metadata.json()["issuer"]
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
        let mut request = self.http.request(method, format!("{}{path}", self.origin));
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
        let id = client_id();
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
        assert_eq!(consent.status, 200, "{}", consent.text);
        assert!(
            consent.text.contains(">Authorize</button>"),
            "{}",
            consent.text
        );
        // A loopback client is not trusted: the page shows its identifier.
        assert!(consent.text.contains("<code>http://localhost?"));
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

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
        common::http::json(self.text.as_bytes())
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
    _listener: common::http::Listener,
    _ops_listener: common::http::Listener,
    origin: String,
    ops_origin: String,
    issuer: String,
    http: reqwest::Client,
    nonce: String,
    cookies: String,
    serial: u32,
    /// The PKCE verifier of the last pushed request.
    verifier: String,
    password: String,
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
        let listener = common::http::bind().await;
        Self::start_on(dir, listener).await
    }

    async fn start_on(dir: DataDir, listener: tokio::net::TcpListener) -> Self {
        linkjar_pds::serve::create_missing_databases(&dir.config).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = app_state(&dir.config).unwrap();
        let router = pds_xrpc::router(state.clone());
        let public_listener = common::http::serve(listener, router);
        let listener = common::http::bind().await;
        let ops_origin = format!("http://{}", listener.local_addr().unwrap());
        let router = linkjar_pds::serve::ops_router(state, &dir.config);
        let listener = common::http::serve(listener, router);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut server = Self {
            _listener: public_listener,
            _ops_listener: listener,
            origin,
            ops_origin,
            issuer: String::new(),
            http,
            nonce: String::new(),
            cookies: String::new(),
            serial: 0,
            verifier: String::new(),
            password: PASSWORD.to_owned(),
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

    async fn json_post(&self, nsid: &str, body: &Value) -> Answer {
        let response = common::http::post(&self.http, &self.origin, nsid, None, Some(body))
            .send()
            .await
            .unwrap();
        Answer {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            text: response.text().await.unwrap(),
        }
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
        let password = self.password.clone();
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
        let consent = if page.text.contains(">Authorize<") {
            page
        } else {
            let mut form = vec![
                ("client_id", id.as_str()),
                ("request_uri", request_uri),
                ("csrf", csrf.as_str()),
                ("username", ALICE),
                ("password", password.as_str()),
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
        assert!(consent.text.contains(">Authorize<"), "{}", consent.text);
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

#[path = "oauth/accounts.rs"]
mod accounts;
#[path = "oauth/authorization.rs"]
mod authorization;
#[path = "oauth/brand.rs"]
mod brand;
#[path = "oauth/console.rs"]
mod console;
#[path = "oauth/entryway.rs"]
mod entryway;
#[path = "oauth/mcp.rs"]
mod mcp;
#[path = "oauth/migration.rs"]
mod migration;
#[path = "oauth/recovery.rs"]
mod recovery;

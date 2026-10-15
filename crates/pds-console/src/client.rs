//! The console as a client of the PDS (`docs/sidecars.md` §1.4).
//!
//! It is an OAuth client, and a client of the ops listener. The console
//! holds no password: an operator signs in on the pages of the PDS, and
//! the console keeps the session with the DPoP key it made for it.

use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pds_identity::{Algorithm, Keypair};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Error as WsError, client::IntoClientRequest as _},
};

use crate::config::{Config, SCOPE};

/// Why a call to the PDS did not give what the console asked for.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The PDS could not be reached, or sent no JSON.
    #[error("the server did not answer: {0}")]
    Transport(String),
    /// The PDS refused.
    #[error("{message}")]
    Refused {
        /// The HTTP status.
        status: u16,
        /// The name of the error.
        error: String,
        /// The message of the server.
        message: String,
    },
}

impl ClientError {
    /// Whether the PDS no longer takes the session.
    pub const fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Refused { status: 401, .. })
    }
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Random bytes as base64url text.
pub fn random_text<const N: usize>() -> String {
    let mut bytes = [0_u8; N];
    rand::fill(&mut bytes[..]);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A new DPoP key.
pub fn new_key() -> Keypair {
    Keypair::generate(Algorithm::P256, |bytes| rand::fill(bytes))
}

/// The DPoP proof of a request: `htu` is the address the PDS knows itself
/// by, without the query.
fn proof(
    key: &Keypair,
    method: &str,
    htu: &str,
    nonce: Option<&str>,
    token: Option<&str>,
) -> String {
    let jti = random_text::<16>();
    pds_oauth::agent::dpop_proof(
        key,
        pds_oauth::agent::ProofOf {
            method,
            htu,
            nonce,
            token,
            now_s: now_s(),
            jti: &jti,
        },
    )
}

/// An authorization that the browser has not finished.
#[derive(Debug)]
pub struct Pending {
    /// The DPoP key of the session to be.
    pub key: Keypair,
    /// The PKCE verifier.
    pub verifier: String,
}

/// What the token endpoint gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    /// The access token.
    pub access: String,
    /// The refresh token.
    pub refresh: Option<String>,
    /// The account.
    pub did: String,
}

/// A request to the ops listener.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    /// The path with its query, such as `/xrpc/internal.ops.getOverview`.
    pub path: &'a str,
    /// The reason of an action.
    pub reason: Option<&'a str>,
    /// The body of an action. `None` makes a GET.
    pub body: Option<&'a Value>,
}

impl<'a> Call<'a> {
    /// A read.
    pub const fn get(path: &'a str) -> Self {
        Self {
            path,
            reason: None,
            body: None,
        }
    }

    /// An action with its reason.
    pub const fn post(path: &'a str, reason: &'a str, body: &'a Value) -> Self {
        Self {
            path,
            reason: Some(reason),
            body: Some(body),
        }
    }
}

/// The connection of the console to the PDS.
#[derive(Debug)]
pub struct Client {
    config: Config,
    http: reqwest::Client,
    /// The last DPoP nonce the PDS gave.
    nonce: Mutex<Option<String>>,
}

impl Client {
    /// A client for the configuration. Requests end after 15 seconds and
    /// follow no redirect.
    pub fn new(config: Config) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Ok(Self {
            config,
            http,
            nonce: Mutex::new(None),
        })
    }

    /// The configuration.
    pub const fn config(&self) -> &Config {
        &self.config
    }

    fn nonce(&self) -> Option<String> {
        self.nonce
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keeps the nonce of an answer. `true` when it is a new one.
    fn keep_nonce(&self, headers: &HeaderMap) -> bool {
        let Some(given) = headers.get("dpop-nonce").and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let mut nonce = self.nonce.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = nonce.as_deref() != Some(given);
        *nonce = Some(given.to_owned());
        fresh
    }

    /// Sends a request with a proof. When the PDS answers with a nonce the
    /// console did not have, the request goes once more with it.
    async fn send(
        &self,
        key: &Keypair,
        method: &reqwest::Method,
        base: &str,
        path: &str,
        token: Option<&str>,
        shape: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<(u16, Value), ClientError> {
        let route = path.split('?').next().unwrap_or(path);
        let htu = format!("{}{route}", self.config.pds_public_url);
        let mut again = true;
        loop {
            let proof = proof(key, method.as_str(), &htu, self.nonce().as_deref(), token);
            let mut request = self
                .http
                .request(method.clone(), format!("{base}{path}"))
                .header("dpop", proof);
            if let Some(token) = token {
                request = request.header("authorization", format!("DPoP {token}"));
            }
            let response = shape(request)
                .send()
                .await
                .map_err(|error| ClientError::Transport(error.to_string()))?;
            let status = response.status().as_u16();
            let fresh = self.keep_nonce(response.headers());
            if again && fresh && matches!(status, 400 | 401) {
                again = false;
                continue;
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|error| ClientError::Transport(error.to_string()))?;
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            return Ok((status, body));
        }
    }

    fn refused(status: u16, body: &Value) -> ClientError {
        let text = |key: &str| body.get(key).and_then(Value::as_str);
        ClientError::Refused {
            status,
            error: text("error").unwrap_or("Error").to_owned(),
            message: text("message")
                .or_else(|| text("error_description"))
                .or_else(|| text("error"))
                .unwrap_or("the server refused the request")
                .to_owned(),
        }
    }

    async fn endpoint(
        &self,
        key: &Keypair,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<Value, ClientError> {
        let id = self.config.client_id();
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &id)
            .extend_pairs(form)
            .finish();
        let base = self.config.pds_internal_url.clone();
        let (status, answer) = self
            .send(key, &reqwest::Method::POST, &base, path, None, |request| {
                request
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(body.clone())
            })
            .await?;
        if (200..300).contains(&status) {
            Ok(answer)
        } else {
            Err(Self::refused(status, &answer))
        }
    }

    /// Pushes an authorization request. Gives the address to send the
    /// browser to, and what the console keeps until the browser is back.
    pub async fn start(&self, state: &str) -> Result<(String, Pending), ClientError> {
        let key = new_key();
        let verifier = random_text::<32>();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let redirect = self.config.redirect_uri();
        let pushed = self
            .endpoint(
                &key,
                "/oauth/par",
                &[
                    ("response_type", "code"),
                    ("code_challenge", &challenge),
                    ("code_challenge_method", "S256"),
                    ("state", state),
                    ("redirect_uri", &redirect),
                    ("scope", SCOPE),
                ],
            )
            .await?;
        let request_uri = pushed
            .get("request_uri")
            .and_then(Value::as_str)
            .ok_or_else(|| ClientError::Transport("the server gave no request_uri".to_owned()))?;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.config.client_id())
            .append_pair("request_uri", request_uri)
            .finish();
        let address = format!("{}/oauth/authorize?{query}", self.config.pds_public_url);
        Ok((address, Pending { key, verifier }))
    }

    fn tokens(answer: &Value) -> Result<Tokens, ClientError> {
        let text = |key: &str| answer.get(key).and_then(Value::as_str).map(str::to_owned);
        match (text("access_token"), text("sub")) {
            (Some(access), Some(did)) => Ok(Tokens {
                access,
                refresh: text("refresh_token"),
                did,
            }),
            _ => Err(ClientError::Transport(
                "the token endpoint gave no session".to_owned(),
            )),
        }
    }

    /// Exchanges the code of the redirect for a session.
    pub async fn finish(&self, pending: &Pending, code: &str) -> Result<Tokens, ClientError> {
        let redirect = self.config.redirect_uri();
        let answer = self
            .endpoint(
                &pending.key,
                "/oauth/token",
                &[
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("code_verifier", &pending.verifier),
                    ("redirect_uri", &redirect),
                ],
            )
            .await?;
        Self::tokens(&answer)
    }

    /// Takes new tokens for a session.
    pub async fn refresh(&self, key: &Keypair, refresh: &str) -> Result<Tokens, ClientError> {
        let answer = self
            .endpoint(
                key,
                "/oauth/token",
                &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
            )
            .await?;
        Self::tokens(&answer)
    }

    /// Ends a session at the PDS. A failure is not reported: the console
    /// forgets the session in every case.
    pub async fn revoke(&self, key: &Keypair, token: &str) {
        let _ = self
            .endpoint(key, "/oauth/revoke", &[("token", token)])
            .await;
    }

    /// Opens a stream of the ops listener with the session of an operator:
    /// a WebSocket whose handshake carries the token and its proof. When
    /// the PDS answers with a nonce the console did not have, the handshake
    /// goes once more with it.
    pub async fn stream(
        &self,
        key: &Keypair,
        token: &str,
        path: &str,
    ) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, ClientError> {
        let htu = format!("{}{path}", self.config.pds_public_url);
        let address = format!("{}{path}", self.config.ops_url.replacen("http", "ws", 1));
        let mut again = true;
        loop {
            let proof = proof(key, "GET", &htu, self.nonce().as_deref(), Some(token));
            let mut request = address
                .as_str()
                .into_client_request()
                .map_err(|error| ClientError::Transport(error.to_string()))?;
            let headers = request.headers_mut();
            let value = |text: String| {
                HeaderValue::from_str(&text)
                    .map_err(|error| ClientError::Transport(error.to_string()))
            };
            headers.insert("authorization", value(format!("DPoP {token}"))?);
            headers.insert("dpop", value(proof)?);
            match tokio_tungstenite::connect_async(request).await {
                Ok((stream, _)) => return Ok(stream),
                Err(WsError::Http(response)) => {
                    let status = response.status().as_u16();
                    let fresh = self.keep_nonce(response.headers());
                    if again && fresh && matches!(status, 400 | 401) {
                        again = false;
                        continue;
                    }
                    let body = response
                        .body()
                        .as_deref()
                        .and_then(|bytes| serde_json::from_slice(bytes).ok())
                        .unwrap_or(Value::Null);
                    return Err(Self::refused(status, &body));
                }
                Err(error) => return Err(ClientError::Transport(error.to_string())),
            }
        }
    }

    /// Calls the ops listener with the session of an operator.
    pub async fn ops(
        &self,
        key: &Keypair,
        token: &str,
        call: Call<'_>,
    ) -> Result<Value, ClientError> {
        let method = if call.body.is_some() {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };
        let base = self.config.ops_url.clone();
        let (status, answer) = self
            .send(
                key,
                &method,
                &base,
                call.path,
                Some(token),
                |mut request| {
                    if let Some(reason) = call.reason {
                        request = request.header("x-ops-reason", reason);
                    }
                    if let Some(body) = call.body {
                        request = request
                            .header("content-type", "application/json")
                            .body(body.to_string());
                    }
                    request
                },
            )
            .await?;
        if (200..300).contains(&status) {
            Ok(answer)
        } else {
            Err(Self::refused(status, &answer))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proof_names_the_request_the_nonce_and_the_token() {
        let key = new_key();
        let made = proof(
            &key,
            "POST",
            "https://pds.example/oauth/par",
            Some("n-1"),
            Some("tok"),
        );
        let parts: Vec<&str> = made.split('.').collect();
        assert_eq!(parts.len(), 3);
        let decode = |part: &str| -> Value {
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
        };
        let header = decode(parts[0]);
        assert_eq!(header["typ"], "dpop+jwt");
        assert_eq!(header["jwk"]["crv"], "P-256");
        let claims = decode(parts[1]);
        assert_eq!(claims["htm"], "POST");
        assert_eq!(claims["htu"], "https://pds.example/oauth/par");
        assert_eq!(claims["nonce"], "n-1");
        assert_eq!(
            claims["ath"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(b"tok")).as_str()
        );
        assert_eq!(URL_SAFE_NO_PAD.decode(parts[2]).unwrap().len(), 64);
        // Each proof has an identifier of its own.
        let other = proof(&key, "POST", "https://pds.example/oauth/par", None, None);
        assert_ne!(
            decode(other.split('.').nth(1).unwrap())["jti"],
            claims["jti"]
        );
    }
}

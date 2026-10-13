//! The MCP server as a confidential OAuth client of the PDS
//! (`docs/SPEC.md` §22.2).
//!
//! It authenticates with a `private_key_jwt` assertion and holds a DPoP
//! key for each session. A token of an AI client never reaches the PDS,
//! and a token of the PDS never reaches an AI client.

use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pds_identity::{Algorithm, Keypair};
use pds_oauth::agent::{ProofOf, client_assertion, dpop_proof};
use reqwest::header::HeaderMap;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::Config;

/// The header that names the AI client of a write to the PDS, for its
/// audit log.
pub const MCP_CLIENT_HEADER: &str = "x-mcp-client-id";

/// Why a call to the PDS did not give what the server asked for.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdsError {
    /// The PDS could not be reached, or sent no JSON.
    #[error("the PDS did not answer: {0}")]
    Transport(String),
    /// The PDS refused.
    #[error("{message}")]
    Refused {
        /// The HTTP status.
        status: u16,
        /// The name of the error.
        error: String,
        /// The message of the PDS.
        message: String,
    },
}

/// The time in seconds.
pub fn now_s() -> u64 {
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

/// The SHA-256 of a token as text. The server keeps tokens by their hash.
pub fn hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

/// A session at the PDS.
#[derive(Debug)]
pub struct PdsSession {
    /// The DPoP key of the session.
    pub key: Keypair,
    /// The account.
    pub did: String,
    tokens: Mutex<(String, Option<String>)>,
}

impl PdsSession {
    fn tokens(&self) -> (String, Option<String>) {
        self.tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// An authorization at the PDS that the browser has not finished.
#[derive(Debug)]
pub struct PdsPending {
    key: Keypair,
    verifier: String,
}

/// A call of an XRPC method.
#[derive(Debug, Clone, Copy)]
pub struct Xrpc<'a> {
    /// The NSID.
    pub nsid: &'a str,
    /// The parameters of a query.
    pub params: &'a [(&'a str, &'a str)],
    /// The input of a procedure. `None` makes a GET.
    pub input: Option<&'a Value>,
    /// The AI client, for the audit log of the PDS.
    pub ai_client: &'a str,
}

/// The connection of the server to the PDS.
#[derive(Debug)]
pub struct Pds {
    config: Config,
    http: reqwest::Client,
    nonce: Mutex<Option<String>>,
}

impl Pds {
    /// A connection for the configuration. Requests end after 15 seconds
    /// and follow no redirect.
    pub fn new(config: Config) -> Result<Self, PdsError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|error| PdsError::Transport(error.to_string()))?;
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

    fn keep_nonce(&self, headers: &HeaderMap) -> bool {
        let Some(given) = headers.get("dpop-nonce").and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let mut nonce = self.nonce.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = nonce.as_deref() != Some(given);
        *nonce = Some(given.to_owned());
        fresh
    }

    /// Sends a request with a DPoP proof. When the PDS answers with a
    /// nonce the server did not have, the request goes once more.
    async fn send(
        &self,
        key: &Keypair,
        method: &reqwest::Method,
        path: &str,
        token: Option<&str>,
        shape: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<(u16, Value), PdsError> {
        let route = path.split('?').next().unwrap_or(path);
        let htu = format!("{}{route}", self.config.pds_public_url);
        let mut again = true;
        loop {
            let jti = random_text::<16>();
            let nonce = self.nonce();
            let of = ProofOf {
                method: method.as_str(),
                htu: &htu,
                nonce: nonce.as_deref(),
                token,
                now_s: now_s(),
                jti: &jti,
            };
            let mut request = self
                .http
                .request(
                    method.clone(),
                    format!("{}{path}", self.config.pds_internal_url),
                )
                .header("dpop", dpop_proof(key, of));
            if let Some(token) = token {
                request = request.header("authorization", format!("DPoP {token}"));
            }
            let response = shape(request)
                .send()
                .await
                .map_err(|error| PdsError::Transport(error.to_string()))?;
            let status = response.status().as_u16();
            let fresh = self.keep_nonce(response.headers());
            if again && fresh && matches!(status, 400 | 401) {
                again = false;
                continue;
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|error| PdsError::Transport(error.to_string()))?;
            return Ok((
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            ));
        }
    }

    fn refused(status: u16, body: &Value) -> PdsError {
        let text = |key: &str| body.get(key).and_then(Value::as_str);
        PdsError::Refused {
            status,
            error: text("error").unwrap_or("Error").to_owned(),
            message: text("message")
                .or_else(|| text("error_description"))
                .or_else(|| text("error"))
                .unwrap_or("the PDS refused the request")
                .to_owned(),
        }
    }

    /// A request to an endpoint of the authorization server, with the
    /// assertion of the client.
    async fn endpoint(
        &self,
        key: &Keypair,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<Value, PdsError> {
        let id = self.config.client_id();
        let kid = self.config.key_id();
        let (status, answer) = self
            .send(key, &reqwest::Method::POST, path, None, |request| {
                // An assertion is good once, so each try has its own.
                let assertion = client_assertion(
                    &self.config.client_key,
                    &kid,
                    &id,
                    &self.config.pds_public_url,
                    now_s(),
                    &random_text::<16>(),
                );
                let body = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("client_id", &id)
                    .append_pair(
                        "client_assertion_type",
                        "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                    )
                    .append_pair("client_assertion", &assertion)
                    .extend_pairs(form)
                    .finish();
                request
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(body)
            })
            .await?;
        if (200..300).contains(&status) {
            Ok(answer)
        } else {
            Err(Self::refused(status, &answer))
        }
    }

    /// Pushes an authorization request for `scope`. Gives the address to
    /// send the browser to, and what the server keeps until it is back.
    pub async fn start(&self, state: &str, scope: &str) -> Result<(String, PdsPending), PdsError> {
        let key = Keypair::generate(Algorithm::P256, |bytes| rand::fill(bytes));
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
                    ("scope", scope),
                ],
            )
            .await?;
        let request_uri = pushed
            .get("request_uri")
            .and_then(Value::as_str)
            .ok_or_else(|| PdsError::Transport("the PDS gave no request_uri".to_owned()))?;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.config.client_id())
            .append_pair("request_uri", request_uri)
            .finish();
        let address = format!("{}/oauth/authorize?{query}", self.config.pds_public_url);
        Ok((address, PdsPending { key, verifier }))
    }

    fn tokens(answer: &Value) -> Result<(String, Option<String>, String), PdsError> {
        let text = |key: &str| answer.get(key).and_then(Value::as_str).map(str::to_owned);
        match (text("access_token"), text("sub")) {
            (Some(access), Some(did)) => Ok((access, text("refresh_token"), did)),
            _ => Err(PdsError::Transport(
                "the token endpoint gave no session".to_owned(),
            )),
        }
    }

    /// Exchanges the code of the redirect for a session.
    pub async fn finish(&self, pending: PdsPending, code: &str) -> Result<PdsSession, PdsError> {
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
        let (access, refresh, did) = Self::tokens(&answer)?;
        Ok(PdsSession {
            key: pending.key,
            did,
            tokens: Mutex::new((access, refresh)),
        })
    }

    /// Ends a session at the PDS. A failure is not reported: the server
    /// forgets the session in every case.
    pub async fn revoke(&self, session: &PdsSession) {
        let (access, _) = session.tokens();
        let _ = self
            .endpoint(&session.key, "/oauth/revoke", &[("token", &access)])
            .await;
    }

    async fn refresh(&self, session: &PdsSession, refresh: &str) -> Result<String, PdsError> {
        let answer = self
            .endpoint(
                &session.key,
                "/oauth/token",
                &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
            )
            .await?;
        let (access, next, _) = Self::tokens(&answer)?;
        *session
            .tokens
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (access.clone(), next);
        Ok(access)
    }

    async fn once(
        &self,
        session: &PdsSession,
        access: &str,
        call: Xrpc<'_>,
    ) -> Result<(u16, Value), PdsError> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(call.params)
            .finish();
        let mut path = format!("/xrpc/{}", call.nsid);
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query);
        }
        let method = if call.input.is_some() {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };
        self.send(&session.key, &method, &path, Some(access), |mut request| {
            request = request.header(MCP_CLIENT_HEADER, call.ai_client);
            if let Some(input) = call.input {
                request = request
                    .header("content-type", "application/json")
                    .body(input.to_string());
            }
            request
        })
        .await
    }

    /// Calls a method of the PDS with a session. When the PDS no longer
    /// takes the access token, the server refreshes the session once.
    pub async fn xrpc(&self, session: &PdsSession, call: Xrpc<'_>) -> Result<Value, PdsError> {
        let (access, refresh) = session.tokens();
        let (mut status, mut answer) = self.once(session, &access, call).await?;
        if status == 401
            && let Some(refresh) = refresh
        {
            let access = self.refresh(session, &refresh).await?;
            (status, answer) = self.once(session, &access, call).await?;
        }
        if (200..300).contains(&status) {
            Ok(answer)
        } else {
            Err(Self::refused(status, &answer))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A session of `did` with tokens that no PDS issued.
    pub fn session(did: &str) -> PdsSession {
        PdsSession {
            key: Keypair::generate(Algorithm::P256, |bytes| bytes.fill(9)),
            did: did.to_owned(),
            tokens: Mutex::new(("access".to_owned(), None)),
        }
    }

    #[test]
    fn a_token_is_kept_by_its_hash() {
        assert_eq!(hash("a"), hash("a"));
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(hash("a").len(), 43);
        assert_ne!(random_text::<16>(), random_text::<16>());
    }
}

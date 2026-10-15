//! A sidecar as an OAuth client of the PDS with DPoP (`docs/SPEC.md` §22):
//! the pushed authorization request, the token endpoint, the revocation,
//! and every request that a proof of the session's key signs.
//!
//! A sidecar holds no password. The person signs in on the pages of the
//! PDS, and the sidecar keeps the session with the DPoP key it made for it.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pds_identity::{Algorithm, Keypair};
use pds_oauth::agent::{ProofOf, client_assertion, dpop_proof};
use reqwest::header::HeaderMap;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Why a call to the PDS did not give what the sidecar asked for.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
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

impl ClientError {
    /// Whether the PDS no longer takes the session.
    pub const fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Refused { status: 401, .. })
    }
}

/// The time in seconds.
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

/// The SHA-256 of a token as base64url text, for a table that keeps
/// tokens by their hash.
pub fn hash(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
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
    dpop_proof(
        key,
        ProofOf {
            method,
            htu,
            nonce,
            token,
            now_s: now_s(),
            jti: &jti,
        },
    )
}

/// The key of a confidential client, which signs a `private_key_jwt`
/// assertion for each request to the authorization server (RFC 7523).
#[derive(Debug, Clone, Copy)]
pub struct ClientKey<'a> {
    /// The key.
    pub key: &'a Keypair,
    /// The name of the key in the key set of the client.
    pub kid: &'a str,
}

/// The PDS as the sidecar sees it, and the sidecar as the PDS sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// The public address of the PDS: its issuer, which every proof names.
    pub issuer: String,
    /// Where the sidecar reaches the PDS. Often the issuer.
    pub internal_url: String,
    /// The OAuth client id of the sidecar.
    pub client_id: String,
    /// Where the PDS sends the browser after the authorization.
    pub redirect_uri: String,
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

/// The connection of a sidecar to the PDS.
#[derive(Debug)]
pub struct DpopClient {
    http: reqwest::Client,
    peer: Peer,
    /// The last DPoP nonce the PDS gave.
    nonce: Mutex<Option<String>>,
}

impl DpopClient {
    /// A client of `peer`. Requests end after 15 seconds and follow no
    /// redirect.
    pub fn new(peer: Peer) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| ClientError::Transport(error.to_string()))?;
        Ok(Self {
            http,
            peer,
            nonce: Mutex::new(None),
        })
    }

    fn nonce(&self) -> Option<String> {
        self.nonce
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The DPoP proof of a request to `route` of the PDS with the last
    /// nonce the PDS gave. `route` has no query. A request that `send`
    /// cannot make, such as the handshake of a WebSocket, takes its proof
    /// here and gives the answer to [`Self::keep_nonce`].
    pub fn proof(&self, key: &Keypair, method: &str, route: &str, token: Option<&str>) -> String {
        let htu = format!("{}{route}", self.peer.issuer);
        proof(key, method, &htu, self.nonce().as_deref(), token)
    }

    /// Keeps the nonce of an answer. `true` when it is a new one, which
    /// a 400 or a 401 asks the request to go once more with.
    pub fn keep_nonce(&self, headers: &HeaderMap) -> bool {
        let Some(given) = headers.get("dpop-nonce").and_then(|v| v.to_str().ok()) else {
            return false;
        };
        let mut nonce = self.nonce.lock().unwrap_or_else(PoisonError::into_inner);
        let fresh = nonce.as_deref() != Some(given);
        *nonce = Some(given.to_owned());
        fresh
    }

    /// Sends a request with a proof of `key` to `path` at `base`: the PDS,
    /// or one of its listeners. The proof names the issuer. When the PDS
    /// answers with a nonce the sidecar did not have, the request goes once
    /// more with it. Gives the status and the JSON body, `Null` when there
    /// is none.
    pub async fn send(
        &self,
        key: &Keypair,
        method: &reqwest::Method,
        base: &str,
        path: &str,
        token: Option<&str>,
        shape: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<(u16, Value), ClientError> {
        let route = path.split('?').next().unwrap_or(path);
        let mut again = true;
        loop {
            let proof = self.proof(key, method.as_str(), route, token);
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

    /// The refusal in an answer: the error and the message the PDS named.
    pub fn refused(status: u16, body: &Value) -> ClientError {
        let text = |key: &str| body.get(key).and_then(Value::as_str);
        ClientError::Refused {
            status,
            error: text("error").unwrap_or("Error").to_owned(),
            message: text("message")
                .or_else(|| text("error_description"))
                .or_else(|| text("error"))
                .unwrap_or("the PDS refused the request")
                .to_owned(),
        }
    }

    /// A request to an endpoint of the authorization server: a form with
    /// the client id. A confidential client adds its assertion, a new one
    /// for each try, because an assertion is good once.
    pub async fn endpoint(
        &self,
        key: &Keypair,
        path: &str,
        form: &[(&str, &str)],
        assertion: Option<ClientKey<'_>>,
    ) -> Result<Value, ClientError> {
        let id = &self.peer.client_id;
        let (status, answer) = self
            .send(
                key,
                &reqwest::Method::POST,
                &self.peer.internal_url,
                path,
                None,
                |request| {
                    let mut body = url::form_urlencoded::Serializer::new(String::new());
                    body.append_pair("client_id", id);
                    if let Some(client) = assertion {
                        let assertion = client_assertion(
                            client.key,
                            client.kid,
                            id,
                            &self.peer.issuer,
                            now_s(),
                            &random_text::<16>(),
                        );
                        body.append_pair(
                            "client_assertion_type",
                            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                        )
                        .append_pair("client_assertion", &assertion);
                    }
                    request
                        .header("content-type", "application/x-www-form-urlencoded")
                        .body(body.extend_pairs(form).finish())
                },
            )
            .await?;
        if (200..300).contains(&status) {
            Ok(answer)
        } else {
            Err(Self::refused(status, &answer))
        }
    }

    /// Pushes an authorization request for `scope`. Gives the address to
    /// send the browser to, and what the sidecar keeps until the browser
    /// is back.
    pub async fn start(
        &self,
        state: &str,
        scope: &str,
        assertion: Option<ClientKey<'_>>,
    ) -> Result<(String, Pending), ClientError> {
        let key = new_key();
        let verifier = random_text::<32>();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let pushed = self
            .endpoint(
                &key,
                "/oauth/par",
                &[
                    ("response_type", "code"),
                    ("code_challenge", &challenge),
                    ("code_challenge_method", "S256"),
                    ("state", state),
                    ("redirect_uri", &self.peer.redirect_uri),
                    ("scope", scope),
                ],
                assertion,
            )
            .await?;
        let request_uri = pushed
            .get("request_uri")
            .and_then(Value::as_str)
            .ok_or_else(|| ClientError::Transport("the PDS gave no request_uri".to_owned()))?;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &self.peer.client_id)
            .append_pair("request_uri", request_uri)
            .finish();
        let address = format!("{}/oauth/authorize?{query}", self.peer.issuer);
        Ok((address, Pending { key, verifier }))
    }

    /// The session in an answer of the token endpoint.
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
    pub async fn finish(
        &self,
        pending: &Pending,
        code: &str,
        assertion: Option<ClientKey<'_>>,
    ) -> Result<Tokens, ClientError> {
        let answer = self
            .endpoint(
                &pending.key,
                "/oauth/token",
                &[
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("code_verifier", &pending.verifier),
                    ("redirect_uri", &self.peer.redirect_uri),
                ],
                assertion,
            )
            .await?;
        Self::tokens(&answer)
    }

    /// Takes new tokens for a session.
    pub async fn refresh(
        &self,
        key: &Keypair,
        refresh: &str,
        assertion: Option<ClientKey<'_>>,
    ) -> Result<Tokens, ClientError> {
        let answer = self
            .endpoint(
                key,
                "/oauth/token",
                &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
                assertion,
            )
            .await?;
        Self::tokens(&answer)
    }

    /// Ends a session at the PDS. A failure is not reported: the sidecar
    /// forgets the session in every case.
    pub async fn revoke(&self, key: &Keypair, token: &str, assertion: Option<ClientKey<'_>>) {
        let _ = self
            .endpoint(key, "/oauth/revoke", &[("token", token)], assertion)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

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

    #[test]
    fn a_token_is_kept_by_its_hash() {
        assert_eq!(hash("a"), hash("a"));
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(hash("a").len(), 43);
        assert_ne!(random_text::<16>(), random_text::<16>());
    }

    #[test]
    fn an_answer_names_the_refusal_or_the_session() {
        let refused = DpopClient::refused(403, &json!({ "error": "Forbidden" }));
        assert_eq!(
            refused,
            ClientError::Refused {
                status: 403,
                error: "Forbidden".to_owned(),
                message: "Forbidden".to_owned(),
            }
        );
        assert!(!refused.is_unauthorized());
        let bare = DpopClient::refused(401, &Value::Null);
        assert!(bare.is_unauthorized());
        assert_eq!(bare.to_string(), "the PDS refused the request");
        let described = DpopClient::refused(
            400,
            &json!({ "error": "invalid_grant", "error_description": "the code ended" }),
        );
        assert_eq!(described.to_string(), "the code ended");

        let tokens = DpopClient::tokens(&json!({
            "access_token": "a", "refresh_token": "r", "sub": "did:plc:x"
        }))
        .unwrap();
        assert_eq!(
            tokens,
            Tokens {
                access: "a".to_owned(),
                refresh: Some("r".to_owned()),
                did: "did:plc:x".to_owned(),
            }
        );
        assert!(DpopClient::tokens(&json!({ "access_token": "a" })).is_err());
    }
}

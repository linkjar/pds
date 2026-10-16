//! The OAuth authorization server and the resource-server check
//! (`docs/SPEC.md` §11) over HTTP.
//!
//! The rules are in `pds-oauth`. This module holds what needs the server:
//! the routes, the rows of `account.sqlite`, client metadata from the
//! network, and the state that lives in memory.

pub mod account;
pub mod authorize;
pub mod captcha;
pub mod clients;
pub mod endpoints;
pub mod external;
pub mod metadata;
pub mod notices;
pub mod resource;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_oauth::dpop::{DpopError, Expect, Nonces, Proof, check_proof};
use pds_storage::{AccountDb, StorageError};
use serde_json::{Value, json};
use sha2::Digest;

use crate::state::AppState;
use crate::table::Swept;

pub use pds_oauth::client::validate as validate_client;

/// Reads an OAuth access token: the claims, and the scope when the token
/// carries it. A server with an entryway, and the entryway itself, read
/// the ES256K tokens of the entryway (SPEC §21.3); any other server reads
/// the tokens of its own secret.
pub(crate) fn read_access_token(
    state: &AppState,
    token: &str,
) -> Option<(pds_oauth::token::AccessClaims, Option<String>)> {
    let auth = &state.config().auth;
    match &auth.entryway {
        Some(entryway) => {
            let from = state
                .config()
                .entryway
                .as_ref()
                .map_or_else(|| issuer(state), |entryway| entryway.url.clone());
            pds_oauth::token::verify_access_es256k(&entryway.key, &from, token)
                .map(|(claims, scope)| (claims, Some(scope)))
        }
        None => pds_oauth::token::verify_access(&auth.jwt_secret, &issuer(state), token)
            .map(|claims| (claims, None)),
    }
}

/// The OAuth settings.
#[derive(Clone, Default)]
pub struct OAuthConfig {
    /// `PDS_DPOP_SECRET`: 64 hexadecimal digits, or any text, which the
    /// server hashes. Without one the server draws its own, and nonces do
    /// not outlive the process.
    pub dpop_secret: Option<String>,
    /// `PDS_OAUTH_TRUSTED_CLIENTS`: the `client_id` of each trusted client.
    pub trusted_clients: Vec<String>,
    /// `PDS_DISABLE_SSRF_PROTECTION`: a client document may come from a
    /// private address.
    pub disable_ssrf_protection: bool,
    /// `PDS_HCAPTCHA_SITE_KEY`, `_SECRET_KEY` and `_TOKEN_SALT`, when all
    /// three are set.
    pub hcaptcha: Option<captcha::Hcaptcha>,
    /// `PDS_OAUTH_KNOWN_CLIENTS_FILE`: clients the operator knows, each
    /// with its metadata document. The server fetches nothing for them,
    /// so a sidecar on a private network can be one (SPEC §22).
    pub known_clients: Vec<(String, Value)>,
}

impl std::fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthConfig")
            .field("trusted_clients", &self.trusted_clients)
            .field("disable_ssrf_protection", &self.disable_ssrf_protection)
            .finish_non_exhaustive()
    }
}

/// How many entries a memory table holds before it drops the ended ones.
const TABLE_SWEEP_AT: usize = 4096;

/// What the authorization server keeps in memory.
pub struct OAuthState {
    nonces: Nonces,
    /// Things that are good once, with the time each stops mattering.
    seen: Mutex<Swept<String, i64, TABLE_SWEEP_AT>>,
    clients: Mutex<Swept<String, (Arc<clients::LoadedClient>, i64), TABLE_SWEEP_AT>>,
    pending: Mutex<Swept<String, authorize::PendingSignUp, TABLE_SWEEP_AT>>,
    /// The sign-ins at identity providers that are in progress.
    pub(crate) flows: external::Flows,
}

impl std::fmt::Debug for OAuthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthState").finish_non_exhaustive()
    }
}

impl OAuthState {
    /// The state for `config`.
    pub fn new(config: &OAuthConfig) -> Self {
        Self {
            nonces: Nonces::new(
                config
                    .dpop_secret
                    .as_deref()
                    .map_or_else(rand::random, |text| {
                        data_encoding::HEXLOWER_PERMISSIVE
                            .decode(text.as_bytes())
                            .ok()
                            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                            .unwrap_or_else(|| sha2::Sha256::digest(text.as_bytes()).into())
                    }),
            ),
            seen: Mutex::default(),
            clients: Mutex::new(clients::known(config).into()),
            pending: Mutex::default(),
            flows: external::Flows::default(),
        }
    }

    /// Records `key` until `until_ms`. `false` means it was recorded
    /// before and that record still counts.
    pub fn unique(&self, key: String, until_ms: i64, now_ms: i64) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.sweep(|until| *until > now_ms);
        match seen.get(&key) {
            Some(until) if *until > now_ms => false,
            _ => {
                seen.insert(key, until_ms);
                true
            }
        }
    }
}

/// A refusal of the authorization server: `{error, error_description}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthError {
    /// The status.
    pub status: StatusCode,
    /// The error code.
    pub error: String,
    /// What went wrong.
    pub description: String,
}

impl OAuthError {
    /// A refusal with status 400.
    pub fn new(error: &str, description: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: error.to_owned(),
            description: description.into(),
        }
    }

    /// `invalid_request`.
    pub fn invalid_request(description: impl Into<String>) -> Self {
        Self::new("invalid_request", description)
    }

    /// `invalid_grant`.
    pub fn invalid_grant(description: impl Into<String>) -> Self {
        Self::new("invalid_grant", description)
    }

    /// `invalid_client`.
    pub fn invalid_client(description: impl Into<String>) -> Self {
        Self::new("invalid_client", description)
    }

    /// A failure of the server.
    pub fn server(cause: impl std::fmt::Display) -> Self {
        tracing::error!(%cause, "the authorization server failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "server_error".to_owned(),
            description: "Internal Server Error".to_owned(),
        }
    }
}

impl From<StorageError> for OAuthError {
    fn from(error: StorageError) -> Self {
        Self::server(error)
    }
}

impl From<DpopError> for OAuthError {
    fn from(error: DpopError) -> Self {
        match error {
            DpopError::Invalid(message) => Self::new("invalid_dpop_proof", message),
            DpopError::UseNonce(message) => Self::new("use_dpop_nonce", message),
        }
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        json(
            self.status,
            &json!({ "error": self.error, "error_description": self.description }),
        )
    }
}

/// A JSON answer that no cache keeps.
pub fn json(status: StatusCode, body: &Value) -> Response {
    let mut response = (status, body.to_string()).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

/// The issuer: the public origin of the server.
pub fn issuer(state: &AppState) -> String {
    state.config().public_url()
}

/// The time as the Reference writes it in a row.
pub fn iso(ms: i64) -> String {
    pds_types::iso_millis(ms)
}

/// The milliseconds of a time from a row.
pub fn millis(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso).map_or(0, |time| time.timestamp_millis())
}

/// Random bytes for an identifier.
pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    rand::fill(&mut bytes[..]);
    bytes
}

/// A new identifier of the kind.
pub fn new_id(kind: pds_oauth::token::Id) -> String {
    let bytes: [u8; 32] = random();
    kind.format(&bytes[..kind.bytes()])
}

/// Runs a call on the account database, on the blocking pool.
pub async fn accounts<T, F>(state: &AppState, call: F) -> Result<T, OAuthError>
where
    T: Send + 'static,
    F: FnOnce(&mut AccountDb) -> Result<T, StorageError> + Send + 'static,
{
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        let writes = state
            .writes()
            .ok_or_else(|| OAuthError::server("a read-only server has no authorization server"))?;
        writes.with_accounts(call).map_err(OAuthError::from)
    })
    .await
    .map_err(OAuthError::server)?
}

/// The fields of a form or of a JSON object. A field with an empty value
/// counts as absent.
pub type Fields = BTreeMap<String, String>;

/// The largest body of an OAuth request.
const BODY_LIMIT: usize = 64 * 1024;

/// Reads the body of a request to the authorization server.
pub async fn read_fields(headers: &HeaderMap, body: Body) -> Result<Fields, OAuthError> {
    let kind = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let bytes = crate::body::collect(body, BODY_LIMIT)
        .await
        .map_err(|error| OAuthError::invalid_request(error.message))?;
    let mut fields = Fields::new();
    match kind.as_str() {
        "application/x-www-form-urlencoded" => {
            for (name, value) in url::form_urlencoded::parse(&bytes) {
                if !value.is_empty() {
                    fields.insert(name.into_owned(), value.into_owned());
                }
            }
        }
        "application/json" => {
            let object: serde_json::Map<String, Value> = serde_json::from_slice(&bytes)
                .map_err(|_| OAuthError::invalid_request("Invalid JSON body"))?;
            for (name, value) in object {
                match value {
                    Value::String(text) if text.is_empty() => {}
                    Value::String(text) => {
                        fields.insert(name, text);
                    }
                    Value::Null => {}
                    other => {
                        fields.insert(name, other.to_string());
                    }
                }
            }
        }
        _ => return Err(OAuthError::invalid_request("Unsupported content-type")),
    }
    Ok(fields)
}

/// Checks the DPoP proof of a request, when it has one. `access_token` is
/// the token the proof must name, on a resource request.
pub fn dpop_proof(
    state: &AppState,
    headers: &HeaderMap,
    method: &Method,
    path: &str,
    access_token: Option<&str>,
) -> Result<Option<Proof>, DpopError> {
    let mut values = headers.get_all("dpop").iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    let invalid = |message: &str| DpopError::Invalid(message.to_owned());
    if values.next().is_some() {
        return Err(invalid("DPoP header must contain a single proof"));
    }
    let proof = value
        .to_str()
        .map_err(|_| invalid("DPoP header must contain a single proof"))?;
    let oauth = state.oauth();
    let now_ms = state.now_ms();
    let known = |nonce: &str| oauth.nonces.check(nonce, now_ms);
    let proof = check_proof(
        proof,
        Expect {
            method: method.as_str(),
            url: &format!("{}{path}", issuer(state)),
            access_token,
            now_s: now_ms / 1000,
        },
        Some(&known),
    )?;
    // A proof is good once. A nonce is taken for three minutes, so the record lasts as long.
    let until = now_ms + 2 * pds_oauth::dpop::NONCE_MAX_AGE_MS;
    if !oauth.unique(format!("dpop:{}", proof.jti), until, now_ms) {
        return Err(invalid("DPoP proof replayed"));
    }
    Ok(Some(proof))
}

/// Adds the nonce a client should use next, and lets a browser read it.
pub fn with_nonce(state: &AppState, mut response: Response) -> Response {
    let nonce = state.oauth().nonces.next(state.now_ms());
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&nonce) {
        headers.insert("dpop-nonce", value);
    }
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("DPoP-Nonce, WWW-Authenticate"),
    );
    response
}

/// A file of the pages, with a year of cache.
fn asset(content_type: &'static str, bytes: &'static [u8]) -> Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn method_not_allowed() -> Response {
    OAuthError {
        status: StatusCode::METHOD_NOT_ALLOWED,
        error: "invalid_request".to_owned(),
        description: "Method not allowed".to_owned(),
    }
    .into_response()
}

/// The values of `Sec-Fetch-Site` that the authorization page takes. The
/// Reference names them in this order, and its message lists them so.
const AUTHORIZE_SITES: [&str; 4] = ["same-origin", "same-site", "cross-site", "none"];

/// Checks that a page is loaded as a browser loads a page: by navigation,
/// into a document, from its own origin when the browser names one
/// (SPEC §11.5). The Reference checks the same headers in the same order,
/// and a missing header fails as a wrong one does. `sites` is the values
/// of `Sec-Fetch-Site` the page takes; `None` leaves the header unchecked.
/// `Err` is the Reference's message.
fn navigation(headers: &HeaderMap, issuer: &str, sites: Option<&[&str]>) -> Result<(), String> {
    let expect = |name: &str, allowed: &[&str]| {
        let mut values = headers.get_all(name).iter();
        let value = values.next();
        if values.next().is_some() {
            return Err(format!("Invalid {name} header"));
        }
        match value.map(|value| value.to_str().unwrap_or_default()) {
            Some(value) if allowed.contains(&value) => Ok(()),
            Some(value) => Err(format!(
                "Forbidden {name} header \"{value}\" (expected {})",
                allowed.join(",")
            )),
            None => Err(format!("Missing {name} header")),
        }
    };
    if let Some(sites) = sites {
        expect("sec-fetch-site", sites)?;
    }
    expect("sec-fetch-mode", &["navigate"])?;
    expect("sec-fetch-dest", &["document"])?;
    match headers.get(header::ORIGIN) {
        Some(origin) if origin.as_bytes() != issuer.as_bytes() => Err(format!(
            "Invalid origin {}",
            origin.to_str().unwrap_or_default()
        )),
        _ => Ok(()),
    }
}

/// A page of the authorization server, after the checks of [`navigation`].
fn page_load(
    state: &AppState,
    headers: &HeaderMap,
    issuer: &str,
    sites: Option<&[&str]>,
) -> Option<Response> {
    navigation(headers, issuer, sites)
        .err()
        .map(|message| authorize::error_page(state, StatusCode::BAD_REQUEST, &[], &message))
}

/// Serves a route of the authorization server. `None` means the path is
/// not one.
pub async fn serve(
    state: &AppState,
    method: &Method,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    peer: Option<std::net::IpAddr>,
    body: &mut Option<Body>,
) -> Option<Response> {
    let reads = method == Method::GET || method == Method::HEAD;
    // The typeface and the marks of the pages (SPEC §11.4). Each keeps
    // its name for good, so a browser may keep it for a year.
    if let Some(name) = path.strip_prefix("/oauth/assets/") {
        let (content_type, bytes) = pds_oauth::theme::asset(name)?;
        return Some(if reads {
            asset(content_type, bytes)
        } else {
            method_not_allowed()
        });
    }
    let known = matches!(
        path,
        "/.well-known/oauth-authorization-server"
            | "/.well-known/oauth-protected-resource"
            | "/oauth/jwks"
            | "/oauth/par"
            | "/oauth/token"
            | "/oauth/revoke"
            | "/oauth/authorize"
            | "/oauth/authorize/sign-in"
            | "/oauth/authorize/sign-up"
            | "/oauth/authorize/accept"
            | "/oauth/authorize/reject"
            | "/account"
            | "/account/sign-in"
            | "/account/revoke"
            | "/account/sign-out"
            | "/account/unlink"
    ) || (path.starts_with("/account/") && path.ends_with("/manage"));
    state.writes()?;
    // A member host is a resource server only: its entryway is the
    // authorization server and has the account pages (SPEC §21.3).
    if state.config().entryway.is_some() {
        return (reads && path == "/.well-known/oauth-protected-resource")
            .then(|| metadata::protected_resource(state));
    }
    if path.starts_with("/oauth/external/") {
        return external::serve(state, method, path, query, headers, peer, body).await;
    }
    if !known {
        return None;
    }
    let take = |body: &mut Option<Body>| body.take().unwrap_or_else(Body::empty);
    let response = match path {
        "/.well-known/oauth-authorization-server" if reads => metadata::authorization_server(state),
        "/.well-known/oauth-protected-resource" if reads => metadata::protected_resource(state),
        "/oauth/jwks" if reads => metadata::jwks(),
        "/oauth/authorize" if reads => {
            match page_load(state, headers, &issuer(state), Some(&AUTHORIZE_SITES)) {
                Some(refused) => refused,
                None => authorize::page(state, query, headers, peer).await,
            }
        }
        _ if reads
            && (path == "/account"
                || (path.starts_with("/account/") && path.ends_with("/manage"))) =>
        {
            match page_load(state, headers, &issuer(state), None) {
                Some(refused) => refused,
                None => account::page(state, path, query, headers, peer).await,
            }
        }
        _ if method != Method::POST => method_not_allowed(),
        "/oauth/par" => {
            let answer = endpoints::par(state, headers, take(body)).await;
            with_nonce(state, answer.unwrap_or_else(IntoResponse::into_response))
        }
        "/oauth/token" => {
            let answer = endpoints::token(state, headers, take(body)).await;
            with_nonce(state, answer.unwrap_or_else(IntoResponse::into_response))
        }
        "/oauth/revoke" => {
            let answer = endpoints::revoke(state, headers, take(body)).await;
            with_nonce(state, answer.unwrap_or_else(IntoResponse::into_response))
        }
        "/oauth/authorize/sign-in" => authorize::sign_in(state, headers, peer, take(body)).await,
        "/oauth/authorize/sign-up" => authorize::sign_up(state, headers, peer, take(body)).await,
        "/oauth/authorize/accept" => authorize::accept(state, headers, peer, take(body)).await,
        "/oauth/authorize/reject" => authorize::reject(state, headers, peer, take(body)).await,
        "/account/sign-in" | "/account/revoke" | "/account/sign-out" | "/account/unlink" => {
            account::post(state, path, headers, peer, take(body)).await
        }
        _ => method_not_allowed(),
    };
    Some(response)
}

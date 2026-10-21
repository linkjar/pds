//! The proxy (`docs/SPEC.md` §5.4).
//!
//! A method this server has no handler for goes to another service, with
//! a token that the account's key signs, as `pipethrough.ts` at the pin.
//! `com.atproto.server.getServiceAuth` gives such a token to the caller.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use data_encoding::HEXLOWER;
use pds_identity::safe_fetch::{self, Policy};
use pds_identity::{DidDocument, Keypair};
use pds_types::Did;
use serde_json::{Value, json};

use super::Query;
use crate::auth::{self, Credentials, SCOPE_ACCESS, SCOPE_APP_PASS_PRIVILEGED, SCOPE_TAKENDOWN};
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::state::AppState;

/// Account management: never proxied and never the method of a service
/// token.
const PROTECTED_METHODS: &[&str] = &[
    "com.atproto.admin.sendEmail",
    "com.atproto.identity.requestPlcOperationSignature",
    "com.atproto.identity.signPlcOperation",
    "com.atproto.identity.updateHandle",
    "com.atproto.server.activateAccount",
    "com.atproto.server.confirmEmail",
    "com.atproto.server.createAppPassword",
    "com.atproto.server.deactivateAccount",
    "com.atproto.server.getAccountInviteCodes",
    "com.atproto.server.getSession",
    "com.atproto.server.listAppPasswords",
    "com.atproto.server.requestAccountDelete",
    "com.atproto.server.requestEmailConfirmation",
    "com.atproto.server.requestEmailUpdate",
    "com.atproto.server.revokeAppPassword",
    "com.atproto.server.updateEmail",
];

/// The response headers of the other service that the caller receives.
const FORWARDED: &[&str] = &[
    "content-length",
    "content-encoding",
    "content-type",
    "content-language",
    "atproto-repo-rev",
    "atproto-content-labelers",
    "retry-after",
];

/// The method whose service the request body names.
const REGISTER_PUSH: &str = "app.bsky.notification.registerPush";

/// The longest request body the proxy forwards.
const BODY_LIMIT: usize = 16 * 1024 * 1024;

fn listed(list: &[&str], lxm: &str) -> bool {
    list.iter().any(|name| name.eq_ignore_ascii_case(lxm))
}

fn privileged(lxm: &str) -> bool {
    // New chat methods must inherit the same app-password restriction.
    lxm.get(.."chat.bsky.".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("chat.bsky."))
        || lxm.eq_ignore_ascii_case("com.atproto.server.createAccount")
}

fn bad_method() -> XrpcError {
    XrpcError::bad_request("InvalidToken", "Bad token method")
}

/// A service token: `iss` asks `aud` for the method `lxm`, signed with the
/// key of `iss`, as `createServiceJwt` of the Reference.
pub fn service_jwt(
    keypair: &Keypair,
    iss: &str,
    aud: &str,
    lxm: Option<&str>,
    iat: i64,
    exp: i64,
    jti: &[u8; 16],
) -> String {
    service_jwt_with(keypair, iss, aud, lxm, iat, exp, jti, &[])
}

/// [`service_jwt`] with more claims.
#[allow(clippy::too_many_arguments)]
fn service_jwt_with(
    keypair: &Keypair,
    iss: &str,
    aud: &str,
    lxm: Option<&str>,
    iat: i64,
    exp: i64,
    jti: &[u8; 16],
    extra: &[(&str, &str)],
) -> String {
    let header = json!({"typ": "JWT", "alg": "ES256K"});
    let mut payload = serde_json::Map::new();
    for (name, value) in extra {
        payload.insert((*name).to_owned(), (*value).into());
    }
    payload.insert("iat".to_owned(), iat.into());
    payload.insert("iss".to_owned(), iss.into());
    payload.insert("aud".to_owned(), aud.into());
    payload.insert("exp".to_owned(), exp.into());
    if let Some(lxm) = lxm {
        payload.insert("lxm".to_owned(), lxm.into());
    }
    payload.insert("jti".to_owned(), HEXLOWER.encode(jti).into());
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(Value::Object(payload).to_string())
    );
    let signature = keypair.sign(signed.as_bytes());
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// A service token of the account `did`, signed with its stored key.
pub(crate) async fn account_jwt(
    state: &AppState,
    did: &str,
    aud: &str,
    lxm: Option<&str>,
    exp: Option<i64>,
) -> Result<String, XrpcError> {
    account_jwt_with(state, did, aud, lxm, exp, &[]).await
}

/// [`account_jwt`] with more claims.
pub(crate) async fn account_jwt_with(
    state: &AppState,
    did: &str,
    aud: &str,
    lxm: Option<&str>,
    exp: Option<i64>,
    extra: &[(&str, &str)],
) -> Result<String, XrpcError> {
    let keypair = state.actor_key_async(did).await?;
    let mut jti = [0_u8; 16];
    if let Some(writes) = state.writes() {
        for chunk in jti.chunks_mut(8) {
            chunk.copy_from_slice(&writes.random().to_be_bytes());
        }
    }
    let iat = state.now_s();
    Ok(service_jwt_with(
        &keypair,
        did,
        aud,
        lxm,
        iat,
        exp.unwrap_or(iat + 60),
        &jti,
        extra,
    ))
}

/// Where a request goes: the URL of the service and its DID.
struct Target {
    url: String,
    did: String,
    /// Only explicit operator configuration can admit a private service.
    trusted: bool,
}

/// Reads `atproto-proxy`, as `parseProxyHeader` of the Reference.
async fn from_header(state: &AppState, proxy_to: &str) -> Result<Target, XrpcError> {
    let refuse = |message: &str| Err(XrpcError::invalid_request(message));
    let Some(hash) = proxy_to.find('#') else {
        return refuse("no service id specified in proxy header");
    };
    if hash == 0 {
        return refuse("no did specified in proxy header");
    }
    if hash == proxy_to.len() - 1 {
        return refuse("no service id specified in proxy header");
    }
    let (did, service) = (&proxy_to[..hash], &proxy_to[hash + 1..]);
    if service.contains('#') {
        return refuse("invalid proxy header format");
    }
    if proxy_to.contains(' ') {
        return refuse("proxy header cannot contain spaces");
    }
    let config = state.config();
    if let (Some(url), Some(appview)) = (&config.appview_url, &config.appview_did)
        && proxy_to == format!("{appview}#bsky_appview")
    {
        return Ok(Target {
            url: url.clone(),
            did: did.to_owned(),
            trusted: true,
        });
    }
    // A value that names no DID is the caller's error, a deliberate
    // difference (SPEC §2.3).
    let unresolved = || XrpcError::invalid_request("could not resolve proxy did");
    let parsed: Did = did.parse().map_err(|_| unresolved())?;
    let doc = state
        .resolver()
        .resolve(&parsed, false)
        .await
        .map_err(|_| unresolved())?
        .ok_or_else(unresolved)?;
    let endpoint = DidDocument::from_value(&doc)
        .ok()
        .and_then(|doc| doc.service_endpoint(&format!("#{service}"), None).ok())
        .ok_or_else(|| XrpcError::invalid_request("could not resolve proxy did service url"))?;
    Ok(Target {
        url: endpoint.origin(),
        did: did.to_owned(),
        trusted: false,
    })
}

/// The service a method goes to without `atproto-proxy`.
fn default_target(state: &AppState, lxm: &str) -> Result<Target, XrpcError> {
    let config = state.config();
    let service = if lxm.starts_with("tools.ozone.") {
        config.mod_service.clone()
    } else if lxm == "com.atproto.moderation.createReport" {
        config.report_service.clone()
    } else {
        config.appview_url.clone().zip(config.appview_did.clone())
    };
    service
        .map(|(url, did)| Target {
            url,
            did,
            trusted: true,
        })
        .ok_or_else(|| XrpcError::invalid_request(format!("No service configured for {lxm}")))
}

/// The name the Reference gives a status that came without one.
fn type_name(status: u16) -> (&'static str, &'static str) {
    match status {
        400 => ("InvalidRequest", "Invalid Request"),
        401 => ("AuthenticationRequired", "Authentication Required"),
        403 => ("Forbidden", "Forbidden"),
        404 => ("XRPCNotSupported", "XRPC Not Supported"),
        406 => ("NotAcceptable", "Not Acceptable"),
        413 => ("PayloadTooLarge", "Payload Too Large"),
        415 => ("UnsupportedMediaType", "Unsupported Media Type"),
        429 => ("RateLimitExceeded", "Rate Limit Exceeded"),
        501 => ("MethodNotImplemented", "Method Not Implemented"),
        502 => ("UpstreamFailure", "Upstream Failure"),
        503 => ("NotEnoughResources", "Not Enough Resources"),
        504 => ("UpstreamTimeout", "Upstream Timeout"),
        _ => ("InternalServerError", "Internal Server Error"),
    }
}

fn upstream_failure(message: &str) -> XrpcError {
    XrpcError {
        status: StatusCode::BAD_GATEWAY,
        error: "UpstreamFailure".to_owned(),
        message: message.to_owned(),
    }
}

fn outbound_request(
    state: &AppState,
    target: &Target,
    method: &Method,
    path: &str,
) -> Result<reqwest::RequestBuilder, XrpcError> {
    let url = format!("{}{path}", target.url);
    if target.trusted {
        return Ok(state.http().request(method.clone(), &url));
    }
    let policy = if state.config().dev_mode {
        Policy::Development
    } else {
        Policy::Public
    };
    safe_fetch::request(method.clone(), &url, policy)
        .map_err(|_| XrpcError::invalid_request("proxy destination is not allowed"))
}

/// Sends a method that this server does not handle to the service that
/// does. `path` is the path and the query as they arrived.
pub async fn proxy(
    state: &AppState,
    lxm: &str,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    if method != Method::GET && method != Method::HEAD && method != Method::POST {
        return Err(XrpcError::invalid_request(
            "XRPC requests only supports GET and POST",
        ));
    }
    if listed(PROTECTED_METHODS, lxm) {
        return Err(bad_method());
    }
    let (did, scope) =
        match auth::optional_credentials(headers, &state.config().auth, &[], state.now_s())? {
            Credentials::Access { did, scope } => (did, scope),
            Credentials::None => return Err(XrpcError::auth_required("AuthMissing")),
            Credentials::Admin => {
                return Err(XrpcError::bad_request(
                    "InvalidToken",
                    "Unexpected authorization type",
                ));
            }
        };
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    // A push registration names its service in the body.
    let mut collected = None;
    let mut push_service = None;
    if lxm == REGISTER_PUSH && method == Method::POST {
        let bytes = body::collect(body.take().unwrap_or_default(), BODY_LIMIT).await?;
        push_service = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|input| input.get("serviceDid")?.as_str().map(str::to_owned))
            .filter(|service| Some(service) != state.config().appview_did.as_ref());
        collected = Some(bytes);
    }
    let target = match (header("atproto-proxy"), &push_service) {
        (_, Some(service)) => from_header(state, &format!("{service}#bsky_notif")).await?,
        (Some(proxy_to), None) => from_header(state, proxy_to).await?,
        (None, None) => default_target(state, lxm)?,
    };
    if privileged(lxm) && scope != SCOPE_ACCESS && scope != SCOPE_APP_PASS_PRIVILEGED {
        return Err(bad_method());
    }
    // An OAuth session calls the services its scope names.
    let service = if lxm.starts_with("tools.ozone.") || lxm == "com.atproto.moderation.createReport"
    {
        "atproto_labeler"
    } else if lxm == REGISTER_PUSH {
        "bsky_notif"
    } else {
        "bsky_appview"
    };
    let audience =
        header("atproto-proxy").map_or_else(|| format!("{}#{service}", target.did), str::to_owned);
    crate::oauth::resource::check_rpc(lxm, &audience)?;
    let token = account_jwt(state, &did, &target.did, Some(lxm), None).await?;

    let mut request = outbound_request(state, &target, method, path)?
        .timeout(state.config().proxy_timeout)
        .header(
            "accept-encoding",
            header("accept-encoding").unwrap_or("identity"),
        )
        .header("authorization", format!("Bearer {token}"));
    for (name, value) in headers {
        let name = name.as_str();
        let passed = name == "accept-language"
            || name == "atproto-accept-labelers"
            || name == "x-bsky-topics"
            || name.starts_with("x-atproto-");
        if passed {
            request = request.header(name, value);
        }
    }
    if method == Method::POST {
        for name in ["content-type", "content-encoding"] {
            if let Some(value) = headers.get(name) {
                request = request.header(name, value);
            }
        }
        let bytes = match collected {
            Some(bytes) => bytes,
            None => body::collect(body.take().unwrap_or_default(), BODY_LIMIT).await?,
        };
        request = request.body(bytes);
    }
    let upstream = request.send().await.map_err(|error| {
        tracing::error!(%error, "Upstream service unreachable");
        if error.is_timeout() {
            XrpcError {
                status: StatusCode::GATEWAY_TIMEOUT,
                error: "UpstreamTimeout".to_owned(),
                message: "Upstream service timed out".to_owned(),
            }
        } else {
            upstream_failure("Upstream service unreachable")
        }
    })?;

    // The reads that the account's own writes change (SPEC §4.5).
    if method != Method::POST && super::read_after_write::applies(lxm) {
        let query = path.split_once('?').map(|(_, query)| query);
        return super::read_after_write::overlay(state, lxm, &did, query, upstream).await;
    }
    answer(upstream).await
}

/// A refusal of the other service as this server's refusal, with the name
/// and the message it gave.
pub(crate) async fn refusal(upstream: reqwest::Response) -> XrpcError {
    let status = upstream.status();
    let is_json = upstream
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|kind| kind.contains("json"));
    let parsed: Value = if is_json {
        let Ok(bytes) = safe_fetch::read_bounded(upstream, 64 * 1024).await else {
            return upstream_failure("upstream error response failed or exceeded the size limit");
        };
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    // A failure of the other service is not this server's failure.
    let status = if status.as_u16() == 500 {
        502
    } else {
        status.as_u16()
    };
    let (name, text) = type_name(status);
    XrpcError {
        status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        error: parsed["error"].as_str().unwrap_or(name).to_owned(),
        message: parsed["message"].as_str().unwrap_or(text).to_owned(),
    }
}

/// The answer of the other service as this server's: a refusal with its
/// name and message, or the status, the forwarded headers and the body as
/// it arrives.
pub(crate) async fn answer(upstream: reqwest::Response) -> Result<Response, XrpcError> {
    let status = upstream.status();
    if status.as_u16() >= 400 {
        return Err(refusal(upstream).await);
    }
    let mut response = Response::builder().status(status);
    for name in FORWARDED {
        if let Some(value) = upstream.headers().get(*name)
            && let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_bytes(value.as_bytes()),
            )
        {
            response = response.header(name, value);
        }
    }
    let stream = futures_util::stream::unfold(upstream, |mut upstream| async move {
        match upstream.chunk().await {
            Ok(Some(chunk)) => Some((Ok(chunk), upstream)),
            Ok(None) => None,
            Err(error) => Some((Err(error), upstream)),
        }
    });
    response
        .body(Body::from_stream(stream))
        .map_err(XrpcError::internal)
}

/// `com.atproto.server.getServiceAuth`.
pub async fn get_service_auth(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let credentials = auth::optional_credentials(
        query.headers,
        &state.config().auth,
        &[SCOPE_TAKENDOWN],
        state.now_s(),
    )?;
    let Credentials::Access { did, scope } = credentials else {
        return Err(XrpcError::auth_required("AuthMissing"));
    };
    let aud = query.params.text("aud");
    let lxm = query.params.get("lxm");
    let exp = query
        .params
        .get("exp")
        .and_then(|exp| exp.parse::<i64>().ok());
    let aud_did = aud.split_once('#').map_or(aud, |(did, _)| did);
    if aud_did.parse::<Did>().is_err() || aud.ends_with('#') {
        return Err(XrpcError::invalid_request(
            "aud must be a valid atproto DID or did#serviceId reference",
        ));
    }
    crate::oauth::resource::check_rpc(lxm.unwrap_or("*"), aud)?;
    if scope == SCOPE_TAKENDOWN && lxm != Some("com.atproto.server.createAccount") {
        return Err(XrpcError::bad_request("InvalidToken", "Bad token scope"));
    }
    if let Some(lxm) = lxm
        && privileged(lxm)
        && scope != SCOPE_ACCESS
        && scope != SCOPE_APP_PASS_PRIVILEGED
    {
        return Err(XrpcError::invalid_request(format!(
            "insufficient access to request a service auth token for the following method: {lxm}"
        )));
    }
    if let Some(exp) = exp {
        let left = exp - state.now_s();
        let bad = |message: &str| Err(XrpcError::bad_request("BadExpiration", message));
        if left < 0 {
            return bad("expiration is in past");
        }
        if left > 3600 {
            return bad(
                "cannot request a token with an expiration more than an hour in the future",
            );
        }
        if lxm.is_none() && left > 60 {
            return bad(
                "cannot request a method-less token with an expiration more than a minute in the future",
            );
        }
    }
    if let Some(lxm) = lxm
        && listed(PROTECTED_METHODS, lxm)
    {
        return Err(XrpcError::invalid_request(format!(
            "cannot request a service auth token for the following protected method: {lxm}"
        )));
    }
    let token = account_jwt(state, &did, aud, lxm, exp).await?;
    Ok(json_response(StatusCode::OK, &json!({ "token": token })))
}

#[cfg(test)]
mod tests {
    use pds_identity::Algorithm;

    use super::*;

    #[test]
    fn a_service_token_carries_its_claims_and_a_signature_of_the_account_key() {
        let keypair = Keypair::generate(Algorithm::Secp256k1, |bytes| bytes.fill(5));
        let token = service_jwt(
            &keypair,
            "did:plc:abc",
            "did:web:api.example",
            Some("app.bsky.feed.getTimeline"),
            100,
            160,
            &[7; 16],
        );
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let decode = |part: &str| -> Value {
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
        };
        assert_eq!(decode(parts[0]), json!({"typ": "JWT", "alg": "ES256K"}));
        assert_eq!(
            decode(parts[1]),
            json!({"iat": 100, "iss": "did:plc:abc", "aud": "did:web:api.example", "exp": 160,
                   "lxm": "app.bsky.feed.getTimeline", "jti": "07070707070707070707070707070707"})
        );
        let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        keypair
            .public_key()
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
            .unwrap();
        assert!(privileged("chat.bsky.convo.getConvo") && privileged("CHAT.bsky.convo.getlog"));
        assert!(listed(PROTECTED_METHODS, "com.atproto.server.getSession"));
        assert_eq!(type_name(404).0, "XRPCNotSupported");
    }
}

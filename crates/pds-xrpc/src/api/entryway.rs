//! A member host behind an entryway (`docs/SPEC.md` §21.3).
//!
//! The entryway owns the credentials, the sessions and the handles of the
//! accounts. A member host keeps the repositories. A client talks to its
//! member host, so the member passes the methods of the entryway on to
//! it, with the credential the client sent.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

use serde_json::Value;

use crate::error::XrpcError;
use crate::state::{AppState, Entryway};

/// The largest body the server passes on, in either direction.
const BODY_LIMIT: usize = 1024 * 1024;

/// The methods that the entryway answers for a member host: sessions,
/// credentials, email, the handle, the status and the deletion of an
/// account.
const FORWARDED: &[&str] = &[
    "com.atproto.server.createSession",
    "com.atproto.server.refreshSession",
    "com.atproto.server.getSession",
    "com.atproto.server.deleteSession",
    "com.atproto.server.createAppPassword",
    "com.atproto.server.listAppPasswords",
    "com.atproto.server.revokeAppPassword",
    "com.atproto.server.requestPasswordReset",
    "com.atproto.server.resetPassword",
    "com.atproto.server.requestEmailConfirmation",
    "com.atproto.server.confirmEmail",
    "com.atproto.server.requestEmailUpdate",
    "com.atproto.server.updateEmail",
    "com.atproto.server.getAccountInviteCodes",
    "com.atproto.server.activateAccount",
    "com.atproto.server.deactivateAccount",
    "com.atproto.server.requestAccountDelete",
    "com.atproto.server.deleteAccount",
    "com.atproto.identity.updateHandle",
    "com.atproto.identity.requestPlcOperationSignature",
    "com.atproto.identity.signPlcOperation",
    "io.linkjar.account.getSignupReceipt",
    "io.linkjar.account.acknowledgeSignupReceipt",
];

/// The admin methods that read or change what the entryway owns: the
/// credentials, the handles, the email addresses and the invites of the
/// accounts. The server passes them on with the admin credential of the
/// entryway.
///
/// The entryway calls `admin.updateAccountHandle` and `admin.deleteAccount`
/// on a member host itself, with [`FROM_ENTRYWAY`]: the member host applies
/// those calls and passes on the calls of its own admin.
const FORWARDED_ADMIN: &[&str] = &[
    "com.atproto.admin.updateAccountHandle",
    "com.atproto.admin.deleteAccount",
    "com.atproto.admin.updateAccountEmail",
    "com.atproto.admin.updateAccountPassword",
    "com.atproto.admin.sendEmail",
    "com.atproto.admin.getAccountInfo",
    "com.atproto.admin.getAccountInfos",
    "com.atproto.admin.searchAccounts",
    "com.atproto.admin.getInviteCodes",
    "com.atproto.admin.disableInviteCodes",
    "com.atproto.admin.disableAccountInvites",
    "com.atproto.admin.enableAccountInvites",
    "com.atproto.server.createInviteCode",
    "com.atproto.server.createInviteCodes",
];

/// Whether a member host passes the method on to its entryway.
pub fn is_forwarded(nsid: &str) -> bool {
    FORWARDED.contains(&nsid) || FORWARDED_ADMIN.contains(&nsid)
}

/// The header of a call that an entryway makes on its member host. It
/// keeps the member host from passing the call back. The entryway calls
/// with the admin password of the member host, and the header counts only
/// with it: a client could set it on any method.
pub const FROM_ENTRYWAY: &str = "x-linkjar-entryway-call";

/// Whether the request is a call of the entryway, see [`FROM_ENTRYWAY`].
pub fn from_entryway(state: &AppState, headers: &HeaderMap) -> bool {
    headers.contains_key(FROM_ENTRYWAY) && crate::auth::is_admin(headers, &state.config().auth)
}

/// The header of a call that a member host makes on its entryway, which
/// keeps the entryway from passing the call back. It grants nothing.
pub const FROM_MEMBER: &str = "x-linkjar-member-call";

/// Whether the request is a call of the member host, see [`FROM_MEMBER`].
pub fn from_member(headers: &HeaderMap) -> bool {
    headers.contains_key(FROM_MEMBER)
}

/// The headers of the client that go to the entryway.
const PASSED_ON: &[HeaderName] = &[
    header::AUTHORIZATION,
    header::CONTENT_TYPE,
    header::ACCEPT,
    header::ACCEPT_LANGUAGE,
    header::USER_AGENT,
];

/// The headers of the entryway that go back to the client.
const PASSED_BACK: &[&str] = &[
    "content-type",
    "cache-control",
    "www-authenticate",
    "ratelimit-limit",
    "ratelimit-remaining",
    "ratelimit-reset",
    "ratelimit-policy",
    "retry-after",
];

fn unavailable(message: &str) -> XrpcError {
    XrpcError {
        status: StatusCode::BAD_GATEWAY,
        error: "UpstreamFailure".to_owned(),
        message: message.to_owned(),
    }
}

/// The methods that an OAuth token may call at a member host and that the
/// member host passes on.
///
/// A DPoP proof names one server, so the member host checks the token
/// itself and asks the entryway with a service token of the account, as
/// the Reference does. The entryway takes such a token for these methods
/// only.
pub fn takes_service_token(nsid: &str) -> bool {
    matches!(
        nsid,
        "com.atproto.server.getSession"
            | "com.atproto.identity.updateHandle"
            | "com.atproto.server.activateAccount"
            | "com.atproto.server.deactivateAccount"
            | "io.linkjar.account.getSignupReceipt"
            | "io.linkjar.account.acknowledgeSignupReceipt"
    )
}

/// The claim of a service token that names the OAuth session the member
/// host checked. The receipt of a sign-up belongs to one session (SPEC
/// §12.8), so the entryway needs it.
pub const SESSION_CLAIM: &str = "io.linkjar.oauthSession";
/// The claim that names the client of that session.
pub const CLIENT_CLAIM: &str = "io.linkjar.oauthClient";

/// Whether an entryway answers the method itself.
///
/// These are the methods a member host passes on to it, the creation of
/// an account, and what reads the accounts, the handles and the invites.
/// Every other method goes to the member host, which keeps the
/// repositories.
pub fn is_entryway_method(nsid: &str) -> bool {
    is_forwarded(nsid)
        || nsid.starts_with("io.linkjar.account.")
        || matches!(
            nsid,
            "com.atproto.server.createAccount"
                | "com.atproto.server.describeServer"
                | "com.atproto.identity.resolveHandle"
                | "com.atproto.admin.updateAccountHandle"
                | "com.atproto.admin.updateSubjectStatus"
                | "com.atproto.admin.deleteAccount"
        )
}

/// Calls a method of the member host as its admin, and gives the answer.
/// The entryway uses it for what both servers must know.
pub async fn ask_member(
    state: &AppState,
    host: &crate::state::MemberHost,
    nsid: &str,
    input: &Value,
) -> Result<Value, XrpcError> {
    let url = format!("{}/xrpc/{nsid}", host.internal_url);
    let request = state.http().post(url).header(FROM_ENTRYWAY, "1");
    ask(request, state, &host.admin_password, input, "member host").await
}

/// Calls a method of the entryway as its admin, and gives the answer. A
/// member host uses it for what the entryway must know: a takedown.
pub async fn ask_entryway(
    state: &AppState,
    entryway: &Entryway,
    nsid: &str,
    input: &Value,
) -> Result<Value, XrpcError> {
    let password = entryway
        .admin_token
        .as_deref()
        .ok_or_else(|| unavailable("the server has no admin credential of the entryway"))?;
    let url = format!("{}/xrpc/{nsid}", entryway.url);
    let request = state.http().post(url).header(FROM_MEMBER, "1");
    ask(request, state, password, input, "entryway").await
}

async fn ask(
    request: reqwest::RequestBuilder,
    state: &AppState,
    password: &str,
    input: &Value,
    who: &str,
) -> Result<Value, XrpcError> {
    let answer = request
        .basic_auth("admin", Some(password))
        .header(header::CONTENT_TYPE, "application/json")
        .body(input.to_string())
        .timeout(state.config().proxy_timeout)
        .send()
        .await
        .map_err(|_| unavailable(&format!("the {who} did not answer")))?;
    let status = answer.status().as_u16();
    let bytes = answer
        .bytes()
        .await
        .map_err(|_| unavailable(&format!("the {who} did not finish its answer")))?;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let text = |key: &str| body.get(key).and_then(Value::as_str);
    // What the host refuses for the caller's input is the caller's error.
    Err(XrpcError {
        status: if status < 500 {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::BAD_GATEWAY
        },
        error: text("error").unwrap_or("UpstreamFailure").to_owned(),
        message: text("message")
            .map_or_else(|| format!("the {who} refused the request"), str::to_owned),
    })
}

/// Passes a request on to the member host, with the credential of the
/// client, and gives its answer.
pub async fn forward_to_member(
    state: &AppState,
    host: &crate::state::MemberHost,
    nsid: &str,
    method: &Method,
    raw_query: Option<&str>,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let target = Entryway {
        url: host.internal_url.clone(),
        did: String::new(),
        plc_rotation_key: String::new(),
        admin_token: None,
    };
    pass_on(
        state, &target, nsid, method, raw_query, headers, body, false,
    )
    .await
}

/// Passes a request on to the entryway and gives its answer.
pub async fn forward(
    state: &AppState,
    entryway: &Entryway,
    nsid: &str,
    method: &Method,
    raw_query: Option<&str>,
    headers: &HeaderMap,
    body: &mut Option<Body>,
) -> Result<Response, XrpcError> {
    let as_admin = FORWARDED_ADMIN.contains(&nsid);
    pass_on(
        state, entryway, nsid, method, raw_query, headers, body, as_admin,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn pass_on(
    state: &AppState,
    entryway: &Entryway,
    nsid: &str,
    method: &Method,
    raw_query: Option<&str>,
    headers: &HeaderMap,
    body: &mut Option<Body>,
    as_admin: bool,
) -> Result<Response, XrpcError> {
    // A DPoP proof names the address it was made for, so a request with
    // one cannot go to another server. A member host checked the OAuth
    // token on arrival; for the methods an OAuth token may call it asks
    // the entryway with a service token of the account instead. Every
    // other DPoP request is refused, as the Reference refuses it.
    let service_token = service_token(state, entryway, nsid, headers).await?;
    let url = match raw_query {
        Some(query) => format!("{}/xrpc/{nsid}?{query}", entryway.url),
        None => format!("{}/xrpc/{nsid}", entryway.url),
    };
    let mut request = state.http().request(method.clone(), url);
    for name in PASSED_ON {
        // The admin credential of this host stays here.
        if as_admin && name == header::AUTHORIZATION {
            continue;
        }
        if let Some(token) = &service_token
            && name == header::AUTHORIZATION
        {
            request = request.bearer_auth(token);
            continue;
        }
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
    }
    // The other server limits by the address of the client, not by the
    // address of this server. It takes the header from a proxy it trusts.
    if let Some(ip) = crate::ratelimit::current_ip() {
        request = request.header("x-forwarded-for", ip);
    }
    // The other server checks the key against its own.
    if let Some(key) = headers.get("x-ratelimit-bypass") {
        request = request.header("x-ratelimit-bypass", key);
    }
    if as_admin {
        // The caller is the admin of this host. The entryway has its own.
        use crate::auth::{Credentials, optional_credentials};
        let admin = optional_credentials(headers, &state.config().auth, &[], state.now_s());
        if !matches!(admin, Ok(Credentials::Admin)) {
            return Err(XrpcError::auth_required("AuthenticationRequired"));
        }
        let token = entryway
            .admin_token
            .as_deref()
            .ok_or_else(|| unavailable("the server has no admin credential of the entryway"))?;
        request = request.basic_auth("admin", Some(token));
    }
    // A body of at most 1 MiB is read whole and sent with its length. A
    // larger one, or one of unknown length, goes on as it arrives: an
    // upload may be larger than memory should hold, and the other server
    // applies its own limits.
    let deadline = tokio::time::Instant::now() + state.config().proxy_timeout;
    if let Some(body) = body.take() {
        let large = match declared_length(headers) {
            Some(length) => length > BODY_LIMIT as u64,
            None => headers.contains_key(header::TRANSFER_ENCODING),
        };
        request = if large {
            request.body(reqwest::Body::wrap(SyncBody(std::sync::Mutex::new(body))))
        } else {
            let bytes = axum::body::to_bytes(body, BODY_LIMIT)
                .await
                .map_err(|_| XrpcError::invalid_request("request entity too large"))?;
            request.body(bytes)
        };
    }
    // The time limit is for the answer to begin, and for a small answer
    // to end. A large answer streams, with a limit for each piece.
    let answer = tokio::time::timeout_at(deadline, request.send())
        .await
        .map_err(|_| unavailable("the entryway did not answer"))?
        .map_err(|_| unavailable("the entryway did not answer"))?;
    let status = StatusCode::from_u16(answer.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut passed = HeaderMap::new();
    for name in PASSED_BACK {
        if let Some(value) = answer.headers().get(*name)
            && let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_bytes(value.as_bytes()),
            )
        {
            passed.insert(name, value);
        }
    }
    // A large answer streams, unless the server changes it on the way:
    // that is a session, which is small.
    let large = answer
        .content_length()
        .is_none_or(|length| length > BODY_LIMIT as u64);
    let changed = service_token.is_some() && nsid == "com.atproto.server.getSession";
    if large && !changed {
        let body = streamed(answer, state.config().proxy_timeout);
        return Ok((status, passed, body).into_response());
    }
    let mut bytes = Vec::new();
    let mut answer = answer;
    while let Some(chunk) = tokio::time::timeout_at(deadline, answer.chunk())
        .await
        .map_err(|_| unavailable("the entryway did not finish its answer"))?
        .map_err(|_| unavailable("the entryway did not finish its answer"))?
    {
        if bytes.len() + chunk.len() > BODY_LIMIT {
            return Err(unavailable("the answer of the entryway is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, passed, hide_email(nsid, bytes)).into_response())
}

/// The answer as a body that streams. Each piece must come within `idle`:
/// a stalled answer ends instead of holding the client.
fn streamed(answer: reqwest::Response, idle: std::time::Duration) -> Body {
    let stream = futures_util::stream::unfold(Some(answer), move |answer| async move {
        let mut answer = answer?;
        match tokio::time::timeout(idle, answer.chunk()).await {
            Ok(Ok(Some(chunk))) => Some((Ok(chunk), Some(answer))),
            Ok(Ok(None)) => None,
            Ok(Err(error)) => Some((Err(axum::Error::new(error)), None)),
            Err(elapsed) => Some((Err(axum::Error::new(elapsed)), None)),
        }
    });
    Body::from_stream(stream)
}

/// The length a request declares in `Content-Length`.
fn declared_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

/// A request body that the HTTP client can hold: it asks for `Sync`, and
/// a body is `Send` only. The mutex is never locked: polling takes it by
/// `&mut`.
struct SyncBody(std::sync::Mutex<Body>);

impl axum::body::HttpBody for SyncBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let body = self
            .get_mut()
            .0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::pin::Pin::new(body).poll_frame(cx)
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.0
            .lock()
            .map_or_else(|_| http_body::SizeHint::default(), |body| body.size_hint())
    }

    fn is_end_stream(&self) -> bool {
        self.0.lock().is_ok_and(|body| body.is_end_stream())
    }
}

/// The service token that replaces an OAuth token on the way to the
/// entryway, or `None` for a request without an OAuth token.
async fn service_token(
    state: &AppState,
    entryway: &Entryway,
    nsid: &str,
    headers: &HeaderMap,
) -> Result<Option<String>, XrpcError> {
    if !crate::oauth::resource::is_dpop(headers) {
        return Ok(None);
    }
    match crate::oauth::resource::grant() {
        Some(grant) if state.config().entryway.is_some() && takes_service_token(nsid) => {
            let session = [
                (SESSION_CLAIM, grant.token_id.as_str()),
                (CLIENT_CLAIM, grant.client_id.as_str()),
            ];
            let token = super::proxy::account_jwt_with(
                state,
                &grant.did,
                &entryway.did,
                Some(nsid),
                None,
                &session,
            )
            .await?;
            Ok(Some(token))
        }
        _ => Err(XrpcError::invalid_request(
            "DPoP requests cannot be proxied",
        )),
    }
}

/// The entryway shows the email address to the account. An OAuth token
/// without the scope of the address does not see it, as at the Reference.
fn hide_email(nsid: &str, bytes: Vec<u8>) -> Vec<u8> {
    if nsid != "com.atproto.server.getSession" || crate::oauth::resource::may_read_email() {
        return bytes;
    }
    let Ok(Value::Object(mut session)) = serde_json::from_slice::<Value>(&bytes) else {
        return bytes;
    };
    for field in ["email", "emailConfirmed", "emailAuthFactor"] {
        session.remove(field);
    }
    Value::Object(session).to_string().into_bytes()
}

/// Serves the firehose of the member host at an entryway: the frames of
/// `com.atproto.sync.subscribeRepos` go through as they come, in both
/// directions. `None` when the request is no WebSocket upgrade.
///
/// The connection to the member host is plain: its internal address is on
/// a private network.
pub async fn firehose(
    host: &crate::state::MemberHost,
    parts: &mut axum::http::request::Parts,
) -> Option<Response> {
    use axum::extract::FromRequestParts;
    use axum::extract::ws::WebSocketUpgrade;
    let upgrade = WebSocketUpgrade::from_request_parts(parts, &())
        .await
        .ok()?;
    let base = host
        .internal_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let url = match parts.uri.query() {
        Some(query) => format!("{base}/xrpc/com.atproto.sync.subscribeRepos?{query}"),
        None => format!("{base}/xrpc/com.atproto.sync.subscribeRepos"),
    };
    Some(upgrade.on_upgrade(move |client| relay_frames(client, url)))
}

async fn relay_frames(client: axum::extract::ws::WebSocket, url: String) {
    use axum::extract::ws::{CloseFrame, Message};
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Upstream;
    let (mut to_client, mut from_client) = client.split();
    let Ok((upstream, _)) = tokio_tungstenite::connect_async(url.as_str()).await else {
        tracing::warn!(%url, "the member host refused the firehose");
        let close = CloseFrame {
            code: 1011,
            reason: "the member host is not reachable".into(),
        };
        let _ = to_client.send(Message::Close(Some(close))).await;
        return;
    };
    let (mut to_member, mut from_member) = upstream.split();
    let down = async {
        while let Some(Ok(frame)) = from_member.next().await {
            let message = match frame {
                Upstream::Binary(bytes) => Message::Binary(bytes),
                Upstream::Text(text) => Message::Text(text.as_str().into()),
                Upstream::Close(frame) => Message::Close(frame.map(|frame| CloseFrame {
                    code: frame.code.into(),
                    reason: frame.reason.as_str().into(),
                })),
                // The libraries answer pings themselves.
                Upstream::Ping(_) | Upstream::Pong(_) | Upstream::Frame(_) => continue,
            };
            let closing = matches!(message, Message::Close(_));
            if to_client.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = to_client.close().await;
    };
    let up = async {
        while let Some(Ok(message)) = from_client.next().await {
            let frame = match message {
                Message::Binary(bytes) => Upstream::Binary(bytes),
                Message::Text(text) => Upstream::Text(text.as_str().into()),
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => continue,
            };
            if to_member.send(frame).await.is_err() {
                break;
            }
        }
        let _ = to_member.close().await;
    };
    // Either side that ends ends the other.
    tokio::select! {
        () = down => {}
        () = up => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_and_credentials_go_to_the_entryway_and_repositories_do_not() {
        for forwarded in [
            "com.atproto.server.createSession",
            "com.atproto.server.refreshSession",
            "com.atproto.identity.updateHandle",
            "com.atproto.server.activateAccount",
            "com.atproto.admin.updateAccountPassword",
            "com.atproto.admin.getAccountInfo",
            "com.atproto.admin.updateAccountHandle",
            "com.atproto.admin.deleteAccount",
        ] {
            assert!(is_forwarded(forwarded), "{forwarded}");
        }
        for local in [
            "com.atproto.repo.createRecord",
            "com.atproto.sync.getRepo",
            "com.atproto.server.createAccount",
            "com.atproto.server.describeServer",
            "com.atproto.admin.updateSubjectStatus",
        ] {
            assert!(!is_forwarded(local), "{local}");
        }
    }

    #[test]
    fn a_service_token_goes_with_a_forwarded_method_only() {
        for nsid in FORWARDED.iter().chain(FORWARDED_ADMIN) {
            if takes_service_token(nsid) {
                assert!(FORWARDED.contains(nsid), "{nsid}");
            }
        }
        assert!(!takes_service_token("com.atproto.server.createAppPassword"));
        assert!(!takes_service_token("com.atproto.repo.createRecord"));
    }
}

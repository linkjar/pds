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
/// keeps the member host from passing the call back: it grants nothing,
/// as the call needs the admin password of the member host too.
pub const FROM_ENTRYWAY: &str = "x-linkjar-entryway-call";

/// Whether the request is a call of the entryway, see [`FROM_ENTRYWAY`].
pub fn from_entryway(headers: &HeaderMap) -> bool {
    headers.contains_key(FROM_ENTRYWAY)
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
    input: &serde_json::Value,
) -> Result<serde_json::Value, XrpcError> {
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
    input: &serde_json::Value,
) -> Result<serde_json::Value, XrpcError> {
    let password = entryway
        .admin_token
        .as_deref()
        .ok_or_else(|| unavailable("the server has no admin credential of the entryway"))?;
    let url = format!("{}/xrpc/{nsid}", entryway.url);
    ask(state.http().post(url), state, password, input, "entryway").await
}

async fn ask(
    request: reqwest::RequestBuilder,
    state: &AppState,
    password: &str,
    input: &serde_json::Value,
    who: &str,
) -> Result<serde_json::Value, XrpcError> {
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
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let text = |key: &str| body.get(key).and_then(serde_json::Value::as_str);
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
    if let Some(body) = body.take() {
        let bytes = axum::body::to_bytes(body, BODY_LIMIT)
            .await
            .map_err(|_| XrpcError::invalid_request("request entity too large"))?;
        request = request.body(bytes);
    }
    let answer = request
        .timeout(state.config().proxy_timeout)
        .send()
        .await
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
    let mut bytes = Vec::new();
    let mut answer = answer;
    while let Some(chunk) = answer
        .chunk()
        .await
        .map_err(|_| unavailable("the entryway did not finish its answer"))?
    {
        if bytes.len() + chunk.len() > BODY_LIMIT {
            return Err(unavailable("the answer of the entryway is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if service_token.is_some() {
        bytes = hide_email(nsid, bytes);
    }
    Ok((status, passed, bytes).into_response())
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

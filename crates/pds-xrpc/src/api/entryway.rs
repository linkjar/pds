//! A member host behind an entryway (`docs/SPEC.md` §21.3).
//!
//! The entryway owns the credentials, the sessions and the handles of the
//! accounts. A member host keeps the repositories. A client talks to its
//! member host, so the member passes the methods of the entryway on to
//! it, with the credential the client sent.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::error::XrpcError;
use crate::state::{AppState, Entryway};

/// The largest body the server passes on, in either direction.
const BODY_LIMIT: usize = 1024 * 1024;

/// The methods that the entryway answers for a member host: sessions,
/// credentials, email, the handle, and the deletion of an account.
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
    "com.atproto.server.requestAccountDelete",
    "com.atproto.server.deleteAccount",
    "com.atproto.identity.updateHandle",
    "com.atproto.identity.requestPlcOperationSignature",
    "com.atproto.identity.signPlcOperation",
];

/// The admin methods that change what the entryway owns. The server
/// passes them on with the admin credential of the entryway.
const FORWARDED_ADMIN: &[&str] = &[
    "com.atproto.admin.updateAccountEmail",
    "com.atproto.admin.updateAccountPassword",
    "com.atproto.admin.sendEmail",
];

/// Whether a member host passes the method on to its entryway.
pub fn is_forwarded(nsid: &str) -> bool {
    FORWARDED.contains(&nsid) || FORWARDED_ADMIN.contains(&nsid)
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
    // A DPoP proof names the address it was made for, so a request with
    // one cannot go to another server. The Reference refuses it the same
    // way.
    if crate::oauth::resource::is_dpop(headers) {
        return Err(XrpcError::invalid_request(
            "DPoP requests cannot be proxied",
        ));
    }
    let url = match raw_query {
        Some(query) => format!("{}/xrpc/{nsid}?{query}", entryway.url),
        None => format!("{}/xrpc/{nsid}", entryway.url),
    };
    let as_admin = FORWARDED_ADMIN.contains(&nsid);
    let mut request = state.http().request(method.clone(), url);
    for name in PASSED_ON {
        // The admin credential of this host stays here.
        if as_admin && name == header::AUTHORIZATION {
            continue;
        }
        if let Some(value) = headers.get(name) {
            request = request.header(name, value);
        }
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
    Ok((status, passed, bytes).into_response())
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
            "com.atproto.admin.updateAccountPassword",
        ] {
            assert!(is_forwarded(forwarded), "{forwarded}");
        }
        for local in [
            "com.atproto.repo.createRecord",
            "com.atproto.sync.getRepo",
            "com.atproto.server.createAccount",
            "com.atproto.server.describeServer",
            "com.atproto.admin.updateSubjectStatus",
            "com.atproto.admin.updateAccountHandle",
            "com.atproto.server.activateAccount",
        ] {
            assert!(!is_forwarded(local), "{local}");
        }
    }
}

//! Service tokens that this server accepts (`docs/SPEC.md` §5.4): a token
//! that an account's key signed for one method of this server, as
//! `verifyJwt` of `@atproto/xrpc-server` and `userServiceAuth` at the pin.

use std::cell::RefCell;
use std::future::Future;

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pds_identity::DidDocument;
use pds_types::Did;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::XrpcError;
use crate::state::AppState;

tokio::task_local! {
    /// The service tokens that the request this task serves reserved.
    static RESERVED: RefCell<Vec<String>>;
}

/// Serves a request in which a service token is good once (SPEC §5.4): a
/// token that [`verify`] took stays spent when the answer is 2xx, and is
/// released otherwise, so the caller can retry it.
pub async fn scope<F>(state: &AppState, serve: F) -> Response
where
    F: Future<Output = Response>,
{
    RESERVED
        .scope(RefCell::default(), async {
            let response = serve.await;
            if !response.status().is_success() {
                for key in RESERVED.with(RefCell::take) {
                    state.oauth().forget(&key);
                }
            }
            response
        })
        .await
}

/// Spends the token until `exp_s`: `iss` with `jti`, or the hash of a
/// token without one.
fn spend(state: &AppState, token: &str, iss: &str, claims: &Value, exp_s: f64) -> bool {
    let key = match claims.get("jti").and_then(Value::as_str) {
        Some(jti) => format!("service:{iss}:{jti}"),
        None => format!(
            "service-hash:{}",
            URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
        ),
    };
    #[allow(clippy::cast_possible_truncation)] // a time in milliseconds fits
    let until_ms = (exp_s * 1000.0) as i64;
    if !state.oauth().unique(key.clone(), until_ms, state.now_ms()) {
        return false;
    }
    let _ = RESERVED.try_with(|reserved| reserved.borrow_mut().push(key));
    true
}

fn denied(error: &str, message: impl Into<String>) -> XrpcError {
    XrpcError {
        status: StatusCode::UNAUTHORIZED,
        error: error.to_owned(),
        message: message.into(),
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (kind, token) = value.split_once(' ')?;
    (kind.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// The claims of the service token of the request, unchecked. Read them
/// after [`verify`].
pub fn claims(headers: &HeaderMap) -> Option<Value> {
    bearer(headers)
        .and_then(|token| token.split('.').nth(1))
        .and_then(part)
}

fn part(text: &str) -> Option<Value> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(text).ok()?).ok()
}

/// Whether the request carries a service token: a bearer token whose
/// claims name a method.
pub fn is_service_token(headers: &HeaderMap) -> bool {
    bearer(headers)
        .and_then(|token| token.split('.').nth(1))
        .and_then(part)
        .is_some_and(|claims| claims.get("lxm").is_some_and(|lxm| !lxm.is_null()))
}

/// The key that signs for `iss`, from its DID document.
async fn signing_key(
    state: &AppState,
    iss: &str,
    force_refresh: bool,
) -> Result<pds_identity::PublicKey, XrpcError> {
    let did = iss.split_once('#').map_or(iss, |(did, _)| did);
    let parsed: Did = did
        .parse()
        .map_err(|_| denied("BadJwtIss", "jwt iss is not a valid did"))?;
    let unresolved = || denied("AuthenticationRequired", "could not resolve iss did");
    let doc = state
        .resolver()
        .resolve(&parsed, force_refresh)
        .await
        .map_err(|_| unresolved())?
        .ok_or_else(unresolved)?;
    DidDocument::from_value(&doc)
        .ok()
        .and_then(|doc| doc.signing_key().ok())
        .ok_or_else(|| denied("AuthenticationRequired", "missing or bad key in did doc"))
}

/// The account that signed the service token of the request for the method
/// `lxm` of this server.
pub async fn verify(state: &AppState, headers: &HeaderMap, lxm: &str) -> Result<String, XrpcError> {
    let token = bearer(headers).ok_or_else(|| denied("MissingJwt", "missing jwt"))?;
    let parts: Vec<&str> = token.split('.').collect();
    let [head, body, signature] = parts.as_slice() else {
        return Err(denied("BadJwt", "poorly formatted jwt"));
    };
    let bad = || denied("BadJwt", "poorly formatted jwt");
    let header = part(head).ok_or_else(bad)?;
    let claims = part(body).ok_or_else(bad)?;
    if let Some(typ) = header.get("typ").and_then(Value::as_str)
        && matches!(typ, "at+jwt" | "refresh+jwt" | "dpop+jwt")
    {
        return Err(denied("BadJwtType", format!("Invalid jwt type \"{typ}\"")));
    }
    #[allow(clippy::cast_precision_loss)] // seconds since 1970 fit a double exactly
    let Some(exp) = claims
        .get("exp")
        .and_then(Value::as_f64)
        .filter(|exp| state.now_s() as f64 <= *exp)
    else {
        return Err(denied("JwtExpired", "jwt expired"));
    };
    match claims.get("lxm").and_then(Value::as_str) {
        Some(named) if named == lxm => {}
        Some(_) => {
            return Err(denied(
                "BadJwtLexiconMethod",
                format!("bad jwt lexicon method (\"lxm\"). must match: {lxm}"),
            ));
        }
        None => {
            return Err(denied(
                "BadJwtLexiconMethod",
                format!("missing jwt lexicon method (\"lxm\"). must match: {lxm}"),
            ));
        }
    }
    let iss = claims
        .get("iss")
        .and_then(Value::as_str)
        .ok_or_else(|| denied("BadJwtIss", "jwt iss is not a valid did"))?;
    let unverifiable = || denied("BadJwtSignature", "could not verify jwt signature");
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| unverifiable())?;
    // The audience costs nothing to check; the issuer's document is a
    // fetch, so a token for another service never causes one.
    if claims.get("aud").and_then(Value::as_str) != Some(state.config().auth.service_did.as_str()) {
        return Err(denied(
            "BadJwtAudience",
            "jwt audience does not match service did",
        ));
    }
    let signed = format!("{head}.{body}");
    let key = signing_key(state, iss, false).await?;
    if key.verify(signed.as_bytes(), &signature).is_err() {
        // The key may have changed since the document was cached.
        let fresh = signing_key(state, iss, true).await?;
        if fresh == key || fresh.verify(signed.as_bytes(), &signature).is_err() {
            return Err(denied(
                "BadJwtSignature",
                "jwt signature does not match jwt issuer",
            ));
        }
    }
    if !spend(state, token, iss, &claims, exp) {
        return Err(denied("BadJwt", "jwt replayed"));
    }
    Ok(iss.split_once('#').map_or(iss, |(did, _)| did).to_owned())
}

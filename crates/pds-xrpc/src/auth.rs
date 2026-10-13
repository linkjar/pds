//! Credentials of a read request (`docs/SPEC.md` §5.1, §5.3).
//!
//! The read methods take an optional credential: none, the admin password,
//! or a legacy access token. With it an account's owner or an admin reads a
//! repository that is deactivated or taken down. OAuth access tokens arrive
//! with the authorization server (`docs/plan.md`, unit 5); until then a
//! `DPoP` credential cannot be verified and is refused.

use axum::http::{HeaderMap, header};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::XrpcError;

/// The scopes of a legacy access token that every method accepts.
pub const ACCESS_STANDARD: &[&str] = &[
    "com.atproto.access",
    "com.atproto.appPass",
    "com.atproto.appPassPrivileged",
];
/// The scope of a full session. A method that changes the account itself
/// accepts no other.
pub const ACCESS_FULL: &[&str] = &["com.atproto.access"];
/// The scope of a token that a taken-down account holds.
pub const SCOPE_TAKENDOWN: &str = "com.atproto.takendown";

/// What the server needs to check a credential.
#[derive(Clone)]
pub struct AuthConfig {
    /// `PDS_JWT_SECRET`.
    pub jwt_secret: Vec<u8>,
    /// `PDS_ADMIN_PASSWORD`.
    pub admin_password: String,
    /// The service DID, the audience of the tokens this server mints.
    pub service_did: String,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("service_did", &self.service_did)
            .finish_non_exhaustive()
    }
}

/// Who sent a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credentials {
    /// No credential.
    None,
    /// The admin password.
    Admin,
    /// A legacy access token.
    Access {
        /// The account.
        did: String,
        /// The token's scope.
        scope: String,
    },
}

impl Credentials {
    /// Whether the caller is an admin or the account `did` itself.
    pub fn is_admin_or(&self, did: &str) -> bool {
        match self {
            Self::None => false,
            Self::Admin => true,
            Self::Access { did: own, .. } => own == did,
        }
    }
}

fn invalid_token(message: &str) -> XrpcError {
    XrpcError::bad_request("InvalidToken", message)
}

/// Reads the optional credential of a request. `additional` names scopes
/// that the method accepts besides [`ACCESS_STANDARD`]. `now_s` is the time
/// in seconds since the Unix epoch.
pub fn optional_credentials(
    headers: &HeaderMap,
    config: &AuthConfig,
    additional: &[&str],
    now_s: i64,
) -> Result<Credentials, XrpcError> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(Credentials::None);
    };
    let value = value
        .to_str()
        .map_err(|_| invalid_token("Malformed authorization header"))?;
    if value.is_empty() {
        return Ok(Credentials::None);
    }
    let parts: Vec<&str> = value.split(' ').collect();
    let [kind, token] = parts.as_slice() else {
        return Err(invalid_token("Malformed authorization header"));
    };
    match kind.to_ascii_uppercase().as_str() {
        "BASIC" => admin(token, config),
        "BEARER" => access(token, config, additional, now_s),
        // An OAuth access token. Nothing verifies one until unit 5.
        "DPOP" => Err(invalid_token("Token could not be verified")),
        _ => Err(invalid_token(&format!(
            "Unsupported authorization type: {kind}"
        ))),
    }
}

/// The account of a request that must carry a session: a legacy access
/// token. `only` narrows the scopes the method accepts; `None` accepts
/// [`ACCESS_STANDARD`].
///
/// The answers are those of `authorization()` in the Reference: no
/// credential is 401 `AuthMissing`, and the admin password is not a session.
pub fn session(
    state: &crate::state::AppState,
    headers: &HeaderMap,
    only: Option<&[&str]>,
) -> Result<String, XrpcError> {
    if headers
        .get(header::AUTHORIZATION)
        .is_none_or(axum::http::HeaderValue::is_empty)
    {
        return Err(XrpcError::auth_required("AuthMissing"));
    }
    let unexpected = || invalid_token("Unexpected authorization type");
    match optional_credentials(headers, &state.config().auth, &[], state.now_s()) {
        Ok(Credentials::Access { did, scope }) => {
            if only.is_some_and(|only| !only.contains(&scope.as_str())) {
                return Err(invalid_token("Bad token scope"));
            }
            Ok(did)
        }
        Ok(Credentials::Admin | Credentials::None) => Err(unexpected()),
        Err(error) if error.error == "AuthenticationRequired" => Err(unexpected()),
        Err(error) => Err(error),
    }
}

fn admin(token: &str, config: &AuthConfig) -> Result<Credentials, XrpcError> {
    let denied = || XrpcError::auth_required("AuthenticationRequired");
    let decoded = STANDARD.decode(token).map_err(|_| denied())?;
    let decoded = String::from_utf8_lossy(&decoded);
    // The password may hold a colon; the user name may not.
    let (user, password) = decoded.split_once(':').ok_or_else(denied)?;
    let user_ok = user.as_bytes().ct_eq(b"admin");
    let password_ok = password.as_bytes().ct_eq(config.admin_password.as_bytes());
    if bool::from(user_ok & password_ok) {
        Ok(Credentials::Admin)
    } else {
        Err(denied())
    }
}

fn access(
    token: &str,
    config: &AuthConfig,
    additional: &[&str],
    now_s: i64,
) -> Result<Credentials, XrpcError> {
    let unverified = || invalid_token("Token could not be verified");
    let malformed = || invalid_token("Malformed token");
    let mut parts = token.split('.');
    let (Some(head), Some(body), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(unverified());
    };
    let json = |part: &str| -> Option<serde_json::Value> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()
    };
    let header = json(head).ok_or_else(unverified)?;
    if header.get("alg").and_then(|v| v.as_str()) != Some("HS256") {
        return Err(unverified());
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| unverified())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&config.jwt_secret).map_err(|_| unverified())?;
    mac.update(head.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    mac.verify_slice(&signature).map_err(|_| unverified())?;

    let claims = json(body).ok_or_else(unverified)?;
    if header.get("typ").and_then(|v| v.as_str()) != Some("at+jwt") {
        return Err(unverified());
    }
    if claims.get("aud").and_then(|v| v.as_str()) != Some(config.service_did.as_str()) {
        return Err(unverified());
    }
    match claims.get("exp") {
        None => {}
        Some(exp) => {
            let exp = exp.as_f64().ok_or_else(unverified)?;
            #[allow(clippy::cast_precision_loss)] // seconds since 1970 fit a double exactly
            if exp <= now_s as f64 {
                return Err(XrpcError::bad_request("ExpiredToken", "Token has expired"));
            }
        }
    }
    if claims.get("lxm").is_some() || claims.get("cnf").is_some() {
        return Err(malformed());
    }
    let did = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .filter(|sub| sub.parse::<pds_types::Did>().is_ok())
        .ok_or_else(malformed)?;
    if claims.get("jti").is_some_and(|jti| !jti.is_string()) {
        return Err(malformed());
    }
    let scope = claims
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !ACCESS_STANDARD.contains(&scope) && !additional.contains(&scope) {
        return Err(invalid_token("Bad token scope"));
    }
    Ok(Credentials::Access {
        did: did.to_owned(),
        scope: scope.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AuthConfig {
        AuthConfig {
            jwt_secret: b"secret".to_vec(),
            admin_password: "pa:ss".to_owned(),
            service_did: "did:web:pds.test".to_owned(),
        }
    }

    fn token(claims: &serde_json::Value, typ: &str, secret: &[u8]) -> String {
        token_with_header(
            &serde_json::json!({"alg": "HS256", "typ": typ}),
            claims,
            secret,
        )
    }

    /// A token with any header, signed with HMAC-SHA-256 whatever `alg` the
    /// header names.
    fn token_with_header(
        header: &serde_json::Value,
        claims: &serde_json::Value,
        secret: &[u8],
    ) -> String {
        let head = URL_SAFE_NO_PAD.encode(header.to_string());
        let body = URL_SAFE_NO_PAD.encode(claims.to_string());
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(format!("{head}.{body}").as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{head}.{body}.{signature}")
    }

    fn with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        headers
    }

    fn claims() -> serde_json::Value {
        serde_json::json!({"scope": "com.atproto.access", "sub": "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa", "aud": "did:web:pds.test", "iat": 100, "exp": 200})
    }

    #[test]
    fn no_header_is_no_credential() {
        assert_eq!(
            optional_credentials(&HeaderMap::new(), &config(), &[], 0).unwrap(),
            Credentials::None
        );
    }

    #[test]
    fn the_admin_password_may_hold_a_colon() {
        let good = STANDARD.encode("admin:pa:ss");
        assert_eq!(
            optional_credentials(&with(&format!("Basic {good}")), &config(), &[], 0).unwrap(),
            Credentials::Admin
        );
        let bad = STANDARD.encode("admin:nope");
        let error =
            optional_credentials(&with(&format!("basic {bad}")), &config(), &[], 0).unwrap_err();
        assert_eq!(
            (error.status.as_u16(), error.error.as_str()),
            (401, "AuthenticationRequired")
        );
    }

    #[test]
    fn an_access_token_names_its_account() {
        let headers = with(&format!("Bearer {}", token(&claims(), "at+jwt", b"secret")));
        let got = optional_credentials(&headers, &config(), &[], 150).unwrap();
        assert!(got.is_admin_or("did:plc:aaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(!got.is_admin_or("did:plc:bbbbbbbbbbbbbbbbbbbbbbbb"));
    }

    #[test]
    fn refusals_carry_the_reference_names() {
        let check = |headers: HeaderMap, now: i64, error: &str, message: &str| {
            let got = optional_credentials(&headers, &config(), &[], now).unwrap_err();
            assert_eq!((got.error.as_str(), got.message.as_str()), (error, message));
        };
        let good = token(&claims(), "at+jwt", b"secret");
        check(
            with(&format!("Bearer {good}")),
            200,
            "ExpiredToken",
            "Token has expired",
        );
        check(
            with("Bearer abc"),
            0,
            "InvalidToken",
            "Token could not be verified",
        );
        check(
            with(&format!("Bearer {}", token(&claims(), "at+jwt", b"other"))),
            150,
            "InvalidToken",
            "Token could not be verified",
        );
        check(
            with(&format!(
                "Bearer {}",
                token(&claims(), "refresh+jwt", b"secret")
            )),
            150,
            "InvalidToken",
            "Token could not be verified",
        );
        check(
            with("Weird abc"),
            0,
            "InvalidToken",
            "Unsupported authorization type: Weird",
        );
        check(
            with("Bearer a b"),
            0,
            "InvalidToken",
            "Malformed authorization header",
        );
        let mut refresh = claims();
        refresh["scope"] = "com.atproto.refresh".into();
        check(
            with(&format!("Bearer {}", token(&refresh, "at+jwt", b"secret"))),
            150,
            "InvalidToken",
            "Bad token scope",
        );
    }

    /// The messages are the ones this server gives. The Reference's wording
    /// for these cases is not compared here.
    #[test]
    fn a_token_that_fails_a_check_is_refused() {
        let check = |authorization: &str, message: &str| {
            let got = optional_credentials(&with(authorization), &config(), &[], 150).unwrap_err();
            assert_eq!(
                (got.error.as_str(), got.message.as_str()),
                ("InvalidToken", message),
                "{authorization}"
            );
        };
        let bearer = |header: &serde_json::Value, claims: &serde_json::Value| {
            format!("Bearer {}", token_with_header(header, claims, b"secret"))
        };
        let good_header = serde_json::json!({"alg": "HS256", "typ": "at+jwt"});
        let with_claim = |name: &str, value: serde_json::Value| {
            let mut changed = claims();
            changed[name] = value;
            bearer(&good_header, &changed)
        };

        // An unsigned token: `alg` is `none` and the signature is empty.
        let none = serde_json::json!({"alg": "none", "typ": "at+jwt"});
        let head = URL_SAFE_NO_PAD.encode(none.to_string());
        let body = URL_SAFE_NO_PAD.encode(claims().to_string());
        check(
            &format!("Bearer {head}.{body}."),
            "Token could not be verified",
        );
        // The signature is good for the secret; only `alg` is wrong.
        check(&bearer(&none, &claims()), "Token could not be verified");
        check(
            &bearer(
                &serde_json::json!({"alg": "HS512", "typ": "at+jwt"}),
                &claims(),
            ),
            "Token could not be verified",
        );
        check(
            &with_claim("aud", "did:web:other.test".into()),
            "Token could not be verified",
        );
        check(
            &with_claim("exp", "soon".into()),
            "Token could not be verified",
        );
        check(
            &with_claim("lxm", "com.atproto.sync.getRepo".into()),
            "Malformed token",
        );
        check(
            &with_claim("cnf", serde_json::json!({"jkt": "abc"})),
            "Malformed token",
        );
        check(&with_claim("sub", "alice".into()), "Malformed token");
        check(&with_claim("jti", 1.into()), "Malformed token");
        check("DPoP abc", "Token could not be verified");
    }

    /// A recorded decision: a token without `exp` does not expire.
    #[test]
    fn a_token_without_an_expiry_is_accepted() {
        let mut eternal = claims();
        eternal.as_object_mut().unwrap().remove("exp");
        let headers = with(&format!("Bearer {}", token(&eternal, "at+jwt", b"secret")));
        assert_eq!(
            optional_credentials(&headers, &config(), &[], i64::MAX).unwrap(),
            Credentials::Access {
                did: "did:plc:aaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
                scope: "com.atproto.access".to_owned(),
            }
        );
    }

    #[test]
    fn a_takendown_token_is_read_only_where_the_method_allows_it() {
        let mut taken = claims();
        taken["scope"] = SCOPE_TAKENDOWN.into();
        let headers = with(&format!("Bearer {}", token(&taken, "at+jwt", b"secret")));
        assert!(optional_credentials(&headers, &config(), &[], 150).is_err());
        assert!(optional_credentials(&headers, &config(), &[SCOPE_TAKENDOWN], 150).is_ok());
    }
}

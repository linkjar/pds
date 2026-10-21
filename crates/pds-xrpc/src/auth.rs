//! Credentials of a read request (`docs/SPEC.md` §5.1, §5.3).
//!
//! The read methods take an optional credential: none, the admin password,
//! or a legacy access token. With it an account's owner or an admin reads a
//! repository that is deactivated or taken down. An OAuth access token with
//! its `DPoP` proof is verified here too (§5.5).

use std::cell::RefCell;
use std::future::Future;

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
    /// The entryway whose tokens the server takes (SPEC §21.3). With one,
    /// the server mints no token and takes none that `jwt_secret` signed.
    pub entryway: Option<EntrywayAuth>,
    /// The key of an entryway (SPEC §21.3). With one, the server signs
    /// its tokens with it, and member hosts verify them.
    pub signer: Option<EntrywaySigner>,
}

/// What an entryway signs its tokens with.
#[derive(Clone)]
pub struct EntrywaySigner {
    /// The secp256k1 key.
    pub key: std::sync::Arc<pds_identity::Keypair>,
    /// The audience of the refresh tokens from before the entryway: the
    /// DID of the server that signed them with `jwt_secret`. The entryway
    /// takes such a token once and answers with its own.
    pub legacy_audience: Option<String>,
}

/// What a member host knows of its entryway to check a token.
#[derive(Clone)]
pub struct EntrywayAuth {
    /// `PDS_ENTRYWAY_DID`. A token may name it as its audience.
    pub did: String,
    /// `PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX`: the secp256k1
    /// key that signs the tokens.
    pub key: pds_identity::PublicKey,
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
        // A service token that the entry point took from a member host
        // carries its account in a grant (SPEC §21.3).
        "BEARER" => crate::oauth::resource::grant().map_or_else(
            || access(token, config, additional, now_s),
            |grant| {
                Ok(Credentials::Access {
                    did: grant.did,
                    scope: SCOPE_ACCESS.to_owned(),
                })
            },
        ),
        // An OAuth access token. The entry point checked it, and what it
        // found travels with the task. The methods it may call are an allow
        // list there, so here it counts as a full session.
        "DPOP" => crate::oauth::resource::grant().map_or_else(
            || Err(invalid_token("Token could not be verified")),
            |grant| {
                Ok(Credentials::Access {
                    did: grant.did,
                    scope: SCOPE_ACCESS.to_owned(),
                })
            },
        ),
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

/// The claims of a token this server signed.
#[derive(Clone)]
struct Verified {
    did: String,
    scope: String,
    jti: Option<String>,
    credential_epoch: Option<String>,
}

struct RequestCredential {
    state: crate::state::AppState,
    token: String,
    checked_at: i64,
    claims: Verified,
}

tokio::task_local! {
    static REQUEST_CREDENTIAL: RefCell<Option<RequestCredential>>;
}

/// Keeps verified claims within one request. Recovery state is never cached.
pub(crate) async fn with_request<F: Future>(serve: F) -> F::Output {
    REQUEST_CREDENTIAL.scope(RefCell::new(None), serve).await
}

fn verified_access(token: &str, config: &AuthConfig, now_s: i64) -> Result<Verified, XrpcError> {
    let cached = REQUEST_CREDENTIAL
        .try_with(|slot| {
            slot.borrow().as_ref().and_then(|cached| {
                (cached.token == token
                    && cached.checked_at == now_s
                    && std::ptr::eq(&raw const cached.state.config().auth, config))
                .then(|| cached.claims.clone())
            })
        })
        .ok()
        .flatten();
    cached.map_or_else(|| verify(token, config, "at+jwt", Some(now_s)), Ok)
}

/// Checks the signature, the type, the audience and the expiry of a token,
/// and the shape of its claims, as `verifyBearerJwt` of the Reference.
fn verify(
    token: &str,
    config: &AuthConfig,
    typ: &str,
    now_s: Option<i64>,
) -> Result<Verified, XrpcError> {
    let unverified = || invalid_token("Token could not be verified");
    let malformed = || invalid_token("Malformed token");
    let mut parts = token.split('.');
    let (Some(head), Some(body), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(unverified());
    };
    // What was signed: the token up to its second dot.
    let signed = &token[..head.len() + 1 + body.len()];
    let json = |part: &str| -> Option<serde_json::Value> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()
    };
    let header = json(head).ok_or_else(unverified)?;
    let alg = header.get("alg").and_then(|v| v.as_str());
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| unverified())?;
    let mut legacy = false;
    if let Some(entryway) = &config.entryway {
        // A member host takes the tokens of its entryway and no other. A
        // token from before the entryway, which this server signed with
        // its secret, has ended: the client then refreshes its session,
        // and the entryway answers (SPEC §21.3).
        if alg == Some("HS256") && !config.jwt_secret.is_empty() {
            let own = Hmac::<Sha256>::new_from_slice(&config.jwt_secret).is_ok_and(|mut mac| {
                mac.update(signed.as_bytes());
                mac.verify_slice(&signature).is_ok()
            });
            // The entryway itself takes such a refresh token, so that a
            // session from before it goes on.
            legacy = own && config.signer.is_some() && typ == "refresh+jwt";
            if own && !legacy {
                return Err(XrpcError::bad_request("ExpiredToken", "Token has expired"));
            }
        }
        if !legacy {
            if alg != Some("ES256K") {
                return Err(unverified());
            }
            entryway
                .key
                .verify_any_s(signed.as_bytes(), &signature)
                .map_err(|_| unverified())?;
        }
    } else {
        if alg != Some("HS256") {
            return Err(unverified());
        }
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&config.jwt_secret).map_err(|_| unverified())?;
        mac.update(signed.as_bytes());
        mac.verify_slice(&signature).map_err(|_| unverified())?;
    }

    let claims = json(body).ok_or_else(unverified)?;
    if header.get("typ").and_then(|v| v.as_str()) != Some(typ) {
        return Err(unverified());
    }
    // The audience is this server, or the entryway for a token that the
    // server passes on to it.
    let aud = claims.get("aud").and_then(|v| v.as_str());
    let for_entryway = config
        .entryway
        .as_ref()
        .is_some_and(|entryway| aud == Some(entryway.did.as_str()));
    let from_before = legacy
        && config
            .signer
            .as_ref()
            .and_then(|signer| signer.legacy_audience.as_deref())
            == aud;
    if aud != Some(config.service_did.as_str()) && !for_entryway && !from_before {
        return Err(unverified());
    }
    match (claims.get("exp"), now_s) {
        (None, _) | (_, None) => {}
        (Some(exp), Some(now_s)) => {
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
    let jti = match claims.get("jti") {
        None => None,
        Some(serde_json::Value::String(jti)) => Some(jti.clone()),
        Some(_) => return Err(malformed()),
    };
    let scope = claims
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Ok(Verified {
        did: did.to_owned(),
        scope: scope.to_owned(),
        jti,
        credential_epoch: epoch_claim(&claims),
    })
}

fn epoch_claim(claims: &serde_json::Value) -> Option<String> {
    claims
        .get("pdsCredentialEpoch")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// An opaque version of the password hash that a sign-in verified. Binding
/// it to the credential prevents an old login racing recovery from gaining
/// the recovered account's version.
pub(crate) fn credential_fingerprint(config: &AuthConfig, password_hash: &str) -> Option<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&config.jwt_secret).ok()?;
    mac.update(b"linkjar-pds:credential-epoch:v1\0");
    mac.update(password_hash.as_bytes());
    Some(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

/// Rejects a legacy access token invalidated by account recovery. OAuth,
/// service tokens, refresh tokens and admin credentials have their own checks.
pub(crate) async fn check_recovery(
    state: &crate::state::AppState,
    headers: &HeaderMap,
) -> Result<(), XrpcError> {
    let Some((kind, token)) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
    else {
        return Ok(());
    };
    if !kind.eq_ignore_ascii_case("bearer") {
        return Ok(());
    }
    let header = token
        .split('.')
        .next()
        .and_then(|part| URL_SAFE_NO_PAD.decode(part).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    if header
        .as_ref()
        .and_then(|value| value.get("typ"))
        .and_then(serde_json::Value::as_str)
        != Some("at+jwt")
    {
        return Ok(());
    }
    let checked_at = state.now_s();
    let checked = verify(token, &state.config().auth, "at+jwt", Some(checked_at))?;
    let current = if state.config().entryway.is_some() {
        member_token_active(state, token, "legacy", &checked.did, None).await?
    } else {
        credential_current(state, checked.clone(), false).await?
    };
    if !current {
        return Err(XrpcError::bad_request(
            "ExpiredToken",
            "Token has been revoked by account recovery",
        ));
    }
    let _ = REQUEST_CREDENTIAL.try_with(|slot| {
        *slot.borrow_mut() = Some(RequestCredential {
            state: state.clone(),
            token: token.to_owned(),
            checked_at,
            claims: checked,
        });
    });
    Ok(())
}

async fn credential_current(
    state: &crate::state::AppState,
    checked: Verified,
    require_actor: bool,
) -> Result<bool, XrpcError> {
    state
        .blocking(move |state| {
            let db = state.reads().accounts()?;
            if require_actor
                && db
                    .actor(&checked.did, pds_storage::Availability::ANY)?
                    .is_none()
            {
                return Ok(false);
            }
            credential_matches(&db, &checked)
        })
        .await
}

fn credential_matches(db: &pds_storage::AccountDb, checked: &Verified) -> Result<bool, XrpcError> {
    let expected = db.credential_epoch(&checked.did)?;
    Ok(expected.is_none_or(|expected| {
        bool::from(
            expected
                .as_bytes()
                .ct_eq(checked.credential_epoch.as_deref().unwrap_or("").as_bytes()),
        )
    }))
}

/// Rechecks a legacy credential under the account transaction before
/// creating another credential that could survive the current request.
pub(crate) fn require_current_session(
    db: &pds_storage::AccountDb,
    config: &AuthConfig,
    headers: &HeaderMap,
    now_s: i64,
) -> Result<(), XrpcError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .map(|(_, token)| token)
        .ok_or_else(|| invalid_token("Invalid token"))?;
    let checked = verify(token, config, "at+jwt", Some(now_s))?;
    if !credential_matches(db, &checked)? {
        return Err(XrpcError::bad_request(
            "ExpiredToken",
            "Token has been revoked by account recovery",
        ));
    }
    Ok(())
}

/// The private entryway status endpoint checks a legacy token locally.
pub(crate) async fn legacy_token_active(
    state: &crate::state::AppState,
    token: &str,
) -> Result<Option<String>, XrpcError> {
    let Ok(checked) = verify(token, &state.config().auth, "at+jwt", Some(state.now_s())) else {
        return Ok(None);
    };
    let did = checked.did.clone();
    Ok(credential_current(state, checked, true)
        .await?
        .then_some(did))
}

/// The member verifies the token and proof before asking its configured
/// credential owner whether the session still exists. No status is cached.
pub(crate) async fn member_token_active(
    state: &crate::state::AppState,
    token: &str,
    kind: &str,
    did: &str,
    jti: Option<&str>,
) -> Result<bool, XrpcError> {
    let unavailable = || XrpcError {
        status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
        error: "UpstreamFailure".to_owned(),
        message: "The entryway could not confirm this session".to_owned(),
    };
    let entryway = state.config().entryway.as_ref().ok_or_else(unavailable)?;
    let password = entryway.admin_token.as_deref().ok_or_else(unavailable)?;
    let response = state
        .http()
        .post(format!(
            "{}/oauth/introspect",
            entryway.url.trim_end_matches('/')
        ))
        .basic_auth("admin", Some(password))
        .header(header::CONTENT_TYPE, "application/json")
        .body(serde_json::json!({"token": token, "kind": kind}).to_string())
        .timeout(
            state
                .config()
                .proxy_timeout
                .min(std::time::Duration::from_secs(5)),
        )
        .send()
        .await
        .map_err(|_| unavailable())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(unavailable());
    }
    let bytes = pds_identity::safe_fetch::read_bounded(response, 16 * 1024)
        .await
        .map_err(|_| unavailable())?;
    let body: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    match body.get("active").and_then(serde_json::Value::as_bool) {
        Some(false) => Ok(false),
        Some(true)
            if body.get("sub").and_then(serde_json::Value::as_str) == Some(did)
                && body.get("kind").and_then(serde_json::Value::as_str) == Some(kind)
                && body.get("jti").and_then(serde_json::Value::as_str) == jti =>
        {
            Ok(true)
        }
        _ => Err(unavailable()),
    }
}

fn access(
    token: &str,
    config: &AuthConfig,
    additional: &[&str],
    now_s: i64,
) -> Result<Credentials, XrpcError> {
    let verified = verified_access(token, config, now_s)?;
    let scope = verified.scope.as_str();
    if !ACCESS_STANDARD.contains(&scope) && !additional.contains(&scope) {
        return Err(invalid_token("Bad token scope"));
    }
    Ok(Credentials::Access {
        did: verified.did,
        scope: verified.scope,
    })
}

/// The scope of a refresh token.
pub const SCOPE_REFRESH: &str = "com.atproto.refresh";
/// The scope of a session that an app password opened.
pub const SCOPE_APP_PASS: &str = "com.atproto.appPass";
/// The scope of a session that a privileged app password opened.
pub const SCOPE_APP_PASS_PRIVILEGED: &str = "com.atproto.appPassPrivileged";
/// The scope of a session of the account password.
pub const SCOPE_ACCESS: &str = "com.atproto.access";

/// How long an access token lives: 120 minutes.
pub const ACCESS_LIFETIME_S: i64 = 120 * 60;
/// How long a refresh token lives: 90 days.
pub const REFRESH_LIFETIME_S: i64 = 90 * 24 * 60 * 60;

/// The account and the id of the refresh token of a request, as
/// `refresh()` of the Reference reads them. `allow_expired` accepts a token
/// past its expiry: `deleteSession` revokes one whenever it is presented.
pub fn refresh(
    headers: &HeaderMap,
    config: &AuthConfig,
    now_s: i64,
    allow_expired: bool,
) -> Result<(String, String), XrpcError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let token = match value.split_once(' ') {
        Some((kind, token)) if kind.eq_ignore_ascii_case("bearer") && !token.is_empty() => token,
        _ => return Err(XrpcError::auth_required("AuthMissing")),
    };
    let verified = verify(
        token,
        config,
        "refresh+jwt",
        (!allow_expired).then_some(now_s),
    )?;
    if verified.scope != SCOPE_REFRESH {
        return Err(invalid_token("Bad token scope"));
    }
    let Some(id) = verified.jti else {
        return Err(XrpcError {
            status: axum::http::StatusCode::UNAUTHORIZED,
            error: "MissingTokenId".to_owned(),
            message: "Unexpected missing refresh token id".to_owned(),
        });
    };
    Ok((verified.did, id))
}

fn sign(config: &AuthConfig, typ: &str, claims: &serde_json::Value) -> String {
    let alg = if config.signer.is_some() {
        "ES256K"
    } else {
        "HS256"
    };
    let head = URL_SAFE_NO_PAD.encode(serde_json::json!({"typ": typ, "alg": alg}).to_string());
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let signature = match &config.signer {
        // An entryway signs with its key, which its member hosts know.
        Some(signer) => signer
            .key
            .sign(format!("{head}.{body}").as_bytes())
            .to_vec(),
        // HMAC takes a key of any length, so there is no other arm.
        None => Hmac::<Sha256>::new_from_slice(&config.jwt_secret).map_or_else(
            |_| Vec::new(),
            |mut mac| {
                mac.update(format!("{head}.{body}").as_bytes());
                mac.finalize().into_bytes().to_vec()
            },
        ),
    };
    format!("{head}.{body}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// An access token for `did`, as `createAccessToken` of the Reference
/// mints it.
pub fn mint_access(config: &AuthConfig, did: &str, scope: &str, now_s: i64) -> String {
    mint_access_bound(config, did, scope, now_s, None)
}

/// An access token bound to the credential that opened or refreshed it.
pub(crate) fn mint_access_bound(
    config: &AuthConfig,
    did: &str,
    scope: &str,
    now_s: i64,
    epoch: Option<&str>,
) -> String {
    let mut claims = serde_json::json!({
        "scope": scope, "aud": config.service_did, "sub": did,
        "iat": now_s, "exp": now_s + ACCESS_LIFETIME_S,
    });
    if let Some(epoch) = epoch {
        claims["pdsCredentialEpoch"] = epoch.into();
    }
    sign(config, "at+jwt", &claims)
}

/// A refresh token for `did` with the id `jti`. Returns the token and the
/// second it ends.
pub fn mint_refresh(config: &AuthConfig, did: &str, jti: &str, now_s: i64) -> (String, i64) {
    let exp = now_s + REFRESH_LIFETIME_S;
    let token = sign(
        config,
        "refresh+jwt",
        &serde_json::json!({
            "scope": SCOPE_REFRESH, "aud": config.service_did, "sub": did,
            "jti": jti, "iat": now_s, "exp": exp,
        }),
    );
    (token, exp)
}

#[cfg(test)]
mod tests;

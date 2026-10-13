//! The endpoints a client calls (`docs/SPEC.md` §11.3): the pushed
//! authorization request, the token endpoint and revocation.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use pds_oauth::dpop::Proof;
use pds_oauth::pkce::verify_s256;
use pds_oauth::scope::is_scope;
use pds_oauth::token::{
    ASSERTION_TYPE, AccessClaims, CODE_CHALLENGE_REPLAY_MS, Id, PAR_EXPIRES_IN_MS,
    REQUEST_URI_PREFIX, TOKEN_MAX_AGE_MS, check_assertion, mint_access, verify_access,
};
use pds_storage::oauth::{RequestRow, Rotation, TokenRow};
use pds_storage::{Availability, StorageError};
use serde_json::{Map, Value, json};

use super::clients::{self, LoadedClient};
use super::{
    Fields, OAuthError, accounts, dpop_proof, iso, issuer, json as answer, millis, new_id,
};
use crate::state::AppState;

/// The parameters of an authorization request that the server keeps.
const PARAMETERS: [&str; 16] = [
    "client_id",
    "state",
    "redirect_uri",
    "scope",
    "response_type",
    "code_challenge",
    "code_challenge_method",
    "dpop_jkt",
    "response_mode",
    "nonce",
    "max_age",
    "claims",
    "login_hint",
    "ui_locales",
    "id_token_hint",
    "display",
];

fn invalid(description: impl Into<String>) -> OAuthError {
    OAuthError::invalid_request(description)
}

fn grant(description: impl Into<String>) -> OAuthError {
    OAuthError::invalid_grant(description)
}

struct Authenticated {
    client: Arc<LoadedClient>,
    /// How the client authenticated: the `clientAuth` of a row.
    auth: Value,
}

/// Finds the client of a request and checks how it authenticates.
async fn authenticate(
    state: &AppState,
    fields: &Fields,
    proof: Option<&Proof>,
    proof_required: bool,
) -> Result<Authenticated, OAuthError> {
    let client_id = fields
        .get("client_id")
        .ok_or_else(|| OAuthError::invalid_request("client_id is required"))?;
    let client = clients::get(state, client_id).await?;
    if proof.is_none() && proof_required {
        return Err(OAuthError::new("invalid_dpop_proof", "DPoP proof required"));
    }
    if !client.confidential() {
        return Ok(Authenticated {
            client,
            auth: json!({ "method": "none" }),
        });
    }
    let (Some(kind), Some(assertion)) = (
        fields.get("client_assertion_type"),
        fields.get("client_assertion"),
    ) else {
        return Err(OAuthError::invalid_request(
            "client_assertion_type and client_assertion are required for this client",
        ));
    };
    if kind != ASSERTION_TYPE {
        return Err(OAuthError::invalid_request(format!(
            "Unsupported client_assertion_type \"{kind}\""
        )));
    }
    let jwks = client.jwks.clone().unwrap_or(Value::Null);
    let now_ms = state.now_ms();
    let assertion = check_assertion(assertion, &jwks, client_id, &issuer(state), now_ms / 1000)
        .map_err(OAuthError::invalid_client)?;
    if proof.is_some_and(|proof| proof.jkt == assertion.jkt) {
        return Err(OAuthError::invalid_client(
            "The DPoP proof must be signed with a different key than the client assertion",
        ));
    }
    let key = format!("assertion:{client_id}:{}", assertion.jti);
    if !state.oauth().unique(key, assertion.exp * 1000, now_ms) {
        return Err(OAuthError::invalid_grant("private_key_jwt jti reused"));
    }
    Ok(Authenticated {
        client,
        auth: json!({
            "method": "private_key_jwt",
            "jkt": assertion.jkt,
            "alg": assertion.alg,
            "kid": assertion.kid,
        }),
    })
}

fn is_login_hint(hint: &str) -> bool {
    hint.parse::<pds_types::Did>().is_ok() || hint.parse::<pds_types::Handle>().is_ok()
}

/// Refuses the parameters the server does not support, and a request
/// without PKCE under S256.
fn check_supported(fields: &Fields) -> Result<(), OAuthError> {
    let get = |name: &str| fields.get(name).map(String::as_str);
    for unsupported in [
        "claims",
        "id_token_hint",
        "nonce",
        "request",
        "authorization_details",
    ] {
        if get(unsupported).is_some() {
            return Err(invalid(format!("Unsupported \"{unsupported}\" parameter")));
        }
    }
    match get("response_type") {
        Some("code") => {}
        Some(other) => {
            return Err(OAuthError::new(
                "unsupported_response_type",
                format!("Unsupported response_type \"{other}\""),
            ));
        }
        None => return Err(invalid("response_type is required")),
    }
    if !matches!(
        get("response_mode"),
        None | Some("query" | "fragment" | "form_post")
    ) {
        return Err(invalid("Unsupported response_mode"));
    }
    if !matches!(
        get("prompt"),
        None | Some("none" | "login" | "consent" | "select_account" | "create")
    ) {
        return Err(invalid("Unsupported prompt"));
    }
    match (get("code_challenge"), get("code_challenge_method")) {
        (Some(_), None | Some("plain" | "S256")) => {}
        (Some(_), Some(other)) => {
            return Err(invalid(format!(
                "Unsupported code_challenge_method \"{other}\""
            )));
        }
        (None, Some(_)) => {
            return Err(invalid(
                "code_challenge is required when code_challenge_method is provided",
            ));
        }
        (None, None) => return Err(invalid("Use of PKCE is required")),
    }
    if get("code_challenge_method") != Some("S256") {
        return Err(invalid(
            "atproto requires use of \"S256\" code_challenge_method",
        ));
    }
    Ok(())
}

/// Checks the parameters of an authorization request against the server
/// and the client, as `RequestManager.validate` of the Reference, and
/// returns them as the server keeps them.
fn checked_parameters(
    client: &LoadedClient,
    fields: &Fields,
    proof: Option<&Proof>,
) -> Result<Map<String, Value>, OAuthError> {
    check_supported(fields)?;
    let get = |name: &str| fields.get(name).map(String::as_str);

    // What the client may ask for.
    let declared = client.client.scopes();
    let requested: Vec<&str> = get("scope")
        .map(|scope| scope.split(' ').filter(|scope| !scope.is_empty()).collect())
        .unwrap_or_default();
    if let Some(scope) = requested.iter().find(|scope| !declared.contains(scope)) {
        return Err(OAuthError::new(
            "invalid_scope",
            format!("Scope \"{scope}\" is not declared in the client metadata"),
        ));
    }
    let redirect_uri = match get("redirect_uri") {
        Some(uri) if client.has_redirect_uri(uri) => uri.to_owned(),
        Some(uri) => {
            return Err(invalid(format!(
                "Invalid redirect_uri {uri} (allowed: {})",
                client.client.redirect_uris().join(" ")
            )));
        }
        None => match client.client.redirect_uris().as_slice() {
            [only] => (*only).to_owned(),
            _ => return Err(invalid("Missing \"redirect_uri\"")),
        },
    };

    let mut scopes: Vec<&str> = Vec::new();
    for scope in &requested {
        if !scopes.contains(scope) {
            scopes.push(scope);
        }
    }
    if !scopes.contains(&"atproto") {
        return Err(OAuthError::new(
            "invalid_scope",
            "The \"atproto\" scope is required",
        ));
    }
    if scopes.contains(&"openid") {
        return Err(OAuthError::new(
            "invalid_scope",
            "OpenID Connect is not compatible with atproto",
        ));
    }
    let mut prompt = get("prompt").map(str::to_owned);
    if !client.confidential() && !client.trusted {
        if prompt.as_deref() == Some("none") {
            return Err(OAuthError::new(
                "consent_required",
                "Public clients are not allowed to use silent-sign-on",
            ));
        }
        if prompt.as_deref() != Some("create") {
            prompt = Some("consent".to_owned());
        }
    }
    let login_hint = get("login_hint").map(str::to_lowercase);
    if let Some(hint) = login_hint.as_deref().filter(|hint| !is_login_hint(hint)) {
        return Err(invalid(format!("Invalid login_hint \"{hint}\"")));
    }
    let dpop_jkt = match (get("dpop_jkt"), proof) {
        (Some(given), Some(proof)) if given != proof.jkt => {
            return Err(OAuthError::new(
                "invalid_dpop_key_binding",
                "Invalid DPoP key binding",
            ));
        }
        (Some(given), _) => Some(given.to_owned()),
        (None, proof) => proof.map(|proof| proof.jkt.clone()),
    };

    let mut parameters = Map::new();
    for name in PARAMETERS {
        if let Some(value) = get(name) {
            parameters.insert(name.to_owned(), Value::String(value.to_owned()));
        }
    }
    let scope: Vec<&str> = scopes.into_iter().filter(|scope| is_scope(scope)).collect();
    let mut set = |name: &str, value: Option<String>| match value {
        Some(value) => parameters.insert(name.to_owned(), Value::String(value)),
        None => parameters.remove(name),
    };
    set("redirect_uri", Some(redirect_uri));
    set("scope", Some(scope.join(" ")));
    set("prompt", prompt);
    set("login_hint", login_hint);
    set("dpop_jkt", dpop_jkt);
    Ok(parameters)
}

/// `POST /oauth/par`.
pub async fn par(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, OAuthError> {
    let fields = super::read_fields(headers, body).await?;
    let proof = dpop_proof(state, headers, &Method::POST, "/oauth/par", None)?;
    let Authenticated { client, auth } =
        authenticate(state, &fields, proof.as_ref(), false).await?;
    let parameters = checked_parameters(&client, &fields, proof.as_ref())?;
    let now_ms = state.now_ms();
    let id = new_id(Id::Request);
    let row = RequestRow {
        id: id.clone(),
        did: None,
        device_id: None,
        client_id: client.client.id.clone(),
        client_auth: auth.to_string(),
        parameters: Value::Object(parameters).to_string(),
        expires_at: iso(now_ms + PAR_EXPIRES_IN_MS),
        code: None,
    };
    accounts(state, move |db| {
        db.delete_expired_requests(&iso(now_ms - PAR_EXPIRES_IN_MS))?;
        db.create_request(&row)
    })
    .await?;
    Ok(answer(
        StatusCode::CREATED,
        &json!({
            "request_uri": format!("{REQUEST_URI_PREFIX}{id}"),
            "expires_in": PAR_EXPIRES_IN_MS / 1000,
        }),
    ))
}

/// What a session began with: a pushed request or a token row.
struct Initial<'a> {
    client_id: &'a str,
    client_auth: &'a str,
    parameters: &'a Map<String, Value>,
}

/// Checks that the client and the key of this request are the ones of the
/// session, as `compareClientAuth` of the Reference.
fn compare_client_auth(
    client: &LoadedClient,
    auth: &Value,
    proof: Option<&Proof>,
    initial: &Initial<'_>,
) -> Result<(), OAuthError> {
    if initial.client_id != client.client.id {
        return Err(grant("Token was not issued to this client"));
    }
    if let Some(jkt) = initial.parameters.get("dpop_jkt").and_then(Value::as_str) {
        match proof {
            None => return Err(grant("DPoP proof is required for this request")),
            Some(proof) if proof.jkt != jkt => {
                return Err(grant("DPoP proof does not match the expected JKT"));
            }
            Some(_) => {}
        }
    }
    let before: Value = serde_json::from_str(initial.client_auth).unwrap_or(Value::Null);
    let method = |auth: &Value| {
        auth.get("method")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match method(&before).as_deref() {
        None | Some("none") => Ok(()),
        Some("private_key_jwt" | ASSERTION_TYPE) => {
            if method(auth).as_deref() != Some("private_key_jwt") {
                return Err(grant(
                    "Client authentication method mismatch (expected private_key_jwt)",
                ));
            }
            if ["kid", "alg", "jkt"]
                .iter()
                .any(|part| auth.get(part) != before.get(part))
            {
                return Err(grant(
                    "The session was initiated with a different key than the client assertion currently used",
                ));
            }
            Ok(())
        }
        Some(other) => Err(grant(format!("Invalid method \"{other}\""))),
    }
}

fn parameters_of(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).unwrap_or_default()
}

/// The answer of the token endpoint, and the access token of the row.
fn token_response(state: &AppState, row: &TokenRow, now_ms: i64) -> Response {
    let config = &state.config().auth;
    let parameters = parameters_of(&row.parameters);
    let jkt = parameters
        .get("dpop_jkt")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let expires_ms = millis(&row.expires_at);
    let token_type = if jkt.is_some() { "DPoP" } else { "Bearer" };
    let access = mint_access(
        &config.jwt_secret,
        &issuer(state),
        &config.service_did,
        &AccessClaims {
            jti: row.token_id.clone(),
            sub: row.did.clone(),
            iat: now_ms / 1000,
            exp: expires_ms / 1000,
            jkt,
            client_id: row.client_id.clone(),
        },
    );
    let mut body = json!({
        "access_token": access,
        "token_type": token_type,
        "scope": row.scope,
        "expires_in": (expires_ms - now_ms) / 1000,
        "sub": row.did,
    });
    if let Some(refresh) = &row.current_refresh_token {
        body["refresh_token"] = Value::String(refresh.clone());
    }
    answer(StatusCode::OK, &body)
}

/// When the access token of a session ends: after 60 minutes, or with the
/// session.
fn token_expiry(client: &LoadedClient, now_ms: i64, created_ms: i64) -> Result<i64, OAuthError> {
    let lifetime = TOKEN_MAX_AGE_MS.min(created_ms + client.session_lifetime_ms() - now_ms);
    if lifetime <= 250 {
        return Err(OAuthError::invalid_grant("Session expired"));
    }
    Ok(now_ms + lifetime)
}

/// The checks of a code against the request it came from: the client, the
/// key, the redirect URI and the PKCE verifier. Returns the parameters of
/// the session.
fn code_parameters(
    state: &AppState,
    fields: &Fields,
    proof: Option<&Proof>,
    client: &LoadedClient,
    auth: &Value,
    request: &RequestRow,
) -> Result<Map<String, Value>, OAuthError> {
    let now_ms = state.now_ms();
    let mut parameters = parameters_of(&request.parameters);
    compare_client_auth(
        client,
        auth,
        proof,
        &Initial {
            client_id: &request.client_id,
            client_auth: &request.client_auth,
            parameters: &parameters,
        },
    )?;
    if let (None, Some(proof)) = (parameters.get("dpop_jkt"), proof) {
        parameters.insert("dpop_jkt".to_owned(), Value::String(proof.jkt.clone()));
    }
    if parameters.get("dpop_jkt").is_none() {
        return Err(grant("DPoP JKT is required for DPoP bound access tokens"));
    }
    let text = |name: &str| parameters.get(name).and_then(Value::as_str);
    if text("redirect_uri") != fields.get("redirect_uri").map(String::as_str) {
        return Err(grant(
            "The redirect_uri parameter must match the one used in the authorization request",
        ));
    }
    let challenge = text("code_challenge").unwrap_or_default().to_owned();
    let verifier = fields
        .get("code_verifier")
        .ok_or_else(|| grant("code_verifier is required"))?;
    if verifier.len() < 43 {
        return Err(grant("code_verifier too short"));
    }
    if !verify_s256(verifier, &challenge) {
        return Err(grant("Invalid code_verifier"));
    }
    let until = now_ms + CODE_CHALLENGE_REPLAY_MS;
    if !state
        .oauth()
        .unique(format!("challenge:{challenge}"), until, now_ms)
    {
        return Err(grant("Code challenge already used"));
    }
    Ok(parameters)
}

async fn code_grant(
    state: &AppState,
    fields: &Fields,
    proof: Option<&Proof>,
    client: &LoadedClient,
    auth: &Value,
) -> Result<Response, OAuthError> {
    let code = fields
        .get("code")
        .filter(|code| Id::Code.matches(code))
        .ok_or_else(|| grant("Invalid code"))?
        .clone();
    let now_ms = state.now_ms();
    let receipts = state.config().profile.keeps_signup_receipts();
    let taken = {
        let code = code.clone();
        accounts(state, move |db| {
            // The receipt of an account that was created in this
            // authorization goes with the code into the session.
            let signup = if receipts {
                db.signup_of_code(&code)?
            } else {
                None
            };
            let request = db
                .consume_request_code(&code)?
                .map(|request| (request, signup));
            if request.is_none() {
                // A code that comes twice ends the session it opened the first time.
                if let Some(token) = db.token_by_code(&code)? {
                    db.delete_token(&token.token_id)?;
                    if let Some(device) = &token.device_id {
                        db.remove_device_account(device, &token.did)?;
                    }
                }
            }
            Ok(request)
        })
        .await?
    };
    let (request, signup) = taken.ok_or_else(|| grant("Invalid code"))?;
    let (Some(did), Some(device_id)) = (request.did.clone(), request.device_id.clone()) else {
        return Err(OAuthError::server("a request with a code has no account"));
    };
    if millis(&request.expires_at) < now_ms {
        return Err(grant("This code has expired"));
    }
    let parameters = code_parameters(state, fields, proof, client, auth, &request)?;
    let text = |name: &str| parameters.get(name).and_then(Value::as_str);

    let refresh = client
        .client
        .grant_types()
        .contains(&"refresh_token")
        .then(|| new_id(Id::Refresh));
    let row = TokenRow {
        id: 0,
        did,
        token_id: new_id(Id::Token),
        created_at: iso(now_ms),
        updated_at: iso(now_ms),
        expires_at: iso(token_expiry(client, now_ms, now_ms)?),
        client_id: client.client.id.clone(),
        client_auth: auth.to_string(),
        device_id: Some(device_id),
        scope: Some(text("scope").unwrap_or("atproto").to_owned()),
        parameters: Value::Object(parameters).to_string(),
        code: Some(code),
        current_refresh_token: refresh,
    };
    let stored = row.clone();
    let found = accounts(state, move |db| {
        // An account that was deleted or taken down since the consent gets no session.
        let account = db.actor(
            &stored.did,
            Availability {
                include_taken_down: false,
                include_deactivated: true,
            },
        )?;
        if account.is_some() {
            db.create_token(&stored)?;
            // Only the session of the account that was created holds its receipt.
            if signup.as_deref() == Some(stored.did.as_str()) {
                db.set_token_signup(&stored.token_id, &stored.did)?;
            }
        }
        Ok(account.is_some())
    })
    .await?;
    if !found {
        return Err(grant("Invalid code"));
    }
    Ok(token_response(state, &row, now_ms))
}

/// The session of a refresh token, and whether the token was used
/// before. A token that comes twice ends the session.
async fn find_session(
    state: &AppState,
    refresh: String,
) -> Result<Option<(TokenRow, bool)>, OAuthError> {
    accounts(state, move |db| {
        let Some(token) = db.token_by_refresh_token(&refresh)? else {
            return Ok(None);
        };
        // A session of an account that is gone or taken down is not found.
        let available = Availability {
            include_taken_down: false,
            include_deactivated: true,
        };
        if db.actor(&token.did, available)?.is_none() {
            return Ok(None);
        }
        let replayed = token.current_refresh_token.as_deref() != Some(refresh.as_str());
        if replayed {
            db.delete_token(&token.token_id)?;
        }
        Ok(Some((token, replayed)))
    })
    .await
}

async fn refresh_grant(
    state: &AppState,
    fields: &Fields,
    proof: Option<&Proof>,
    client: &LoadedClient,
    auth: &Value,
) -> Result<Response, OAuthError> {
    let refresh = fields
        .get("refresh_token")
        .filter(|token| Id::Refresh.matches(token))
        .ok_or_else(|| grant("Invalid refresh token"))?
        .clone();
    let now_ms = state.now_ms();
    let found = find_session(state, refresh.clone()).await?;
    let (token, replayed) = found.ok_or_else(|| grant("Invalid refresh token"))?;
    if replayed {
        return Err(grant("Refresh token replayed"));
    }

    let parameters = parameters_of(&token.parameters);
    let checked = compare_client_auth(
        client,
        auth,
        proof,
        &Initial {
            client_id: &token.client_id,
            client_auth: &token.client_auth,
            parameters: &parameters,
        },
    )
    .and_then(|()| {
        if !client.client.grant_types().contains(&"refresh_token") {
            return Err(grant("Refresh token grant not allowed"));
        }
        if now_ms - millis(&token.created_at) > client.session_lifetime_ms() {
            return Err(grant("Session expired"));
        }
        if now_ms - millis(&token.updated_at) > client.refresh_lifetime_ms() {
            return Err(grant("Refresh token expired"));
        }
        token_expiry(client, now_ms, millis(&token.created_at))
    });
    let expires_ms = match checked {
        Ok(expires_ms) => expires_ms,
        Err(error) => {
            // A refresh that fails ends the session.
            let id = token.token_id.clone();
            accounts(state, move |db| db.delete_token(&id)).await?;
            return Err(error);
        }
    };
    let next = TokenRow {
        token_id: new_id(Id::Token),
        current_refresh_token: Some(new_id(Id::Refresh)),
        updated_at: iso(now_ms),
        expires_at: iso(expires_ms),
        client_auth: auth.to_string(),
        scope: Some(
            token
                .scope
                .clone()
                .or_else(|| {
                    parameters
                        .get("scope")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "atproto".to_owned()),
        ),
        ..token
    };
    let stored = next.clone();
    let rotated = accounts(state, move |db| -> Result<bool, StorageError> {
        db.rotate_token(
            stored.id,
            Rotation {
                token_id: &stored.token_id,
                refresh_token: stored.current_refresh_token.as_deref().unwrap_or_default(),
                updated_at: &stored.updated_at,
                expires_at: &stored.expires_at,
                client_auth: &stored.client_auth,
                scope: stored.scope.as_deref().unwrap_or_default(),
            },
        )
    })
    .await?;
    if !rotated {
        return Err(OAuthError::server("a new refresh token was used before"));
    }
    Ok(token_response(state, &next, now_ms))
}

/// `POST /oauth/token`.
pub async fn token(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, OAuthError> {
    let fields = super::read_fields(headers, body).await?;
    let proof = dpop_proof(state, headers, &Method::POST, "/oauth/token", None)?;
    let Authenticated { client, auth } = authenticate(state, &fields, proof.as_ref(), true).await?;
    let grant_type = fields
        .get("grant_type")
        .ok_or_else(|| OAuthError::invalid_request("grant_type is required"))?
        .as_str();
    if !matches!(grant_type, "authorization_code" | "refresh_token") {
        return Err(OAuthError::invalid_grant(format!(
            "Grant type \"{grant_type}\" is not supported by the server"
        )));
    }
    if !client.client.grant_types().contains(&grant_type) {
        return Err(OAuthError::invalid_grant(format!(
            "\"{grant_type}\" grant type is not allowed for this client"
        )));
    }
    if grant_type == "authorization_code" {
        code_grant(state, &fields, proof.as_ref(), &client, &auth).await
    } else {
        refresh_grant(state, &fields, proof.as_ref(), &client, &auth).await
    }
}

/// `POST /oauth/revoke` (RFC 7009). A token nobody holds is revoked
/// already.
pub async fn revoke(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, OAuthError> {
    let fields = super::read_fields(headers, body).await?;
    let proof = dpop_proof(state, headers, &Method::POST, "/oauth/revoke", None)?;
    let Authenticated { client, auth } = authenticate(state, &fields, proof.as_ref(), true).await?;
    let token = fields
        .get("token")
        .ok_or_else(|| OAuthError::invalid_request("token is required"))?
        .clone();
    let jti = if token.split('.').count() == 3 {
        let config = &state.config().auth;
        verify_access(&config.jwt_secret, &issuer(state), &token).map(|claims| claims.jti)
    } else {
        None
    };
    let known = jti.is_some()
        || Id::Token.matches(&token)
        || Id::Code.matches(&token)
        || Id::Refresh.matches(&token);
    if !known && token.split('.').count() != 3 {
        return Err(OAuthError::invalid_request("Invalid token"));
    }
    let found = accounts(state, move |db| {
        if let Some(jti) = jti {
            db.token(&jti)
        } else if Id::Token.matches(&token) {
            db.token(&token)
        } else if Id::Code.matches(&token) {
            db.token_by_code(&token)
        } else if Id::Refresh.matches(&token) {
            db.token_by_refresh_token(&token)
        } else {
            Ok(None)
        }
    })
    .await?;
    if let Some(row) = found {
        compare_client_auth(
            &client,
            &auth,
            proof.as_ref(),
            &Initial {
                client_id: &row.client_id,
                client_auth: &row.client_auth,
                parameters: &parameters_of(&row.parameters),
            },
        )?;
        accounts(state, move |db| db.delete_token(&row.token_id)).await?;
    }
    Ok(answer(StatusCode::OK, &json!({})))
}

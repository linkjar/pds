//! The metadata documents of `docs/SPEC.md` §11.1, with the fields and the
//! values of the Reference at the pin.

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

use super::{issuer, json as answer};
use crate::state::AppState;

/// The signing algorithms of a DPoP proof and of a client assertion.
///
/// The Reference lists ten. The Candidate verifies the two that the
/// protocol's keys use.
const SIGNING_ALGS: [&str; 2] = ["ES256", "ES256K"];

/// `GET /.well-known/oauth-authorization-server`.
pub fn authorization_server(state: &AppState) -> Response {
    let issuer = issuer(state);
    // An entryway also protects its member host. An atproto client
    // refuses an authorization server that does not name the server of
    // the account.
    let mut resources = vec![issuer.clone()];
    if let Some(host) = &state.config().member_host {
        resources.push(host.url.clone());
    }
    answer(
        StatusCode::OK,
        &json!({
            "issuer": issuer,
            "request_parameter_supported": false,
            "request_uri_parameter_supported": true,
            "require_request_uri_registration": true,
            "scopes_supported": pds_oauth::scope::STATIC,
            "subject_types_supported": ["public"],
            "response_types_supported": ["code"],
            "response_modes_supported": ["query", "fragment", "form_post"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "ui_locales_supported": ["en-US"],
            "display_values_supported": ["page", "popup", "touch"],
            "prompt_values_supported": ["none", "login", "consent", "select_account", "create"],
            "authorization_response_iss_parameter_supported": true,
            "request_object_encryption_alg_values_supported": [],
            "request_object_encryption_enc_values_supported": [],
            "jwks_uri": format!("{issuer}/oauth/jwks"),
            "authorization_endpoint": format!("{issuer}/oauth/authorize"),
            "token_endpoint": format!("{issuer}/oauth/token"),
            "token_endpoint_auth_methods_supported": ["none", "private_key_jwt"],
            "token_endpoint_auth_signing_alg_values_supported": SIGNING_ALGS,
            "revocation_endpoint": format!("{issuer}/oauth/revoke"),
            "pushed_authorization_request_endpoint": format!("{issuer}/oauth/par"),
            "require_pushed_authorization_requests": true,
            "dpop_signing_alg_values_supported": SIGNING_ALGS,
            "protected_resources": resources,
            "client_id_metadata_document_supported": true,
        }),
    )
}

/// `GET /.well-known/oauth-protected-resource`.
pub fn protected_resource(state: &AppState) -> Response {
    let issuer = issuer(state);
    let authorization_server = state
        .config()
        .entryway
        .as_ref()
        .map_or_else(|| issuer.clone(), |entryway| entryway.url.clone());
    answer(
        StatusCode::OK,
        &json!({
            "resource": issuer,
            "authorization_servers": [authorization_server],
            "scopes_supported": [],
            "bearer_methods_supported": ["header"],
            "resource_documentation": "https://atproto.com",
        }),
    )
}

/// `GET /oauth/jwks`. The server signs with a secret, so it publishes no
/// key.
pub fn jwks() -> Response {
    answer(StatusCode::OK, &json!({ "keys": [] }))
}

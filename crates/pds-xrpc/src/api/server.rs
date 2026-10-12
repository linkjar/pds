//! `com.atproto.server` reads.

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{Map, Value, json};

use crate::error::json_response;
use crate::state::AppState;

/// `com.atproto.server.describeServer`.
pub fn describe_server(state: &AppState) -> Response {
    let config = state.config();
    let mut links = Map::new();
    if let Some(url) = &config.privacy_policy_url {
        links.insert("privacyPolicy".to_owned(), url.clone().into());
    }
    if let Some(url) = &config.terms_of_service_url {
        links.insert("termsOfService".to_owned(), url.clone().into());
    }
    let mut contact = Map::new();
    if let Some(email) = &config.contact_email {
        contact.insert("email".to_owned(), email.clone().into());
    }
    let body = json!({
        "did": config.service_did,
        "availableUserDomains": config.handle_domains,
        "inviteCodeRequired": config.invite_required,
        "blobUploadLimit": config.blob_upload_limit,
        "links": Value::Object(links),
        "contact": Value::Object(contact),
    });
    json_response(StatusCode::OK, &body)
}

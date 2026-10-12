//! XRPC errors and their HTTP form (`docs/SPEC.md` §4.2).

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// The content type of every JSON answer, as the Reference sends it.
pub const JSON: &str = "application/json; charset=utf-8";

/// An error answer: a status and the body `{"error", "message"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XrpcError {
    /// The HTTP status.
    pub status: StatusCode,
    /// The error name.
    pub error: String,
    /// The message for a person.
    pub message: String,
}

impl XrpcError {
    /// 400 with a name of the caller's choice.
    pub fn bad_request(error: &str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: error.to_owned(),
            message: message.into(),
        }
    }

    /// 400 `InvalidRequest`.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::bad_request("InvalidRequest", message)
    }

    /// 401 with the Reference's default message.
    pub fn auth_required(error: &str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: error.to_owned(),
            message: "Authentication Required".to_owned(),
        }
    }

    /// 401 with a message.
    pub fn unauthorized(error: &str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: error.to_owned(),
            message: message.into(),
        }
    }

    /// 500. The cause goes to the log and never to the client.
    pub fn internal(cause: impl std::fmt::Display) -> Self {
        tracing::error!(%cause, "request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "InternalServerError".to_owned(),
            message: "Internal Server Error".to_owned(),
        }
    }

    /// 501 for a method this build does not serve yet.
    pub fn not_implemented(nsid: &str) -> Self {
        Self {
            status: StatusCode::NOT_IMPLEMENTED,
            error: "MethodNotImplemented".to_owned(),
            message: format!("Method Not Implemented: {nsid}"),
        }
    }
}

impl IntoResponse for XrpcError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.error, "message": self.message });
        json_response(self.status, &body)
    }
}

/// A JSON answer with the Reference's content type.
pub fn json_response(status: StatusCode, body: &serde_json::Value) -> Response {
    let mut response = (status, body.to_string()).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(JSON));
    response
}

/// A `Content-Type` as Express sends it: a type that carries text gets
/// the UTF-8 charset when it names none.
///
/// Express takes the charset from its media-type table. The types below are
/// the ones in that table that a blob or a forwarded answer can have here.
pub fn express_content_type(given: &str) -> String {
    let essence = given
        .split(';')
        .next()
        .unwrap_or(given)
        .trim()
        .to_ascii_lowercase();
    let textual = essence.starts_with("text/")
        || matches!(
            essence.as_str(),
            "application/json" | "application/javascript" | "application/manifest+json"
        );
    if textual && !given.to_ascii_lowercase().contains("charset") {
        format!("{given}; charset=utf-8")
    } else {
        given.to_owned()
    }
}

impl From<pds_storage::StorageError> for XrpcError {
    fn from(error: pds_storage::StorageError) -> Self {
        Self::internal(error)
    }
}

impl From<pds_repo::RepoError> for XrpcError {
    fn from(error: pds_repo::RepoError) -> Self {
        Self::internal(error)
    }
}

impl From<tokio::task::JoinError> for XrpcError {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::internal(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_textual_type_is_sent_with_a_charset() {
        assert_eq!(
            express_content_type("text/plain"),
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            express_content_type("application/json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(
            express_content_type("text/plain; charset=latin1"),
            "text/plain; charset=latin1"
        );
        assert_eq!(express_content_type("image/png"), "image/png");
        assert_eq!(
            express_content_type("application/vnd.ipld.car"),
            "application/vnd.ipld.car"
        );
    }
}

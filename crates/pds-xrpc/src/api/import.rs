//! `com.atproto.repo.importRepo` (`docs/SPEC.md` §6.5).

use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;

use crate::error::XrpcError;
use crate::state::AppState;

/// `com.atproto.repo.importRepo`.
pub async fn import_repo(
    _state: &AppState,
    _headers: &HeaderMap,
    _body: Body,
) -> Result<Response, XrpcError> {
    Err(XrpcError::not_implemented("com.atproto.repo.importRepo"))
}

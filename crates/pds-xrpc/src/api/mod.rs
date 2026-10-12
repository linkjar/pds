//! The handlers, by namespace, and the way from a request to one.

pub mod identity;
pub mod repo;
pub mod root;
pub mod server;
pub mod sync;

use axum::extract::State;
use axum::http::{HeaderMap, Method, Uri, header};
use axum::response::{IntoResponse, Response};
use pds_storage::{AccountDb, Actor, Availability};

use crate::auth::{Credentials, optional_credentials};
use crate::error::XrpcError;
use crate::params::{Format, Params, Spec};
use crate::state::AppState;

/// One query as a handler sees it.
#[derive(Debug)]
pub struct Query<'a> {
    /// The validated parameters.
    pub params: Params,
    /// The request headers.
    pub headers: &'a HeaderMap,
    /// The raw query string, for a request that is forwarded.
    pub raw_query: Option<&'a str>,
}

impl Query<'_> {
    /// The optional credential of the request. `additional` names scopes
    /// the method accepts besides the standard access scopes.
    pub fn credentials(
        &self,
        state: &AppState,
        additional: &[&str],
    ) -> Result<Credentials, XrpcError> {
        optional_credentials(
            self.headers,
            &state.config().auth,
            additional,
            state.now_s(),
        )
    }
}

/// The parameters of each query this unit serves, in the order its lexicon
/// declares them. `None` for a method without a handler.
fn parameters(nsid: &str) -> Option<&'static [Spec]> {
    const DID: Spec = Spec::required("did", Format::Did);
    Some(match nsid {
        "com.atproto.sync.getRepo" => const { &[DID, Spec::optional("since", Format::Tid)] },
        "com.atproto.sync.getCheckout"
        | "com.atproto.sync.getLatestCommit"
        | "com.atproto.sync.getHead"
        | "com.atproto.sync.getRepoStatus" => const { &[DID] },
        "com.atproto.sync.getRecord" => {
            const {
                &[
                    DID,
                    Spec::required("collection", Format::Nsid),
                    Spec::required("rkey", Format::RecordKey),
                ]
            }
        }
        "com.atproto.sync.getBlocks" => const { &[DID, Spec::list("cids", Format::Cid)] },
        "com.atproto.sync.listRepos" => {
            const {
                &[
                    Spec::integer("limit", 1, 1000, 500),
                    Spec::optional("cursor", Format::Any),
                ]
            }
        }
        "com.atproto.sync.listBlobs" => {
            const {
                &[
                    DID,
                    Spec::optional("since", Format::Tid),
                    Spec::integer("limit", 1, 1000, 500),
                    Spec::optional("cursor", Format::Any),
                ]
            }
        }
        "com.atproto.sync.getBlob" => const { &[DID, Spec::required("cid", Format::Cid)] },
        "com.atproto.repo.getRecord" => {
            const {
                &[
                    Spec::required("repo", Format::AtIdentifier),
                    Spec::required("collection", Format::Nsid),
                    Spec::required("rkey", Format::RecordKey),
                    Spec::optional("cid", Format::Cid),
                ]
            }
        }
        "com.atproto.repo.listRecords" => {
            const {
                &[
                    Spec::required("repo", Format::AtIdentifier),
                    Spec::required("collection", Format::Nsid),
                    Spec::integer("limit", 1, 100, 50),
                    Spec::optional("cursor", Format::Any),
                    Spec::boolean("reverse", false),
                ]
            }
        }
        "com.atproto.repo.describeRepo" => {
            const { &[Spec::required("repo", Format::AtIdentifier)] }
        }
        "com.atproto.server.describeServer" => const { &[] },
        "com.atproto.identity.resolveHandle" => {
            const { &[Spec::required("handle", Format::Handle)] }
        }
        _ => return None,
    })
}

/// The method name of an `/xrpc/` path, as the Reference extracts it:
/// letters and digits in parts separated by single dots or hyphens, at
/// least two characters, with an optional trailing slash.
fn path_nsid(path: &str) -> Option<&str> {
    let name = path.strip_prefix("/xrpc/")?;
    let name = name.strip_suffix('/').unwrap_or(name);
    let mut separator_allowed = false;
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() {
            separator_allowed = true;
        } else if (byte == b'-' || byte == b'.') && separator_allowed {
            separator_allowed = false;
        } else {
            return None;
        }
    }
    (separator_allowed && name.len() >= 2).then_some(name)
}

/// Every request enters here.
pub async fn entry(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    let reads = method == Method::GET || method == Method::HEAD;
    match path {
        "/" if reads => return root::banner(),
        "/robots.txt" if reads => return root::robots(),
        "/xrpc/_health" if reads => return root::health(&state).await,
        "/.well-known/atproto-did" if reads => return root::atproto_did(&state, &headers).await,
        _ => {}
    }
    if !path.starts_with("/xrpc/") {
        return root::not_found(&method, path);
    }
    let Some(nsid) = path_nsid(path) else {
        return XrpcError::invalid_request("invalid xrpc path").into_response();
    };
    if !reads && method != Method::POST {
        return XrpcError::invalid_request("XRPC requests only supports GET and POST")
            .into_response();
    }
    xrpc(&state, nsid, &method, uri.query(), &headers)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn xrpc(
    state: &AppState,
    nsid: &str,
    method: &Method,
    raw_query: Option<&str>,
    headers: &HeaderMap,
) -> Result<Response, XrpcError> {
    let Some(specs) = parameters(nsid) else {
        return Err(unhandled(state, nsid, headers));
    };
    if method != Method::GET && method != Method::HEAD {
        return Err(XrpcError::invalid_request(format!(
            "Incorrect HTTP method ({method}) expected GET"
        )));
    }
    let query = Query {
        params: Params::parse(nsid, raw_query, specs)?,
        headers,
        raw_query,
    };
    match nsid {
        "com.atproto.sync.getRepo" => sync::get_repo(state, &query, true).await,
        "com.atproto.sync.getCheckout" => sync::get_repo(state, &query, false).await,
        "com.atproto.sync.getRecord" => sync::get_record(state, &query).await,
        "com.atproto.sync.getBlocks" => sync::get_blocks(state, &query).await,
        "com.atproto.sync.getLatestCommit" => sync::get_latest_commit(state, &query).await,
        "com.atproto.sync.getHead" => sync::get_head(state, &query).await,
        "com.atproto.sync.getRepoStatus" => sync::get_repo_status(state, &query).await,
        "com.atproto.sync.listRepos" => sync::list_repos(state, &query).await,
        "com.atproto.sync.listBlobs" => sync::list_blobs(state, &query).await,
        "com.atproto.sync.getBlob" => sync::get_blob(state, &query).await,
        "com.atproto.repo.getRecord" => repo::get_record(state, &query).await,
        "com.atproto.repo.listRecords" => repo::list_records(state, &query).await,
        "com.atproto.repo.describeRepo" => repo::describe_repo(state, &query).await,
        "com.atproto.server.describeServer" => Ok(server::describe_server(state)),
        _ => identity::resolve_handle(state, &query).await,
    }
}

/// A method with no handler. At the pin it goes to the catch-all proxy,
/// which wants a session first (SPEC §4.2, VP-6): no credential is 401, and
/// the admin password is not a session. The proxy itself is not in this
/// unit, so a request with a valid session is 501.
fn unhandled(state: &AppState, nsid: &str, headers: &HeaderMap) -> XrpcError {
    if headers
        .get(header::AUTHORIZATION)
        .is_none_or(axum::http::HeaderValue::is_empty)
    {
        return XrpcError::auth_required("AuthMissing");
    }
    match optional_credentials(headers, &state.config().auth, &[], state.now_s()) {
        Ok(Credentials::Access { .. }) => XrpcError::not_implemented(nsid),
        Ok(Credentials::Admin | Credentials::None) => {
            XrpcError::bad_request("InvalidToken", "Unexpected authorization type")
        }
        Err(error) if error.error == "AuthenticationRequired" => {
            XrpcError::bad_request("InvalidToken", "Unexpected authorization type")
        }
        Err(error) => error,
    }
}

/// The account behind a repository read, as `assertRepoAvailability` of the
/// Reference finds it: an admin or the account itself reads it whatever its
/// status; anyone else reads an active account only.
pub fn repo_availability(
    state: &AppState,
    handle_or_did: &str,
    admin_or_self: bool,
) -> Result<Actor, XrpcError> {
    let accounts = AccountDb::open(&state.layout().account_db)?;
    let actor = accounts
        .actor(handle_or_did, Availability::ANY)?
        .ok_or_else(|| {
            XrpcError::bad_request(
                "RepoNotFound",
                format!("Could not find repo for DID: {handle_or_did}"),
            )
        })?;
    if admin_or_self {
        return Ok(actor);
    }
    if actor.takedown_ref.is_some() {
        return Err(XrpcError::bad_request(
            "RepoTakendown",
            format!("Repo has been takendown: {handle_or_did}"),
        ));
    }
    if actor.deactivated_at.is_some() {
        return Err(XrpcError::bad_request(
            "RepoDeactivated",
            format!("Repo has been deactivated: {handle_or_did}"),
        ));
    }
    Ok(actor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_method_name_is_read_from_the_path_as_the_reference_reads_it() {
        assert_eq!(
            path_nsid("/xrpc/com.atproto.sync.getRepo"),
            Some("com.atproto.sync.getRepo")
        );
        assert_eq!(
            path_nsid("/xrpc/com.atproto.sync.getRepo/"),
            Some("com.atproto.sync.getRepo")
        );
        // Not an NSID, and still a name: the Reference sends it to its proxy.
        assert_eq!(path_nsid("/xrpc/nope"), Some("nope"));
        for bad in [
            "/xrpc/",
            "/xrpc/a",
            "/xrpc/a/b",
            "/xrpc/_health",
            "/xrpc/a..b",
            "/xrpc/.a",
            "/xrpc/a.",
            "/xrpc/a b",
        ] {
            assert_eq!(path_nsid(bad), None, "{bad}");
        }
    }

    #[test]
    fn every_served_method_declares_its_parameters() {
        assert!(parameters("com.atproto.sync.getRepo").is_some());
        assert!(parameters("com.atproto.server.createSession").is_none());
    }
}

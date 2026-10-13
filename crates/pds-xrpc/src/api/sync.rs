//! `com.atproto.sync` reads.

use std::io::{Read, Write};

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use cid::Cid;
use pds_repo::{CarWriter, Repo, write_car};
use pds_storage::{ActorDb, RepoRoot};
use serde_json::{Map, Value, json};

use super::{Query, repo_availability};
use crate::auth::SCOPE_TAKENDOWN;
use crate::error::{XrpcError, express_content_type, json_response};
use crate::state::AppState;
use crate::stream::blocking_body;

const CAR: &str = "application/vnd.ipld.car";

fn car_response(body: impl Into<axum::body::Body>) -> Response {
    let mut response = Response::new(body.into());
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(CAR));
    response
}

fn io_error(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

/// The store of a hosted account and its latest commit.
fn open_with_root(state: &AppState, did: &str) -> Result<(ActorDb, RepoRoot), XrpcError> {
    let store = ActorDb::open(&state.layout().actor_db(did))?;
    let root = store
        .root()?
        .ok_or_else(|| XrpcError::internal("the store has no repository root"))?;
    Ok((store, root))
}

/// `com.atproto.sync.getRepo`, and `getCheckout` when `with_since` is
/// false.
///
/// The blocks go out in the Reference's order: from the newest revision
/// down, then by CID down. That order comes from the block table and not
/// from the tree, so the export is the same bytes as the Reference's for the
/// same data directory.
pub async fn get_repo(
    state: &AppState,
    query: &Query<'_>,
    with_since: bool,
) -> Result<Response, XrpcError> {
    let did = query.params.text("did").to_owned();
    let since = query.params.get("since").map(str::to_owned);
    let additional: &[&str] = if with_since { &[SCOPE_TAKENDOWN] } else { &[] };
    let credentials = query.credentials(state, additional)?;
    // The export has a bucket of its own and spends nothing of the
    // address bucket.
    crate::ratelimit::consume(
        state,
        &[(
            crate::ratelimit::of_route("com.atproto.sync.getRepo-0", 300, 6000),
            crate::ratelimit::Key::Ip,
            1,
        )],
    )?;
    let (store, root) = state
        .blocking(move |state| {
            repo_availability(state, &did, credentials.is_admin_or(&did))?;
            let (store, root) = open_with_root(state, &did)?;
            Ok((store, root.cid))
        })
        .await?;
    let body = blocking_body(move |out| {
        let mut car = CarWriter::new(out, Some(&root))?;
        let mut after: Option<(String, String)> = None;
        loop {
            let position = after
                .as_ref()
                .map(|(rev, cid)| (rev.as_str(), cid.as_str()));
            let page = store
                .block_page(since.as_deref(), position)
                .map_err(io_error)?;
            let Some(last) = page.last() else {
                break;
            };
            after = Some((last.rev.clone(), last.cid.clone()));
            for block in &page {
                let cid = Cid::try_from(block.cid.as_str()).map_err(io_error)?;
                car.block(&cid, &block.content)?;
            }
        }
        Ok(())
    });
    Ok(car_response(body))
}

/// `com.atproto.sync.getRecord`: the commit, the record when it exists, and
/// the tree nodes that prove it.
pub async fn get_record(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let did = query.params.text("did").to_owned();
    let key = format!(
        "{}/{}",
        query.params.text("collection"),
        query.params.text("rkey")
    );
    let credentials = query.credentials(state, &[])?;
    let car = state
        .blocking(move |state| {
            repo_availability(state, &did, credentials.is_admin_or(&did))?;
            let (store, root) = open_with_root(state, &did)?;
            let repo = Repo::load(&store, root.cid)?;
            let proof = repo.record_proof(&key)?;
            Ok(write_car(Some(&root.cid), &proof))
        })
        .await?;
    Ok(car_response(car))
}

/// `com.atproto.sync.getBlocks`: the named blocks in a CAR without a root.
///
/// The handler checks that the store holds every block before the status
/// line goes out. It then streams the blocks, so the server never holds the
/// whole answer in memory.
pub async fn get_blocks(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let did = query.params.text("did").to_owned();
    let cids: Vec<Cid> = query
        .params
        .list("cids")
        .into_iter()
        .filter_map(|cid| Cid::try_from(cid).ok())
        .collect();
    let credentials = query.credentials(state, &[])?;
    let (store, cids) = state
        .blocking(move |state| {
            repo_availability(state, &did, credentials.is_admin_or(&did))?;
            let store = ActorDb::open(&state.layout().actor_db(&did))?;
            let missing = store.missing_blocks(&cids)?;
            if !missing.is_empty() {
                let list: Vec<String> = missing.iter().map(Cid::to_string).collect();
                return Err(XrpcError::invalid_request(format!(
                    "Could not find cids: {}",
                    list.join(",")
                )));
            }
            Ok((store, cids))
        })
        .await?;
    let body = blocking_body(move |out| {
        let mut car = CarWriter::new(out, None)?;
        for batch in cids.chunks(pds_storage::BLOCK_BATCH) {
            let blocks = store.blocks(batch).map_err(io_error)?;
            if let Some(cid) = blocks.missing.first() {
                return Err(io_error(format!("block {cid} left the store")));
            }
            for (cid, bytes) in &blocks.found {
                car.block(cid, bytes)?;
            }
        }
        Ok(())
    });
    Ok(car_response(body))
}

/// The latest commit of an account the caller may read.
async fn readable_root(state: &AppState, query: &Query<'_>) -> Result<RepoRoot, XrpcError> {
    let did = query.params.text("did").to_owned();
    let credentials = query.credentials(state, &[])?;
    state
        .blocking(move |state| {
            repo_availability(state, &did, credentials.is_admin_or(&did))?;
            Ok(open_with_root(state, &did)?.1)
        })
        .await
}

/// `com.atproto.sync.getLatestCommit`.
pub async fn get_latest_commit(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let root = readable_root(state, query).await?;
    Ok(json_response(
        StatusCode::OK,
        &json!({ "cid": root.cid.to_string(), "rev": root.rev }),
    ))
}

/// `com.atproto.sync.getHead`, deprecated and still served at the pin.
pub async fn get_head(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let root = readable_root(state, query).await?;
    Ok(json_response(
        StatusCode::OK,
        &json!({ "root": root.cid.to_string() }),
    ))
}

/// `com.atproto.sync.getRepoStatus`: answered for every hosted account,
/// whatever its status.
pub async fn get_repo_status(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let did = query.params.text("did").to_owned();
    let body = state
        .blocking(move |state| {
            let actor = repo_availability(state, &did, true)?;
            let mut body = Map::new();
            body.insert("did".to_owned(), did.clone().into());
            body.insert("active".to_owned(), actor.status().is_none().into());
            if let Some(status) = actor.status() {
                body.insert("status".to_owned(), status.into());
            } else {
                let (_, root) = open_with_root(state, &did)?;
                body.insert("rev".to_owned(), root.rev.into());
            }
            Ok(Value::Object(body))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &body))
}

/// The cursor of `listRepos`: `<creation time in ms>::<did>`.
pub fn parse_repo_cursor(cursor: &str) -> Result<(String, String), XrpcError> {
    let malformed = || XrpcError::invalid_request("Malformed cursor");
    let mut parts = cursor.split("::");
    let (Some(primary), Some(secondary), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(malformed());
    };
    if primary.is_empty() || secondary.is_empty() {
        return Err(malformed());
    }
    // As JavaScript's parseInt: leading white space, a sign, then digits;
    // what follows the digits is ignored.
    let trimmed = primary.trim_start();
    let (sign, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let millis: i64 = digits.parse().map_err(|_| malformed())?;
    let time = chrono::DateTime::from_timestamp_millis(sign * millis).ok_or_else(malformed)?;
    Ok((
        time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        secondary.to_owned(),
    ))
}

/// `com.atproto.sync.listRepos`.
pub async fn list_repos(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let limit = query.params.limit("limit");
    let cursor = query
        .params
        .get("cursor")
        .map(parse_repo_cursor)
        .transpose()?;
    let rows = state
        .blocking(move |state| {
            let accounts = state.layout().accounts()?;
            let after = cursor
                .as_ref()
                .map(|(created, did)| (created.as_str(), did.as_str()));
            Ok(accounts.list_repos(limit, after)?)
        })
        .await?;
    let mut body = Map::new();
    if let Some(last) = rows.last() {
        let created =
            chrono::DateTime::parse_from_rfc3339(&last.actor.created_at).map_err(|e| {
                XrpcError::internal(format!("actor.createdAt of {}: {e}", last.actor.did))
            })?;
        body.insert(
            "cursor".to_owned(),
            format!("{}::{}", created.timestamp_millis(), last.actor.did).into(),
        );
    }
    let repos: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut repo = Map::new();
            repo.insert("did".to_owned(), row.actor.did.clone().into());
            repo.insert("head".to_owned(), row.head.clone().into());
            repo.insert("rev".to_owned(), row.rev.clone().into());
            repo.insert("active".to_owned(), row.actor.status().is_none().into());
            if let Some(status) = row.actor.status() {
                repo.insert("status".to_owned(), status.into());
            }
            Value::Object(repo)
        })
        .collect();
    body.insert("repos".to_owned(), repos.into());
    Ok(json_response(StatusCode::OK, &body.into()))
}

/// `com.atproto.sync.listBlobs`.
pub async fn list_blobs(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let did = query.params.text("did").to_owned();
    let since = query.params.get("since").map(str::to_owned);
    let limit = query.params.limit("limit");
    let cursor = query.params.get("cursor").map(str::to_owned);
    let credentials = query.credentials(state, &[SCOPE_TAKENDOWN])?;
    let cids = state
        .blocking(move |state| {
            repo_availability(state, &did, credentials.is_admin_or(&did))?;
            let store = ActorDb::open(&state.layout().actor_db(&did))?;
            Ok(store.list_blobs(since.as_deref(), cursor.as_deref(), limit)?)
        })
        .await?;
    let mut body = Map::new();
    if let Some(last) = cids.last() {
        body.insert("cursor".to_owned(), last.clone().into());
    }
    body.insert("cids".to_owned(), cids.into());
    Ok(json_response(StatusCode::OK, &body.into()))
}

/// `com.atproto.sync.getBlob`, with the headers that keep a browser from
/// running what it downloads (SPEC §6.4, §15.3).
pub async fn get_blob(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let did = query.params.text("did").to_owned();
    let cid = query.params.text("cid").to_owned();
    let credentials = query.credentials(state, &[SCOPE_TAKENDOWN])?;
    let not_found = || XrpcError::invalid_request("Blob not found");
    let name = cid.clone();
    let (meta, mut file) = state
        .blocking(move |state| {
            repo_availability(state, &did, credentials.is_admin_or(&did))?;
            let store = ActorDb::open(&state.layout().actor_db(&did))?;
            let meta = store.blob(&cid)?.ok_or_else(not_found)?;
            let blobs = state
                .blobs()
                .ok_or_else(|| XrpcError::internal("no blob store is configured"))?;
            let file = match blobs.open(&did, &cid) {
                Ok(file) => file,
                Err(pds_blob::BlobError::NotFound) => return Err(not_found()),
                Err(error) => return Err(XrpcError::internal(error)),
            };
            Ok((meta, file))
        })
        .await?;
    let body = blocking_body(move |out| {
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            out.write_all(buffer.get(..read).unwrap_or_default())?;
        }
    });
    let mime = if meta.mime_type.is_empty() {
        "application/octet-stream".to_owned()
    } else {
        express_content_type(&meta.mime_type)
    };
    let header_value = |value: &str| HeaderValue::from_str(value).map_err(XrpcError::internal);
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, header_value(&mime)?);
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(meta.size));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        header_value(&format!("attachment; filename=\"{name}\""))?,
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_cursor_is_read_as_the_reference_reads_it() {
        assert_eq!(
            parse_repo_cursor("1791843389820::did:plc:abc").unwrap(),
            (
                "2026-10-12T22:16:29.820Z".to_owned(),
                "did:plc:abc".to_owned()
            )
        );
        // parseInt ignores what follows the digits.
        assert_eq!(
            parse_repo_cursor("5x::did").unwrap().0,
            "1970-01-01T00:00:00.005Z"
        );
        for bad in [
            "zzz",
            "abc::did",
            "1::",
            "::did",
            "1::a::b",
            "9999999999999999999::did",
        ] {
            assert_eq!(
                parse_repo_cursor(bad).unwrap_err().message,
                "Malformed cursor",
                "{bad}"
            );
        }
    }
}

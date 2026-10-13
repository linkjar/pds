//! `com.atproto.repo` reads.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pds_identity::resolver::claimed_handle;
use pds_storage::{AccountDb, ActorDb, Availability, RecordRow};
use pds_types::{Data, Did, Handle};
use serde_json::{Map, Value, json};

use super::{Query, repo_availability};
use crate::error::{XrpcError, json_response};
use crate::state::AppState;

/// Record data as JSON, with the keys of each object in the order of the
/// stored block: by length, then by bytes. The Reference decodes the block
/// and writes its keys as they come, so the bodies agree byte for byte.
fn lex_json(data: &Data) -> Result<Value, XrpcError> {
    Ok(match data {
        Data::Array(items) => Value::Array(items.iter().map(lex_json).collect::<Result<_, _>>()?),
        Data::Object(fields) => {
            let mut entries: Vec<(&String, &Data)> = fields.iter().collect();
            entries.sort_by(|(a, _), (b, _)| {
                a.len()
                    .cmp(&b.len())
                    .then_with(|| a.as_bytes().cmp(b.as_bytes()))
            });
            let mut object = Map::with_capacity(entries.len());
            for (key, value) in entries {
                object.insert(key.clone(), lex_json(value)?);
            }
            Value::Object(object)
        }
        scalar => serde_json::to_value(scalar).map_err(XrpcError::internal)?,
    })
}

/// A record block as the JSON a client reads.
pub(crate) fn record_value(row: &RecordRow) -> Result<Value, XrpcError> {
    let data = Data::from_dag_cbor(&row.content)
        .map_err(|e| XrpcError::internal(format!("record block of {}: {e}", row.uri)))?;
    lex_json(&data)
}

/// `com.atproto.repo.getRecord`. A repository this server does not host is
/// read from the AppView, without a credential (SPEC §4.4).
pub async fn get_record(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let repo = query.params.text("repo").to_owned();
    let collection = query.params.text("collection").to_owned();
    let rkey = query.params.text("rkey").to_owned();
    let cid = query.params.get("cid").map(str::to_owned);
    let found = state
        .blocking(move |state| {
            let accounts = AccountDb::open(&state.layout().account_db)?;
            let Some(actor) = accounts.actor(&repo, Availability::ACTIVE)? else {
                return Ok(None);
            };
            let uri = format!("at://{}/{collection}/{rkey}", actor.did);
            let store = ActorDb::open(&state.layout().actor_db(&actor.did))?;
            let record = store.record(&uri, cid.as_deref())?.ok_or_else(|| {
                XrpcError::bad_request("RecordNotFound", format!("Could not locate record: {uri}"))
            })?;
            let value = record_value(&record)?;
            Ok(Some(
                json!({ "uri": uri, "cid": record.cid, "value": value }),
            ))
        })
        .await?;
    match found {
        Some(body) => Ok(json_response(StatusCode::OK, &body)),
        None => forward_to_appview(state, query).await,
    }
}

/// Sends the request to the AppView as it came, without a credential, and
/// returns the AppView's answer.
async fn forward_to_appview(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let appview = state
        .config()
        .appview_url
        .as_deref()
        .ok_or_else(|| XrpcError::invalid_request("Could not locate record"))?;
    let upstream_failure = |_| XrpcError {
        status: StatusCode::BAD_GATEWAY,
        error: "UpstreamFailure".to_owned(),
        message: "Upstream Failure".to_owned(),
    };
    let mut url = format!("{appview}/xrpc/com.atproto.repo.getRecord");
    if let Some(raw) = query.raw_query {
        url.push('?');
        url.push_str(raw);
    }
    let response = state
        .http()
        .get(url)
        .timeout(state.config().proxy_timeout)
        .send()
        .await
        .map_err(upstream_failure)?;
    // The Reference reports an upstream 500 as its own 502 and keeps the
    // upstream's error body.
    let status = match response.status().as_u16() {
        500 => StatusCode::BAD_GATEWAY,
        other => StatusCode::from_u16(other).map_err(XrpcError::internal)?,
    };
    // Express names the charset of a textual type that came without one.
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| HeaderValue::from_str(&crate::error::express_content_type(value)).ok());
    let bytes = response.bytes().await.map_err(upstream_failure)?;
    let mut answer = (status, bytes).into_response();
    if let Some(content_type) = content_type {
        answer
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type);
    }
    Ok(answer)
}

/// `com.atproto.repo.listRecords`.
pub async fn list_records(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let repo = query.params.text("repo").to_owned();
    let collection = query.params.text("collection").to_owned();
    let limit = query.params.limit("limit");
    let cursor = query.params.get("cursor").map(str::to_owned);
    let reverse = query.params.boolean("reverse");
    let rows = state
        .blocking(move |state| {
            let accounts = AccountDb::open(&state.layout().account_db)?;
            let actor = accounts
                .actor(&repo, Availability::ACTIVE)?
                .ok_or_else(|| {
                    XrpcError::invalid_request(format!("Could not find repo: {repo}"))
                })?;
            let store = ActorDb::open(&state.layout().actor_db(&actor.did))?;
            Ok(store.list_records(&collection, limit, reverse, cursor.as_deref())?)
        })
        .await?;
    let mut body = Map::new();
    let records = rows
        .iter()
        .map(|row| Ok(json!({ "uri": row.uri, "cid": row.cid, "value": record_value(row)? })))
        .collect::<Result<Vec<Value>, XrpcError>>()?;
    body.insert("records".to_owned(), records.into());
    if let Some(rkey) = rows.last().and_then(|row| row.uri.rsplit('/').next()) {
        body.insert("cursor".to_owned(), rkey.into());
    }
    Ok(json_response(StatusCode::OK, &body.into()))
}

/// `com.atproto.repo.describeRepo`.
pub async fn describe_repo(state: &AppState, query: &Query<'_>) -> Result<Response, XrpcError> {
    let repo = query.params.text("repo").to_owned();
    let actor = state
        .blocking(move |state| repo_availability(state, &repo, false))
        .await?;
    let did: Did = actor
        .did
        .parse()
        .map_err(|e| XrpcError::internal(format!("actor.did {}: {e}", actor.did)))?;
    let did_doc = state
        .resolver()
        .ensure_resolve(&did, false)
        .await
        .map_err(|error| XrpcError::invalid_request(format!("Could not resolve DID: {error}")))?;
    let handle_is_correct =
        claimed_handle(&did_doc) == actor.handle.as_deref() && actor.handle.is_some();
    let collections = state
        .blocking({
            let did = actor.did.clone();
            move |state| Ok(ActorDb::open(&state.layout().actor_db(&did))?.list_collections()?)
        })
        .await?;
    let body = json!({
        "handle": actor.handle.as_deref().unwrap_or(Handle::INVALID),
        "did": actor.did,
        "didDoc": did_doc,
        "collections": collections,
        "handleIsCorrect": handle_is_correct,
    });
    Ok(json_response(StatusCode::OK, &body))
}

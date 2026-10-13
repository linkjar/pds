//! `com.atproto.repo` writes (`docs/SPEC.md` §6.3, §6.4).
//!
//! Each handler follows its file under
//! `packages/pds/src/api/com/atproto/repo/` at the pin: the same checks in
//! the same order, with the same error names and messages.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt as _;
use pds_storage::{AccountDb, Actor, Availability};
use serde_json::{Map, Value, json};

use super::Query;
use crate::auth::session;
use crate::body;
use crate::error::{XrpcError, json_response};
use crate::params::Format;
use crate::state::{AppState, WriteState};
use crate::write::{
    BlobUse, Committed, Prepared, RecordWrite, Swap, prepare_record, record_uri, transact,
};

/// The most bytes of a JSON write (`jsonLimit` of the Reference's routes).
const JSON_LIMIT: usize = 1_000_000;
/// The most bytes of a JSON body where the route sets no limit: the
/// default of Express.
const DEFAULT_JSON_LIMIT: usize = 100 * 1024;

const CREATE: &str = "com.atproto.repo.applyWrites#create";
const UPDATE: &str = "com.atproto.repo.applyWrites#update";
const DELETE: &str = "com.atproto.repo.applyWrites#delete";

fn write_state(state: &AppState) -> Result<&WriteState, XrpcError> {
    state
        .writes()
        .ok_or_else(|| XrpcError::internal("the server is read-only"))
}

/// The account a request names, as `findAccount` of the Reference finds
/// it. This call blocks.
fn find_account(
    state: &AppState,
    handle_or_did: &str,
    check_takedown: bool,
    check_deactivated: bool,
) -> Result<Actor, XrpcError> {
    let accounts = AccountDb::open(&state.layout().account_db)?;
    let Some(actor) = accounts.actor(handle_or_did, Availability::ANY)? else {
        return Err(XrpcError {
            status: StatusCode::FORBIDDEN,
            error: "AccountNotFound".to_owned(),
            message: "Account not found".to_owned(),
        });
    };
    let unauthorized = |error: &str, message: &str| XrpcError {
        status: StatusCode::UNAUTHORIZED,
        error: error.to_owned(),
        message: message.to_owned(),
    };
    if check_takedown && actor.takedown_ref.is_some() {
        return Err(unauthorized(
            "AccountTakedown",
            "Account has been taken down",
        ));
    }
    if check_deactivated && actor.deactivated_at.is_some() {
        return Err(unauthorized("AccountDeactivated", "Account is deactivated"));
    }
    Ok(actor)
}

/// The repository a write names. It must be the caller's own, active and
/// not taken down. This call blocks.
fn own_repo(state: &AppState, repo: &str, caller: &str) -> Result<String, XrpcError> {
    let account = find_account(state, repo, true, true)?;
    if account.did != caller {
        return Err(XrpcError::auth_required("AuthenticationRequired"));
    }
    Ok(account.did)
}

fn commit_json(commit: &Committed) -> Value {
    json!({ "cid": commit.cid.to_string(), "rev": commit.rev })
}

fn written_json(write: &RecordWrite, commit: Option<&Committed>) -> Value {
    let mut out = Map::new();
    out.insert("uri".to_owned(), Value::String(write.uri.clone()));
    out.insert("cid".to_owned(), Value::String(write.cid.to_string()));
    if let Some(commit) = commit {
        out.insert("commit".to_owned(), commit_json(commit));
    }
    if let Some(status) = write.validation_status {
        out.insert(
            "validationStatus".to_owned(),
            Value::String(status.to_owned()),
        );
    }
    Value::Object(out)
}

/// `com.atproto.repo.createRecord`.
pub async fn create_record(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, XrpcError> {
    let caller = session(state, headers, None)?;
    let input = body::read_json(headers, body, JSON_LIMIT).await?;
    let repo = body::string(&input, "repo", true, Format::AtIdentifier, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let collection = body::string(&input, "collection", true, Format::Nsid, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let rkey =
        body::string(&input, "rkey", false, Format::RecordKey, Some(512), "$")?.map(str::to_owned);
    let validate = body::boolean(&input, "validate", "$")?;
    let record = body::record(&input, "record", "$")?.clone();
    let swap_commit = body::cid(&input, "swapCommit", "$")?;

    let (write, commit) = state
        .blocking(move |state| {
            let writes = write_state(state)?;
            let did = own_repo(state, &repo, &caller)?;
            let write = prepare_record(
                writes,
                &did,
                &collection,
                rkey.as_deref(),
                &record,
                validate,
                "$.record",
            )?;
            let commit = transact(state, &did, |session| {
                // A new follow, block, like or repost replaces the one the
                // account already has for the same subject.
                let mut batch = Vec::new();
                if validate != Some(false) {
                    for link in crate::write::backlinks(&write.uri, &write.data) {
                        let rkeys = session.txn().backlink_rkeys(
                            &write.collection,
                            link.path,
                            &link.link_to,
                        )?;
                        for rkey in rkeys {
                            batch.push(Prepared::Delete {
                                uri: record_uri(&did, &write.collection, &rkey),
                                collection: write.collection.clone(),
                                rkey,
                                swap: Swap::Unset,
                            });
                        }
                    }
                }
                batch.push(Prepared::Create(write.clone()));
                session.process_writes(&batch, swap_commit.as_ref())
            })?;
            Ok((write, commit))
        })
        .await?;
    Ok(json_response(
        StatusCode::OK,
        &written_json(&write, Some(&commit)),
    ))
}

/// `swapRecord` of `putRecord`: a CID, or `null` for "there is none".
fn swap_record(input: &Map<String, Value>) -> Result<Swap, XrpcError> {
    if input.get("swapRecord") == Some(&Value::Null) {
        return Ok(Swap::Absent);
    }
    Ok(body::cid(input, "swapRecord", "$")?.map_or(Swap::Unset, Swap::Cid))
}

/// `com.atproto.repo.putRecord`.
pub async fn put_record(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, XrpcError> {
    let caller = session(state, headers, None)?;
    let input = body::read_json(headers, body, JSON_LIMIT).await?;
    let repo = body::string(&input, "repo", true, Format::AtIdentifier, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let collection = body::string(&input, "collection", true, Format::Nsid, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let rkey = body::string(&input, "rkey", true, Format::RecordKey, Some(512), "$")?
        .unwrap_or_default()
        .to_owned();
    let validate = body::boolean(&input, "validate", "$")?;
    let record = body::record(&input, "record", "$")?.clone();
    let swap = swap_record(&input)?;
    let swap_commit = body::cid(&input, "swapCommit", "$")?;

    let (write, commit) = state
        .blocking(move |state| {
            let writes = write_state(state)?;
            let did = own_repo(state, &repo, &caller)?;
            let uri = record_uri(&did, &collection, &rkey);
            transact(state, &did, |session| {
                let current = session.txn().record_cid(&uri)?;
                let mut write = prepare_record(
                    writes,
                    &did,
                    &collection,
                    Some(&rkey),
                    &record,
                    validate,
                    "$.record",
                )?;
                write.swap = swap;
                // The same content makes no commit (SPEC §6.3).
                if current.as_deref() == Some(write.cid.to_string().as_str()) {
                    return Ok((write, None));
                }
                let prepared = if current.is_some() {
                    Prepared::Update(write.clone())
                } else {
                    Prepared::Create(write.clone())
                };
                let commit = session.process_writes(&[prepared], swap_commit.as_ref())?;
                Ok((write, Some(commit)))
            })
        })
        .await?;
    Ok(json_response(
        StatusCode::OK,
        &written_json(&write, commit.as_ref()),
    ))
}

/// `com.atproto.repo.deleteRecord`.
pub async fn delete_record(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, XrpcError> {
    let caller = session(state, headers, None)?;
    let input = body::read_json(headers, body, DEFAULT_JSON_LIMIT).await?;
    let repo = body::string(&input, "repo", true, Format::AtIdentifier, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let collection = body::string(&input, "collection", true, Format::Nsid, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let rkey = body::string(&input, "rkey", true, Format::RecordKey, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let swap = body::cid(&input, "swapRecord", "$")?.map_or(Swap::Unset, Swap::Cid);
    let swap_commit = body::cid(&input, "swapCommit", "$")?;

    let commit = state
        .blocking(move |state| {
            let did = own_repo(state, &repo, &caller)?;
            let uri = record_uri(&did, &collection, &rkey);
            transact(state, &did, |session| {
                // A record that is gone needs no commit.
                if session.txn().record_cid(&uri)?.is_none() {
                    return Ok(None);
                }
                let write = Prepared::Delete {
                    uri: uri.clone(),
                    collection,
                    rkey,
                    swap,
                };
                session
                    .process_writes(&[write], swap_commit.as_ref())
                    .map(Some)
            })
        })
        .await?;
    let mut out = Map::new();
    if let Some(commit) = &commit {
        out.insert("commit".to_owned(), commit_json(commit));
    }
    Ok(json_response(StatusCode::OK, &Value::Object(out)))
}

/// One entry of `writes` in `applyWrites`, checked and not yet prepared.
enum BatchWrite {
    Create {
        collection: String,
        rkey: Option<String>,
        value: Map<String, Value>,
    },
    Update {
        collection: String,
        rkey: String,
        value: Map<String, Value>,
    },
    Delete {
        collection: String,
        rkey: String,
    },
}

fn batch_write(entry: &Value, at: &str) -> Result<BatchWrite, XrpcError> {
    let refuse = |message: String| XrpcError::invalid_request(format!("{message} at {at}"));
    let Value::Object(fields) = entry else {
        return Err(refuse(pds_lexicon::wording::invalid_type(
            &["object"],
            &body::stringify(entry),
        )));
    };
    let tag = match fields.get("$type") {
        Some(Value::String(tag)) => tag.as_str(),
        _ => {
            return Err(refuse(pds_lexicon::wording::invalid_type(
                &["an object which includes the \"$type\" property"],
                &body::stringify(entry),
            )));
        }
    };
    let collection = |fields| -> Result<String, XrpcError> {
        Ok(
            body::string(fields, "collection", true, Format::Nsid, None, at)?
                .unwrap_or_default()
                .to_owned(),
        )
    };
    let rkey = |fields, required| -> Result<Option<String>, XrpcError> {
        Ok(
            body::string(fields, "rkey", required, Format::RecordKey, Some(512), at)?
                .map(str::to_owned),
        )
    };
    match tag {
        CREATE => Ok(BatchWrite::Create {
            collection: collection(fields)?,
            rkey: rkey(fields, false)?,
            value: body::record(fields, "value", at)?.clone(),
        }),
        UPDATE => Ok(BatchWrite::Update {
            collection: collection(fields)?,
            rkey: rkey(fields, true)?.unwrap_or_default(),
            value: body::record(fields, "value", at)?.clone(),
        }),
        DELETE => Ok(BatchWrite::Delete {
            collection: collection(fields)?,
            rkey: rkey(fields, true)?.unwrap_or_default(),
        }),
        other => Err(XrpcError::invalid_request(format!(
            "Expected one of \"{CREATE}\", \"{UPDATE}\" or \"{DELETE}\" (got {}) at {}",
            pds_lexicon::wording::stringify_str(other),
            body::child(at, "$type")
        ))),
    }
}

/// `com.atproto.repo.applyWrites`.
pub async fn apply_writes(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, XrpcError> {
    let caller = session(state, headers, None)?;
    let input = body::read_json(headers, body, JSON_LIMIT).await?;
    let repo = body::string(&input, "repo", true, Format::AtIdentifier, None, "$")?
        .unwrap_or_default()
        .to_owned();
    let validate = body::boolean(&input, "validate", "$")?;
    let entries = body::array(&input, "writes", "$")?;
    let mut batch = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        batch.push(batch_write(entry, &format!("$.writes[{index}]"))?);
    }
    let swap_commit = body::cid(&input, "swapCommit", "$")?;

    let (prepared, commit) = state
        .blocking(move |state| {
            let writes = write_state(state)?;
            let did = own_repo(state, &repo, &caller)?;
            if batch.len() > pds_repo::repo::MAX_WRITES {
                return Err(XrpcError::invalid_request("Too many writes. Max: 200"));
            }
            let mut prepared = Vec::with_capacity(batch.len());
            for (index, write) in batch.into_iter().enumerate() {
                let at = format!("$.writes[{index}].record");
                prepared.push(match write {
                    BatchWrite::Create {
                        collection,
                        rkey,
                        value,
                    } => Prepared::Create(prepare_record(
                        writes,
                        &did,
                        &collection,
                        rkey.as_deref(),
                        &value,
                        validate,
                        &at,
                    )?),
                    BatchWrite::Update {
                        collection,
                        rkey,
                        value,
                    } => Prepared::Update(prepare_record(
                        writes,
                        &did,
                        &collection,
                        Some(&rkey),
                        &value,
                        validate,
                        &at,
                    )?),
                    BatchWrite::Delete { collection, rkey } => Prepared::Delete {
                        uri: record_uri(&did, &collection, &rkey),
                        collection,
                        rkey,
                        swap: Swap::Unset,
                    },
                });
            }
            let commit = transact(state, &did, |session| {
                session.process_writes(&prepared, swap_commit.as_ref())
            })?;
            Ok((prepared, commit))
        })
        .await?;

    let results: Vec<Value> = prepared
        .iter()
        .map(|write| {
            let (tag, record) = match write {
                Prepared::Create(record) => ("com.atproto.repo.applyWrites#createResult", record),
                Prepared::Update(record) => ("com.atproto.repo.applyWrites#updateResult", record),
                Prepared::Delete { .. } => {
                    return json!({ "$type": "com.atproto.repo.applyWrites#deleteResult" });
                }
            };
            let mut out = Map::new();
            out.insert("$type".to_owned(), Value::String(tag.to_owned()));
            out.insert("cid".to_owned(), Value::String(record.cid.to_string()));
            out.insert("uri".to_owned(), Value::String(record.uri.clone()));
            if let Some(status) = record.validation_status {
                out.insert(
                    "validationStatus".to_owned(),
                    Value::String(status.to_owned()),
                );
            }
            Value::Object(out)
        })
        .collect();
    Ok(json_response(
        StatusCode::OK,
        &json!({ "commit": commit_json(&commit), "results": results }),
    ))
}

/// A key for a staged upload: 32 characters of base32, as the Reference
/// draws them.
fn staging_key() -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    (0..32)
        .map(|_| char::from(ALPHABET[usize::from(rand::random::<u8>() & 31)]))
        .collect()
}

/// `com.atproto.repo.uploadBlob`.
///
/// The body goes to the staging directory while it is hashed (SPEC §6.4).
/// The blob waits there until a record references it.
pub async fn upload_blob(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<Response, XrpcError> {
    let caller = session(state, headers, None)?;
    {
        let did = caller.clone();
        state
            .blocking(move |state| find_account(state, &did, true, false).map(|_| ()))
            .await?;
    }
    body::expect_body(headers)?;
    let declared = body::request_encoding(headers)?;
    let limit = state.config().blob_upload_limit;
    if body::content_length(headers)?.is_some_and(|length| length > limit) {
        return Err(body::payload_too_large());
    }
    let blobs = state
        .blobs()
        .ok_or_else(|| XrpcError::internal("no disk blob store is configured"))?
        .clone();

    // The file is written on the blocking pool; the request body feeds it
    // through a short queue, so a slow disk slows the client down.
    let (chunks, mut queue) = tokio::sync::mpsc::channel::<axum::body::Bytes>(8);
    let did = caller.clone();
    let key = staging_key();
    let writer = tokio::task::spawn_blocking(move || {
        let mut staging = blobs.stage(&did, &key).map_err(XrpcError::internal)?;
        while let Some(chunk) = queue.blocking_recv() {
            staging.write(&chunk).map_err(XrpcError::internal)?;
        }
        Ok::<_, XrpcError>(staging)
    });
    let mut stream = body.into_data_stream();
    let mut received = 0_u64;
    let mut failure = None;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            failure = Some(XrpcError::invalid_request("The request body ended early"));
            break;
        };
        received += chunk.len() as u64;
        if received > limit {
            failure = Some(body::payload_too_large());
            break;
        }
        if chunks.send(chunk).await.is_err() {
            break;
        }
    }
    drop(chunks);
    // Dropping the staging value removes the file.
    let staging = writer.await??;
    if let Some(failure) = failure {
        return Err(failure);
    }

    let did = caller;
    let blob = state
        .blocking(move |state| {
            let staged = staging.finish().map_err(XrpcError::internal)?;
            let cid = staged.cid.to_string();
            let mime_type = staged.sniffed.map_or(declared, str::to_owned);
            let size = i64::try_from(staged.size).map_err(XrpcError::internal)?;
            let now = write_state(state)?.sequencer().now();
            let kept = transact(state, &did, |session| {
                if session.txn().blob_is_taken_down(&cid)? {
                    return Err(XrpcError::invalid_request(
                        "Blob has been takendown, cannot re-upload",
                    ));
                }
                session
                    .txn()
                    .track_upload(&cid, &mime_type, size, &staged.key, &now)?;
                let stored = session.txn().stored_blob(&cid)?;
                let kept = stored
                    .as_ref()
                    .is_some_and(|stored| stored.temp_key.as_deref() == Some(&staged.key));
                // A record may already reference the blob: an import wrote
                // the reference before the bytes arrived.
                if session.txn().has_records_for_blob(&cid)? {
                    session.make_permanent(&BlobUse {
                        cid: cid.clone(),
                        mime_type: mime_type.clone(),
                        size,
                    })?;
                }
                Ok(kept)
            });
            let unused = !matches!(kept, Ok(true));
            if unused && let Some(blobs) = state.blobs() {
                // The store holds this blob already, or the upload was
                // refused: the staged copy is of no use.
                let _ = blobs.delete_staged(&did, &staged.key);
            }
            kept?;
            Ok(json!({
                "$type": "blob",
                "ref": { "$link": cid },
                "mimeType": mime_type,
                "size": size,
            }))
        })
        .await?;
    Ok(json_response(StatusCode::OK, &json!({ "blob": blob })))
}

/// `com.atproto.repo.listMissingBlobs`.
pub async fn list_missing_blobs(
    state: &AppState,
    query: &Query<'_>,
) -> Result<Response, XrpcError> {
    let did = session(state, query.headers, None)?;
    let limit = query.params.limit("limit");
    let cursor = query.params.get("cursor").map(str::to_owned);
    let blobs = state
        .blocking(move |state| {
            let store = pds_storage::ActorDb::open(&state.layout().actor_db(&did))?;
            Ok(store.list_missing_blobs(cursor.as_deref(), limit)?)
        })
        .await?;
    let mut out = Map::new();
    if let Some(last) = blobs.last() {
        out.insert("cursor".to_owned(), Value::String(last.cid.clone()));
    }
    out.insert(
        "blobs".to_owned(),
        blobs
            .iter()
            .map(|blob| json!({ "cid": blob.cid, "recordUri": blob.record_uri }))
            .collect(),
    );
    Ok(json_response(StatusCode::OK, &Value::Object(out)))
}

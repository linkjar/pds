//! The S3-compatible store against a server in this process that speaks
//! the part of the S3 API the store uses. The server checks no signature:
//! the parity Harness runs the store against a real one.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::significant_drop_tightening,
    clippy::cast_possible_truncation,
    clippy::format_push_string,
    // A failure must not print megabytes.
    clippy::manual_assert_eq,
    missing_docs
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse as _, Response};
use pds_blob::{BlobError, BlobStore, S3BlobStore, S3Config};

#[derive(Default)]
struct Fake {
    objects: BTreeMap<String, Vec<u8>>,
    uploads: BTreeMap<String, BTreeMap<u32, Vec<u8>>>,
    /// Answers 503 this many times before it serves.
    busy: u32,
    /// Every request, as `METHOD path?query`.
    seen: Vec<String>,
}

type Shared = Arc<Mutex<Fake>>;

fn xml(status: StatusCode, body: String) -> Response {
    (status, body).into_response()
}

fn no_such_key() -> Response {
    xml(
        StatusCode::NOT_FOUND,
        "<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message></Error>"
            .to_owned(),
    )
}

async fn serve(State(fake): State<Shared>, request: Request) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let body = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap();
    let query: BTreeMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let path = percent(uri.path());
    let key = path.strip_prefix("/blobs/").unwrap_or_default().to_owned();
    let mut fake = fake.lock().unwrap();
    fake.seen
        .push(format!("{method} {}", uri.path_and_query().unwrap()));
    assert!(headers.contains_key("authorization"));
    assert!(headers.contains_key("x-amz-content-sha256"));
    if fake.busy > 0 {
        fake.busy -= 1;
        return xml(StatusCode::SERVICE_UNAVAILABLE, String::new());
    }
    if method == Method::GET && path == "/blobs" {
        return list(&fake, &query);
    }
    match method {
        Method::POST if query.contains_key("uploads") => {
            let id = format!("upload-{}", fake.uploads.len() + 1);
            fake.uploads.insert(id.clone(), BTreeMap::new());
            xml(
                StatusCode::OK,
                format!(
                    "<InitiateMultipartUploadResult><UploadId>{id}</UploadId></InitiateMultipartUploadResult>"
                ),
            )
        }
        Method::POST => {
            let Some(parts) = fake.uploads.remove(&query["uploadId"]) else {
                return xml(StatusCode::NOT_FOUND, String::new());
            };
            let listed = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(listed.matches("<Part>").count(), parts.len());
            fake.objects
                .insert(key, parts.into_values().flatten().collect());
            xml(
                StatusCode::OK,
                "<CompleteMultipartUploadResult/>".to_owned(),
            )
        }
        Method::PUT if headers.contains_key("x-amz-copy-source") => {
            let source = percent(headers["x-amz-copy-source"].to_str().unwrap());
            let source = source.strip_prefix("blobs/").unwrap();
            match fake.objects.get(source).cloned() {
                Some(bytes) => {
                    fake.objects.insert(key, bytes);
                    xml(StatusCode::OK, "<CopyObjectResult/>".to_owned())
                }
                None => no_such_key(),
            }
        }
        Method::PUT if query.contains_key("partNumber") => {
            let number: u32 = query["partNumber"].parse().unwrap();
            fake.uploads
                .get_mut(&query["uploadId"])
                .unwrap()
                .insert(number, body.to_vec());
            ([("etag", format!("\"part-{number}\""))], "").into_response()
        }
        Method::PUT => {
            fake.objects.insert(key, body.to_vec());
            StatusCode::OK.into_response()
        }
        Method::DELETE if query.contains_key("uploadId") => {
            fake.uploads.remove(&query["uploadId"]);
            StatusCode::NO_CONTENT.into_response()
        }
        Method::DELETE => {
            fake.objects.remove(&key);
            StatusCode::NO_CONTENT.into_response()
        }
        Method::HEAD | Method::GET => match fake.objects.get(&key) {
            Some(bytes) => Bytes::from(bytes.clone()).into_response(),
            None => no_such_key(),
        },
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn percent(text: &str) -> String {
    url::form_urlencoded::parse(text.replace('+', "%2B").as_bytes())
        .map(|(name, _)| name.into_owned())
        .collect()
}

/// Two keys to a page, so that the store has to follow the token.
fn list(fake: &Fake, query: &BTreeMap<String, String>) -> Response {
    assert_eq!(query["list-type"], "2");
    let after = query.get("continuation-token").cloned().unwrap_or_default();
    let mut keys = fake
        .objects
        .keys()
        .filter(|key| key.starts_with(&query["prefix"]) && **key > after);
    let mut body = String::from("<ListBucketResult>");
    let mut last = None;
    for key in keys.by_ref().take(2) {
        body.push_str(&format!("<Contents><Key>{key}</Key></Contents>"));
        last = Some(key);
    }
    if let (Some(last), Some(_)) = (last, keys.next()) {
        body.push_str(&format!(
            "<NextContinuationToken>{last}</NextContinuationToken>"
        ));
    }
    body.push_str("</ListBucketResult>");
    xml(StatusCode::OK, body)
}

async fn start() -> (Shared, BlobStore) {
    let fake = Shared::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new().fallback(serve).with_state(Arc::clone(&fake));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = S3Config {
        bucket: "blobs".to_owned(),
        region: "auto".to_owned(),
        endpoint: Some(format!("http://127.0.0.1:{port}")),
        force_path_style: true,
        access_key_id: "key".to_owned(),
        secret_access_key: "secret".to_owned(),
        session_token: None,
        upload_timeout: Duration::from_secs(20),
        request_timeout: None,
    };
    let store = S3BlobStore::new(
        config,
        tokio::runtime::Handle::current(),
        Arc::new(|| 1_760_000_000_000),
    )
    .unwrap();
    (fake, store.into())
}

async fn blocking<T: Send + 'static>(call: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(call).await.unwrap()
}

const DID: &str = "did:plc:abc";

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_is_staged_moved_read_listed_and_deleted() {
    let (fake, store) = start().await;
    let shared = Arc::clone(&fake);
    blocking(move || {
        let mut staging = store.stage(DID, "key1").unwrap();
        staging.write(b"a text ").unwrap();
        staging.write(b"blob").unwrap();
        let staged = staging.finish().unwrap();
        let cid = staged.cid.to_string();
        assert_eq!(
            cid,
            "bafkreib5njcqaneu6gbwsmfbqeswb654woqznfim3pjbn7klh3alxjsrhm"
        );
        assert!(
            shared
                .lock()
                .unwrap()
                .objects
                .contains_key("tmp/did:plc:abc/key1")
        );
        assert!(!store.has(DID, &cid).unwrap());
        assert!(matches!(store.open(DID, &cid), Err(BlobError::NotFound)));

        store.make_permanent(DID, "key1", &cid).unwrap();
        {
            let fake = shared.lock().unwrap();
            let names: Vec<&String> = fake.objects.keys().collect();
            assert_eq!(names, [&format!("blocks/did:plc:abc/{cid}")]);
        }
        // A second call finds the blob in place.
        store.make_permanent(DID, "key1", &cid).unwrap();
        // A staged upload that never was is an error.
        assert!(store.make_permanent(DID, "none", "bafkreiother").is_err());

        let mut bytes = Vec::new();
        store
            .open(DID, &cid)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"a text blob");

        for name in ["a", "b", "c", "d"] {
            let mut staging = store.stage(DID, name).unwrap();
            staging.write(name.as_bytes()).unwrap();
            staging.finish().unwrap();
            store.make_permanent(DID, name, name).unwrap();
        }
        let stored = store.stored(DID).unwrap().unwrap();
        assert_eq!(stored.len(), 5);
        assert!(stored.contains(&cid) && stored.contains("d"));

        store.delete(DID, &cid).unwrap();
        store.delete(DID, &cid).unwrap();
        assert!(!store.has(DID, &cid).unwrap());
        store.delete_staged(DID, "gone").unwrap();
        assert!(store.stage("..", "key").is_err());
        assert!(store.has(DID, "a/b").is_err());
    })
    .await;
    // The path on the wire is encoded once.
    let fake = fake.lock().unwrap();
    assert_eq!(fake.seen[0], "PUT /blobs/tmp/did%3Aplc%3Aabc/key1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_large_upload_goes_in_parts_and_a_dropped_one_leaves_nothing() {
    let (fake, store) = start().await;
    let shared = Arc::clone(&fake);
    let part = pds_blob::s3::PART_LEN;
    blocking(move || {
        let bytes: Vec<u8> = (0..part * 2 + 5).map(|i| (i % 251) as u8).collect();
        let mut staging = store.stage(DID, "big").unwrap();
        for chunk in bytes.chunks(1024 * 1024 + 7) {
            staging.write(chunk).unwrap();
        }
        let staged = staging.finish().unwrap();
        assert_eq!(staged.size, bytes.len() as u64);
        {
            let fake = shared.lock().unwrap();
            assert!(fake.objects["tmp/did:plc:abc/big"] == bytes);
            assert!(fake.uploads.is_empty());
            let parts = fake
                .seen
                .iter()
                .filter(|line| line.contains("partNumber"))
                .count();
            assert_eq!(parts, 3);
        }

        let mut dropped = store.stage(DID, "dropped").unwrap();
        dropped.write(&bytes[..=part]).unwrap();
        assert_eq!(shared.lock().unwrap().uploads.len(), 1);
        drop(dropped);
    })
    .await;
    // The abort runs in a task of its own.
    for _ in 0..100 {
        if fake.lock().unwrap().uploads.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let fake = fake.lock().unwrap();
    assert!(fake.uploads.is_empty());
    assert!(!fake.objects.contains_key("tmp/did:plc:abc/dropped"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_store_is_asked_again() {
    let (fake, store) = start().await;
    fake.lock().unwrap().busy = 2;
    let shared = Arc::clone(&fake);
    blocking(move || {
        assert!(!store.has(DID, "bafkreinone").unwrap());
        assert_eq!(shared.lock().unwrap().seen.len(), 3);
        // Three answers of 503 are the answer.
        shared.lock().unwrap().busy = 3;
        assert!(matches!(
            store.has(DID, "bafkreinone"),
            Err(BlobError::Remote { .. })
        ));
    })
    .await;
}

/// The same calls against a store that checks signatures. It runs when
/// `PDS_BLOB_TEST_S3` holds `endpoint,bucket,region,key id,secret`.
#[tokio::test(flavor = "multi_thread")]
async fn a_store_that_checks_signatures_accepts_every_call() {
    let Ok(setting) = std::env::var("PDS_BLOB_TEST_S3") else {
        return;
    };
    let parts: Vec<&str> = setting.split(',').collect();
    let now = || {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        i64::try_from(elapsed.as_millis()).unwrap()
    };
    let config = S3Config {
        endpoint: Some(parts[0].to_owned()),
        bucket: parts[1].to_owned(),
        region: parts[2].to_owned(),
        force_path_style: true,
        access_key_id: parts[3].to_owned(),
        secret_access_key: parts[4].to_owned(),
        session_token: None,
        upload_timeout: Duration::from_secs(60),
        request_timeout: None,
    };
    let store: BlobStore =
        S3BlobStore::new(config, tokio::runtime::Handle::current(), Arc::new(now))
            .unwrap()
            .into();
    blocking(move || {
        let did = "did:plc:signed";
        let bytes: Vec<u8> = (0..pds_blob::s3::PART_LEN + 1234)
            .map(|i| (i % 249) as u8)
            .collect();
        for (key, body) in [("small", &b"a text blob"[..]), ("large", &bytes[..])] {
            let mut staging = store.stage(did, key).unwrap();
            staging.write(body).unwrap();
            let cid = staging.finish().unwrap().cid.to_string();
            assert!(!store.has(did, &cid).unwrap());
            store.make_permanent(did, key, &cid).unwrap();
            store.make_permanent(did, key, &cid).unwrap();
            assert!(store.has(did, &cid).unwrap());
            let mut read = Vec::new();
            store
                .open(did, &cid)
                .unwrap()
                .read_to_end(&mut read)
                .unwrap();
            assert!(read == body);
            assert!(store.stored(did).unwrap().unwrap().contains(&cid));
            store.delete(did, &cid).unwrap();
            store.delete(did, &cid).unwrap();
            assert!(matches!(store.open(did, &cid), Err(BlobError::NotFound)));
        }
        assert!(store.stored(did).unwrap().unwrap().is_empty());
        assert!(store.make_permanent(did, "none", "bafkreinone").is_err());
        store.delete_staged(did, "none").unwrap();
        let mut dropped = store.stage(did, "dropped").unwrap();
        dropped.write(&bytes).unwrap();
        drop(dropped);
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
}

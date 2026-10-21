//! Transfer admission and body deadlines through the HTTP boundary.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use common::{ALICE, DataDir, JWT_SECRET};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::json;
use sha2::Sha256;
use std::time::Duration;

fn token() -> String {
    let head = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"at+jwt"}"#);
    let body = URL_SAFE_NO_PAD.encode(json!({"scope":"com.atproto.access","sub":ALICE,"aud":"did:web:pds.test","iat":1,"exp":9_999_999_999_u64}).to_string());
    let mut mac = Hmac::<Sha256>::new_from_slice(JWT_SECRET.as_bytes()).unwrap();
    mac.update(format!("{head}.{body}").as_bytes());
    format!(
        "Bearer {head}.{body}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    )
}

async fn start(dir: &DataDir) -> (pds_xrpc::AppState, String, tokio::task::JoinHandle<()>) {
    let state = linkjar_pds::serve::app_state(&dir.config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = pds_xrpc::router(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (state, url, task)
}

#[tokio::test(flavor = "multi_thread")]
async fn exhausted_transfer_budgets_refuse_work_and_leave_health_available() {
    let dir = DataDir::new(1);
    let (state, url, task) = start(&dir).await;
    let http = reqwest::Client::new();
    let uploads: Vec<_> = (0..8)
        .map(|_| state.resources().upload().unwrap())
        .collect();
    let imports: Vec<_> = (0..2)
        .map(|_| state.resources().import().unwrap())
        .collect();
    let exports: Vec<_> = (0..8)
        .map(|_| state.resources().export().unwrap())
        .collect();
    let subscriptions: Vec<_> = (0..32)
        .map(|_| state.resources().subscription().unwrap())
        .collect();
    for (method, encoding) in [
        ("com.atproto.repo.uploadBlob", "image/png"),
        ("com.atproto.repo.importRepo", "application/vnd.ipld.car"),
    ] {
        let response = http
            .post(format!("{url}/xrpc/{method}"))
            .header("authorization", token())
            .header("content-type", encoding)
            .body(vec![0_u8])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 503, "{method}");
    }
    assert_eq!(
        http.get(format!("{url}/xrpc/com.atproto.sync.getRepo?did={ALICE}"))
            .send()
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(
        http.get(format!("{url}/xrpc/_health"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let error = tokio_tungstenite::connect_async(format!(
        "{}/xrpc/com.atproto.sync.subscribeRepos",
        url.replacen("http://", "ws://", 1)
    ))
    .await
    .unwrap_err();
    assert!(
        matches!(error, tokio_tungstenite::tungstenite::Error::Http(ref response) if response.status() == 503)
    );
    drop(subscriptions);
    drop((uploads, imports, exports));
    assert_eq!(
        http.get(format!("{url}/xrpc/com.atproto.sync.getRepo?did={ALICE}"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_upload_times_out_and_releases_its_writer() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = DataDir::new(1);
    let (state, url, task) = start(&dir).await;
    let mut socket = tokio::net::TcpStream::connect(url.trim_start_matches("http://"))
        .await
        .unwrap();
    socket.write_all(format!("POST /xrpc/com.atproto.repo.uploadBlob HTTP/1.1\r\nHost: pds.test\r\nAuthorization: {}\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\n\r\n", token()).as_bytes()).await.unwrap();
    let mut bytes = [0_u8; 1024];
    let result = tokio::time::timeout(Duration::from_secs(18), socket.read(&mut bytes)).await;
    assert!(
        result.is_ok(),
        "stalled upload retained its blocking writer past the idle deadline"
    );
    let read = result.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&bytes[..read]).starts_with("HTTP/1.1 408"));
    let _uploads: Vec<_> = (0..8)
        .map(|_| state.resources().upload().unwrap())
        .collect();
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_block_reads_preserve_the_export_order() {
    let dir = DataDir::new(5);
    let (_, url, task) = start(&dir).await;
    let db = dir.actor(ALICE);
    let cids: Vec<cid::Cid> = db
        .prepare("select cid from repo_block order by cid desc")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap().parse().unwrap())
        .collect();
    let store = pds_storage::ActorDb::open(&dir.config.layout.actor_db(ALICE)).unwrap();
    let mut expected = Vec::new();
    let mut car = pds_repo::CarWriter::new(&mut expected, None).unwrap();
    for (cid, content) in store.blocks(&cids).unwrap().found {
        car.block(&cid, &content).unwrap();
    }
    let mut query = vec![("did", ALICE.to_owned())];
    query.extend(cids.iter().map(|cid| ("cids", cid.to_string())));
    let mut endpoint = url::Url::parse(&format!("{url}/xrpc/com.atproto.sync.getBlocks")).unwrap();
    endpoint.query_pairs_mut().extend_pairs(query);
    let response = reqwest::Client::new().get(endpoint).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), expected);
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_audit_wait_still_records_the_completed_action() {
    let dir = DataDir::new(1);
    let state = linkjar_pds::serve::app_state(&dir.config).unwrap();
    state.set_audit(pds_xrpc::audit::AuditLog::open(&dir.config.layout.audit_db, None).unwrap());
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(state.resources().audit().await.unwrap());
    }
    let recording = {
        let state = state.clone();
        tokio::spawn(async move {
            pds_xrpc::audit::record(
                &state,
                pds_audit::Draft {
                    action: "completed".to_owned(),
                    ..pds_audit::Draft::default()
                },
            )
            .await;
        })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    recording.abort();
    let _ = recording.await;
    drop(held);
    let db = rusqlite::Connection::open(&dir.config.layout.audit_db).unwrap();
    let written = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let count: i64 = db
                .query_row("select count(*) from entry", [], |row| row.get(0))
                .unwrap();
            if count == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        written.is_ok(),
        "a completed action lost its audit entry when the caller disconnected"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_uploads_and_imports_stop_draining_after_five_seconds() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = DataDir::new_with(
        1,
        &[
            ("PDS_BLOB_UPLOAD_LIMIT", "64"),
            ("PDS_MAX_REPO_IMPORT_SIZE", "64"),
        ],
    );
    let (_, url, task) = start(&dir).await;
    let cases = [
        ("uploadBlob", "image/png"),
        ("importRepo", "application/vnd.ipld.car"),
    ];
    futures_util::future::join_all(cases.into_iter().map(|(method, encoding)| {
        let address = url.trim_start_matches("http://").to_owned();
        async move {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            let request = format!("POST /xrpc/com.atproto.repo.{method} HTTP/1.1\r\nHost: pds.test\r\nAuthorization: {}\r\nContent-Type: {encoding}\r\nTransfer-Encoding: chunked\r\n\r\n80\r\n{}\r\n", token(), "a".repeat(128));
            socket.write_all(request.as_bytes()).await.unwrap();
            let mut bytes = [0_u8; 1024];
            let result = tokio::time::timeout(Duration::from_secs(8), socket.read(&mut bytes)).await;
            assert!(result.is_ok(), "{method} kept draining a refused request");
            let read = result.unwrap().unwrap();
            assert!(String::from_utf8_lossy(&bytes[..read]).starts_with("HTTP/1.1 413"), "{method}");
        }
    })).await;
    task.abort();
}

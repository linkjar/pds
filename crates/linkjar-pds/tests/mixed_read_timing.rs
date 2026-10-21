//! Current-code characterization of large pages mixed with small reads,
//! and authenticated legacy sessions. This is not production throughput.
//!
//! Run on a quiet host, after compiling, with:
//! `cargo test --release --locked -p linkjar-pds --test mixed_read_timing -- --ignored --nocapture --test-threads=1`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // benchmark diagnostics
#![allow(clippy::print_stdout, clippy::cast_precision_loss)] // measurement output

mod common;

use common::{ADMIN_PASSWORD, ALICE, DataDir, PASSWORD, POST};
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LARGE_COLLECTION: &str = "com.example.performance.large";
const LARGE_RECORDS: usize = 50;
const PAYLOAD_BYTES: usize = 48 * 1024;
const WINDOW: Duration = Duration::from_secs(5);
const WARMUPS_PER_CLIENT: usize = 20;
// Bound stored samples even if a different host completes requests much faster.
const MAX_SAMPLES_PER_CLIENT: usize = 100_000;

#[derive(Clone)]
struct Endpoint {
    name: &'static str,
    url: String,
    token: Option<String>,
    response_bytes: usize,
}

impl Endpoint {
    async fn request(&self, http: &Client) -> reqwest::Response {
        let request = http.get(&self.url);
        let request = match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        };
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 200, "{}", self.name);
        response
    }

    async fn timed(&self, http: &Client) -> Duration {
        let started = Instant::now();
        let bytes = self.request(http).await.bytes().await.unwrap();
        assert_eq!(bytes.len(), self.response_bytes, "{}", self.name);
        started.elapsed()
    }

    async fn new(
        http: &Client,
        name: &'static str,
        url: String,
        token: Option<String>,
    ) -> (Self, Value) {
        let mut endpoint = Self {
            name,
            url,
            token,
            response_bytes: 0,
        };
        let response = endpoint.request(http).await;
        assert_eq!(
            response.headers()["content-type"],
            "application/json; charset=utf-8"
        );
        let bytes = response.bytes().await.unwrap();
        endpoint.response_bytes = bytes.len();
        (endpoint, common::http::json(&bytes))
    }
}

/// Set up through HTTP, outside measurement. Recovery gives the session a
/// persisted credential epoch, so each timed request checks a real marker.
async fn seed(http: &Client, origin: &str) -> String {
    let reset = common::http::post(
        http,
        origin,
        "com.atproto.admin.updateAccountPassword",
        None,
        Some(&json!({"did": ALICE, "password": PASSWORD})),
    )
    .basic_auth("admin", Some(ADMIN_PASSWORD))
    .send()
    .await
    .unwrap();
    let (status, body) = common::http::json_response(reset).await;
    assert_eq!(status, 200, "{body}");
    let login = common::http::post(
        http,
        origin,
        "com.atproto.server.createSession",
        None,
        Some(&json!({"identifier": "alice.pds.test", "password": PASSWORD})),
    )
    .send()
    .await
    .unwrap();
    let (status, login) = common::http::json_response(login).await;
    assert_eq!(status, 200);
    let token = login["accessJwt"].as_str().unwrap().to_owned();
    let payload = "x".repeat(PAYLOAD_BYTES);
    for first in (0..LARGE_RECORDS).step_by(10) {
        let writes: Vec<_> = (first..(first + 10).min(LARGE_RECORDS))
            .map(|index| {
                json!({
                    "$type": "com.atproto.repo.applyWrites#create",
                    "collection": LARGE_COLLECTION,
                    "rkey": format!("large{index:02}"),
                    "value": {"$type": LARGE_COLLECTION, "index": index, "text": payload}
                })
            })
            .collect();
        let response = common::http::post(
            http,
            origin,
            "com.atproto.repo.applyWrites",
            Some(&token),
            Some(&json!({"repo": ALICE, "validate": false, "writes": writes})),
        )
        .send()
        .await
        .unwrap();
        let (status, body) = common::http::json_response(response).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["results"].as_array().unwrap().len(), 10);
    }
    token
}

fn percentile(sorted: &[Duration], percent: usize) -> u128 {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)].as_micros()
}

async fn measure(http: &Client, name: &str, groups: &[(&Endpoint, usize)]) {
    let mut warmups = Vec::new();
    for &(endpoint, clients) in groups {
        for _ in 0..clients {
            warmups.push(async move {
                for _ in 0..WARMUPS_PER_CLIENT {
                    endpoint.timed(http).await;
                }
            });
        }
    }
    futures_util::future::join_all(warmups).await;

    let clients = groups.iter().map(|(_, count)| count).sum::<usize>();
    let barrier = Arc::new(tokio::sync::Barrier::new(clients + 1));
    let mut tasks = Vec::new();
    for (group, &(endpoint, clients)) in groups.iter().enumerate() {
        for _ in 0..clients {
            let (http, endpoint, barrier) = (http.clone(), endpoint.clone(), Arc::clone(&barrier));
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                let until = Instant::now() + WINDOW;
                let mut times = Vec::new();
                while Instant::now() < until && times.len() < MAX_SAMPLES_PER_CLIENT {
                    times.push(endpoint.timed(&http).await);
                }
                assert!(
                    times.len() < MAX_SAMPLES_PER_CLIENT,
                    "sample cap reached; shorten the window"
                );
                (group, times)
            }));
        }
    }
    let started = Instant::now();
    barrier.wait().await;
    let mut samples = vec![Vec::new(); groups.len()];
    for task in tasks {
        let (group, times) = task.await.unwrap();
        samples[group].extend(times);
    }
    let elapsed = started.elapsed();
    for ((endpoint, clients), mut times) in groups.iter().zip(samples) {
        times.sort_unstable();
        assert_ne!(times.len(), 0);
        println!(
            "| {name} | {} | {clients} | {} | {} | {} | {} | {:.1} | {} | {:.3} |",
            endpoint.name,
            percentile(&times, 50),
            percentile(&times, 95),
            percentile(&times, 99),
            times.len(),
            times.len() as f64 / elapsed.as_secs_f64(),
            times.len() * endpoint.response_bytes,
            elapsed.as_secs_f64(),
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "local release measurement; run on a quiet host"]
#[allow(clippy::assertions_on_constants)] // Compile in debug, but reject debug measurements.
async fn large_pages_small_reads_and_legacy_sessions() {
    // Compile with ordinary debug tests, but reject a debug measurement.
    assert!(
        !cfg!(debug_assertions),
        "run this measurement with --release"
    );
    let dir = DataDir::production(500);
    let listener = common::http::start(pds_xrpc::router(
        linkjar_pds::serve::app_state(&dir.config).unwrap(),
    ))
    .await;
    let http = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let origin = &listener.origin;
    let token = seed(&http, origin).await;
    let (small, record) = Endpoint::new(
        &http,
        "getRecord",
        format!(
            "{origin}/xrpc/com.atproto.repo.getRecord?repo={ALICE}&collection={POST}&rkey=r0000"
        ),
        None,
    )
    .await;
    assert_eq!(record["value"]["text"], "post 0");
    let (large, page) = Endpoint::new(&http, "listRecords 50 x 48KiB", format!("{origin}/xrpc/com.atproto.repo.listRecords?repo={ALICE}&collection={LARGE_COLLECTION}&limit=50"), None).await;
    let records = page["records"].as_array().unwrap();
    assert_eq!(records.len(), LARGE_RECORDS);
    assert!(
        records
            .iter()
            .all(|record| record["value"]["text"].as_str().unwrap().len() == PAYLOAD_BYTES)
    );
    let (session, account) = Endpoint::new(
        &http,
        "getSession legacy",
        format!("{origin}/xrpc/com.atproto.server.getSession"),
        Some(token),
    )
    .await;
    assert_eq!(account["did"], ALICE);
    println!(
        "\nCurrent-code local benchmark: production SQLite schema, 500 small records, {LARGE_RECORDS} distinct {PAYLOAD_BYTES}-byte payload records, four Tokio workers, {WARMUPS_PER_CLIENT} warmups/client, {}s/window.",
        WINDOW.as_secs()
    );
    println!(
        "Response bytes: getRecord={}, listRecords={}, getSession={}",
        small.response_bytes, large.response_bytes, session.response_bytes
    );
    println!(
        "| workload | endpoint | clients | p50 µs | p95 µs | p99 µs | completed | req/s | body bytes | elapsed s |"
    );
    println!("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    measure(&http, "small only", &[(&small, 4)]).await;
    measure(&http, "mixed", &[(&small, 4), (&large, 4)]).await;
    measure(&http, "authenticated only", &[(&session, 4)]).await;
}

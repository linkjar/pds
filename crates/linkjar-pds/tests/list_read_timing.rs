//! Focused local feedback for eight concurrent small-page list reads.
//! Build matched release binaries before timing; this is not a CI speed assertion.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // benchmark diagnostics
#![allow(clippy::print_stdout, clippy::cast_precision_loss)] // measurement output

mod common;

use common::{ALICE, DataDir, POST};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CLIENTS: usize = 8;
const WARMUPS: usize = 100;
const WINDOW: Duration = Duration::from_secs(5);
const MAX_SAMPLES: usize = 100_000;

async fn request(http: &Client, url: &str, expected_bytes: usize) -> Duration {
    let started = Instant::now();
    let response = http.get(url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.len(), expected_bytes);
    started.elapsed()
}

fn percentile(sorted: &[Duration], percent: usize) -> u128 {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)].as_micros()
}

async fn preflight(http: &Client, url: &str) -> usize {
    let response = http.get(url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    let bytes = response.bytes().await.unwrap();
    let page = common::http::json(&bytes);
    let records = page["records"].as_array().unwrap();
    assert_eq!(records.len(), 50);
    // The fixture's minimum-width keys sort lexically: r9999 precedes r19999.
    assert_eq!(page["cursor"], "r9950");
    for (record, index) in records.iter().zip((9950..10_000).rev()) {
        assert_eq!(record["uri"], format!("at://{ALICE}/{POST}/r{index}"));
        assert_eq!(record["value"]["text"], format!("post {index}"));
    }
    println!(
        "LIST_PREFLIGHT records=50 cursor=r9950 bytes={} sha256={} workers=10",
        bytes.len(),
        data_encoding::HEXLOWER.encode(&Sha256::digest(&bytes))
    );
    bytes.len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 10)]
#[ignore = "local release comparison; run on a quiet host"]
#[allow(clippy::assertions_on_constants)] // Compile in debug, but reject debug measurements.
async fn list_records_steady_timed() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let dir = DataDir::production(20_000);
    let listener = common::http::start(pds_xrpc::router(
        linkjar_pds::serve::app_state(&dir.config).unwrap(),
    ))
    .await;
    let url = Arc::new(format!(
        "{}/xrpc/com.atproto.repo.listRecords?repo={ALICE}&collection={POST}&limit=50",
        listener.origin
    ));
    let http = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let expected_bytes = preflight(&http, &url).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(CLIENTS + 1));
    let mut tasks = Vec::new();
    for _ in 0..CLIENTS {
        let (http, url, barrier) = (http.clone(), Arc::clone(&url), Arc::clone(&barrier));
        tasks.push(tokio::spawn(async move {
            for _ in 0..WARMUPS {
                request(&http, &url, expected_bytes).await;
            }
            let mut times = Vec::with_capacity(10_000);
            barrier.wait().await;
            let started = Instant::now();
            let until = started + WINDOW;
            while Instant::now() < until && times.len() < MAX_SAMPLES {
                times.push(request(&http, &url, expected_bytes).await);
            }
            let finished = Instant::now();
            assert!(times.len() < MAX_SAMPLES, "sample cap reached");
            (started, finished, times)
        }));
    }
    tokio::time::timeout(Duration::from_secs(60), barrier.wait())
        .await
        .expect("clients did not complete warmup");
    let mut starts = Vec::new();
    let mut completions = Vec::new();
    let mut times = Vec::new();
    for task in tasks {
        let (started, finished, samples) = task.await.unwrap();
        starts.push(started);
        completions.push(finished);
        times.extend(samples);
    }
    // Per-client timestamps exclude task creation, result merging, and sorting.
    let elapsed = *completions.iter().max().unwrap() - *starts.iter().min().unwrap();
    times.sort_unstable();
    assert_ne!(times.len(), 0);
    println!(
        "LIST_STEADY clients={CLIENTS} warmups={WARMUPS} p50_us={} p95_us={} p99_us={} requests={} req_per_s={:.3} body_bytes={} elapsed_s={:.6}",
        percentile(&times, 50),
        percentile(&times, 95),
        percentile(&times, 99),
        times.len(),
        times.len() as f64 / elapsed.as_secs_f64(),
        times.len() * expected_bytes,
        elapsed.as_secs_f64(),
    );
}

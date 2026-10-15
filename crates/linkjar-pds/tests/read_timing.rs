//! The cost of a read request over HTTP: `getRecord` and `listRecords` on
//! a store of 20,000 posts, one request after the other and eight at a
//! time. The numbers of the read-connection pool came from here.
//!
//! The test is ignored: it is a measurement, run by hand in release mode.
//!
//! ```sh
//! cargo test --release -p linkjar-pds --test read_timing -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code
#![allow(clippy::print_stdout, clippy::cast_precision_loss)] // a report

mod common;

use std::time::{Duration, Instant};

use common::{ALICE, DataDir, POST};
use linkjar_pds::serve::app_state;

/// How many requests one series sends.
const REQUESTS: usize = 2000;
/// How many clients send at once in the concurrent series.
const CLIENTS: usize = 8;

async fn start(dir: &DataDir) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = pds_xrpc::router(app_state(&dir.config).unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await });
    origin
}

fn percentile(sorted: &[Duration], hundredths: usize) -> Duration {
    let index = (sorted.len() * hundredths / 100).min(sorted.len() - 1);
    sorted[index]
}

fn micros(duration: Duration) -> u128 {
    duration.as_micros()
}

/// Sends `url` `count` times, one after the other, and gives the time of
/// each request.
async fn series(http: &reqwest::Client, url: &str, count: usize) -> Vec<Duration> {
    let mut times = Vec::with_capacity(count);
    for _ in 0..count {
        let started = Instant::now();
        let response = http.get(url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        drop(response.bytes().await.unwrap());
        times.push(started.elapsed());
    }
    times.sort_unstable();
    times
}

fn report(name: &str, times: &[Duration]) {
    let mean = times.iter().sum::<Duration>() / u32::try_from(times.len()).unwrap();
    println!(
        "| {name:<32} | {:>7} | {:>7} | {:>7} | {:>7} |",
        micros(percentile(times, 50)),
        micros(percentile(times, 90)),
        micros(percentile(times, 99)),
        micros(mean),
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, run by hand in release mode"]
async fn read_requests_timed() {
    let dir = DataDir::new(20_000);
    let origin = start(&dir).await;
    let http = reqwest::Client::new();
    let get_record = format!(
        "{origin}/xrpc/com.atproto.repo.getRecord?repo={ALICE}&collection={POST}&rkey=r12345"
    );
    let list_records = format!(
        "{origin}/xrpc/com.atproto.repo.listRecords?repo={ALICE}&collection={POST}&limit=50"
    );
    let describe_repo = format!("{origin}/xrpc/com.atproto.repo.describeRepo?repo={ALICE}");

    // Warm the server, the client's connection and the page cache.
    drop(series(&http, &get_record, 100).await);
    drop(series(&http, &list_records, 100).await);

    println!();
    println!("| request (µs)                     |     p50 |     p90 |     p99 |    mean |");
    println!("|----------------------------------|---------|---------|---------|---------|");
    report(
        "getRecord, sequential",
        &series(&http, &get_record, REQUESTS).await,
    );
    report(
        "listRecords 50, sequential",
        &series(&http, &list_records, REQUESTS).await,
    );
    report(
        "describeRepo, sequential",
        &series(&http, &describe_repo, REQUESTS).await,
    );

    for (name, url) in [
        ("getRecord", &get_record),
        ("listRecords 50", &list_records),
    ] {
        let started = Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..CLIENTS {
            let http = http.clone();
            let url = url.clone();
            tasks.push(tokio::spawn(async move {
                series(&http, &url, REQUESTS / CLIENTS).await
            }));
        }
        let mut times = Vec::with_capacity(REQUESTS);
        for task in tasks {
            times.extend(task.await.unwrap());
        }
        times.sort_unstable();
        let elapsed = started.elapsed();
        report(&format!("{name}, {CLIENTS} at a time"), &times);
        println!(
            "| {:<32} | {:>7} req/s                           |",
            format!("{name}, {CLIENTS} at a time: rate"),
            (REQUESTS as f64 / elapsed.as_secs_f64()).round()
        );
    }
    println!();
}

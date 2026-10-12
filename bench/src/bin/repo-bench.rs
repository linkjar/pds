//! S1 to S3 of SPEC §16.1 as micro-benchmarks of the repository engine.
//!
//!   cargo run --release --bin repo-bench -- <inhouse|rsky|rsky-main> [full|smoke] [out.json]
//!
//! The sequence is the harness's (`parity/scenarios/perf/01-repository`):
//! at each size, seed in batches of at most 200 until the repository has
//! that many records, time 100 single creates (S1), then time five batches
//! of 200 (S2). After the last size, export the repository (S3).

use std::time::{Duration, Instant};

use pds_bench::inhouse::InHouse;
use pds_bench::rsky::Rsky;
use pds_bench::{Engine, percentile};

#[global_allocator]
static ALLOC: pds_bench::alloc::Counting = pds_bench::alloc::Counting;

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut engine: Box<dyn Engine> = match args.first().map(String::as_str) {
        Some("inhouse") => Box::new(InHouse::new()),
        Some("rsky") => Box::new(Rsky::new()),
        Some("rsky-main") => Box::new(pds_bench::rsky_main::Rsky::new()),
        _ => {
            eprintln!("usage: repo-bench <inhouse|rsky|rsky-main> [full|smoke] [out.json]");
            std::process::exit(2);
        }
    };
    let full = args.get(1).is_none_or(|s| s == "full");
    let sizes: &[usize] = if full { &[10, 1_000, 10_000, 100_000] } else { &[10, 400, 2_000] };
    let samples = if full { 100 } else { 20 };
    let started = Instant::now();

    let mut s1 = Vec::new();
    let mut s2 = Vec::new();
    for &size in sizes {
        while engine.records() < size {
            let count = (size - engine.records()).min(200);
            engine.write(count);
        }
        let at = engine.records();
        let singles: Vec<_> = (0..samples).map(|_| engine.write(1)).collect();
        let times: Vec<Duration> = singles.iter().map(|s| s.elapsed).collect();
        let n = singles.len();
        s1.push(serde_json::json!({
            "size": size,
            "recordsAtStart": at,
            "p50Ms": ms(percentile(&times, 50.0)),
            "p99Ms": ms(percentile(&times, 99.0)),
            "eventCarBytesPerWrite": (singles.iter().map(|s| s.event_car_len).sum::<usize>() as f64 / n as f64).round(),
            "newBlocksPerWrite": singles.iter().map(|s| s.new_blocks).sum::<usize>() as f64 / n as f64,
            "removedBlocksPerWrite": singles.iter().map(|s| s.removed_blocks).sum::<usize>() as f64 / n as f64,
        }));
        let at = engine.records();
        let batches: Vec<_> = (0..5).map(|_| engine.write(200)).collect();
        let times: Vec<Duration> = batches.iter().map(|s| s.elapsed).collect();
        s2.push(serde_json::json!({
            "size": size,
            "recordsAtStart": at,
            "p50Ms": ms(percentile(&times, 50.0)),
            "maxMs": ms(times.iter().copied().max().unwrap_or_default()),
        }));
        eprintln!("{}: size {size} done at {:.1}s", engine.name(), started.elapsed().as_secs_f64());
    }

    // S3: three exports; the first one also pays for cold caches.
    let exports: Vec<_> = (0..3).map(|_| engine.export()).collect();
    let best = exports.iter().min_by_key(|e| e.elapsed).copied().unwrap_or(exports[0]);
    let report = serde_json::json!({
        "engine": engine.name(),
        "scale": if full { "full" } else { "smoke" },
        "S1": s1,
        "S2": s2,
        "S3": {
            "records": engine.records(),
            "wallMs": ms(best.elapsed),
            "wallMsAll": exports.iter().map(|e| ms(e.elapsed)).collect::<Vec<_>>(),
            "carBytes": best.car_len,
            "carMiB": (best.car_len as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0,
            "blocks": engine.stored_blocks(),
            "peakHeapMiB": (exports.iter().map(|e| e.peak_heap).max().unwrap_or(0) as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0,
        },
        "treeRoot": engine.tree_root(),
        "totalSeconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
    });
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    if let Some(path) = args.get(2) {
        if let Err(e) = std::fs::write(path, format!("{text}\n")) {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
    println!("{text}");
}

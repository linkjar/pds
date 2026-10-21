# HTTP read comparison

These historical runs used a reduced compatibility fixture. It omitted production indexes, including `record_collection_idx`. The comparison used the same fixture on both sides, but its absolute latency does not represent the production schema. In particular, the `describeRepo` result includes collection scans that production avoids. Use the [corrected production-fixture measurements](http-reads-production.md) for performance decisions.

Measured on 2026-10-19 using the unchanged `crates/linkjar-pds/tests/read_timing.rs` harness. The baseline is `77bf09e2e4e94079c38b86e47fb2d84e6ed36bbc`; the candidate is the completed `codex/security-import-hardening` working tree.

## Method

Both binaries were built with Rust 1.99.0 in release mode on the same Apple M1 Pro running macOS 27.0. Each process creates an isolated SQLite fixture with 20,000 posts and serves it on loopback. Setup and index creation are outside the request timing.

The harness warms `getRecord` and `listRecords` with 100 requests each. It then measures 2,000 requests per row below. Concurrent rows use eight clients, each sending 250 requests. It checks every HTTP status and reads each response completely.

Three pairs ran in this order: baseline/candidate, candidate/baseline, baseline/candidate. This task's compilations and tests had finished before measurement. Unrelated desktop activity was not controlled. All requests were anonymous, and no production service received benchmark traffic.

To reproduce each checkout's measurement:

```sh
cargo test --release --locked -p linkjar-pds --test read_timing -- --ignored --nocapture --test-threads=1
```

Use separate build directories for different checkouts, or clean their workspace crate artifacts before switching. Compare optimized binaries with the same harness and dependency lockfile.

## Results

Ranges contain all three runs. Latencies are microseconds.

| Request | Baseline p50 | Candidate p50 | Baseline p99 | Candidate p99 |
| --- | ---: | ---: | ---: | ---: |
| `getRecord`, sequential | 80–105 | 82–85 | 164–247 | 156–175 |
| `listRecords`, 50 records, sequential | 206–223 | 206–221 | 306–355 | 272–397 |
| `describeRepo`, sequential | 1,824–2,045 | 1,741–1,843 | 3,215–3,793 | 3,500–4,173 |
| `getRecord`, eight clients | 188–197 | 184–192 | 385–437 | 350–416 |
| `listRecords`, 50 records, eight clients | 1,403–1,544 | 1,424–1,551 | 2,078–2,801 | 2,198–2,587 |

Concurrent `getRecord` throughput was 36,216–38,608 requests/s before and 37,508–38,563 after. Concurrent `listRecords` throughput was 5,107–5,775 requests/s before and 5,182–5,630 after.

The measurements show comparable anonymous read performance on this reduced fixture. Tail latency varied, including a higher candidate `describeRepo` p99 range. Three short runs on an interactive host cannot establish small improvements or regressions reliably. These results do not establish a production collection-query bottleneck.

Raw output:

- Pair 1: [baseline](2026-10-19-http-reads-baseline-run-1.txt), [candidate](2026-10-19-http-reads-changed-run-1.txt).
- Pair 2: [baseline](2026-10-19-http-reads-baseline-run-2.txt), [candidate](2026-10-19-http-reads-changed-run-2.txt).
- Pair 3: [baseline](2026-10-19-http-reads-baseline-run-3.txt), [candidate](2026-10-19-http-reads-changed-run-3.txt).

## Limits

These runs do not measure production concurrency, cold caches, large exports, imports, blob uploads, or authenticated requests. Standalone legacy authentication now reads recovery state. Authenticated member requests add an uncached entryway status request; that cost and availability dependency require a separate deployment-specific measurement.

The [blob-reference index measurement](record-blob-index.md) isolates the intended SQL improvement and its disk/write costs.

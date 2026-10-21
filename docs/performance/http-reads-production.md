# HTTP reads with the production schema

The short concurrent-list results below are historical observations. A [longer controlled follow-up](small-page-followup.md) did not reproduce a consistent 2–8% slowdown. Use that follow-up for the current small-page throughput conclusion.

This comparison corrects the fixture used in the [historical HTTP runs](http-reads.md). Those runs omitted production record indexes. The corrected fixture calls the production schema constructors, including their indexes and SQLite settings. The independent Reference compatibility fixtures remain unchanged.

The baseline production source is commit `77bf09e2e4e94079c38b86e47fb2d84e6ed36bbc`, extracted into a temporary directory. The candidate is the recorded `codex/security-import-hardening` snapshot before the final `getSession` connection-pool change. That endpoint is outside this anonymous workload. The candidate includes both the security fixes and later performance changes; this comparison does not isolate any single change. Only the corrected `tests/common/mod.rs`, `tests/common/http.rs`, and `tests/read_timing.rs` were overlaid onto the baseline. Those three files are byte-identical between builds.

Each build uses its own production schema constructor. Both include the record collection, CID, and revision indexes absent from the historical fixture. The candidate also includes the new blob-reference index and recovery schema. Its lockfile updates `rustls-webpki` from 0.103.15 to 0.103.16 and `webpki-root-certs`/`webpki-roots` from 1.0.9 to 1.0.10. The baseline retains its original lockfile.

## Method

Each process creates a fresh production SQLite fixture containing 20,000 posts and serves it on loopback. Fixture construction and index creation are outside the request timing. A regression checks that collection lookup uses the production covering index and the store uses WAL.

The harness warms `getRecord` and `listRecords` with 100 requests each. Each reported row measures 2,000 requests. Concurrent rows use eight clients, each sending 250 requests. Every response must succeed and its body must be fully read. Requests are anonymous.

Three pairs run in this order: baseline/candidate, candidate/baseline, baseline/candidate. Optimized test executables are built before timing and then run directly. No production service receives benchmark traffic.

The runs used Rust 1.99.0 on an Apple M1 Pro running macOS 27.0. All task-owned builds, tests, and other measurements had stopped. Unrelated desktop work continued: IntelliJ and Syncthing consumed CPU in the [host snapshots](2026-10-21-benchmark-host-load.json). [Provenance](2026-10-21-benchmark-provenance.json) records the source snapshot, fixture hashes, differing lockfile hashes, and exact HTTP executables.

To reproduce the corrected candidate measurement:

```sh
cargo test --release --locked -p linkjar-pds --test read_timing -- --ignored --nocapture --test-threads=1
```

Use separate build directories for different source trees. Keep the fixture, harness, optimization settings, and host conditions consistent. Record dependency-lock differences when the candidate includes dependency updates.

## Results, 2026-10-21

Ranges contain all three runs. Latencies are microseconds.

| Request | Baseline p50 | Candidate p50 | Baseline p99 | Candidate p99 |
| --- | ---: | ---: | ---: | ---: |
| `getRecord`, sequential | 69–75 | 67–76 | 125–173 | 117–197 |
| `listRecords`, 50 records, sequential | 197–204 | 200–203 | 228–618 | 285–358 |
| `describeRepo`, sequential | 93 | 81–92 | 125–149 | 147–192 |
| `getRecord`, eight clients | 170–174 | 168–185 | 321–347 | 326–511 |
| `listRecords`, 50 records, eight clients | 1,364–1,393 | 1,400–1,449 | 2,001–2,254 | 2,386–2,587 |

Concurrent `getRecord` throughput was 39,200–40,280 requests/s before and 36,100–41,014 after. Concurrent `listRecords` throughput was 5,757–5,892 before and 5,420–5,646 after, 2.4–8.0% lower in these three paired runs. Each concurrent-list sample lasted only about 0.35 seconds. These observations do not demonstrate a general HTTP speedup or establish a recurring regression. The candidate's list requests were slower in these samples, including their tail latency; the [longer follow-up](small-page-followup.md) did not reproduce a consistent throughput drop and does not attribute the historical difference to a specific code change.

The corrected `describeRepo` medians are 81–93 µs across both versions. The old fixture's millisecond timings were not evidence of a missing production collection index. They should not be compared with these runs as a production-code improvement.

Raw output:

- Pair 1: [baseline](2026-10-21-http-reads-baseline-run-1.txt), [candidate](2026-10-21-http-reads-changed-run-1.txt).
- Pair 2: [baseline](2026-10-21-http-reads-baseline-run-2.txt), [candidate](2026-10-21-http-reads-changed-run-2.txt).
- Pair 3: [baseline](2026-10-21-http-reads-baseline-run-3.txt), [candidate](2026-10-21-http-reads-changed-run-3.txt).

## Limits

These short local measurements cannot establish small improvements or regressions reliably. They do not measure production concurrency, cold caches, large exports, imports, blob uploads, or authenticated requests. They do not reproduce mixed small and large responses on a constrained runtime; that needs a separate workload.

Standalone legacy authentication checks live recovery state. Authenticated member requests also contact the entryway. Those costs and their availability dependencies are outside this anonymous-read comparison.

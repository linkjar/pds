# Large pages, small reads, and legacy sessions

This compares baseline `77bf09e2e4e94079c38b86e47fb2d84e6ed36bbc` with the reviewed working tree. The harness and fixture helpers are byte-identical. Results measure the cumulative patch, including security checks and dependency updates. They do not isolate the response-encoding change or estimate production throughput.

## Results

Measured on 2026-10-21 with an Apple M1 Pro, 16 GiB memory, macOS 27.0, and Rust 1.99.0. Each version ran three times. The table shows the median of each run's reported statistic; it does not pool samples across runs.

| Workload | Version | p50 µs | p95 µs | p99 µs | Requests/s |
| --- | --- | ---: | ---: | ---: | ---: |
| Small `getRecord`, four clients | Baseline | 124 | 176 | 206 | 31,375.5 |
| Small `getRecord`, four clients | Final | 123 | 175 | 200 | 31,579.1 |
| Small `getRecord` during large reads | Baseline | 199 | 707 | 2,769 | 13,347.9 |
| Small `getRecord` during large reads | Final | 176 | 284 | 384 | 21,526.5 |
| Large `listRecords` during small reads | Baseline | 6,667 | 7,875 | 8,285 | 627.9 |
| Large `listRecords` during small reads | Final | 5,048 | 7,056 | 7,767 | 771.2 |
| Legacy `getSession`, four clients | Baseline | 1,395 | 2,155 | 2,604 | 2,753.8 |
| Legacy `getSession`, four clients | Final | 155 | 211 | 239 | 25,243.2 |

During large reads, small-request p99 ranged from 2,725–2,814 µs in the baseline and 382–410 µs in the final version. Small-request throughput ranged from 13,072.5–13,466.5 to 20,828.4–21,631.0 requests/s. Small-only throughput ranges overlapped; these trials do not show a clear improvement for that workload.

A preserved candidate control measured the code before the final one-line `getSession` change from `layout().accounts()` to `reads().accounts()`. That change reuses the existing connection pool for the account lookup. Its per-request credential-epoch checks remain live. Control p50 was 2,040–2,130 µs, versus 155 µs in all three final trials. Control p99 was 2,759–2,905 µs, versus 239–240 µs afterward. Throughput rose from 1,893.9–1,975.7 to 25,238.8–25,308.9 requests/s in this workload. The older baseline has no credential-epoch check, so baseline session timings are not an equivalent security workload.

All nine benchmark processes passed. Complete responses were 234 bytes for `getRecord`, 2,468,872 bytes for the large page, and 132 bytes for `getSession` in all versions.

## Method

The ignored `crates/linkjar-pds/tests/mixed_read_timing.rs` harness creates an isolated SQLite directory using each version's production schema and indexes. It starts a loopback HTTP listener in the same process as its clients, with four Tokio async workers. Database work uses Tokio's separate blocking pool; the final version also encodes repository responses there.

The fixture contains 500 small posts. Through HTTP, setup resets the account password, signs in, and creates 50 distinct records containing 48 KiB text payloads. The records use a benchmark-specific collection with lexicon validation disabled. Their repository encoding, indexes, and commits follow the normal write path. Ten records fit in each setup request; total large-record payload is 2.34 MiB. Setup is outside measurement.

Each workload warms every client with 20 requests, then runs for five seconds:

- Four clients read one small record.
- Four clients read that record while four other clients request pages of all 50 large records.
- Four clients call `getSession` with a legacy token obtained after password recovery. In both candidate versions, every request checks a persisted recovery marker; no revocation-value cache is enabled.

Every request must return HTTP 200 and the expected complete response length. The initial responses are decoded and checked against the fixture. Timings cover request construction through complete body consumption. Requests that start within the window may finish afterward; rates use actual elapsed time, including that final drain. Stored samples are capped at 100,000 per client; reaching the cap fails the run.

Build before measuring, then stop other builds and benchmarks:

```sh
cargo test --release --locked -p linkjar-pds --test mixed_read_timing --no-run
```

Run the built test binary directly with `--ignored --nocapture --test-threads=1`. Its output includes p50/p95/p99 latency, completed requests, complete body bytes, request rate, and elapsed time for each endpoint. To reproduce the baseline, overlay the same harness plus `tests/common/mod.rs` and `tests/common/http.rs` into an archive of the baseline commit. Keep its original dependency lockfile. The [source and executable provenance](2026-10-21-mixed-reads-provenance.json) identifies that archive and the preserved control; [final provenance](2026-10-21-mixed-reads-final-provenance.json) records the final source aggregate, lockfile, helper hashes, and executable hash. The control executable has an exact retained hash, but its production source aggregate was not captured before the one-line pool edit.

## Limits and evidence

The client and server share CPU and memory bandwidth. Large pages are intentionally expensive, and the custom record size is not typical of a short social post. Runs use warm connections and data, a fixed workload order, SQLite, standalone HS256 authentication, and the fixture's disabled rate limits. They exclude member-host introspection, PostgreSQL, production traffic, concurrent writes, and network latency beyond loopback.

Version groups were not alternated: three control runs came first, then three baseline runs, then three final runs after rebuilding. No project builds or other benchmarks ran during measurement. The final runs also followed shutdown of the temporary PostgreSQL test server. Unrelated desktop processes remained active. Before/after snapshots recorded Syncthing up to 74.6% CPU and indexing processes around 33%; they do not sample the entire interval. These short runs cannot distinguish small effects from host noise or establish sustained production capacity.

Raw runs retain all endpoint statistics and exit results:

- Baseline: [run 1](2026-10-21-mixed-reads-baseline-run-1.txt), [run 2](2026-10-21-mixed-reads-baseline-run-2.txt), [run 3](2026-10-21-mixed-reads-baseline-run-3.txt), [host snapshots](2026-10-21-mixed-reads-baseline-host-load.json).
- Candidate before the pool change: [run 1](2026-10-21-mixed-reads-current-run-1.txt), [run 2](2026-10-21-mixed-reads-current-run-2.txt), [run 3](2026-10-21-mixed-reads-current-run-3.txt), [host snapshots](2026-10-21-mixed-reads-host-load.json).
- Final candidate: [run 1](2026-10-21-mixed-reads-final-run-1.txt), [run 2](2026-10-21-mixed-reads-final-run-2.txt), [run 3](2026-10-21-mixed-reads-final-run-3.txt), [host snapshots](2026-10-21-mixed-reads-final-host-load.json).

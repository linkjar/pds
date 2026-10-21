# Bounded blob-reference checks

Blob cleanup now asks whether each distinct candidate CID still has a record reference. It returns at most one CID per distinct candidate. The previous query returned every matching reference, so one shared blob could produce a large duplicate result. Cleanup also uses a hash set when it checks which removed blobs must remain.

Fixed statements used repeatedly within a repository transaction now use the connection's prepared-statement cache. This includes record-CID lookup, stored-blob lookup, record deletion, and backlink changes. SQL behavior and transaction boundaries remain unchanged.

## Reproduce

```sh
cargo build --locked --release -p pds-storage --example blob_references
target/release/examples/blob_references
```

If `CARGO_TARGET_DIR` is set, use that directory for the executable. The example accepts no database path. It creates a temporary production actor store and removes it when finished.

The fixture contains 100,000 record references to one blob and 500 references to distinct blobs. Both versions use identically seeded databases and existing production indexes. The previous implementation is reproduced with its 500-candidate `IN` query. The new side calls the actual `ActorTxn::referenced_blobs` method. Each trial checks the returned row count. The final database must still contain 100,500 references.

Each case has 20 warmup trials and 201 measured trials. Each case and variant runs in a fresh child process with its own database. The raw comparison connection matches the production writer's open flags, busy timeout, and statement-cache capacity. Transaction begin and rollback are outside timing; SQL preparation, execution, and result collection are inside timing. The record-lookup case compares 200 uncached queries with 200 calls to the cached production method. Both validate the same CID. These measurements exclude commit and fsync costs.

## Results, 2026-10-21

Three separate release runs used Rust 1.99.0 and bundled SQLite 3.53.2 on an Apple M1 Pro running macOS 27.0. This task's other builds, tests, and measurements were stopped. Unrelated desktop activity continued; [host snapshots](2026-10-21-blob-references-isolated-host-load.json) record that limitation.

Ranges contain all three runs. Each percentile comes from 201 measured trials. All times are microseconds.

| Operation | Before p50 | After p50 | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| One blob shared by 100,000 records | 9,955.375–10,043.292 | 0.542 | 10,857.625–11,527.958 | 0.583 |
| 500 distinct referenced blobs | 328.958–334.792 | 189.583–196.125 | 333.958–348.208 | 192.792–202.625 |
| 500 missing blob candidates | 193.917–204.291 | 158.875–167.667 | 206.291–211.792 | 163.167–174.125 |
| 200 record-CID lookups | 232.875–245.709 | 61.750–63.791 | 237.791–255.958 | 62.333–64.375 |

The shared-blob query returns one CID instead of 100,000 duplicate CIDs. Ordinary distinct and missing candidates also improved in these isolated trials. The cached-lookup result supports reusing that statement; it does not quantify every newly cached write statement.

Raw outputs: [run 1](2026-10-21-blob-references-isolated-run-1.txt), [run 2](2026-10-21-blob-references-isolated-run-2.txt), [run 3](2026-10-21-blob-references-isolated-run-3.txt). [Source and executable hashes](2026-10-21-blob-references-isolated-provenance.json) identify this final benchmark. The production implementation did not change during the benchmark corrections.

## Limits

This measures SQL and result collection on synthetic data with warm caches. It does not measure total write-request latency, physical blob removal, repository-tree updates, concurrent writers, or production throughput. The high-fanout case intentionally exposes duplicate-result amplification. The distinct and missing cases show the cost when amplification is absent.

The regression in `crates/pds-storage/tests/blob_references.rs` removes a replaced record's references while 10,000 other records still use its shared blob. Repeated candidate CIDs return one retained blob; removed orphan and absent CIDs return none. The HTTP write/import regressions also verify shared-blob retention and last-reference cleanup.

## Superseded exploratory comparison

The first three runs alternated old and new queries inside one process. After the old fanout query collected and freed millions of duplicate strings, later ordinary-case timings changed substantially. Removing only that preceding fanout comparison changed the new distinct-500 median from about 1,339 µs to 192 µs and cached record lookup from 302 µs to 61 µs. The production method, data, and optimization settings stayed the same. Reopening the comparison connection did not fix the discrepancy.

This establishes process-state contamination in the original comparison. Allocator state after the duplicate allocations is the likely cause; it was not instrumented directly. Separate controls using the same connection, and the exact production library with fat LTO, also showed cached lookup improving. The final measurements isolate every case and variant in a fresh process.

The [control outputs](2026-10-21-blob-references-controls.txt) retain both checks.

The initial outputs are retained for traceability and must not be used as the final performance comparison: [run 1](2026-10-21-blob-references-run-1.txt), [run 2](2026-10-21-blob-references-run-2.txt), [run 3](2026-10-21-blob-references-run-3.txt).

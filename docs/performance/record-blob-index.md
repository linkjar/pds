# Blob-reference deletion index

The `record_blob_record_uri_idx` index changes deletion by record URI from a table scan to an indexed lookup. The existing primary key starts with `blobCid`, so it does not support this lookup efficiently.

New actor stores create the index. Existing stores add it on their first writable open. Read-only opens leave the schema unchanged. The Reference's tables and `kysely_migration` ledger remain unchanged. Creating the index takes a SQLite write lock and needs additional disk space.

## Reproduce

Run from the repository root:

```sh
cargo build --locked --release -p pds-storage --example record_blob_index
target/release/examples/record_blob_index
```

If `CARGO_TARGET_DIR` is set, use that directory for the executable. The example accepts no database path. It creates a temporary database, removes that database when finished, and never opens application data.

The example uses this workspace's bundled SQLite and the Reference actor schema. It inserts 100,000 blob references, each with a distinct blob CID and record URI. It compares the same database before and after the production `ActorDb::open_writable` upgrade adds the index.

Each operation has 20 warmup trials and 201 measured trials. Every trial begins a transaction and rolls it back. The timed deletion prepares the production SQL, executes `DELETE ... WHERE recordUri IN (?) RETURNING blobCid`, and collects the returned CID. Exactly one row must match. Transaction begin and rollback are outside the timing. A second measurement inserts 100 new references with the production `associate_blob` statement to show index maintenance cost. Final row count must remain 100,000.

## Results, 2026-10-19

Three separate release runs on an Apple M1 Pro, macOS 27.0, Rust 1.99.0, bundled SQLite 3.53.2. Compilation was paused during measurement. SQLite used WAL, `cache_size=-2000`, and `synchronous=2`, with no benchmark-specific tuning.

Ranges below contain all three runs. Each percentile comes from that run's 201 trials.

| Operation | Before p50, µs | After p50, µs | Before p95, µs | After p95, µs |
| --- | ---: | ---: | ---: | ---: |
| Delete one record's blob reference | 6,777.042–7,158.041 | 6.458–6.542 | 8,017.958–8,678.125 | 6.625–6.875 |
| Insert 100 blob references | 141.792–146.917 | 180.750–183.500 | 149.500–171.959 | 196.875–228.000 |

The production writable-open operation, including index creation, commit, and connection close, took 65.535–72.945 ms. Database pages grew from 28,966,912 to 36,294,656 bytes: 7,327,744 additional bytes, about 7.0 MiB. These sizes exclude transient WAL and shared-memory files. The insert measurement's median increased by about 25–28% across the paired runs.

Before:

```text
SCAN record_blob
```

After:

```text
SEARCH record_blob USING COVERING INDEX record_blob_record_uri_idx (recordUri=?)
```

Raw outputs: [run 1](2026-10-19-record-blob-index-run-1.txt), [run 2](2026-10-19-record-blob-index-run-2.txt), [run 3](2026-10-19-record-blob-index-run-3.txt).

## Limits

This measures one SQL operation on synthetic data with warm filesystem caches. It does not measure total HTTP latency, import latency, production throughput, transaction fsync cost, concurrent writers, or larger repositories. Actual blob fanout, URI lengths, storage, and cache pressure affect the result. Index construction can take longer and use more space on larger stores.

The regression tests in `crates/pds-storage/tests/indexes.rs` check the new-store query plan and an old store's writable upgrade. They also check unchanged migration rows, read-only behavior, repeated writable opens, and deletion results.

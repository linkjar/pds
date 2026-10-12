# Repository engine comparison

[Decision D2](../docs/research/2026-10-12-repository-crate.md) · [SPEC §16](../docs/SPEC.md#16-performance) · [Reference baseline](../parity/results/perf-reference.md)

This package holds the evidence for decision D2: the repository engine in
[`crates/pds-repo`](../crates/pds-repo) beside the `rsky-repo` crate, on the
same vectors and the same workload. It is outside the Cargo workspace, like
`fuzz/`, so that `rsky-repo` and its dependencies stay out of the server's
lockfile, its `cargo deny` policy and its `cargo vet` set. CI does not build
it.

`rsky-repo` is here twice: version 0.0.6 as published on crates.io, and
version 0.1.4 from the main branch of its repository at commit `7964ffa3`
(2026-10-10), which is not published.

## Vectors

```sh
cargo test --release --manifest-path bench/Cargo.toml -- --nocapture
```

[`tests/vectors.rs`](tests/vectors.rs) runs each group of vectors against
all three and prints one line per group. The in-house engine must pass.
What each `rsky-repo` version did on 2026-10-12 is written into the test as
constants, so the test fails when either side changes.

| Group | Source |
|---|---|
| MST key heights and common prefix | `interop/mst/` |
| Firehose commit proofs | `interop/firehose/commit-proof-fixtures.json` |
| Crypto signature fixtures | `interop/crypto/signature-fixtures.json` |
| Data model fixtures | `interop/data-model/data-model-fixtures.json` |
| Tree roots, commits, export, import | `crates/pds-repo/tests/fixtures/`, written by `@atproto/repo` at the pin |
| Trees that reference one node more than once | The same fixtures: the shapes of upstream's `mst-dag.test.ts` |
| A commit signed with each curve | Built in the test |

## Micro-benchmarks for S1 to S3

```sh
cargo build --release --manifest-path bench/Cargo.toml
bench/target/release/repo-bench inhouse   full out.json
bench/target/release/repo-bench rsky      full out.json   # 0.0.6
bench/target/release/repo-bench rsky-main full out.json   # 0.1.4
( cd parity && node src/tools/repo-micro.ts full out.json )   # @atproto/repo at the pin
```

`smoke` in place of `full` runs the same code at a fraction of the size.

**The workload is the harness's.** Collection `com.example.parity.note`,
record keys `r0000000` upward, the body `{$type, n, text}` of
[`parity/scenarios/perf/01-repository`](../parity/scenarios/perf/01-repository.scenario.ts),
written in the same order: for each size of 10, 1,000, 10,000 and 100,000
records, seed in batches of at most 200, time 100 single creates (S1), then
time five batches of 200 (S2). After the last size, export the 101,100
records (S3). The four programs therefore build the same repository, and
the tree root they print must be the same.

**One sample is the engine's share of one write.** It starts with parsed
and validated record values and ends with the commit applied to a block
store in memory:

1. encode each record as DAG-CBOR and take its CID;
2. open the repository at its head commit;
3. change the tree;
4. find the blocks that are new and the stored blocks no longer used;
5. collect the proof blocks for the firehose event;
6. sign the commit with a secp256k1 key;
7. write the CAR of the event's blocks;
8. put and delete blocks in the store.

A sample leaves out everything else a `createRecord` does: HTTP, the
access token, lexicon validation, SQLite, the sequencer and the firehose.
The Reference's numbers in `parity/results/perf-reference.md` include all
of that, so they bound these from above and are not the same quantity.

**S3** exports into a sink that counts bytes and drops them, as a socket
would take them. The export is run three times and the fastest is
reported. The two Rust programs also report the peak of the heap during
the export above the heap before it, from a counting allocator; the
repository itself, about 15 MiB of blocks in the memory store, is not in
that number.

**Signing differs by library**: `k256` for the in-house engine,
`libsecp256k1` (C) for `rsky-repo`, `@noble/curves` for the Reference. All
three sign deterministically (RFC 6979).

**What the numbers do not show.** A memory block store answers in tens of
nanoseconds. Over SQLite each block read is a query, so the number of
blocks an engine reads per write matters more there than here.
`rsky-repo` is asynchronous; here each of its operations runs to
completion on a current-thread tokio runtime.

The published numbers, with the host and the versions, are in the
[decision record](../docs/research/2026-10-12-repository-crate.md#5-micro-benchmarks-for-s1-to-s3).

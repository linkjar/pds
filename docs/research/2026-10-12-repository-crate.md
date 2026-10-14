# The repository crate: the decision on the repository engine

Research record, 2026-10-12. Companion to [SPEC.md](../SPEC.md) §3.1, §6,
§16 and §17.2, and to the decision on the repository engine in Appendix D.
It records the decision of unit 2,
the evidence for it, and what the comparison found on the way.

**Decision.** The repository engine is written in this repository, in
[`crates/pds-repo`](../../crates/pds-repo). The server does not depend on
`rsky-repo`.

**Method.** Both options ran on the same inputs on 2026-10-12: the upstream
interop vectors vendored in [`interop/`](../../interop/README.md) (commit
`056e5741`), fixtures written by `@atproto/repo` 0.10.14 at the pin, and the
workload of the harness's performance scenario. The comparison is the
package [`bench/`](../../bench/README.md), outside the Cargo workspace. Each
"observed" statement below is asserted by
[`bench/tests/vectors.rs`](../../bench/tests/vectors.rs) or by a test of the
crate named, so it can be run again. Facts about crates come from the
crates.io API and the GitHub API; source statements name the file and the
commit. Toolchain: Rust 1.99.0, Node 24.21.0.

## 1. The two options

**`rsky-repo` exists in two forms that behave differently.** crates.io has
one version, 0.0.6, published on 2026-08-16 under Apache-2.0, with 20
downloads on 2026-10-12. The main branch of its repository is at 0.1.4 and
is not published. Between the two lie the September changes that matter
here: the crate began to run the upstream MST and commit-proof vectors, to
put the covering proof into a commit's blocks, and to verify commit
signatures over the digest on both curves.
Sources: [crates.io API](https://crates.io/api/v1/crates/rsky-repo);
[`rsky-repo/Cargo.toml`](https://github.com/blacksky-algorithms/rsky/blob/7964ffa3cc80c15b247e0ea6398e2f9880fc9c4d/rsky-repo/Cargo.toml) at `7964ffa3` (2026-10-10);
commits [`e9d96f3b`](https://github.com/blacksky-algorithms/rsky/commit/e9d96f3ba58bd823fcfde6d307e2bd34b78b5d3c),
[`bb3dd800`](https://github.com/blacksky-algorithms/rsky/commit/bb3dd800a79cb3d6b2bd7bfa394f3870b1aa400d) and
[`544808c0`](https://github.com/blacksky-algorithms/rsky/commit/544808c0168e9beb18261dc8a1674ef6302f8e7b), all of 2026-09-07.

Both forms were tested. The published one is what `cargo add` gives. The
main branch needs a git dependency, which SPEC §14.5 denies
(`unknown-git`), so using it means an exception to the policy or waiting
for a release.

**`rsky-repo` is a port of `@atproto/repo`.** Its tree is a persistent
structure of `Arc<RwLock<...>>` nodes with the Reference's method names.
Every operation is `async`, and storage is a trait object behind a tokio
`RwLock` whose methods return boxed futures. `format_commit` takes the
revision from `Ticker::new()`, which reads `SystemTime::now()`. Records are
a value type of its own, and errors are `anyhow`.
Sources, in the published crate: `src/mst/mod.rs` lines 437 to 443 and 735;
`src/storage/types.rs`; `src/repo.rs` lines 217 to 300;
`rsky-common-0.1.3/src/tid.rs` lines 95 to 135.

**The in-house engine** is 2,960 lines in `crates/pds-repo/src` with its
unit tests, and 888 lines of integration tests. It is synchronous. It does
no I/O: blocks come from a `BlockStore` trait with one required method, the
signature from a closure over the caller's key, and the revision from an
argument. It builds on `pds-types` for DAG-CBOR and CIDs and has no other
dependency of its own beyond `sha2`, `cid` and `thiserror`. For
comparison, `rsky-repo` 0.0.6 is 7,353 lines.

**Other crates were not run.** The decision on the repository engine
names two options. Three more Rust crates
implement the tree; they are listed so that the next reader does not have
to search.

| Crate | Version | Licence | Published |
|---|---|---|---|
| `atrium-repo` | 0.1.8 | MIT | 2026-03-26 |
| `jacquard-repo` | 0.12.2 | MPL-2.0 | 2026-09-05 |
| `atproto-repo` | 0.14.5 | MIT | 2026-04-02 |

Source: crates.io API for each name, read 2026-10-12.

## 2. The vectors

Counts are vectors that pass. "Not run" marks a group that the option has
no function for at this layer.

| Group | In-house | `rsky-repo` 0.0.6 | `rsky-repo` 0.1.4 |
|---|---|---|---|
| MST key heights (9) and common prefix (13) | 22 | 22 | 22 |
| Firehose commit proofs (6): root before and after | 6 | 6 | 6 |
| Firehose commit proofs (6): blocks in the proof | 6 | 1; 16 blocks missing in all | 6 |
| Crypto signature fixtures (6) | 6 | 6 | 6 |
| Crypto `did:key` fixtures (6) | 6 | not run | not run |
| Data model fixtures (3): JSON to DAG-CBOR to CID | 3 | 1 | 3 |
| A float in a record | refused | encoded | encoded |
| Data model valid (5) and invalid (12) | 17 | not run | not run |
| Lexicon record data valid (3) and invalid (50) | 53 | not run | not run |
| Trees that reference a node more than once (6), refused on import | 6 | 0 | 0 |

The same against the fixtures from `@atproto/repo` at the pin
([`crates/pds-repo/tests/fixtures`](../../crates/pds-repo/tests/fixtures/README.md)):

| Check | In-house | `rsky-repo` 0.0.6 | `rsky-repo` 0.1.4 |
|---|---|---|---|
| Tree roots for five sets of keys | 5 | 5 | 5 |
| CIDs of 501 record bodies | 501 | 501 | 501 |
| Commit CIDs for a given key and `rev` (13) | 13 | 13 | 13 |
| Tree root, new blocks and removed blocks of each commit (13) | 13 | 13 | 13 |
| Firehose blocks of each commit (13) | 13 | 9 | 13 |
| CAR of each commit's firehose blocks | same bytes | not comparable | not comparable |
| Export of the repository | same bytes | same blocks, another order | same blocks, another order |
| Import of the Reference's export (308 records) | verifies | verifies | verifies |
| A commit signed with P-256 verifies | yes | no | yes |

Findings behind the rows:

- **Commit proofs.** `rsky-repo` 0.0.6 puts the nodes on the path to each
  key into a commit and has no covering proof, so a consumer lacks the
  blocks to check a delete against the previous root. The Reference at the
  pin sends the covering proof
  ([`repo.ts`](https://github.com/bluesky-social/atproto/blob/7ca16cc6989f8247637615aca17c5abb911b8fb1/packages/repo/src/repo.ts) lines 146 to 150). The main branch has it.
- **Not comparable.** `rsky-repo` signs the `rev` it reads from the clock,
  so a commit it formats cannot equal a fixture byte for byte. Its commit
  encoding and signing were checked apart from `format_commit`, with the
  fixture's `rev`: all thirteen CIDs match.
- **P-256.** In 0.0.6 the two curve plugins disagree on whether they hash
  the message, and `verify_commit_sig` passes a digest to both. A commit
  signed with a P-256 key does not verify. SPEC §6.1 allows both curves.
  The main branch fixes it. Sources: `rsky-crypto-0.2.0/src/verify.rs`
  lines 20 to 25; `rsky-repo-0.0.6/src/util.rs` lines 33 to 45.
- **Data model.** `rsky-repo` 0.0.6 encodes one of the three JSON fixtures
  to the expected bytes; the main branch encodes all three. Neither refuses
  a float: the value type holds any JSON number.
- **Export order.** `rsky-repo` writes the same blocks in another order.
  The order is not normative. The in-house engine reproduces the
  Reference's order, so the harness can compare exports as bytes.

**KNOWN_FAILURES.** No vector fails in `pds-repo`, `pds-identity` or
`pds-lexicon`. `pds-types` keeps the three entries of unit 0: three CID
strings that the JavaScript parser accepts and the Rust `cid` crate
rejects, none of which can occur as a repository CID
(`crates/pds-types/src/syntax.rs`, `CID_KNOWN_FAILURES`).

## 3. The requirements of the upstream 0.5.37 note

The note on upstream 0.5.35 to 0.5.37
([pull request #20](https://github.com/linkjar/pds/pull/20), §6) sets eight
requirements for this crate. Requirements 1 to 4, 6 and 8 are met and
tested: [`tests/import.rs`](../../crates/pds-repo/tests/import.rs),
[`tests/tree.rs`](../../crates/pds-repo/tests/tree.rs) and
[`tests/reference.rs`](../../crates/pds-repo/tests/reference.rs) in
`crates/pds-repo`, and the fuzz target `repo_car`. Requirements 5 and 7
are about the HTTP answer and the `verify` command; they belong to units 4
and 3, and the engine gives them an error that names the defect. The last
row of the first table in §2 is the result for requirement 8.

## 4. Fit with the SPEC

| SPEC requires | In-house | `rsky-repo`, both forms |
|---|---|---|
| A synchronous SQLite driver on a blocking pool, one writer per actor (§3.2) | Synchronous; the store is a trait with one read method | Asynchronous throughout; the store sits behind a tokio `RwLock` and returns boxed futures, so a blocking store needs a bridge on every block read |
| Time enters through an injectable provider; the Harness can give the `rev` (§3.2, §3.3) | `rev` is an argument | `rev` comes from the system clock inside `format_commit`; a fork or an upstream change is needed |
| Cryptography from RustCrypto (§3.2) | `k256`, `p256`, `sha2`; the engine itself holds no key | `secp256k1`, a binding to the C library |
| The actor's key may be secp256k1 or P-256 (§6.1) | The caller signs, with any key | `format_commit` takes a `secp256k1::Keypair`; a P-256 key or a key in a KMS cannot sign |
| Decode limits before any other processing (§6.1) | Applied in the reader: size, depth, container length, key length, CID length, integer range, no floats | None in the crate |
| A valid tree only (§6.1) | The whole tree is checked on import | Key syntax per node; no check of key layer or of order across nodes |
| `cargo deny` with advisories, licences and sources (§14.5) | Passes | Fails; see below |
| `#![forbid(unsafe_code)]` in workspace crates (§3.2) | Yes | Not a workspace crate; it brings one C library |

**Dependencies.** `pds-repo` resolves to 57 crates, almost all of them
through `pds-types`. `rsky-repo` resolves to 256 in both forms. It depends
on `rsky-common`, which depends on `rsky-identity`, which brings `reqwest`
with rustls and `hickory-resolver`: an HTTP client and a DNS resolver
inside the repository engine. It also brings `tokio` with every feature
and `serde_cbor`, which is unmaintained
([RUSTSEC-2021-0127](https://rustsec.org/advisories/RUSTSEC-2021-0127.html)).
Observed with `cargo tree -e normal` on `bench/Cargo.lock`.

**`cargo deny` with this repository's `deny.toml` fails on that tree.** Two
advisories against `hickory-proto`
([RUSTSEC-2026-0118](https://rustsec.org/advisories/RUSTSEC-2026-0118),
[RUSTSEC-2026-0119](https://rustsec.org/advisories/RUSTSEC-2026-0119)); one
licence outside the allow list, CDLA-Permissive-2.0 on `webpki-roots`; and,
for the main branch, the git source. The licence could be allowed. The
advisories are in code the engine never calls, and they would still need an
`ignore` entry with a reason and an expiry (§14.5).
Observed 2026-10-12 with cargo-deny 0.20.2.

## 5. Micro-benchmarks for the single-write, batch-write and export scenarios

**Host.** Apple M1 Pro, 10 cores, 16 GiB, macOS 27.0 (darwin 27.0.0,
arm64). The machine was in use: a backup and a file synchroniser ran
beside the measurements. Rust 1.99.0, release profile with fat LTO; Node
24.21.0.

**Method.** [`bench/README.md`](../../bench/README.md) has it in full. In
short: the four programs replay the harness's single-write, batch-write and export sequence against a
block store in memory. One sample is the engine's share of one write: from
validated record values to the commit applied to the store, with the CAR
of the firehose blocks written and the commit signed. HTTP, SQLite,
lexicon validation and the sequencer are not in it. Each program ran five
times, interleaved, so that a busy minute did not fall on one engine. A
cell is the median of the five runs, with the lowest and highest run in
parentheses. Raw results:
[`bench/results/2026-10-12/`](../../bench/results/2026-10-12).

**All four build the same repository.** Each printed the tree root
`bafyreibhl26xxjw367r4sv3oyafituicjxxuntlzhl3k4wzkd2dlvkawqq` after 101,100 records.

**Single write, one create: p50 in ms.**

| Records at the start | In-house | `rsky-repo` 0.0.6 | `rsky-repo` 0.1.4 | `@atproto/repo` |
|---|---|---|---|---|
| 10 | 0.055 (0.054 to 0.057) | 0.233 (0.227 to 0.262) | 0.285 (0.279 to 0.292) | 0.960 (0.950 to 1.068) |
| 1,110 | 0.064 (0.063 to 0.065) | 0.579 (0.568 to 0.618) | 0.703 (0.691 to 0.728) | 0.990 (0.977 to 1.032) |
| 10,000 | 0.072 (0.071 to 0.073) | 1.160 (1.139 to 1.194) | 1.380 (1.338 to 1.492) | 1.235 (1.185 to 1.281) |
| 100,000 | 0.070 (0.068 to 0.071) | 0.615 (0.603 to 0.631) | 0.769 (0.755 to 0.783) | 1.083 (1.049 to 1.222) |

**Single write: p99 in ms.**

| Records at the start | In-house | `rsky-repo` 0.0.6 | `rsky-repo` 0.1.4 | `@atproto/repo` |
|---|---|---|---|---|
| 10 | 0.119 (0.064 to 0.173) | 0.433 (0.375 to 0.930) | 0.521 (0.507 to 0.579) | 2.476 (2.294 to 2.696) |
| 1,110 | 0.082 (0.073 to 0.104) | 0.812 (0.769 to 1.721) | 1.009 (0.946 to 1.069) | 4.298 (3.946 to 4.508) |
| 10,000 | 0.106 (0.084 to 0.145) | 1.530 (1.507 to 1.614) | 1.923 (1.817 to 20.1) | 4.157 (3.411 to 9.312) |
| 100,000 | 0.144 (0.079 to 0.374) | 0.837 (0.787 to 0.903) | 1.053 (1.037 to 1.124) | 3.881 (3.315 to 4.781) |

**Single write: what a write produces.** Bytes of the CAR of the firehose blocks;
blocks added to the store; blocks removed from the store. Means over the
100 writes.

| Records at the start | In-house | `rsky-repo` 0.0.6 | `rsky-repo` 0.1.4 | `@atproto/repo` |
|---|---|---|---|---|
| 10 | 1,548; 6.52; 5.21 | 1,464; 6.52; 5.21 | 1,548; 6.52; 5.21 | 1,548; 6.52; 5.21 |
| 1,110 | 2,801; 9.69; 8.44 | 2,706; 9.69; 8.44 | 2,801; 9.69; 8.44 | 2,801; 9.69; 8.44 |
| 10,000 | 4,287; 9.85; 8.74 | 4,214; 9.85; 8.74 | 4,287; 9.85; 8.74 | 4,287; 9.85; 8.74 |
| 100,000 | 3,511; 10.79; 9.57 | 3,444; 10.79; 9.57 | 3,511; 10.79; 9.57 | 3,511; 10.79; 9.57 |

**Batch write, a batch of 200 creates: p50 of five batches, in ms.**

| Records at the start | In-house | `rsky-repo` 0.0.6 | `rsky-repo` 0.1.4 | `@atproto/repo` |
|---|---|---|---|---|
| 110 | 1.052 (1.003 to 1.102) | 17.5 (17.2 to 20.5) | 46.3 (45.2 to 47.4) | 93.7 (90.2 to 94.8) |
| 1,210 | 1.090 (1.027 to 1.100) | 17.8 (17.3 to 22.3) | 44.8 (44.3 to 46.0) | 79.8 (77.3 to 91.6) |
| 10,100 | 1.181 (1.158 to 1.215) | 29.3 (28.5 to 29.7) | 78.6 (76.1 to 82.8) | 101 (98.4 to 110) |
| 100,100 | 1.431 (1.264 to 1.643) | 28.2 (27.8 to 28.4) | 75.1 (73.5 to 81.4) | 115 (108 to 120) |

**Batch write: the slowest of five batches, in ms.**

| Records at the start | In-house | `rsky-repo` 0.0.6 | `rsky-repo` 0.1.4 | `@atproto/repo` |
|---|---|---|---|---|
| 110 | 1.181 (1.034 to 1.303) | 21.0 (20.4 to 26.1) | 54.0 (52.8 to 55.2) | 99.7 (95.3 to 106) |
| 1,210 | 1.215 (1.131 to 1.324) | 20.4 (20.1 to 34.1) | 52.4 (52.0 to 55.7) | 83.3 (78.5 to 227) |
| 10,100 | 1.294 (1.178 to 1.516) | 32.7 (32.4 to 33.1) | 90.1 (85.8 to 105) | 116 (110 to 229) |
| 100,100 | 1.939 (1.332 to 2.298) | 32.7 (32.2 to 33.0) | 86.2 (85.7 to 111) | 128 (120 to 134) |

**Export of 101,100 records.** The fastest of three exports in each
run.

| Engine | Wall ms | CAR bytes | Blocks | Peak heap above the start, MiB |
|---|---|---|---|---|
| In-house | 85.1 (81.3 to 92.0) | 20,771,913 | 128,312 | 30.2 |
| `rsky-repo` 0.0.6 | 520 (519 to 580) | 20,771,913 | 128,312 | 52.6 |
| `rsky-repo` 0.1.4 | 518 (506 to 540) | 20,771,913 | 128,312 | 52.6 |
| `@atproto/repo` | 1,781 (1,747 to 2,061) | 20,771,913 | 128,312 | not measured |

**The engine reproduces what unit 1 measured on the Reference server.**
The harness read 1,548, 2,801, 4,287 and 3,511 bytes of new blocks per
write off the firehose of the reference image, and net block rows of 1.3,
1.3, 1.1 and 1.2 per write
([perf-reference.md](../../parity/results/perf-reference.md), the
single-write scenario). The
in-house engine, the Reference's library and `rsky-repo` 0.1.4 give those
CAR sizes exactly, and added less removed blocks come to 1.31, 1.25, 1.11
and 1.22. `rsky-repo` 0.0.6 gives smaller CARs: its commits lack proof
blocks (§2). The export has the 128,312 blocks and 19.8 MiB that the
harness measured in the export scenario.

**Reading the numbers.**

- Against `rsky-repo`, the in-house engine is 4 to 16 times faster on one
  create (0.0.6) and 5 to 19 times (0.1.4), 16 to 25 times and 41 to 67
  times faster on a batch, and 6 times faster on the export, with 30 MiB
  of heap against 53 MiB.
- Against the Reference's library, it is 15 to 17 times faster on one
  create, 73 to 89 times on a batch and 21 times on the export.
- `rsky-repo` got slower on batches between its two forms. The covering
  proof is computed per write, and each proof walks the tree again.
- The Reference server needed about 11 to 13 ms for one create and 1,642
  ms for the export, over HTTP and SQLite
  ([perf-reference.md](../../parity/results/perf-reference.md)). Its
  library alone takes about 1 ms and 1,781 ms here. The export is the
  library's work almost entirely; a create is mostly not.

**Limits of these numbers.** A store in memory answers in tens of
nanoseconds. Over SQLite every block read is a query, so the count of
blocks read per write will weigh more than it does here; the in-house
engine reads the nodes on one path and each of them once. The host was not
idle, which shows in the p99 ranges. `rsky-repo` ran on a current-thread
tokio runtime; a server would drive it from its own runtime and from
SQLite on a blocking pool, which these numbers do not include. The
in-house export keeps a set of the record CIDs it has written; its 30 MiB
of heap is that set and can shrink.

## 6. The decision and what it costs

In-house, for these reasons, in the order they weigh:

1. **Correctness against the Reference is shown, not assumed.** The engine
   gives the bytes of `@atproto/repo` at the pin for tree roots, commits,
   firehose blocks and exports, and the sizes the reference image put on
   its firehose. The published `rsky-repo` fails four of six commit-proof
   vectors.
2. **The engine's rules for a tree from elsewhere are its own to keep.**
   The checks of SPEC §6.1 sit in one guard and one reader that every
   traversal goes through. `rsky-repo` has neither in either form (§2,
   last row of the first table), and adding them means a fork.
3. **It fits the SPEC without a bridge.** Synchronous storage, an injected
   revision and a caller's key are requirements here (§3.2, §3.3, §19).
   Each one needs a change to `rsky-repo`'s public API.
4. **The supply chain stays small.** 57 crates against 256, no HTTP client
   and no DNS resolver in the engine, and a clean `cargo deny`.
5. **It is faster by an order of magnitude** on the engine's share of the
   single-write and batch-write scenarios. This reason is last: the Reference's library is already fast
   enough for one user, and SQLite will set the pace of a write.

What the decision costs:

- **The engine is ours to maintain**: about 3,000 lines. The tree
  algorithm is stable; the upstream changes since the pin that touch it
  are the 0.5.37 guard and nothing else.
- **Upstream changes are ours to follow.** The fixtures answer this: after
  a pin move, `node src/tools/repo-fixtures.ts` writes them again with the
  new library, and the tests show what changed.
- **Nothing is given back yet.** The crate could be published for other
  Rust projects; that is a later decision.

What would reopen the decision: a published `rsky-repo` with a
synchronous core, an injected revision and the tree checks. The comparison
in `bench/` runs against any version by changing one line.

## 7. Found on the way, for the owner

Places where the Rust code as written is stricter than the Reference at
the pin, or follows the SPEC where the SPEC and the Reference differ. None
is listed in SPEC §2.3. Each needs a scenario that records what the pin
does, and then a decision: list it, or follow the pin. None is reachable
before the unit named.

1. **Imported trees (the decision on stricter checks on data from
   elsewhere, unit 4 and 8).** `read_tree` refuses a tree with
   a key at the wrong layer, keys out of order across nodes, an empty node
   below the root, or a node referenced more than once. The Reference's
   import checks none of these
   ([`sync/consumer.ts`](https://github.com/bluesky-social/atproto/blob/7ca16cc6989f8247637615aca17c5abb911b8fb1/packages/repo/src/sync/consumer.ts) lines 56 to 105).
   The pin decision of pull request #20 settles the last one.
2. **DAG-CBOR from elsewhere (unit 4).** The reader of `pds-types` refuses
   map keys out of canonical order, a float with a whole value, and
   `undefined`. The Reference's decoder is strict on integer encoding and
   duplicate keys only, reads `1.0` as an integer, and reads `undefined`
   as null (`@atproto/lex-cbor` 0.1.6, `dist/encoding.js`, `decodeOptions`).
   This matters for record blocks in an imported CAR; records the server
   encodes itself are canonical.
3. **Version 2 commits (unit 8).** `Commit::decode` reads version 3 only.
   The Reference also reads the legacy version 2
   ([`util.ts`](https://github.com/bluesky-social/atproto/blob/7ca16cc6989f8247637615aca17c5abb911b8fb1/packages/repo/src/util.ts) lines 120 to 130).
   No LinkJar account has one.
4. **Blob references (unit 4).** `validate_data_model` follows SPEC §6.3:
   `size` above zero and a non-empty `mimeType`. The Reference's strict
   blob check accepts a size of zero and requires a `/` in the MIME type.
   SPEC §6.4 accepts an empty upload, so a record that references one is
   refused here and accepted there.
5. **Record validation follows `@atproto/lex`, not `@atproto/lexicon`
   (unit 4).** The pinned PDS validates a write with `@atproto/lex`
   (`packages/pds/src/repo/prepare.ts` lines 2 and 68 to 80). The two packages differ on
   `unknown`, on union members written with `#main`, and on blob
   constraints; the module comment of
   [`validate.rs`](../../crates/pds-lexicon/src/validate.rs) lists each.
6. **Handle verification answers (unit 3).** Following SPEC §9.2, two TXT
   records with the same DID are one answer, and the HTTPS body is the
   whole trimmed body. The Reference fails on any two `did=` records and
   reads the first line of the body
   (`packages/identity/src/handle/index.ts` at the pin, lines 66 and 90 to 100).
7. **The DID-document reader (unit 3).** Following SPEC §9.3, the signing
   key's `controller` must be the DID, and the PDS endpoint must be a
   scheme, host and port. The Reference's reader checks neither
   (`packages/common-web/src/did-doc.ts` at the pin, lines 39 to 110).

Two things that are not differences:

- **Two input bounds of upstream 0.5.37** are in place without a visible
  effect: a key string over 128 characters is refused before decoding
  ([#5583](https://github.com/bluesky-social/atproto/pull/5583)), and a
  `$link` string over 256 characters likewise, with the 100-byte limit of
  SPEC §6.1 on the decoded CID. A valid key or CID is shorter than either.
- **JSON numbers and base64.** `123.0` is an integer and `$bytes` need not
  have zero trailing bits, as in the Reference. Both were needed for
  upstream vectors to pass.

One thing about the development environment: `devenv shell` did not start
on devenv 2.4 (a missing `git-hooks` input, now declared), and once it
starts it installs a `pre-commit` hook in the shared `.git/hooks` that
points at the worktree it ran in. With several worktrees, that hook runs
for all of them and breaks when that worktree is removed.

## 8. What the next units inherit

- **Unit 3** implements `BlockStore` over `repo_block` and serves
  `sync.getRepo` through `Repo::export`. It adds the record proof for
  `sync.getRecord`: the nodes on the path to one key, which the tree can
  already produce for a commit. It adds the DID resolver and handle
  verification on top of the readers in `pds-identity`.
- **Unit 4** calls `Repo::format_commit` inside the actor transaction. A
  commit's `removed_cids` names record blocks too; a record that another
  key still holds must be kept, as the Reference's transactor does.
- **Import (units 4 and 8)** has `verify_repo_car` and `verify_diff_car`.
  The second reads both trees in full, so its cost follows the size of the
  repository and not of the difference. That is acceptable for an import;
  a walk that skips shared subtrees can replace it if S-numbers ask for it.
- **The Harness** can compare `getRepo` between the Reference and the
  Candidate as bytes, since the export order is the Reference's.

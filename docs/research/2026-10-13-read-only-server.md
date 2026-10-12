# The read-only server: unit 3

Research record, 2026-10-13. Companion to [SPEC.md](../SPEC.md) §4, §8, §9,
§14.2, §17 and §18.2, and to [plan.md](../plan.md), unit 3. It records what
the unit built, how it is held to the Reference, what the comparison found
on the way, and what the unit leaves out.

**Result.** The Candidate serves the read methods of a data directory that
the Reference wrote, and answers as the Reference does. On 2026-10-13 the
restore run sent 331 requests to both servers on the same data directory.
Status, compared headers and normalised body agreed on every one, and the
bytes of 328 bodies agreed by hash, repository exports among them. `verify`
reported zero differences on that directory.

**One part of the gate is not met.** The plan asks for `verify` with zero
differences on a staging restore of production data. No staging restore was
available to this unit. `verify` ran on the data directories the Harness
writes, and on generated ones with each kind of damage. Running it on the
staging restore is the owner's step; the command is in §5.

## 1. What the unit built

| Crate | Now holds |
|---|---|
| `pds-storage` | Read access to the Reference's SQLite files: `account.sqlite`, one `store.sqlite` per actor, `sequencer.sqlite` and `did_cache.sqlite`, with the layout of SPEC §8.1. The actor store is a `BlockStore` for the repository engine. Every call is synchronous. |
| `pds-blob` | The disk blob store, read only. |
| `pds-identity` | A DID resolver for `did:plc` and `did:web` behind a cache with the Reference's stale and maximum lifetimes. |
| `pds-repo` | `Repo::record_proof`: the blocks of `sync.getRecord`. |
| `pds-xrpc` | The server: one entry point, parameter validation, credentials, CORS, the handlers, and response bodies streamed from the blocking pool. |
| `linkjar-pds` | Configuration from the environment and the commands `serve`, `verify`, `export-car`, `check-config`, `health` and `version`. |

**Served.** `com.atproto.sync`: `getRepo` with `since`, `getCheckout`,
`getRecord`, `getBlocks`, `getLatestCommit`, `getHead`, `getRepoStatus`,
`listRepos`, `listBlobs`, `getBlob`. `com.atproto.repo`: `getRecord`,
`listRecords`, `describeRepo`. `com.atproto.server.describeServer`.
`com.atproto.identity.resolveHandle`. The plain routes of SPEC §4.1: the
banner, `robots.txt`, the health route and `/.well-known/atproto-did`.

**The image.** [`Dockerfile`](../../Dockerfile) builds the binary with Rust
1.99 and puts it on `debian:trixie-slim` with CA certificates. The server
runs as uid 1000, the uid of the reference image, so a data directory moves
between the two without a change of owner.

## 2. How it is held to the Reference

**The restore run** ([`parity/src/restore.ts`](../../parity/src/restore.ts),
`pnpm parity restore`) uses one stack and two servers:

1. The reference image starts on a fresh stack. A seed scenario writes six
   accounts: one with posts, a deleted record, a replaced record, a like and
   two blobs; one with 700 records, which is more blocks than one page of
   the export; one deactivated by its owner; one taken down by an admin; one
   active with a record and a blob taken down; one that changed its handle.
2. Four read scenarios run against the Reference.
3. The PDS container is replaced by the Candidate image on the same volume.
4. The same four scenarios run against the Candidate.
5. The Candidate's `verify` runs on the data directory.
6. The two sets of transcripts are compared. No difference is allowed.

Both servers read the same bytes. Each body is therefore recorded twice:
normalised, as every scenario records it, and as the SHA-256 of its bytes.

**What the scenarios cover.**

| Scenario | Requests | Covers |
|---|---|---|
| `10-plain-routes` | 37 | The plain routes, the shape of an `/xrpc/` path, methods, CORS, a method without a handler. |
| `11-sync` | 155 | Every sync read, with `since` at each revision, proofs of presence and absence, both account statuses read with no credential, as another account, as the owner and as an admin, and thirteen credentials that do not verify. |
| `12-repo` | 57 | The repository reads, a repository hosted elsewhere through the AppView, `describeServer`, `resolveHandle`. |
| `13-parameters` | 82 | Parameter validation: missing, empty, repeated, wrong format, integers and booleans, and the order of the checks. |

Three exchanges have no body hash: the `HEAD` request and the two
preflights.

**Tests in the workspace.** `crates/linkjar-pds/tests` builds a data
directory in the Reference's schema with the repository engine.
[`server.rs`](../../crates/linkjar-pds/tests/server.rs) drives the server
through a real listener: nine tests. [`verify.rs`](../../crates/linkjar-pds/tests/verify.rs)
checks that a clean directory has no differences and that fourteen kinds of
damage are named, a tree with a shared node among them: eleven tests. The validation and credential modules
of `pds-xrpc` carry the Reference's answers as unit tests.

## 3. What the comparison found

Each item was observed on the reference image, revision 11 or 12, on
2026-10-12 and 2026-10-13.

1. **The export follows the block table, not the tree.** `sync.getRepo`
   writes the rows of `repo_block` from the newest revision down, then by
   CID down, 500 at a time
   (`packages/pds/src/actor-store/repo/sql-repo-reader.ts` at the pin). The
   Candidate reads the table in the same order, and the export is the same
   bytes. The decision record of unit 2 expected the export to come from
   `Repo::export`; that order is the one `@atproto/repo` gives a library
   caller, and the `export-car` command writes it.
2. **A record proof has a fixed block order.** `sync.getRecord` writes the
   commit, then the record when it exists, then the tree nodes from the root
   down. The order comes from the block cache of the Reference's reader.
3. **Parameters are checked in two phases.** The query is read in the order
   it was written: an empty value is dropped, and a value for an integer or
   boolean parameter is converted or refused. The parameters are then
   checked in the order the lexicon declares them. So `limit=abc` is
   reported before a missing `repo`, whatever the order in the query. A
   repeated scalar is reported as a list of its first two values. A string
   of 256 or more characters is cut in the message. An integer beyond
   2^53 − 1 is reported as the JavaScript number it converts to.
4. **Parameters are checked before the credential.** A bad parameter with a
   bad token is a parameter error.
5. **A path under `/xrpc/` is read loosely.** Letters and digits in parts
   separated by single dots or hyphens, at least two characters, pass as a
   method name: `/xrpc/nope` goes to the catch-all proxy and answers 401.
   Anything else is 400 `InvalidRequest`, "invalid xrpc path", `POST
   /xrpc/_health` among them. A method other than `GET`, `HEAD` and `POST`
   is 400, "XRPC requests only supports GET and POST".
6. **A method without a handler refuses the admin password.** Without a
   credential it answers 401 `AuthMissing`. With the admin password it
   answers 400 `InvalidToken`, "Unexpected authorization type".
7. **Express adds a charset.** A blob stored as `text/plain` is served as
   `text/plain; charset=utf-8`, and so is an AppView answer of type
   `application/json`.
8. **`resolveHandle` passes on what the AppView says.** An answer without a
   `did` gives the body `{}`, and a `did` that is not a DID is passed
   through. A refusal or a failure of the AppView is 400, "Unable to
   resolve handle".
9. **An upstream 500 becomes 502.** `repo.getRecord` for a repository the
   server does not host returns the AppView's status and body, except that
   500 is reported as 502.
10. **`repo_root.rev` can differ from the commit's revision.** After an
    account imports its own export, the Reference gives the root a new
    revision while the commit stays the same
    ([`default/15-import`](../../parity/scenarios/default/15-import.scenario.ts)).
    `verify` therefore does not compare the two.
11. **A deactivated or taken-down account is "not found" for the record
    methods.** `repo.getRecord` forwards the request to the AppView as if
    the repository were hosted elsewhere, and `repo.listRecords` answers
    "Could not find repo", for the owner as for anyone. The sync methods
    answer `RepoDeactivated` and `RepoTakendown`, and let the owner and an
    admin through.

## 4. What the unit leaves out

Each item has a unit in [plan.md](../plan.md). None is reachable by the
restore run, because its scenarios do not ask for it.

| Not in this unit | The Candidate answers | Unit |
|---|---|---|
| The catch-all proxy (SPEC §4.4) | 401 without a credential, as the Reference. 501 `MethodNotImplemented` with a valid session, where the Reference forwards. | 4 |
| The read-after-write methods of `app.bsky` (SPEC §4.5) | As any method without a handler. The Reference validates their parameters first. | 5 |
| Rate limits (SPEC §4.3) | No limiter runs and no rate-limit header is set. The scenarios send the bypass key, with which the Reference sets none either. | 4 |
| OAuth access tokens (SPEC §5.5) | A `DPoP` credential is 400 `InvalidToken`. | 5 |
| Handle resolution over DNS and HTTPS (SPEC §9.2) | With an AppView configured, as the Reference. Without one, a handle that is not hosted does not resolve. | 5 |
| The S3-compatible blob store | `getBlob` needs `PDS_BLOBSTORE_DISK_LOCATION`. | 4 |
| The actor store LRU (SPEC §8.3) | A read opens its own read-only connection. | 4 |
| The metrics and ops listeners, OTLP (SPEC §14.4) | Logs only: JSON lines on standard error. | 8, 9 |

DD-11 and DD-12 of SPEC §2.3 were planned for this unit. Neither can be
observed yet: the server does not verify a handle itself, and
`describeRepo` returns the DID document as it was resolved. Their scenarios
move to unit 5, with `updateHandle` and inbound service auth.

## 5. `verify`

```sh
PDS_DATA_DIRECTORY=/path/to/restore \
PDS_BLOBSTORE_DISK_LOCATION=/path/to/restore/blocks \
linkjar-pds verify            # or: verify --json
```

It opens every database read-only and writes nothing. It exits 0 with no
difference, 1 with one or more, and 2 when it cannot run. Without
`PDS_BLOBSTORE_DISK_LOCATION` it checks the blob index and says that the
blob store was not checked.

| Finding | Meaning |
|---|---|
| `store-missing`, `store-unreadable`, `root-missing` | The actor has no store, the store does not open, or it has no repository root. |
| `root-mismatch` | `account.sqlite` and the actor store name different commits or revisions. |
| `commit-invalid`, `commit-did` | The commit block does not decode, or names another account. |
| `key-unreadable`, `signature-invalid` | The key file does not load as a secp256k1 key, or it did not sign the commit. |
| `tree-shared-node` | The tree references a node more than once. An unpatched Reference accepts such an import (SPEC §2.3, DD-6). |
| `tree-block-missing`, `tree-invalid` | A tree node is absent, or the tree has another defect of SPEC §6.1. |
| `record-index-mismatch`, `record-uri` | The record index and the tree do not hold the same keys and records, or the index holds a record of another repository. |
| `root-recompute-mismatch`, `root-recompute-failed` | The record index, written into an empty tree, does not give the commit's root. |
| `record-block-missing` | A record of the index has no block. |
| `blob-unindexed`, `blob-missing` | A record references a blob that has no row in the blob index, or that is not in the blob store and is not taken down. |
| `sequence-gap` | The event log has a break in its sequence numbers. |

SPEC §18.2 says that the same command run with the Reference serving the
same copy is the oracle. The Reference has no such command. The restore run
is the comparison: both servers serve the same copy, and their answers are
compared.

## 6. Dependencies

The server binary resolves to 248 crates; `pds-repo` alone still resolves
to 57. The HTTP stack is the difference: `tokio`, `axum` on `hyper`,
`reqwest` on `rustls`, and `rusqlite` with the bundled SQLite.

Two points for the owner, both in [plan.md](../plan.md):

- **TLS uses aws-lc.** `reqwest` 0.13 with `rustls` takes `aws-lc-rs` as
  its cryptography, a C library with assembly. SPEC §3.2 names RustCrypto
  for cryptography. The repository's signatures and hashes are RustCrypto;
  outbound TLS is not. `cargo deny` bans OpenSSL only.
- **One licence exception.** `reqwest`'s certificate verifier depends on
  `webpki-root-certs`, the Mozilla root list as data, under
  CDLA-Permissive-2.0. `deny.toml` allows that licence for that crate only.

`cargo vet` passes with 276 exemptions, up from 135. No crate in the tree
has an advisory on 2026-10-13.

## 7. Not measured

The unit has no performance numbers. S3 of SPEC §16.1, the export of a
large repository, is the scenario this unit can now answer for the
Candidate; the Reference's baseline is in
[perf-reference.md](../../parity/results/perf-reference.md). The run seeds
100,000 records through the Reference and takes tens of minutes.

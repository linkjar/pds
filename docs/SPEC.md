# LinkJar PDS Specification

Status: Draft, revision 11, 2026-10-13
Normative keywords: **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, **MAY** ([RFC 2119](https://www.rfc-editor.org/rfc/rfc2119))

LinkJar PDS is an AT Protocol Personal Data Server written in Rust. It is
compatible with the reference TypeScript implementation that this repository
builds and patches today. The account-handling behaviour that LinkJar applies
as source patches becomes native extension points, so the same server serves a
stock operator and LinkJar without a fork. This document is the normative
definition of that server. The decision record, delivery units and status are
in [plan.md](plan.md).

Revision 1 applies the owner's decisions of 2026-10-08, the
[state-of-the-art research](research/2026-10-08-state-of-the-art.md), the
[atproto.com gap report](research/2026-10-08-atproto-doc-sweep.md) and the
[sidecar design](sidecars.md). Changes from revision 0 are listed in
Appendix E.

Revision 2 replaces each "verify at pin" mark with what the parity harness
observed on the reference image, and corrects the places where revision 1
described the protocol documents and not the Reference. The evidence is in
[parity/verify-at-pin.md](../parity/verify-at-pin.md); the changes are listed
in Appendix F.

Revision 3 applies the owner's decisions of 2026-10-09 on Appendix D, D15 and
D16. The Candidate answers 400 where the Reference answers 500 to four client
errors, and it refuses a replayed service token. §2.3 lists these deliberate
differences; the changes are listed in Appendix G.

Revision 4 records decision D2 of unit 2: the repository engine is written
in this repository. The evidence is in the
[decision record](research/2026-10-12-repository-crate.md); the changes are
listed in Appendix H.

Revision 5 applies the owner's decisions of 2026-10-12 on the pin, on D17
and on the rest of D16. The pin stays at 0.5.34 and the legacy build carries
upstream's shared-node fix as a ninth patch. The Candidate keeps its checks
where it is stricter than the Reference, and it collects an upload that no
record references. The evidence is in the
[upstream 0.5.35 to 0.5.37 note](research/2026-10-10-upstream-0.5.35-to-0.5.37.md)
and the [decision record](research/2026-10-12-repository-crate.md) §7; the
changes are listed in Appendix I.

Revision 6 follows unit 3, the read-only server. It states what the
restore run showed: the block order of the export, the two phases of
parameter validation and the shape of an `/xrpc/` path. It also lists what
`verify` checks and names the restore run as the oracle of §18.2. The
evidence is in the
[unit record](research/2026-10-13-read-only-server.md); the changes are
listed in Appendix J.

Revision 7 follows unit 4: writes, the event log, the firehose, blobs and
the simulation. It applies the owner's decisions of 2026-10-13 and states
what the writes run showed: what the Reference answers to data from
elsewhere, that it emits no event for an import, and how the Candidate
orders a commit and its event. The evidence is in the
[unit record](research/2026-10-13-writes-and-firehose.md); the changes are
listed in Appendix K.

Revision 8 follows the first part of unit 4b: the S3-compatible blob
store. It states the object names, how the Candidate uploads and what it
needs at the start, and one more answer of the Reference that the writes
run showed on that store. It also applies the owner's decisions of
2026-10-13 on DD-18 and on `#sync` after an import. The evidence is in the
[unit record](research/2026-10-13-s3-blob-store.md); the changes are listed
in Appendix L.

Revision 9 follows the ninth part of unit 5: the OAuth authorization
server, OAuth access tokens on the XRPC routes and read-after-write. It
states what the Candidate built of §11 and what it did not, and it changes
two rules to what the Reference does at the pin: a handle under `.test`,
and the signing algorithms that the metadata lists. Both changes await the
owner's word. The evidence is in the
[unit record](research/2026-10-13-oauth.md); the changes are listed in
Appendix M.

Revision 10 follows unit 6: the `linkjar` profile. It states how the
profile is built, what of the eight patches the Candidate does in another
way, and what is not built. The evidence is in the
[unit record](research/2026-10-13-linkjar-profile.md); the changes are
listed in Appendix N.

Revision 11 follows unit 7: the audit log. It states the entry and the
checkpoint as built, the bundle of an export, and what the log does not
cover yet. The evidence is in the
[unit record](research/2026-10-13-audit-log.md); the changes are listed in
Appendix O.

## Table of Contents

- [0. Conventions and Terminology](#0-conventions-and-terminology)
- [1. Goals and Non-Goals](#1-goals-and-non-goals)
- [2. Compatibility Baseline](#2-compatibility-baseline)
- [3. Architecture](#3-architecture)
- [4. HTTP and XRPC Surface](#4-http-and-xrpc-surface)
- [5. Authentication and Authorization](#5-authentication-and-authorization)
- [6. Repository Engine](#6-repository-engine)
- [7. Sequencer and Firehose](#7-sequencer-and-firehose)
- [8. Storage and Data Directory](#8-storage-and-data-directory)
- [9. Identity](#9-identity)
- [10. Accounts](#10-accounts)
- [11. OAuth Authorization Server](#11-oauth-authorization-server)
- [12. Extension Points](#12-extension-points)
- [13. The LinkJar Profile](#13-the-linkjar-profile)
- [14. Operations](#14-operations)
- [15. Security](#15-security)
- [16. Performance](#16-performance)
- [17. Parity and Conformance](#17-parity-and-conformance)
- [18. Cutover and Rollback](#18-cutover-and-rollback)
- [19. Simulation and Fault Injection](#19-simulation-and-fault-injection)
- [20. Audit Log](#20-audit-log)
- [21. Deployment Topologies](#21-deployment-topologies)
- [22. Sidecars](#22-sidecars)
- [Appendix A. XRPC Inventory](#appendix-a-xrpc-inventory)
- [Appendix B. Extension Traits (Informative)](#appendix-b-extension-traits-informative)
- [Appendix C. Configuration](#appendix-c-configuration)
- [Appendix D. Open Decisions](#appendix-d-open-decisions)
- [Appendix E. Changes in Revision 1](#appendix-e-changes-in-revision-1)
- [Appendix F. Changes in Revision 2](#appendix-f-changes-in-revision-2)
- [Appendix G. Changes in Revision 3](#appendix-g-changes-in-revision-3)
- [Appendix H. Changes in Revision 4](#appendix-h-changes-in-revision-4)
- [Appendix I. Changes in Revision 5](#appendix-i-changes-in-revision-5)
- [Appendix J. Changes in Revision 6](#appendix-j-changes-in-revision-6)
- [Appendix K. Changes in Revision 7](#appendix-k-changes-in-revision-7)
- [Appendix L. Changes in Revision 8](#appendix-l-changes-in-revision-8)
- [Appendix M. Changes in Revision 9](#appendix-m-changes-in-revision-9)
- [Appendix N. Changes in Revision 10](#appendix-n-changes-in-revision-10)
- [Appendix O. Changes in Revision 11](#appendix-o-changes-in-revision-11)

## 0. Conventions and Terminology

- **Reference**: the pinned upstream PDS, `@atproto/pds@0.5.34` at commit
  `7ca16cc6989f8247637615aca17c5abb911b8fb1` ([upstream.json](../upstream.json)),
  built by [`legacy/Dockerfile`](../legacy/Dockerfile) with the `production`
  patch profile
  at the patch revision named in §13 (the **patch pin**).
- **Stock Reference**: the same build with `PATCH_PROFILE=none`.
- **Candidate**: a build of LinkJar PDS under test.
- **Profile**: the set of extension implementations a Candidate runs with.
  Two profiles are defined: `stock` (§12) and `linkjar` (§13).
- **Harness**: the parity harness in §17.
- **Data directory**: the on-disk state in `PDS_DATA_DIRECTORY` (§8).
- **Operator**: whoever runs the server. LinkJar is one operator.
- **Project lexicons**: lexicons this project defines, published under the
  NSID authority `io.linkjar.pds.*`. Operator-private methods use
  `internal.*` on the reserved `.internal` TLD, the ecosystem convention.
- Section references of the form "SPEC §n" point to the LinkJar protocol
  specification in the `linkjar.io` repository. This document never changes
  that specification: hosted accounts are ordinary AT Protocol accounts and the
  client-side key model is unchanged.
- **Verify at pin** marked, in revision 1, a requirement taken from
  atproto.com whose exact behaviour in the Reference had not been confirmed.
  Units 0 and 1 confirmed each one from the pinned source and with a Harness
  scenario ([verify-at-pin.md](../parity/verify-at-pin.md), ids `VP-n`). No
  mark remains in this revision.
- **At the pin** introduces a statement of what the Reference does that a
  reader of the protocol documents would not expect. The Candidate does the
  same for C1, unless §2.3 lists the behaviour as a deliberate difference.

## 1. Goals and Non-Goals

Goals, in priority order:

- **G1 Interoperability.** Any conforming AT Protocol client, relay, AppView,
  labeler and PLC directory works with the Candidate as it works with the
  Reference. The official Bluesky app can sign in and post through it.
- **G2 Native extension points.** Every behaviour that LinkJar patches into
  the Reference today is a documented extension point with a stock default.
  LinkJar's behaviour is one extension crate, selected by configuration.
- **G3 Measured performance and reliability.** The Candidate is never slower
  than the Reference on any harness scenario and publishes its numbers (§16).
  Crash boundaries the Reference leaves open are closed (§7.4) and exercised
  by simulation (§19).
- **G4 Public.** The `stock` profile is a complete PDS for any operator,
  dual-licensed MIT or Apache-2.0, with a build an outside operator can use.
- **G5 In-place cutover.** The hosted LinkJar PDS moves from the Reference to
  the Candidate without changing hostnames, DIDs, signing keys, sessions or
  firehose cursors, and can roll back during a defined window (§18). A
  hostname change would require a PLC update for every account, which is why
  the hostname never changes.
- **G6 Accountability.** Every account and operator action is recorded in a
  tamper-evident audit log whose checkpoints leave the operator's control
  (§20).
- **G7 Scale by the protocol's own model.** The account and OAuth layer runs
  embedded in one node or as a standalone entryway over member hosts (§21).

Non-goals for version 1:

- Running a relay, AppView, feed generator or labeler.
- Multi-process service of one data directory. One process owns a data
  directory.
- Media resizing or transcoding. The PDS stores and serves blob bytes.
- Changing LinkJar's client-side encryption, key or record model.
- Atproto Spaces (permissioned repositories). Tracked in Appendix D, D13.
- Redis scratch storage.

## 2. Compatibility Baseline

### 2.1 Pins

- The lexicons under `lexicons/com/atproto/**` at the Reference commit, and
  the `app.bsky` lexicons the Reference serves locally or munges (§4.5), are
  vendored into the repository. Request and response types MUST be generated
  from those files. Hand-written types MUST NOT diverge from them. The
  generator MUST handle the `permission` and `permission-set` primary types
  and the `tid` and `record-key` string formats.
- LinkJar's own lexicons (`io.linkjar.account.*`, §13.3) are vendored from
  the patch pin. Project lexicons (§0) are authored in this repository.
- Upstream bumps change the pin in one commit, regenerate the types, and
  rerun the Harness against the new Reference image. The Spring 2026
  service-auth change (§5.4) and any Sync 1.1 removals land this way.

### 2.2 Definition of compatible

A Candidate is compatible when all of the following hold against the Reference
at the same pin:

- **C1 XRPC parity.** Every method classified Core, Deprecated or Proxy in
  Appendix A returns the same status code, the same `error` name, and the
  same response shape for the same request and state. Field order and
  server-generated identifiers are not compared (§17.3).
- **C2 Repository determinism.** The same sequence of writes against the same
  starting repository produces the same MST root CID, and, given the same
  `rev`, the same commit CID (§6.6).
- **C3 Firehose compatibility.** `subscribeRepos` frames decode to the same
  events with the same cursor semantics (§7).
- **C4 OAuth compatibility.** The official `@atproto/oauth-client-node` and
  `@atproto/oauth-client-browser` complete every flow in §17.4.
- **C5 Migration.** The AT Protocol account migration procedure works from
  Reference to Candidate and from Candidate to Reference.
- **C6 Stock equivalence.** With the `stock` profile, the Harness cannot
  distinguish Candidate from Stock Reference on any protocol response
  (XRPC, OAuth endpoints, firehose) except by the version string and the
  deliberate differences of §2.3. HTML pages
  are out of scope for C6: the Candidate's pages are server-rendered and the
  Reference's are a React bundle.

Divergence from the Reference is permitted in two ways only: through §12
extension points, in a non-stock profile; and through the deliberate
differences of §2.3, in every profile.

### 2.3 Deliberate differences

The Candidate differs from the Reference at the pin in the places below, in
every profile. Each is an owner's decision recorded in [plan.md](plan.md).
The Harness records each one as a difference between the Reference and the
Candidate (§17.3): the comparison fails when one is missing and when another
appears.

| Id | Request and state | Reference at the pin | Candidate | Decided |
|---|---|---|---|---|
| DD-1 | `createRecord` with a record key that exists (§6.3) | 500 `InternalServerError` | 400 `InvalidRequest` | 2026-10-09, D15 |
| DD-2 | `applyWrites` with a create whose record key exists (§6.3) | 500 `InternalServerError` | 400 `InvalidRequest` | 2026-10-09, D15 |
| DD-3 | `createAppPassword` with a name the account already uses (§5.2) | 500 `InternalServerError` | 400 `InvalidRequest` | 2026-10-09, D15 |
| DD-4 | A proxied request whose `atproto-proxy` value does not start with a DID (§4.4) | 500 `InternalServerError` | 400 `InvalidRequest` | 2026-10-09, D15 |
| DD-5 | A service token that an earlier successful request has spent (§5.4) | accepted again | 401 `BadJwt` | 2026-10-09, D16 |
| DD-6 | `importRepo` with a CAR whose tree references a node more than once (§6.1) | Stock Reference: 200, and the imported tree replaces the repository. Production Reference, with patch 104: 400 `InvalidRequest` | 400 `InvalidRequest`; nothing is written | 2026-10-12, D17 |
| DD-7 | `importRepo` with a CAR whose tree has a key at the wrong layer, keys out of order across nodes, or an empty node below the root (§6.1) | The first two: 200, and the imported tree replaces the repository. The third: 500 `InternalServerError` | 400 `InvalidRequest`; nothing is written | 2026-10-12, D17 |
| DD-8 | `importRepo` with a record block that is not canonical DAG-CBOR: map keys out of order, a float with a whole value, or `undefined` (§6.1) | 200, and the imported tree replaces the repository | 400 `InvalidRequest`; nothing is written | 2026-10-12, D17 |
| DD-9 | A record that references a blob with `size` zero or an empty `mimeType` (§6.3) | 200 | 400 `InvalidRequest` | 2026-10-12, D17 |
| DD-10 | An upload that no record references one hour after it arrived (§6.4) | kept without limit: the Reference has no collection | collected | 2026-10-12, D16 |
| DD-14 | `importRepo` with a commit that the account's key did not sign, or a commit for another account (§6.5) | 200, and the imported tree replaces the repository | 400 `InvalidRequest`; nothing is written | 2026-10-13 |
| DD-15 | `importRepo` with bytes that are not a CAR (§6.5) | 500 `InternalServerError` | 400 `InvalidRequest` | 2026-10-13, D15 |

A difference that is not in this table is a defect of the Candidate.

The writes run of the Harness (§17.1) records DD-1, DD-2, DD-7 to DD-9,
DD-14 and DD-15 on every run
([`writes/22-elsewhere`](../parity/scenarios/writes/22-elsewhere.scenario.ts),
`default/03-repo-writes`, `default/15-import`). DD-10 cannot be observed in
a run that does not last an hour; a test of the server holds it
(`crates/linkjar-pds/tests/writes.rs`), and the Reference's side is read from
its source, which has no collection of uploads
(`actor-store/blob/transactor.ts` at the pin). DD-3 is observed in
`default/02-sessions` of the writes run. DD-4 and DD-5 are observed when
the proxy is built.

Where both servers refuse, the message may differ: the Candidate names the
first defect it finds in a tree, and a tree can have several. The tree of
`default/15-import` shares nodes and also hangs a subtree below layer zero;
the Reference names the first and the Candidate the second.

The owner decided the differences below (D15, D17). No scenario has yet
recorded what the Reference answers in these states. The unit named writes
that scenario and moves the row into the table above with the Reference's
answer. Until then the Harness does not count the row.

| Id | Request and state | Candidate | Scenario in unit |
|---|---|---|---|
| DD-11 | Handle verification: two TXT records that name the same DID, and an HTTPS body of more than one line (§9.2) | the two records are one answer; the whole trimmed body is read | 5 |
| DD-12 | A DID document whose signing key has another `controller`, or whose PDS endpoint is more than scheme, host and port (§9.3) | refused | 5 |
| DD-13 | A commit of version 2 in a repository from elsewhere (§6.1) | refused | 8 |
| DD-16 | `createAccount` with the handle of a deactivated or taken-down account (§10.1). The pin answers 500 | 400 `InvalidRequest` | 5 |
| DD-17 | `createAccount` with the email of a deactivated or taken-down account (§10.1). The pin answers 500 | 400 `InvalidRequest` | 5 |
| DD-18 | `getBlob` of an upload that no record references, with blobs in the S3-compatible store (§6.4). The pin answers 500: it reads the object at its permanent name and does not map the store's `NoSuchKey`. On the disk store it answers 400 | 400 `InvalidRequest`, "Blob not found", on both stores | 4b |
| DD-19 | `importRepo` into an account that is active (§6.5). The pin appends no event, and a strict relay drops the account's next commit, which does not follow the relay's copy | `#sync` for the imported head, and the account database's root follows | 4b |

DD-11 and DD-12 cannot be observed before the server verifies a handle and
reads a signing key itself, which it first does in unit 5.

DD-6 holds against the Stock Reference only
([`default/15-import`](../parity/scenarios/default/15-import.scenario.ts)).
The production Reference carries upstream's fix as its ninth patch (§13)
and agrees with the Candidate; the stock build applies no patch.

## 3. Architecture

### 3.1 Crates

The repository root is a Cargo workspace (Appendix D, D4). Crate names
are provisional.

| Crate | Owns |
|---|---|
| `pds-types` | DID, handle, NSID, TID, record key, CID, AT URI syntax and validation with the limits of §6.1 and §9; the data model with a strict DAG-CBOR reader and writer that apply the decode limits; lexicon-generated request and response types. |
| `pds-lexicon` | Lexicon loading, record validation in the three modes of §6.3, data-model rules, permission-set resolution and cache (§5.5). |
| `pds-repo` | The repository engine, written in this repository (Appendix D, D2): MST, commit blocks, CAR read and write, commits from writes with the blocks a consumer needs to invert them, export, and the checks on a repository from elsewhere. It does no I/O and holds no key: blocks come from a block store, the signature from the caller's key, the revision from the caller's clock. |
| `pds-storage` | The domain-level storage seams of §8.4 (`ActorStore`, `AccountStore`, `SequencerStore`, `BlobStore`, `DidCache`, `AuditStore`), the SQLite implementations over the Reference schema, migrations, the actor LRU, the notes of commits in flight (`ops.sqlite`, §7.4), the fail points and the `StorageIo` seam of §19. |
| `pds-blob` | Blob stores: disk and S3-compatible; upload staging, hashing, quarantine; the rules of §6.4. |
| `pds-identity` | Keys and signatures on both curves, `did:key`, the DID-document contract of §9; DID resolution with cache, PLC client, handle resolution and verification, signing-key custody. |
| `pds-xrpc` | axum routing, XRPC request parsing, error mapping, rate limiting, proxying, read-after-write, CORS; the write path from a request to a stored and sequenced commit (§6.3), and the repair of §7.4. |
| `pds-sequencer` | The event log, firehose subscriptions, backfill, crawler notification, the lifecycle event sequences of §10.4. |
| `pds-account` | Account lifecycle, sessions, app passwords, email tokens, invites, admin, mail. Designed to run embedded or as an entryway (§21.3). |
| `pds-oauth` | The authorization server and the resource-server verifier: metadata, PAR, authorize, token, DPoP, client metadata, device sessions, server-rendered UI, account pages. Same two-mode design as `pds-account`. |
| `pds-audit` | The audit log of §20: entries, hash chain, checkpoints, proofs, the checkpoint record publisher, and the standalone verifier binary. |
| `pds-ext` | The extension traits of §12 and the `stock` implementations. |
| `pds-ext-linkjar` | The `linkjar` profile (§13). Compiled in behind the Cargo feature `linkjar`. |
| `pds-sim` | The simulation harness of §19: seeded plans, stops at the fail points, the oracles, the seed runner and the determinism meta-test. The fault-injecting VFS and the turmoil scenarios are not written yet (§19.2). |
| `linkjar-pds` | The binary: configuration, wiring, CLI (§14.2), telemetry, the ops listener (§22.1). |
| `parity/` | The Harness (§17), a TypeScript project driven by the official SDK. |

### 3.2 Runtime

- Tokio multi-threaded runtime; axum on hyper for HTTP/1.1 and WebSocket.
  TLS termination is the reverse proxy's job, as with the Reference.
- SQLite through a synchronous driver with the bundled library, run on a
  dedicated blocking pool, opened through the `StorageIo` seam so tests can
  substitute a fault-injecting VFS (§19). Each actor database has one writer
  at a time.
- Cryptography of the protocol from RustCrypto: `k256`, `p256`, `sha2`.
  Outbound TLS uses `rustls` with `aws-lc-rs` (decision of 2026-10-13).
  DAG-CBOR through the
  reader and writer of `pds-types`, which accept and produce canonical
  encoding only and enforce the decode limits of §6.1; `cid` and
  `multihash`.
- No `unsafe` outside vetted dependencies: `#![forbid(unsafe_code)]` in every
  workspace crate except the VFS shim in `pds-sim`, which is reviewed.
- Background work (sequencer crawl notifications, scheduled account deletion,
  blob garbage collection, mail outbox, DID-cache refresh, audit
  checkpointing, permission-set cache refresh) runs as supervised tasks in
  the same process and is observable through metrics and the ops listener.
- Time, randomness and I/O enter the server only through injectable
  providers so that §19 can replace them.

### 3.3 Determinism hooks

Dev mode (`PDS_DEV_MODE=true`) MUST allow injecting the clock, the TID
source and the random source, so the Harness can obtain identical `rev`
values from Reference and Candidate and §19 can replay a seed. Production
builds MUST ignore these hooks.

## 4. HTTP and XRPC Surface

### 4.1 Listeners and non-XRPC routes

Three listeners: the **public** listener (`PDS_PORT`), the **metrics**
listener (`PDS_METRICS_ADDRESS`, Prometheus text) and the **ops** listener
(`PDS_OPS_ADDRESS`, §22.1). The last two bind private addresses and MUST NOT
be reachable from the public hostname.

| Route | Behaviour |
|---|---|
| `GET /` | The Reference's plain-text server banner. |
| `GET /robots.txt` | As the Reference. |
| `GET /xrpc/_health` | `{"version": "<PDS_VERSION>"}`; 503 with `{"version", "error"}` when the account database is unavailable. |
| `GET /.well-known/atproto-did` | Resolves the `Host` header to a hosted actor and returns its DID as `text/plain`; 404 otherwise. |
| `GET /.well-known/oauth-authorization-server`, `GET /.well-known/oauth-protected-resource` | §11.1. |
| `/oauth/*`, `/account/*` | §11. |

The PDS MUST be served on its own registrable domain, not on a subdomain
shared with an OAuth client application, because blob bytes and the
authorization pages would otherwise share an origin with the app.

### 4.2 XRPC semantics

- Queries are `GET`, procedures are `POST`, subscriptions are WebSocket
  upgrades, all under `/xrpc/<nsid>`. A query also answers `HEAD`. Any
  other method is 400 `InvalidRequest`, "XRPC requests only supports GET and
  POST".
- The method name of a path is read as the Reference reads it: letters and
  digits in parts separated by single dots or hyphens, at least two
  characters, with an optional trailing slash. A name that is not an NSID
  still passes, and goes to the catch-all proxy. Any other path under
  `/xrpc/` is 400 `InvalidRequest`, "invalid xrpc path".
- Parameters and bodies are validated against the lexicon. Unknown query
  parameters are ignored. Invalid input returns 400 `InvalidRequest` with the
  Reference's message where the Harness compares messages.
- Query parameters are validated in two phases, and before the credential.
  First the query is read in the order it was written: an empty value is
  ignored, and a value for an integer or boolean parameter is converted or
  refused. Then the parameters are checked in the order the lexicon declares
  them: a required one must be present, a scalar must have one value, and
  each value must have its format and be in its range.
- Errors are `{"error": "<Name>", "message": "<text>"}` on every `/xrpc/`
  path, including upgrade failures. The Candidate MUST use the Reference's
  error names, including `AuthenticationRequired`, `ExpiredToken`,
  `InvalidToken`, `AccountTakedown`, `AccountDeactivated`, `RepoNotFound`,
  `RecordNotFound`, `BlobNotFound`, `InvalidSwap`, `MethodNotImplemented`,
  `RateLimitExceeded`, `UpstreamFailure`, `UpstreamTimeout`,
  `InternalServerError`.
- Status conventions: a request without a credential is 401 `AuthMissing`;
  a credential that cannot be read is 400 `InvalidToken`, and one past its
  expiry 400 `ExpiredToken`; a DPoP failure is 401 with `WWW-Authenticate`;
  the wrong HTTP method is 400 `InvalidRequest`; 413 `PayloadTooLarge` for
  bodies over the configured limit; 429 `RateLimitExceeded` with the
  rate-limit headers of §4.3 and `Retry-After`.
- At the pin a method with no local handler is never 501. It goes to the
  catch-all proxy of §4.4: 401 `AuthMissing` without a session, forwarded
  with one ([VP-6, VP-7](../parity/verify-at-pin.md)). `MethodNotImplemented`
  remains the answer of a build with no proxy target configured, which the
  Harness does not run.
- A plain route answers its own method only. Another method on it, and any
  path that is neither a plain route nor under `/xrpc/`, is 404 with the
  Reference's HTML page.
- Response bodies are `application/json` unless the lexicon declares another
  encoding (`getBlob`, `getRepo`, `getRecord` and `getBlocks` stream CAR or
  raw bytes).
- CORS ([VP-11](../parity/verify-at-pin.md)): every XRPC response carries
  `Access-Control-Allow-Origin: *`. A preflight answers 204 with
  `Access-Control-Allow-Methods: GET,HEAD,PUT,PATCH,POST,DELETE`, the
  requested headers echoed in `Access-Control-Allow-Headers`, and
  `Access-Control-Max-Age: 86400`. A response that carries rate-limit headers
  lists them in `Access-Control-Expose-Headers`. The OAuth endpoints set
  their own headers (§11.1).
- At the pin a read-after-write response that was overlaid (§4.5) carries
  `Atproto-Upstream-Lag`, the distance to the AppView in milliseconds, and
  not the AppView's `Atproto-Repo-Rev`. A response that needed no overlay
  passes `Atproto-Repo-Rev` through from the AppView.

### 4.3 Rate limits

Rate limits default on (`PDS_RATE_LIMITS_ENABLED`). The Candidate MUST
reproduce the Reference's limiters and their `RateLimit-Limit`,
`RateLimit-Remaining`, `RateLimit-Reset` and `RateLimit-Policy` headers:

| Limiter | Window | Points | Keyed by |
|---|---|---|---|
| `global-ip` | 5 minutes | 3,000 | client IP |
| `repo-write-hour` | 1 hour | 5,000 (create 3, put 2, delete 1) | DID |
| `repo-write-day` | 24 hours | 35,000 (same weights) | DID |
| `sync.getRepo` (exempt from `global-ip`) | 5 minutes | 6,000 | client IP |
| `repo.uploadBlob` | 24 hours | 1,000 | client IP |
| `server.createAccount` | 5 minutes | 100 | client IP |
| `server.createSession` | 5 minutes, and 24 hours | 30, and 300 | identifier and client IP together |
| `server.resetPassword` | 5 minutes | 50 | client IP |
| `server.requestPasswordReset` | 1 hour, and 24 hours | 15, and 50 | client IP |
| `server.deleteAccount` | 5 minutes | 50 | client IP |
| `server.requestEmailConfirmation`, `server.requestEmailUpdate`, `server.requestAccountDelete` | 1 hour, and 24 hours | 5, and 15 | DID |
| `identity.updateHandle` | 5 minutes, and 24 hours | 10, and 50 | DID |

This is the complete list at the pin ([VP-12](../parity/verify-at-pin.md)).
A response reports the limiter with the fewest points left:
`RateLimit-Policy` is `<limit>;w=<window in seconds>`. A 429 adds
`Retry-After` in seconds and exposes it to browsers.

`PDS_RATE_LIMIT_BYPASS_KEY` (presented in the `x-ratelimit-bypass` header)
and `PDS_RATE_LIMIT_BYPASS_IPS` bypass as in the Reference: no limiter runs
and no rate-limit header is set. Client IP comes
from the configured trusted-proxy policy, never from an unverified header.
Rate limits apply to proxied requests too.

### 4.4 Proxying

Requests for methods the Candidate does not serve locally are forwarded as the
Reference's pipethrough does:

- The caller MUST hold an authenticated session. At the pin a deactivated
  account still proxies, because an account that is migrating in is
  deactivated until it is activated; a taken-down account's sessions are
  refused.
- A method with no local handler goes to the configured AppView, whatever
  its namespace; `com.atproto.moderation.createReport` goes to the report
  service. `com.atproto.repo.getRecord` for a repository this server does
  not host is forwarded to the AppView without a credential.
- The target is the `atproto-proxy` header (`<did>#<service id>`) or the
  configured AppView for `app.bsky.*`. The target DID MUST resolve to a
  document with a `service` entry whose id matches the fragment; only
  `/xrpc/<nsid>` paths are forwarded.
- Forwarding mints a service-auth JWT (§5.4) for the target with `lxm` set
  to the method. At the pin the Reference sends the bare DID as `aud` while
  checking scopes against `<did>#<service id>` ([VP-2](../parity/verify-at-pin.md));
  the Candidate does the same for C1 and switches to the fragment form when
  the pin moves.
- `atproto-accept-labelers` passes through on the request and
  `atproto-content-labelers` on the response. At the pin the request header
  is not parsed: a malformed value is forwarded and the upstream decides.
- An XRPC error from the upstream comes back with its status and body. A
  failed upstream is 502 `UpstreamFailure`. A proxy target whose service id
  the DID document lacks, or that has no service id, or whose DID does not
  resolve, is 400 `InvalidRequest`. At the pin a value that does not start
  with a DID is 500 `InternalServerError`; the Candidate answers 400
  `InvalidRequest` (§2.3, DD-4).
- Header filtering, timeouts (`PDS_PROXY_*`), response size caps, retry
  policy and HTTP/2 preference follow the Reference configuration.
- Procedures are proxied only where the Reference proxies them.

### 4.5 Read-after-write

The `app.bsky` methods the Reference serves locally or munges (preferences,
push registration, profile and feed reads with local record overlay) are
classified in Appendix A. The Candidate SHOULD implement them in version 1
and MUST implement them before the official Bluesky app is claimed compatible.
The overlay applies when the AppView's `Atproto-Repo-Rev` is a revision at
or after which this repository holds a record; the response then carries
`Atproto-Upstream-Lag` (§4.2). A feed is recognised as the account's own by
its first item.

## 5. Authentication and Authorization

### 5.1 Legacy sessions

- Access and refresh tokens are HS256 JWTs signed with `PDS_JWT_SECRET`,
  with `typ` `at+jwt` for access and `refresh+jwt` for refresh tokens,
  `scope` in `com.atproto.access`, `com.atproto.refresh`,
  `com.atproto.appPass`, `com.atproto.appPassPrivileged`,
  `com.atproto.signupQueued`, `com.atproto.takendown`; `sub` the DID; `aud`
  the service DID; `jti` on refresh tokens.
- An access token lives 120 minutes and a refresh token 90 days. Refresh
  rotates the token. A rotated refresh token stays valid for a grace period
  of two hours and leads to the same successor, so two racing clients end
  with one session; after the grace period it is `ExpiredToken`
  ([VP-9](../parity/verify-at-pin.md)). A password reset and
  `deleteSession` end the session at once.
- The Candidate MUST accept tokens the Reference minted with the same secret,
  so cutover keeps users signed in.

### 5.2 Passwords and app passwords

- Password hashes use the Reference format: `<salt hex>:<scrypt hex>`, a
  random 16-byte salt, Node's default scrypt parameters (N 16384, r 8, p 1)
  and a 64-byte derived key. New passwords are at most 256 bytes; stored
  hashes of older passwords up to 512 bytes still verify.
- The Candidate MUST NOT change the hash format while §18's rollback window is
  open.
- App passwords use the Reference's `xxxx-xxxx-xxxx-xxxx` form, are hashed
  the same way, and carry the privileged flag.
- At the pin `createAppPassword` with a name the account already uses is 500
  `InternalServerError`; the Candidate answers 400 `InvalidRequest` (§2.3,
  DD-3).

### 5.3 Admin

HTTP Basic authentication as `admin` with `PDS_ADMIN_PASSWORD` grants the
admin role for `com.atproto.admin.*`, `createInviteCode`,
`createInviteCodes` and the privileged account methods. It is the
break-glass path; the console (§22) authenticates operators by account (§12.11).

### 5.4 Service auth

- Inbound: an ES256K or ES256 JWT with `typ: JWT`, signed by the caller's
  repository signing key, `iss` the caller DID, `aud` our service DID with or
  without the `#atproto_pds` fragment, `iat` present, `exp` short, `lxm`
  equal to the method NSID. A wrong `aud` is 401 `BadJwtAudience`; a wrong
  or missing `lxm` is 401 `BadJwtLexiconMethod`
  ([VP-3](../parity/verify-at-pin.md)). At the pin two methods accept
  service auth, `createAccount` and `uploadBlob`, and `uploadBlob` reads a
  token without `lxm` as a session token, which fails as `InvalidToken`. At
  the pin `jti` is not checked for replay; the Candidate checks it (next
  item). An optional `kid` header names the verification-method
  fragment, default `#atproto`; only expected key types are accepted. The
  key comes from the resolved DID document (§9.3).
- Replay: a service token is good for one successful request. The
  Candidate MUST refuse a token that an earlier request has spent, until the
  token's `exp`, with 401 `BadJwt` (§2.3, DD-5). A request spends its token
  when the server answers it with a 2xx status. The server reserves the key
  when it verifies the token and releases it when the answer is not 2xx, so
  a caller can retry a failed request with the same token, and two
  concurrent uses cannot both pass. The key is `iss` with `jti`; a token
  without `jti` is keyed by the SHA-256 of the token, so it cannot be
  replayed either. The cache is bounded, per process, and empty after a
  restart. Where more than one process verifies service tokens (§21), a
  token can be used once in each until the shared tier carries the cache.
  The unit that builds the cache sets its bound and adds it to Appendix C.
- Outbound: the same shape, signed with the actor's signing key, `exp` of
  `iat` plus 60 seconds as the Reference's default ([VP-4](../parity/verify-at-pin.md)),
  `lxm` set, `aud` per §4.4, for proxying and for `getServiceAuth`.
  `getServiceAuth` accepts a requested expiry of up to one hour with `lxm`
  and up to one minute without, and answers `BadExpiration` otherwise; it
  refuses the account-management methods.

### 5.5 OAuth sessions and permissions

DPoP-bound access tokens issued by §11 are verified by the resource-server
half of the same crate: signature against the server key, `cnf.jkt` against
the DPoP proof, nonce, scope.

- **Transitional scopes.** `transition:generic` grants record writes, blob
  uploads, preferences, proxying and service-auth minting, and no account
  management and no `chat.bsky.*`; `transition:chat.bsky` requires
  `transition:generic`; `transition:email` exposes the email address and its
  confirmation status on `getSession`. All three are slated for upstream
  removal; the Candidate accepts them while the pin does.
- **Permissions.** The Candidate enforces the permission grammar of the
  atproto permission specification: `rpc` covers proxying and
  `getServiceAuth`; `account:email` distinguishes read from manage;
  `account:repo?action=manage` gates `importRepo`; `identity:handle` and
  `identity:*`; `blob:<accept>` gates `uploadBlob` by MIME glob; `aud` and
  `lxm` may not both be wildcards; unknown resources and parameters inside a
  set are ignored; `include:` sets may reference only their own NSID group or
  its children.
- **Permission sets** resolve through `_lexicon.<reversed authority>` DNS TXT
  to a DID, then the `com.atproto.lexicon.schema` record keyed by NSID, with
  no DNS hierarchy walk. The cache is shared across accounts, stale at most
  24 hours, kept at least the access-token lifetime, expired after 90 days.
  An authorization request fails if a set is unresolvable and uncached.
  Permissions are recomputed on refresh. The Reference at the pin has no
  handler for `com.atproto.lexicon.resolveLexicon` ([VP-6](../parity/verify-at-pin.md));
  the Candidate answers it through the proxy as the Reference does and
  serves it natively once the pin does.

## 6. Repository Engine

### 6.1 Data model and limits

- Commits are version 3: `did`, `version`, `data` (MST root), `rev` (TID),
  `prev` (present in the CBOR and null), `sig`.
- The MST follows the AT Protocol repository specification and the upstream
  interop vectors: keys are `<collection>/<rkey>` of at most 1,024 bytes,
  layer by leading zero bits of SHA-256, canonical DAG-CBOR nodes. Entries
  per node and tree depth are bounded: a node block obeys the decode limits
  below, so it holds at most 131,072 entries in at most 1 MiB, and a key's
  layer is at most 128, so a tree is at most 129 nodes deep. A node or tree
  beyond a bound is rejected as invalid.
- A tree that the server did not build in the same call is read through
  one guard: no node is entered twice, and no more nodes are entered than
  the input has blocks. The engine refuses a tree that reaches one node by
  more than one path, on import, comparison and export. When it imports a
  tree it also checks the whole of it: every key at the layer of its hash,
  keys ascending across node boundaries, no empty node below the root, and
  every pointer a `dag-cbor` SHA-256 CID. The Reference at the pin makes
  none of these checks. The Candidate keeps them (decision of 2026-10-12,
  D17; §2.3, DD-6 and DD-7).
- CIDs are CIDv1, `dag-cbor` with SHA-256 for nodes and records, `raw` with
  SHA-256 for blobs. Any other codec or hash makes the object invalid.
- Signatures are secp256k1 or P-256 depending on the actor's key. Low-S is
  required for both curves. Verification uses library routines only.
- Decode limits, applied to every CBOR and JSON input before any other
  processing: CBOR record 1 MiB, JSON record 2 MiB, nesting depth 32,
  container elements 131,072, object key 8 KiB, CID binary 100 bytes,
  integers within plus or minus 9,007,199,254,740,991, firehose frames
  parsed at the 5 MB frame limit. Floats are rejected.

### 6.2 Record keys and revisions

- Repository paths use `A-Za-z0-9/._-~`. Record keys are 1 to 512
  characters from `A-Za-z0-9._:~-`; `.` and `..` are forbidden; `%` is
  reserved. NSID key types declared in the lexicon are enforced.
- TIDs are 53 bits of microseconds plus a 10-bit clock id with the top bit
  zero, base32-sortable; a generator never repeats a value and stays
  monotonic across same-microsecond generation.
- `rev` is derived from wall-clock time and MUST be strictly greater than the
  previous commit's `rev` for that repository, even across restarts.
  Consumers drop revisions more than a few minutes in the future, so a
  skewed clock is a startup failure in production.

### 6.3 Writes

- `createRecord`, `putRecord`, `deleteRecord`, `applyWrites` (at most 200
  operations) with `swapCommit` and `swapRecord` returning `InvalidSwap`.
- Validation modes: `validate=true` validates against the lexicon and fails
  if the lexicon is unknown and unresolvable; `validate=false` applies
  data-model validation only, even for known lexicons; unset validates known
  lexicons and accepts unknown ones after data-model validation. The success
  response reports whether lexicon validation happened, as the Reference's
  does.
- Data-model validation always: no floats; unknown `$`-prefixed keys
  ignored; blob references with `raw` CIDs, `size` greater than zero and a
  non-empty `mimeType`; the legacy blob form is accepted on read and never
  written; datetimes with uppercase `T`, a timezone, no `-00:00`, whole
  seconds.
- A record that references a blob the server does not hold is rejected.
- A commit MUST NOT exceed 2,000,000 bytes of `blocks` or contain a record
  block over 1,000,000 bytes; a write that would do so is refused with
  `InvalidRequest`. The resulting firehose frame stays under 5 MB.
- A commit writes the new blocks, record rows, blob references and the
  sequencer event, then publishes. The commit path MUST update the MST
  incrementally and MUST NOT rewrite unchanged records or blocks.
- A write that changes nothing makes no commit and no firehose event.
- At the pin `createRecord` on a key that exists, and a batch that contains
  such an operation, answer 500 `InternalServerError`. The Candidate answers
  400 `InvalidRequest` (§2.3, DD-1 and DD-2). In both servers nothing is
  written: no commit, no firehose event, and no other operation of the
  batch. A batch that updates or deletes a key with no record is 500 in
  both servers: the owner's decision names the create only.
- A refused write carries the Reference's message: the JSON path and the
  wording of `@atproto/lex-schema` at the pin. Where a value breaks several
  rules the Candidate may name another of them than the Reference, because
  it checks the properties of an object in sorted order. A body that is not
  JSON gets the Candidate's own message.
- Writes are refused for deactivated, taken-down and deleted repositories with
  the Reference's errors. A profile MAY refuse writes when the account's
  handle no longer verifies (§12.4).

### 6.4 Blobs

- Uploads stream to staging while hashing; the body length MUST equal
  `Content-Length` or the upload is rejected; `Content-Type` is expected;
  the declared MIME type is reconciled with sniffed content as in the
  Reference; `PDS_BLOB_UPLOAD_LIMIT` (default 5 MiB) caps size. The limit is
  a superset of every supported lexicon's blob constraints.
- An upload needs `Content-Type`; a body over the limit is 413
  `PayloadTooLarge`; an empty body is accepted. The stored type follows the
  content when the declared type contradicts it.
- An unreferenced upload is not downloadable and is absent from `listBlobs`.
  A blob belongs to the account that uploaded it: a record that references
  another account's upload is `BlobNotFound`.
- A blob becomes durable when a record references it. At the pin there is no
  grace period: the commit that removes the last reference, by a delete or by
  an update, removes the blob with it, and `getBlob` stops serving it within
  seconds ([VP-10](../parity/verify-at-pin.md)). A blob that another record
  still references stays. The Candidate follows the pin here and keeps no
  grace period. An upload that no record ever references is never collected
  at the pin; the Candidate MUST collect it one hour after it arrived
  (decision of 2026-10-12, D16; §2.3, DD-10). Account deletion deletes all
  blobs within the scheduled purge. `listMissingBlobs` reports referenced blobs the store lacks.
- The stored type follows the content for the formats the Candidate
  recognises: the image, video and audio types the vendored lexicons accept
  and a few containers. The Reference asks the `file-type` package, which
  knows more formats; for any other content the declared type stands in the
  Candidate.
- With blobs in the S3-compatible store the pin answers 500 to `getBlob`
  of an unreferenced upload, where it answers 400 on the disk store. The
  Candidate answers 400 on both (§2.3, DD-18; a case of D15, decided
  2026-10-13).
- The Candidate serves a blob only when its index row is permanent. A commit
  that stops after it moved an upload into place and before it stored the
  reference leaves the file in place and the row waiting; no record
  references such a blob, and it is collected with the other uploads.
- `getBlob` serves by CID with `Content-Type`, `Content-Length`,
  `Content-Security-Policy: default-src 'none'; sandbox` and
  `X-Content-Type-Options: nosniff`. A blob the account does not hold is 400
  `InvalidRequest`, "Blob not found". For a deactivated or taken-down
  account it serves nothing to others (`RepoDeactivated`, `RepoTakendown`)
  and still serves the owner; a blob that was taken down on its own is not
  found.
- The PDS performs no resizing or transcoding.

### 6.5 Preferences and imports

- `app.bsky.actor.getPreferences` and `putPreferences` store in the actor
  database and apply the Reference's scope restrictions for app passwords and
  OAuth scopes.
- `importRepo` accepts a CAR up to `PDS_MAX_REPO_IMPORT_SIZE` when
  `PDS_ACCEPTING_REPO_IMPORTS` is set and the caller's permissions allow it.
  The import verifies structural completeness, that the commit names the
  account, and the commit signature against the account's own key. It
  ignores unreferenced and duplicate blocks and tolerates any block order.
  Repeated imports are idempotent. The Reference at the pin checks neither
  the account nor the signature: it accepts a commit that any key signed
  and a commit for another account, and replaces the repository with it
  (§2.3, DD-14).
- As at the pin, the stored revision of an imported repository is a new
  one, whatever the commit says. An account that moves in is deactivated
  while it imports: the import appends no event to the log and does not
  write the root that the account database holds, as at the pin, and the
  activation emits `#sync` (§10.4).
- An import into an account that is active MUST append `#sync` for the
  imported head and write the account database's root (decision of
  2026-10-13; §2.3, DD-19). The event carries the commit block and names
  the revision of that block, which a relay checks; the store and the
  account database name the new stored revision. The commit, the note and
  the event follow the order of §7.4, and the repair of §7.4 states the
  same head when the server stops between them. The pin appends nothing
  here.

### 6.6 Determinism

Given the same starting repository and the same ordered writes with explicit
record keys, the Candidate MUST produce the same MST root CID as the
Reference. Given also the same `rev` (§3.3), it MUST produce the same commit
CID. The Harness checks both after every write scenario.

`sync.getRepo` and `getCheckout` write the stored blocks from the newest
revision down, then by CID down, as the Reference does: for the same data
directory the Candidate's export MUST be the same bytes. `sync.getRecord`
writes the commit, the record when it exists, and the tree nodes on the
path to the key from the root down. `sync.getBlocks` writes the blocks in
the order the store returns them, in a CAR without a root. The `export-car`
command writes the tree-ordered export that `@atproto/repo` gives a library
caller.

### 6.7 Signing-key rotation

Rotating an actor's signing key creates a new commit signed with the new
key; `data` need not change. The commit is sequenced and the identity
change emitted per §10.4.

## 7. Sequencer and Firehose

### 7.1 Event log

Events are appended to `repo_seq` with a monotonically increasing `seq` in
1 to 2^53 exclusive, never reused, the DID, the event type, the DAG-CBOR body
and the sequencing time. Types: `#commit` (with `ops` carrying `prev` CIDs,
the CAR slice, `blobs` always `[]`, `since`, `rev`, `prevData`, `tooBig`
always `false`), `#sync`, `#identity` (whose `handle` may be
`handle.invalid`), `#account` and `#info`.

The CAR slice of a `#commit` contains the new commit as `roots[0]`, every
MST node new in this revision, the extra nodes needed for operation
inversion, every created or updated record, no deleted record bodies, and
every required block even if it appeared earlier in the repository's
history.

If the sequence state is ever reset, the next `seq` MUST start with a
healthy margin above any value previously emitted.

### 7.2 Framing and connection

Each message is two DAG-CBOR objects: a header `{"op": 1, "t": "#<type>"}`
and the body. Errors are `{"op": -1, "error": "<Name>", "message": "<text>"}`
followed by close. Frames never exceed 5 MB.

An error frame is followed by a close with code 1008. The server sends no
ping: at the pin a ping is the client's to send. At the pin
`subscribeRepos` has no HTTP route: a request without an upgrade, with any
method, falls to the catch-all proxy and is 401 `AuthMissing`. Frames the
client sends are ignored. The public listener serves `wss://` behind the
proxy.

### 7.3 Cursor semantics

| Request | Behaviour |
|---|---|
| no `cursor` | live events only |
| `cursor=0` | the whole retained window from the oldest event, then live |
| `cursor` within the retained window | backfill from `cursor`, then switch to live without gaps or duplicates |
| `cursor` above the latest `seq` | error `FutureCursor`, close |
| `cursor` older than `PDS_REPO_BACKFILL_LIMIT_MS` (default 1 day) | `#info` with `OutdatedCursor`, then stream from the earliest retained event |
| subscriber falls behind `PDS_MAX_SUBSCRIPTION_BUFFER` events (default 500) | error `ConsumerTooSlow`, close |

Backfill is exclusive of `cursor`: the Reference selects `seq > cursor`
(`sequencer.ts` at the pin, [VP-1](../parity/verify-at-pin.md)). The
event-stream specification's "greater-or-equal" wording is noted as the
discrepancy; the Candidate follows the Reference for C1.

`cursor=0` is a cursor like any other: when events older than the window
exist, the stream starts with `#info` `OutdatedCursor`. A cursor that is not
an integer is an `InvalidRequest` error frame. A cursor at the head replays
nothing.

### 7.4 Durability and crash boundaries

- A commit's sequencer row MUST become durable in the same step as the commit,
  or the Candidate MUST detect on startup every repository whose head is ahead
  of its last sequenced event and emit a `#sync` event for it. The Reference
  can lose an event at this boundary; the Harness injects a crash here and
  requires the relay to converge. Sync 1.1 defines `#sync` as the repair for
  exactly this state.
- The Candidate takes the second way. A commit and its event are in two
  files, so no transaction covers both. Before it commits the actor
  transaction, the server notes the account and the revision in
  `ops.sqlite`; it appends the event after the commit is durable, and then
  removes the note. At startup, before the first request, it reads the
  notes: where the repository's head is the noted revision and the log has
  no event of that revision, it appends `#sync` with the head. The
  Reference appends the event first and commits second, so its failure at
  this boundary is an event for a commit that does not exist. The note
  names a commit that stopped in this process; a log or a store that lost
  its tail to a power failure is not found this way (§19.2).
- Account status changes, handle changes and deletions emit their events in
  the same transaction as the state change.
- Frames are served only from the durable log in `seq` order. The in-process
  broadcast carries a wakeup, never a frame, so a lost wakeup costs latency
  and a duplicate wakeup costs an empty query, and neither can cost an event.
- At the pin the Reference feeds live subscribers from a poll of the log that
  backs off to one second when idle. An event therefore reaches a quiet
  stream up to a second after its commit, and a subscription opened just
  after a write still receives that write. The Candidate is not required to
  reproduce the delay; a consumer must not depend on it.

### 7.5 Fan-out

- Each event is encoded once and shared across subscribers by reference.
- Each subscriber has a bounded queue. Backfill reads the log in pages; it
  never materialises the whole range in memory. A page ends after 500
  events or 4 MiB of event bodies.
- A subscriber is too slow (§7.3) when it was at the head of the log and
  more events than `PDS_MAX_SUBSCRIPTION_BUFFER` were appended while it took
  the frames of its last read. A subscriber that waited at the head is idle,
  and a backfill is not held to the buffer; the Reference's buffer has the
  same two exceptions.
- Keepalive pings and idle timeouts match the Reference.
- The ops listener (§22.1) reports every subscriber's cursor, lag and queue
  depth and can disconnect one.

### 7.6 Crawlers

At the pin the Reference calls `com.atproto.sync.requestCrawl` on every host
in `PDS_CRAWLERS`, with its bare hostname, when it sequences the first event
after a start, and then at most once in twenty minutes. It sends nothing at
startup. The Candidate does the same. Inbound `requestCrawl` and
`notifyOfUpdate` have no handler and go to the catch-all proxy (§4.2).

## 8. Storage and Data Directory

### 8.1 Layout

The Candidate MUST use the Reference's layout so that the backup, recovery
and monitoring tooling in `linkjar/infra` keeps working:

| Path | Content |
|---|---|
| `$PDS_DATA_DIRECTORY/account.sqlite` | accounts, sessions, OAuth, invites, email tokens, extension tables |
| `$PDS_DATA_DIRECTORY/sequencer.sqlite` | `repo_seq` |
| `$PDS_DATA_DIRECTORY/did_cache.sqlite` | DID document cache |
| `$PDS_DATA_DIRECTORY/audit.sqlite` | the audit log (§20), Candidate-only |
| `$PDS_DATA_DIRECTORY/ops.sqlite` | the notes of commits in flight (§7.4), and operator state the console writes (§22.1): abuse lists, rate multipliers, job state. Candidate-only |
| `$PDS_ACTOR_STORE_DIRECTORY/<first two hex of sha256(did)>/<did>/store.sqlite` | the actor's repository, records, blob index, preferences |
| the sibling `key` file | the actor's signing key, byte-identical to the Reference's format |
| `$PDS_ACTOR_STORE_DIRECTORY/reserved_keys/<did>` | reserved signing keys |
| `$PDS_BLOBSTORE_DISK_LOCATION/<did>/<cid>` | stored blobs; uploads stage under `tempt/<did>/` (`PDS_BLOBSTORE_DISK_TMP_LOCATION` overrides) and taken-down blobs move to `quarantine/<did>/<cid>`, both siblings of the blob root |

With `PDS_BLOBSTORE_S3_BUCKET` set, blobs are objects of that bucket and not
files. The names are the Reference's (`packages/aws/src/s3.ts` at the pin),
so that a bucket the Reference wrote is served as it is:

| Object | Content |
|---|---|
| `blocks/<did>/<cid>` | a stored blob |
| `tmp/<did>/<key>` | an upload that waits for a record |
| `quarantine/<did>/<cid>` | a taken-down blob |

The Candidate signs each request with AWS Signature Version 4 and the
SHA-256 of the body. An upload goes to the store as the client sends it:
one `PutObject` below 8 MiB, and a multipart upload in parts of 8 MiB
above, so the server holds at most one part of each upload in memory and
writes none of it to disk. An upload that ends early leaves no object and
no part. A staged upload becomes permanent by a copy and a delete, as at
the pin; the move is safe to repeat. A request that fails to connect, or
that the store answers with 429, 500, 502, 503 or 504, is sent three times
at most. The timeouts are the Reference's: 5 seconds to connect,
`PDS_BLOBSTORE_S3_UPLOAD_TIMEOUT_MS` (20 seconds) for one upload from its
first byte to its last, and `PDS_BLOBSTORE_S3_REQUEST_TIMEOUT_MS` for a
request that stalls, by default the upload timeout held between 6 and 15
seconds. `verify` lists the objects of each account and does not ask for
each blob (§18.2).

The individual database locations are overridable by the same variables as
the Reference (Appendix C). Candidate-only state lives only in the files
marked Candidate-only, never in the account, sequencer, DID-cache or actor
databases, so that §8.2 holds.

### 8.2 Schema

- **Schema version 1 is the Reference schema** for the actor, sequencer and
  DID-cache databases, and the Reference schema plus the `linkjar` profile's
  migrations (`007a` to `007d`, §13) for the account database. The Candidate
  MUST serve a data directory the Reference wrote, and the Reference MUST be
  able to serve a data directory the Candidate wrote, until §18 closes the
  rollback window. The Reference ignores the Candidate-only files.
- Migrations are forward-only and run by the `migrate` command (§14.2), never
  implicitly at serve time in production. The Candidate records applied
  migrations in the Reference's migration table with the Reference's names.
- Candidate-only migrations of the shared databases MUST require the explicit
  operator flag of §18.3.
- Later schema work (for example moving blocks out of SQLite rows) is a
  versioned change after the window closes and is out of scope here.

### 8.3 SQLite behaviour

- WAL journal mode; `PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT` hands
  checkpointing to the backup owner (Litestream), as documented in the
  hosting study.
- Actor databases are opened on demand and kept in an LRU of
  `PDS_ACTOR_STORE_CACHE_SIZE` entries (default 100). One writer per actor is
  serialised in-process; readers use separate connections.
- Busy timeouts and `synchronous` level match the Reference. Every file that
  is created or renamed is followed by a directory sync.
- The process takes an exclusive lock file in the data directory at startup
  and refuses to start if another server holds it (§18.4).

### 8.4 Storage seams

Storage is reached only through domain-level traits, each with a transaction
type the extension hooks of §12 join:

| Seam | Owns | Version 1 implementation | Swappable |
|---|---|---|---|
| `ActorStore` | blocks, records, blob index, preferences, the actor's root | SQLite per actor, Reference schema | No in version 1 |
| `AccountStore` | accounts, credentials, sessions, OAuth state, invites, tokens, extension tables | SQLite, Reference schema | Yes: Postgres after cutover (§21.2) |
| `SequencerStore` | the event log and its cursor queries | SQLite, Reference schema | Yes, with the `AccountStore` |
| `BlobStore` | blob bytes, staging, quarantine | disk or S3-compatible | Yes, by configuration |
| `DidCache` | DID documents with TTLs | SQLite | Yes, with the `AccountStore` |
| `AuditStore` | §20 entries and checkpoints | SQLite, `audit.sqlite` | Yes, with the `AccountStore` |

Rules:

- A seam is defined by behaviour, not by SQL. The Postgres implementation of
  the shared tier MAY use a different schema.
- A backend combination that is compiled but unsupported MUST refuse to
  start with a message naming the gap. There are no silent fallbacks.
- Switching the shared tier is the `migrate-storage` command (§14.2): copy
  under a maintenance window, verify row counts and the last checkpoint,
  then switch. Actor stores move one actor at a time under a short
  per-actor write lease with the MST root as the oracle (§18.2).

## 9. Identity

### 9.1 DID methods and PLC

- `did:plc` for new accounts: the genesis operation is signed with the
  service rotation key (`PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` or the
  KMS variant) and includes `PDS_RECOVERY_DID_KEY` and any user-supplied
  recovery key in the Reference's order. `did:web` accounts are accepted for
  migration only, at hostname level, with `localhost` and ports only in
  development.
- DIDs are at most 2048 characters and case-sensitive. The server
  distinguishes invalid syntax, unsupported method and resolution failure in
  its errors.
- `requestPlcOperationSignature` (email token), `signPlcOperation`,
  `submitPlcOperation` (which runs the Reference's safety checks before
  forwarding), `getRecommendedDidCredentials` and
  `updateAccountSigningKey` behave as the Reference. After an account
  migrates away, the Candidate assists PLC recovery for 72 hours unless the
  account was deleted.
- The rotation key belongs in a KMS or HSM. The recovery key's private half
  is kept offline by the operator; the server never holds it.

### 9.2 Handles

- Syntax per the handle specification; at most 253 characters, effectively
  244 because of the `_atproto.` prefix. The TLDs `.alt`, `.arpa`,
  `.example`, `.internal`, `.invalid`, `.local`, `.localhost` and `.onion`
  are refused; `.test` is accepted, as at the pin, where
  `default/11-identity` verifies a custom domain under it (revisions 1 to 8
  accepted it only with `PDS_DEV_MODE`; the owner has not confirmed the
  change); `handle.invalid` is the only `.invalid` value and only as a
  marker.
- The Reference's reserved and explicit-slur lists are vendored.
  `PDS_SERVICE_HANDLE_DOMAINS` defines hosted suffixes.
- Custom domains verify through DNS TXT at `_atproto.<handle>` with a value
  starting `did=`, or through HTTPS `/.well-known/atproto-did` on port 443
  with a 2xx response whose body is stripped of surrounding whitespace and
  with a bounded redirect count. More than one valid TXT record with
  different DIDs fails. On a conflict between DNS and HTTPS, DNS wins.
  `PDS_HANDLE_BACKUP_NAMESERVERS` is consulted as in the Reference.
- `updateHandle` writes the PLC operation first and then the local state,
  and emits `#identity`. `resolveHandle` answers for hosted handles and
  falls back to the AppView.

### 9.3 DID documents and cache

- The resolver reads a DID document as follows: the claimed handle is the
  first `at://` entry in `alsoKnownAs`; the signing key is the first
  `verificationMethod` whose id ends `#atproto`, with type `Multikey` and
  `controller` equal to the DID, accepting the legacy
  `EcdsaSecp256k1VerificationKey2019` and
  `EcdsaSecp256r1VerificationKey2019` forms during the transition; the PDS
  endpoint is the first `service` whose id ends `#atproto_pds`, with type
  `AtprotoPersonalDataServer` and an endpoint of scheme, host and port only.
  Relative and fully qualified ids are both accepted.
- `did_cache.sqlite` with `PDS_DID_CACHE_STALE_TTL` and
  `PDS_DID_CACHE_MAX_TTL`; stale entries refresh in the background. A
  service-auth signature that fails to verify triggers one refresh before the
  request is rejected.

### 9.4 Keys

Per-actor secp256k1 signing keys in the `key` file; reserved keys via
`reserveSigningKey`; keys are zeroised when dropped.

## 10. Accounts

### 10.1 Lifecycle

- **Creation.** `createAccount` with handle, email, password, invite code,
  optional `did` and `plcOp` for migrations, and the `signupQueued` path
  (`temp.checkSignupQueue` always reports activated). Creation of an existing
  DID through service auth creates a deactivated account. The §12.2 gate runs
  before any PLC or storage write.
  As of the first part of unit 5 the Candidate creates an account that
  asks for a hosted handle: the checks of `createAccount.ts` at the pin in
  its order and with its messages, a `did:plc` genesis operation signed
  with `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`, the actor store with
  an empty signed repository, the account rows with the first session, and
  the four events of §10.4. Without that variable the server creates no
  account. Not built yet: an account that brings its DID, which needs
  service auth; the Reference's list of disposable email domains and its
  slur check on handles; and the tombstone of a DID whose account rows
  could not be written. The email address is checked for the common shape
  of an address and not with `@hapi/address`.
- **Email tokens.** Confirm email, reset password, update email, delete
  account and PLC operation tokens use the Reference's format, lifetimes and
  single-use rules.
- **Status.** `active`, `deactivated` (with `deleteAfter`), `takendown`,
  `suspended`, `deleted`; `getRepoStatus` also knows `desynchronized` and
  `throttled` as values it may report. `checkAccountStatus`,
  `deactivateAccount`, `activateAccount` (verifying the published DID
  document) follow the Reference.
- **Deletion.** `requestAccountDelete` and `deleteAccount` schedule the
  purge of the actor directory, blobs, sessions and extension rows, and emit
  `#account` with the Reference's status. Admin `deleteAccount` does the same
  without a token.

### 10.2 Invites, admin, moderation

- **Invites.** `PDS_INVITE_REQUIRED`, `PDS_INVITE_EPOCH`,
  `PDS_INVITE_INTERVAL`, creation, listing, disabling and per-account
  allowances as the Reference.
- **Admin.** `getAccountInfo(s)`, `getSubjectStatus`, `updateSubjectStatus`
  (takedown with blob quarantine), `updateAccountEmail`, `updateAccountHandle`,
  `updateAccountPassword`, `sendEmail`, and invite administration. Every
  admin action is audited (§20).
- **Moderation.** `createReport` forwards to `PDS_REPORT_SERVICE_*` as the
  Reference does. The PDS holds no report queue (decision of 2026-10-08).

### 10.3 Mail

SMTP from `PDS_EMAIL_SMTP_URL` and `PDS_EMAIL_FROM_ADDRESS`; the moderation
mailer uses its own variables. Token mails that gate a user flow are sent
synchronously and fail the request on SMTP failure, as the Reference does.
Notices (§12.6) go through the durable outbox.

### 10.4 Lifecycle event sequences

The Candidate emits events in these orders:

| Moment | Events |
|---|---|
| account created | `#identity`, `#account` (active), the first `#commit` (no operations, `since` null), then `#sync` |
| handle changed | `#identity` with the new handle |
| PLC operation submitted | `#identity` without a handle |
| deactivated, taken down, deleted | `#account` only, with `active: false` and the status; no `#commit` while inactive |
| takedown reversed | `#account` (active) |
| reactivated by the account | `#account` (active), `#identity`, then `#sync`; no commit |
| migrated in and activated | `#account`, then `#sync`, then an empty `#commit` signed with the new key at a higher `rev` |
| CAR imported | none, as at the pin (§6.5) |
| repair after a crash boundary (§7.4) | `#sync` |

The first six rows are the Reference's sequences at the pin, as the Harness
records them. Unit 4 holds the encoding of each event and the two rows that
need no account endpoint: the import and the repair. The other rows are
observed from unit 5. The server MAY wait until a new handle resolves before
emitting `#identity`.

## 11. OAuth Authorization Server

### 11.1 Metadata

- `GET /.well-known/oauth-authorization-server` carries the Reference's fields
  and values at the pin, including `issuer` equal to the public origin with
  no path and no default port, `require_pushed_authorization_requests: true`,
  `require_request_uri_registration` not `false`,
  `dpop_signing_alg_values_supported` containing `ES256`,
  `code_challenge_methods_supported: ["S256"]`,
  `client_id_metadata_document_supported: true`,
  `authorization_response_iss_parameter_supported: true`,
  `token_endpoint_auth_methods_supported` containing `none` and
  `private_key_jwt`, `token_endpoint_auth_signing_alg_values_supported`
  containing `ES256` and never `none`, `grant_types_supported` containing
  `authorization_code` and `refresh_token`, the supported scopes and the
  `protected_resources` list.
- `GET /.well-known/oauth-protected-resource` lists exactly one
  authorization server: this origin, or the entryway's in entryway mode
  (§21.3).
- Both documents, PAR and the token endpoint support CORS. On the
  authorization-server document, `/oauth/jwks`, `/oauth/par`,
  `/oauth/token` and `/oauth/revoke` a preflight answers 200, and every
  answer carries `Access-Control-Allow-Methods: *`,
  `Access-Control-Allow-Headers: Content-Type,DPoP` and
  `Access-Control-Max-Age: 86400`, as the Reference's OAuth router sets
  them.
- The Candidate lists the signing algorithms it verifies, `ES256` and
  `ES256K`, for DPoP proofs and for client assertions. The Reference lists
  ten. The Candidate says `request_parameter_supported: false`, because it
  takes no request object (§11.5).

### 11.2 Clients

- Client metadata is fetched from the `client_id` URL through safe fetch
  (§15.1). The URL MUST be `https://` with no port; the fetch MUST return
  exactly 200 with `application/json`. `client_uri` MUST share the
  `client_id` hostname; `logo_uri`, `tos_uri` and `policy_uri` MUST be
  https; `dpop_bound_access_tokens` MUST be `true`; `grant_types` MUST
  contain `authorization_code`; a confidential client MUST declare
  `private_key_jwt` and exactly one of `jwks` or `jwks_uri` (https).
- Localhost clients: `client_id` is `http://localhost` with no port and an
  empty path, with `redirect_uri` and `scope` as query parameters; defaults
  are `http://127.0.0.1/` and `http://[::1]/`; redirect paths are matched
  and ports ignored; the synthesized metadata is a public native client.
- Redirect URIs: web clients use https with a port only if non-default;
  native custom schemes are the reversed `client_id` host followed by
  `:/path`; native https redirects share the `client_id` origin.
- Metadata is cached with a TTL short enough that a confidential client's
  removed key is rejected promptly; the server re-fetches periodically for
  live confidential sessions and revokes a session whose key disappeared.
  At the pin the cache is in memory: a session keeps refreshing until the
  cache lets the document go, and then the refresh is `invalid_client`.
- A `client_id` under a local top-level domain (`.test`, `.local` and the
  like) is refused, and the Reference's fetcher refuses the documentation
  domains (`example.com` and the like) as hostnames.
- `PDS_OAUTH_TRUSTED_CLIENTS` lists metadata URLs whose grants get the
  Reference's trusted treatment.

### 11.3 Flows

- **PAR** is mandatory: the authorization endpoint answers 400 to
  parameters sent directly. A pushed request lives five minutes
  (`expires_in` 300); opening the authorization page extends it by five
  minutes of inactivity; an expired request redirects to the client with
  `error=access_denied` ([VP-8](../parity/verify-at-pin.md)). PAR refuses a
  missing or `plain` code challenge, a redirect URI or a scope the client
  did not declare, a scope without `atproto`, and a `client_id` under a
  local top-level domain.
- **Authorize** runs on a device session (cookie, `device` and
  `device_account` tables) with remembered accounts, `prompt`, `login_hint`
  (which restricts sign-in to that account), sign-in (password, or §12.3
  providers), sign-up (§12.2 gate), consent, and reactivation gates for
  deactivated accounts. Silent re-authorization is offered only to
  confidential or trusted clients. Untrusted clients are shown by their full
  `client_id` only: `client_name`, `client_uri` and `logo_uri` are not
  displayed for them.
- **Token**: `authorization_code` with PKCE S256 and DPoP binding
  (`cnf.jkt`); reuse of a `code_challenge` is refused for 24 hours; reuse of
  a `code` revokes every session issued from it; `refresh_token` with
  rotation and reuse detection; confidential-client sessions bound to the
  assertion key's `kid`, `alg` and `jkt` for their whole life, with
  assertion `aud` equal to the issuer, `jti` unique for the validity period
  and `iat` within the last minute. Token responses always include `scope`
  and `sub`.
- **Lifetimes** at the pin (`oauth-constants.ts`, `client/client.ts`):
  access tokens 60 minutes; sessions and refresh tokens 2 weeks; for a
  confidential client, and for a first-party client on the trusted list, 2
  years for the session and 3 months without a refresh; the DPoP nonce 3
  minutes; a client assertion 1 minute; a code challenge refused for reuse
  for 1 day; `AUTHENTICATION_MAX_AGE` 7 days and `EPHEMERAL_SESSION_MAX_AGE`
  15 minutes for the device session. The protocol documents ask for access
  tokens under 30 minutes and confidential refresh tokens of at most 180
  days. The Candidate follows the pin during the rollback window and
  tightens afterwards (Appendix D, D14).
- **DPoP**: the nonce rotates with `PDS_DPOP_SECRET` with a lifetime of at
  most 5 minutes; recently stale nonces are accepted; the `jti` replay set
  is scoped to the nonce; `AccountDeactivated` is enforced.
- **Revoke** and **introspect** per their RFCs with the Reference's
  constraints. Revoking a token nobody holds succeeds. Redeeming a code
  twice and replaying a rotated refresh token are `invalid_grant`, and both
  end the session: its access token becomes `invalid_token`.
- A resource request with a proof from another key is 401 `invalid_token`,
  "Invalid DPoP key binding"; without a proof 401 `invalid_dpop_proof`; a
  DPoP-bound token presented as `Bearer` is 400 `InvalidToken`.
- **Account pages** under `/account` for settings, a session list with
  per-session revocation, devices, sign-out, and the §12.5 sign-in methods.
- The Reference's provider key set is one HS256 key built from
  `PDS_JWT_SECRET`, and the DPoP nonce secret is `PDS_DPOP_SECRET`. The
  Candidate MUST sign and verify OAuth access tokens with the same key so
  tokens issued before cutover verify after it. Asymmetric keys for the key
  set are a post-cutover change (Appendix D, D6).

### 11.4 User interface

- Pages are server-rendered from templates compiled into the binary; there is
  no client-side framework bundle. Progressive enhancement scripts MAY be
  inlined with a strict CSP.
- Behaviour parity with the Reference's flows, fields, errors and security
  checks is required; visual parity is not (decision of 2026-10-08). The
  pages follow the LinkJar brand guidelines under the `linkjar` profile and
  the §12.7 branding under `stock`.
- Pages are usable at 390 px width and meet WCAG 2.1 AA for contrast and
  keyboard operation.
- The UI's JSON routes keep the Reference's CSRF, `Origin`, fetch-metadata,
  ephemeral-token and device checks.

### 11.5 State after unit 5

The Candidate serves §11.1 to §11.4 with the differences below. The
[unit record](research/2026-10-13-oauth.md) gives the evidence.

- **Pages.** Every page is one HTML document that works without scripts.
  A form carries a token that the server computes from the device cookie
  and the request, and the server checks `Origin` and `Sec-Fetch-Site`. A
  sign-in that the device does not remember travels as a signed proof in
  the consent form; the proof names the account, the device and the
  request and ends after 15 minutes.
- **Access tokens** carry no scope. The server reads the scope from the
  token's row on every request, as the Reference does in its stateful
  mode, so revocation ends an access token at once.
- **Scope checks.** A method that an OAuth token may call is on an allow
  list. A method with a handler that is not on the list answers 403
  `Forbidden`. A call outside the token's scope answers 403
  `ScopeMissingError`.
- **Account pages.** `/account` shows the applications that have a
  session and the devices that remember the account, and ends each. The
  settings of the Reference's account page and the §12.5 sign-in methods
  are not built.
- **Not built:** request objects (JAR); permission sets, so an `include:`
  scope grants nothing; the reactivation step for a deactivated account,
  which gets `access_denied`; hCaptcha on sign-up and the password reset
  from the sign-in page; address pinning in safe fetch; the periodic
  re-fetch of a confidential client's keys.

## 12. Extension Points

### 12.1 Rules

- **EX-1** The server core calls traits; it never contains operator-specific
  branches. A `Profile` composes one implementation of each trait and is
  selected by `PDS_PROFILE` (`stock` or `linkjar`).
- **EX-2** The `stock` implementations reproduce Stock Reference behaviour
  (C6).
- **EX-3** No request field can select or influence an internal extension
  flag. Internal arguments are passed by the server, never parsed from input.
- **EX-4** Hook state changes commit in the same transaction as the account
  change they accompany, and roll back with it. The transaction is the
  `AccountStore` transaction of §8.4.
- **EX-5** Hooks have bounded time and fail closed for signup, sign-in,
  linking and unlinking.
- **EX-6** Extension tables live in the account database under extension-
  owned migration names and are backed up with it; account deletion removes
  their rows through §12.9.
- **EX-7** Extension routes carry their own lexicons and appear in
  `describeServer` only when the profile enables them.
- **EX-8** Every extension hook that changes account state produces an audit
  entry (§20).

### 12.2 Signup gate

`evaluate(request) -> Allow | Deny(reason) | Challenge(kind)` sees the invite
code, CAPTCHA token, client, IP, device, any verified external identity, and
the operator's abuse lists from `ops.sqlite` (blocked email domains, blocked
IP ranges). It runs before PLC and storage writes for both OAuth sign-up and
legacy `createAccount`. Stock: the Reference's invite and hCaptcha
behaviour plus the abuse lists.

### 12.3 Identity providers

Lists providers for UI hydration (labels, icons, start routes, never secrets);
`start(provider, device, pending request)` and `callback(...)` produce a
`VerifiedIdentity` (issuer, subject, asserted email and its verification,
display-name suggestion); a store keyed by `(provider, subject)` with atomic
link, unlink and list. Flow state is one-use, device-bound and bounded. Stock:
no providers, no routes.

### 12.4 Handle policy

`suggest(context)` for fresh signups, `check(handle)` extending the reserved
lists, `authorize_rename(tx, did, from, to)` for allowances,
`handle_host(host, path)` for responses on handle hostnames, and
`on_unverifiable(did)` deciding whether writes are refused when a custom
handle no longer verifies. Stock: the Reference's lists, no suggestion, no
allowance, well-known only, writes continue.

### 12.5 Sign-in methods

Account-page listing of methods, link and unlink with
`can_unlink(tx, did, method)` enforcing that an account keeps at least one
usable method. Stock: password only.

### 12.6 Notices

`enqueue(tx, did, notice)` into a durable outbox; a dispatcher delivers with
retry and at-least-once semantics. Stock: no notices are produced; the outbox
and dispatcher still exist.

### 12.7 Mail templates and branding

`render(kind, data) -> {subject, html, text}` for confirm-email,
reset-password, update-email, delete-account, PLC-operation and sign-in-notice
mails; a branding provider for the UI. Stock: the Reference's templates and
the `PDS_*` branding variables.

### 12.8 Creation receipt

`record(tx, request, client, device, did)` when an account is created inside
an authorization, carried into the token row only when the authorized DID is
that DID, readable and acknowledgeable per session. Stock: disabled; the
`io.linkjar.account.*` methods are absent.

### 12.9 Account hooks and migrations

`on_created`, `on_deleted`, `on_deactivated`, `on_handle_changed` with the
transaction; a migration list the server applies with the account database.

### 12.10 Storage backends

The seams of §8.4 are themselves extension points: a profile or a build MAY
provide another implementation of the shared tier or the blob store. The
rules of §8.4 apply.

### 12.11 Operator roles

`authorize(operator, action) -> Allow | Deny` for every ops-listener action
(§22.1), where the operator identity comes from an OAuth session on this
server or the entryway. Stock: the basic-auth admin identity is the only
operator. The `linkjar` profile grants roles to named accounts.

## 13. The LinkJar Profile

The `linkjar` profile implements the eight account patches with their
documented invariants at the **patch pin**: the patch stack of this
repository at the commit recorded in `plan.md`. The patch stack is frozen
except for security fixes while the Candidate is built (decision of
2026-10-08); a security fix updates the pin and adds the mirrored behaviour
here.

On 2026-10-12 the owner decided to keep the upstream pin at 0.5.34 and to
carry upstream's shared-node fix
([#5584](https://github.com/bluesky-social/atproto/pull/5584)) as a ninth
patch, [`104-mst-shared-node`](legacy/mst-shared-node.md). It is engine behaviour for every profile, so
its rule is in §6.1 and not in the table below. The patch pin moved with
it: the stack has nine patches, and the reference image is revision 12.

| Patch | Extension point | Invariants the Candidate MUST keep |
|---|---|---|
| [`093-handle-policy`](legacy/handle-policy.md) | §12.4 | Suggested handles from display name, email local part, then `reader`; NFKD, ASCII, 3 to 18 characters, six-hex suffix on collision, at most 32 attempts; the extra infra, brand, numbered and lookalike reservations; one hosted rename within 30 days, same-target retry after an ambiguous PLC write; handle hosts serve only `/.well-known/atproto-did`, 404 for other well-known and XRPC paths, 302 to `https://linkjar.io/jar/<did>` for other GET and HEAD, no cookies, strict CSP, `nosniff`. |
| [`099-external-providers`](legacy/external-providers.md) | §12.3 | Apple (`form_post`, nonce, confidential secret), Google and GitHub (PKCE S256); fixed issuer and JWKS endpoints; GitHub needs a primary verified email; bounded flow store (1,000 flows, three per device, ten minutes); 20 starts and 10 completions per IP per minute; `external_identity` keyed by provider and subject; email collisions refused, never linked; passwordless accounts carry the `!external-identity` sentinel; fresh-signup preauthorization only for explicitly trusted clients. |
| [`099b-signup-receipt`](legacy/signup-receipt.md) | §12.8 | `io.linkjar.account.getSignupReceipt` and `acknowledgeSignupReceipt` over the session's DPoP transport; receipt bound to request, client, device and the created DID; survives rotation, cleared by acknowledgment, removed by revocation; fails closed. |
| [`099c-signup-policy`](legacy/provider-policy.md) | §12.2 | All three hCaptcha variables or startup failure; fresh legacy `createAccount` refused when hCaptcha is configured, with migrations via service auth still allowed. |
| [`100-signin-methods`](legacy/signin-methods.md) | §12.5, §12.6 | Device-bound linking from the account page; idempotent self-link; a foreign identity is refused with only that account's handle; last-method protection in the transaction; notices enqueued in the same transaction, 100 per minute, retry from one minute to one hour, stable `Message-ID`. |
| [`101-mail-templates`](legacy/mail-templates.md) | §12.7 | Six templates in LinkJar's email style with a text alternative and no tracking. |
| [`102-signup-journey`](legacy/signup-journey.md) | §11.4, §12.7 | The sign-up page opens on the email form (email and password, then the username) with the providers under it; the sign-in form sits behind "Or use email" under the providers; the page CSP allows the bundled font. Revision 1 omitted this patch. |
| [`103-invite-handoff`](legacy/signup-journey.md#invitation-hand-off) | §12.2 | A code in the URL fragment `#invite=<code>` is submitted in place of the invite field; a refused code brings the field back with the server's error; the provider start request carries the code, so an invited person can sign up with a provider while invites are required. Revision 1 omitted this patch. |

Each row has Harness scenarios against the running server
([parity/README.md](../parity/README.md#the-port-of-legacytests)).

### 13.1 Configuration

The profile reads the same variables the patches read (`PDS_EXTERNAL_*`,
`PDS_HCAPTCHA_*`, `PDS_OAUTH_TRUSTED_CLIENTS`) so the host's secrets file does
not change at cutover.

### 13.2 Handle hosts

The server itself answers handle hostnames according to §12.4, keyed by the
`Host` header. The Caddy snippet in `legacy/staging/Caddyfile.handles` remains valid
and MAY stay in front of it. The snippet forwards two requests of a handle
host, the DID and the profile redirect (`/.well-known/linkjar-profile`),
and sets the CSP, `nosniff` and referrer headers on them. The server sets
those headers on every other answer of a handle host and not on these two,
so that an answer through the snippet carries each header once.

### 13.3 Lexicons

`io.linkjar.account.getSignupReceipt` and
`io.linkjar.account.acknowledgeSignupReceipt` are vendored from the patch pin
and served only in this profile.

### 13.4 Operator roles

Named accounts hold the `operator` and `moderator` roles for §12.11, stored
in `ops.sqlite`.

### 13.5 State after unit 6

The Candidate has the behaviour of the eight patches under
`PDS_PROFILE=linkjar`. The [unit record](research/2026-10-13-linkjar-profile.md)
gives the evidence.

- The profile applies the four migrations of the patch stack that the
  account database lacks, under the patch stack's names, at the start.
- The identity, the confirmed address, the handle policy row and the
  creation receipt are written in the transaction that creates the
  account. A link, an unlink and a password change write their notice in
  the transaction of the change.
- With hCaptcha configured, the sign-up page shows the challenge on its
  last step and the server asks hCaptcha before it creates anything. The
  legacy `createAccount` is closed to new accounts.
- The account pages are `/account`, which lists the accounts that are
  signed in on the device, and `/account/<did>/manage`, which has the
  sign-in methods, the sessions and the devices.
- **Not built:** setting a password from the account page; operator roles
  (§13.4, with unit 9); the reactivation step of a deactivated account in
  a provider flow; the explicit-slur check on a suggested handle.

## 14. Operations

### 14.1 Configuration

- Configuration is environment variables. The Candidate MUST accept every
  variable in Appendix C with the Reference's default, MUST fail to start on
  an invalid value, and SHOULD warn on an unknown `PDS_*` variable.
- `check-config` validates without starting.

### 14.2 Command line

`linkjar-pds serve`, `migrate` (with `--allow-candidate-only`, §18.3),
`migrate-storage` (§8.4), `check-config`, `verify` (§18.2), `repair` (§19.5),
`export-car <did>`, `audit verify` and `audit export` (§20), `version`, and
`health`, which asks the server on the same host for its health route and
exits 0 when it answers 200. `health` is the container health check: the
image holds no other program to make the request with.

### 14.3 Health and shutdown

`/xrpc/_health` reflects database availability. On `SIGTERM` the server stops
accepting connections, finishes in-flight writes, sends close frames to
firehose subscribers, checkpoints what it owns, and exits within a bounded
time.

### 14.4 Telemetry

- Metrics in Prometheus text format on the metrics listener: request counts
  and latencies by method and status, rate-limit decisions, commit latency,
  sequencer lag, subscriber counts and queue depth, actor LRU hits, SQLite
  busy events, mail outbox depth, audit checkpoint age, background task
  state.
- Traces, metrics and logs exported over OTLP/HTTP to `PDS_OTLP_ENDPOINT`
  when set, with the service name and version as resource attributes. The
  console sidecar (§22.1) is the usual target.
- Metric names are documented in the README and the `linkjar/infra` alert
  rules are updated in the cutover unit (Appendix D, D8).
- Logs are JSON lines with request ids. They never contain tokens, passwords,
  email addresses, provider subjects or record bodies.

### 14.5 Build, gates and distribution

- Toolchain pinned by `rust-toolchain.toml` (latest stable at each pin) and
  `Cargo.lock`; devenv defines the development environment with
  `languages.rust` from the toolchain file, rustfmt and clippy hooks that
  devenv defines and does not install (run by hand with `pds:hooks`; CI
  holds the same gates), `services.postgres` for the shared-tier tests, and `processes` for the
  Harness.
- CI gates on every pull request: `cargo fmt --check`, `cargo clippy
  --all-targets -D warnings`, the unit and property tests, the interop
  vectors, `cargo deny check` with advisories, licences, bans and sources
  (`unknown-registry` and `unknown-git` denied), `cargo vet`, and
  `--locked` on every cargo invocation. `cargo audit` runs on a schedule
  against a pinned advisory database. Every `ignore` entry carries a reason
  and an expiry.
- Fuzz targets for DAG-CBOR, CAR, MST, XRPC input, JWT, DPoP and client
  metadata run on a schedule.
- Nightly: the §19 seed runs and the determinism meta-test.
- Releases are built from tags in CI, `cargo auditable`, an SBOM, SLSA level
  3 provenance through the official generator, signed tags, SHA-pinned
  actions with read-only default permissions. A static binary per Linux
  architecture, a container image with the same entrypoint contract as the
  Reference image, and a Nix package through `languages.rust.import` for the
  NixOS host.
- Dual-licensed MIT and Apache-2.0 (decision of 2026-10-08).

## 15. Security

### 15.1 Safe fetch

Every outbound HTTP request (DID resolution, client metadata, JWKS, handle
verification, lexicon resolution, identity providers, crawler notification)
resolves the host, rejects loopback, private, link-local and multicast
addresses, connects to the resolved address, re-checks on each redirect with
a redirect cap, applies timeouts (`PDS_ID_RESOLVER_TIMEOUT`,
provider-specific limits) and response size caps
(`PDS_FETCH_MAX_RESPONSE_SIZE`). `PDS_DISABLE_SSRF_PROTECTION` is honoured
only with `PDS_DEV_MODE`.

### 15.2 Secrets and comparison

Secrets are read once, never logged, zeroised on drop with the understanding
that `Drop` is best effort, and compared in constant time. Secret-bearing
types implement `Debug` by redaction. Signing keys never leave the process
except through the Reference's export paths.

### 15.3 Browser and blob surfaces

Cookies are `HttpOnly; Secure; SameSite=Lax`. HTML responses carry a strict
CSP and `X-Content-Type-Options: nosniff`. Blob responses carry the headers of
§6.4. The UI JSON routes keep the Reference's CSRF and fetch-metadata checks.

### 15.4 Takedowns

Taken-down accounts stop serving records and blobs, blobs are quarantined,
and sync methods answer as the Reference does.

### 15.5 Input limits

The decode limits of §6.1 and body limits on every ingest path
(`DefaultBodyLimit` or equivalent) are applied before any parsing.
Configuration and authentication types reject unknown fields.

## 16. Performance

### 16.1 Scenarios

The Harness runs each scenario against Reference and Candidate on the same
machine with copies of the same data directory.

| Id | Scenario | Measures |
|---|---|---|
| S1 | one `createRecord` on repositories of 10, 1,000, 10,000 and 100,000 records | latency p50 and p99, SQLite rows and bytes written |
| S2 | `applyWrites` with 200 operations | same |
| S3 | `getRepo` export of 100,000 records | wall time, peak RSS growth |
| S4 | one writer at 50 commits per second with 1, 10, 100 and 1,000 subscribers | delivery lag p99, RSS |
| S5 | backfill of 100,000 events from a cursor | wall time, RSS growth |
| S6 | 10,000 idle accounts | RSS, open file descriptors, time to healthy |
| S7 | token endpoint at 200 requests per second | latency p99, error rate |
| S8 | 20 concurrent 5 MiB uploads | latency, RSS |
| S9 | 50 writers across different actors at 2 commits per second each | sequencer fsync count, lag p99 |

### 16.2 Targets

Initial targets, to be revised after unit 2 publishes measurements:

- **T1** No scenario slower than the Reference at p50 or p99.
- **T2** S1 p99 at 100,000 records at most one third of the Reference.
- **T3** S6 RSS at most one quarter of the Reference.
- **T4** S3 RSS growth under 50 MiB.
- **T5** S6 time to healthy under one second.
- **T6** S4 lag p99 under 100 ms with 1,000 subscribers.
- **T7** S9 sequencer fsyncs per second below the commit rate, through group
  commit on the sequencer file.

Unit 2 measured the engine's share of S1 to S3, without HTTP and SQLite,
for the Candidate's engine and for the Reference's library on one host
([decision record](research/2026-10-12-repository-crate.md) §5). The targets
stand until the Candidate serves the scenarios over HTTP (unit 3 for S3,
unit 4 for S1 and S2).

Results are published in the README with hardware, versions and image
digests. The Reference's baseline for S1 to S5, S8 and S9 is in
[parity/results/](../parity/results/perf-reference.md). In it the Reference
falls behind in S4 at 1,000 subscribers, with a median delivery lag above
ten seconds, so T6 is an absolute target at that size and T1 does not bound
it.

## 17. Parity and Conformance

### 17.1 Harness

`parity/` is a TypeScript project using `@atproto/api`,
`@atproto/oauth-client-node`, `@atproto/repo` and `@atproto/xrpc` at the
pin. Each target (the Reference image by digest, the Stock Reference, the
Candidate) gets its own compose stack with the same configuration: the
production hostnames behind a TLS edge, a local PLC directory, a local Sync
1.1 relay in strict mode, a mail catcher, and fixture services for
everything else the server talks to. Each scenario runs against each target
and writes a transcript; the oracles below compare the transcripts of two
targets. A **restore run** puts two servers on one stack, one after the
other: the Reference writes a data directory and answers the read
scenarios, the Candidate replaces it on the same volume and answers them
again, and the two sets of answers must not differ, body bytes included.
Parity scenarios run without fault injection; fault scenarios (§19)
run against the Candidate alone. [parity/README.md](../parity/README.md)
describes the harness and the contract its page driver relies on.

The writes run, `pnpm parity writes`, holds the Candidate to the Reference
on writes and the firehose before the Candidate has an account endpoint.
Every pass starts on the Reference, which creates a pool of accounts. In the
Candidate's pass the Candidate then takes over the data directory, and the
same scenarios run on each server: record writes, reads, blobs, imports,
the firehose with its cursors, a relay in strict mode, and data from
elsewhere. The differences that show are the deliberate ones of §2.3,
recorded in `parity/differences/reference-vs-candidate.json`; a difference
that is not recorded fails the run, and so does a recorded one that no
longer shows. After the Candidate's pass `verify` checks the directory. A
last pass stops the Candidate between a commit and its event and requires
the repair of §7.4 and a relay that converges.

One stage of the writes run keeps blobs in an S3-compatible store that
checks request signatures, configured as production configures R2: an
endpoint, path style and static credentials. Before the takeover the
Reference stores one blob and stages one upload. The server under test
then serves the blob, makes the staged upload permanent and removes both
with their last references, and the blob scenario of the default profile
runs on that store. `verify` checks the bucket. The stage has its own list,
`parity/differences/reference-s3-vs-candidate-s3.json`, which holds DD-18.

### 17.2 Interop vectors

The upstream `atproto-interop-tests` (CC0: crypto, data model, firehose,
lexicon, MST, syntax) run as Rust unit tests in `pds-types`, `pds-repo`,
`pds-lexicon` and `pds-identity`. Vectors that do not pass are listed in a
`KNOWN_FAILURES` table with the finding that explains each. The planned
upstream protocol test suite is adopted when published.

`pds-repo` is also held to fixtures written by `@atproto/repo` at the pin
([`crates/pds-repo/tests/fixtures`](../crates/pds-repo/tests/fixtures/README.md)):
tree roots, commit CIDs, the blocks of each commit and the export are
compared byte for byte, and the shared-node trees of upstream's
`mst-dag.test.ts` must be refused.

### 17.3 Oracles

- **O1 Response diff.** Status, `error`, headers named in §4.2 and §6.4,
  and the JSON body normalised: field order ignored; server-generated
  identifiers aliased on first sight; times compared when the dev clock is
  injected. Between the Reference and the Candidate the recorded differences
  are the deliberate differences of §2.3 and no others.
- **O2 Repository CIDs.** MST root CID after every write scenario; commit CID
  when `rev` is injected (§3.3).
- **O3 Firehose.** Frames decoded and compared as events: type, `ops`,
  block set, `blobs`, `since`, `rev` and `prevData` relationships, cursor
  behaviour in every row of §7.3, the lifecycle sequences of §10.4, and the
  crash injection of §7.4. A local Sync 1.1 relay in strict mode MUST accept
  every frame.
- **O4 OAuth.** Full flows for two fixture users, after a server restart,
  plus negative cases: duplicate code redemption, refresh reuse, wrong DPoP
  key, cross-user access, deactivated account, confidential client with a
  removed key, localhost client, untrusted-client display.
- **O5 Migration.** The account migration procedure in both directions with
  a repository, blobs and preferences, verified by CID and byte comparison,
  including a `since` diff import and a repeated import.
- **O6 Stock equivalence.** The whole suite with `PDS_PROFILE=stock` against
  Stock Reference (C6).
- **O7 LinkJar journeys.** The browser gate and the kit self-test from
  `linkjar.io` pointed at the Candidate, and this repository's
  `legacy/tests/*.mjs` suites ported to run against an HTTP endpoint.
- **O8 Official app.** A recorded manual acceptance: the Bluesky app signs in
  through the Candidate and posts.
- **O9 Verify at pin.** Every item that revision 1 marked "verify at pin"
  has a scenario that records the Reference's behaviour
  ([verify-at-pin.md](../parity/verify-at-pin.md)). This revision carries
  the results.

### 17.4 Gates

Each delivery unit in [plan.md](plan.md#delivery-units) names the oracles
and scenarios that must pass before it merges. A unit does not merge on a partial oracle.

## 18. Cutover and Rollback

### 18.1 Preconditions

- The Harness passes with the `linkjar` profile against a restored copy of
  the production data directory on staging. Production is touched only at
  cutover (decision of 2026-10-08).
- `verify` (§18.2) reports zero differences on that copy.
- Secrets are unchanged: `PDS_JWT_SECRET`, `PDS_DPOP_SECRET`, the rotation
  key, the admin password, and the SMTP and provider credentials.
- The audit log (§20) is in place so the chain starts at cutover, with the
  pre-cutover state sealed as a one-off signed snapshot.
- The `linkjar/infra` service definition, alert rules and recovery runbook
  are updated for the new binary and metrics.

### 18.2 Verification

`verify` opens the data directory read-only and changes no row and no
database file. SQLite creates the `-shm` and `-wal` files beside a database
in WAL mode when they are absent, so `verify` needs write access to the
directories. For every actor it:

- loads the commit that `repo_root` names, and compares that row with the
  one in the account database;
- reads the whole tree with the checks of §6.1, and names a tree that
  references a node more than once as its own finding;
- compares the keys and records of the tree with the record index;
- writes the record index into an empty tree and compares the root with the
  commit's;
- checks that every record of the index has its block;
- loads the `key` file and checks the commit's signature with it;
- checks that every blob a record references has a row in the blob index
  and, unless it is taken down, a file in the blob store.

A root in the account database that is behind the actor store, or absent,
is counted and is not a difference (`stale account roots`;
`staleAccountRoots` with `--json`): the Reference writes that root after a
commit and logs a failure, and it does not write it on an import. A root
that is ahead of the store is a difference.

It then reads `repo_seq` and reports the breaks in its sequence numbers. A
break is not a difference: the Reference deletes an account's earlier
events when the account is deleted. `verify` prints a report, as text or
with `--json`. It exits 0 with no difference and 1 with one or more. The revision in `repo_root` is not compared with the commit's:
the Reference gives the root a new revision when it imports a repository
whose commit it already holds.

The Reference has no such command. The oracle is the restore run of the
Harness (§17.1): both servers serve the same copy and their answers are
compared.

### 18.3 Procedure

1. Announce maintenance on the status page.
2. Stop the Reference container. Run `verify`.
3. Start the Candidate on the same hostname and data directory. Check health,
   `describeServer`, OAuth metadata, a firehose connection from the relay
   resuming at its last cursor.
4. Smoke: sign in from the web app, iOS and the extension; write a record;
   observe it on the firehose; send a test mail.
5. Watch for 48 hours. The rollback window stays open until the operator
   applies the first Candidate-only migration of a shared database with
   `--allow-candidate-only`.

Rollback within the window: stop the Candidate, start the Reference image on
the same directory. The Candidate-only files are left in place and ignored.

### 18.4 Fencing

Reference and Candidate MUST NOT serve the same data directory at the same
time. The Candidate's lock file (§8.3) and SQLite locking refuse a second
writer; the runbook stops one before starting the other.

## 19. Simulation and Fault Injection

### 19.1 Principle

Faults are random; seeds are not. Every simulation run is a pure function of
`(seed, commit)`. A failure is filed with its seed and reproduces on any
machine. The `pds-sim` crate owns this; it enters the codebase in unit 4
together with the write path (decision of 2026-10-08).

### 19.2 Seams

- **`StorageIo`**: every file operation of the SQLite databases and the blob
  store goes through one trait. Production binds it to the operating system.
  Simulation binds it to an in-memory implementation and, for SQLite, to a
  custom VFS registered through `sqlite-plugin` that honours WAL and shared
  memory and injects: I/O error after N operations (transient or persistent),
  `fsync` failure, torn writes at the declared sector size, bit flips,
  misdirected writes, disk full, and power loss modelled as "pending writes
  not yet synced are lost or reordered".
- **Fail points** on the commit, sequencing, mail outbox, PLC submission and
  audit checkpoint paths, so a run can kill the process between any two
  steps.
- **Network and time**: `turmoil` with the `mad-turmoil` overrides for
  randomness and clocks, running the firehose, crawler notification,
  identity resolution, mail and, later, the entryway protocol as hosts in
  one process.
- Determinism is enforced by a meta-test that reruns a seed and compares
  TRACE logs byte for byte. Any dependency that reads the clock, randomness
  or a hash seed outside the providers of §3.2 fails that test and is
  wrapped or replaced.

As of unit 4b the crate has the fail points of the commit path, the seeded
plans, the oracles of §19.4, the seed runner, the determinism meta-test,
which compares the traces of two runs of a seed, and a file system below
SQLite that loses power.

The seam is `pds_storage::io`: every database of the data directory is
opened there, with the operating system's files in production and through
a named SQLite VFS in the simulation. It is one function and not a trait,
because SQLite's own VFS interface is the trait. The simulation's VFS is
registered through `sqlite-plugin`, whose registration is a safe function,
so the rule of §3.2 on `unsafe` holds without an exception. It passes
every operation to the files of the host and remembers what each write and
truncation replaced until the file is synced. The write-ahead log's shared
memory and all locks are in the process. When the power is lost inside a
write of the plan:

- every operation of the running server fails from that operation on;
- each file keeps a prefix of its writes since its last sync, drawn from
  the seed, and loses the rest;
- the last write that is kept may be torn at a 512-byte sector, which is
  the sector size the VFS declares, with no atomic write and no safe
  append;
- the server starts again and repairs (§7.4), and the run then requires
  that the repository holds the records from before the write or those
  after it, that an acknowledged write is there, and every oracle of §19.4.

Not written yet, and the crate's documentation says so:

- the other storage faults of the list above: an I/O error that the
  process survives, a failed `fsync` that is not a power loss, bit flips,
  misdirected writes and a full disk; and the blob store and the key
  files, which are not behind the seam;
- on the network: a restart of the server, a fault on the link of the
  writing client, and the crawler notification, identity resolution and
  mail, whose HTTP client connects to the host's network. The
  `mad-turmoil` overrides are not used: the clocks and the random sources
  of the server are injected (§3.2);
- concurrent writers: a plan runs one write after the other.

The network runs (`pds-sim --network`) are separate plans. The server, a
relay and a writing client are hosts on `turmoil`'s network, and the
server answers with the router of production over HTTP and WebSocket. The
relay follows `subscribeRepos` from its cursor, checks every commit
strictly, fetches a repository with `getRepo` on `#sync`, and connects
again when a connection ends or stays silent for 2 seconds. The plan
partitions the link between the relay and the server, repairs it, and
holds its messages. At the end the link is whole, and the run requires
that the relay received every event of the log once and in order over all
of its connections, that it dropped no commit and needed no repair, that
its copy of each repository is the server's, and that each repository
holds what the client was told. With a small subscriber buffer the server
closes the relay as `ConsumerTooSlow`, and the same holds. The server does
its storage work on threads that run in real time, so the plan and the
final state are functions of the seed and the interleaving of frames and
writes is not.

`PDS_FAIL_POINTS` arms fail points in a running server, in dev mode only
(§3.3): `name=action` pairs, where an action is `abort`,
`abort-once:<marker file>` or `error`. The points of the commit path are
`commit.before-store`, `commit.stored` and `commit.sequenced`.

### 19.3 Scenarios

Random interaction plans drawn from the seed: writes across many actors,
subscribers connecting and disconnecting with cursors, imports, account
lifecycle transitions, OAuth sessions, mail, with faults injected at random
points. Hand-written scenarios for the crash boundaries of §7.4 and §10.4
run with every seed.

### 19.4 Oracles

- After every simulated crash, `verify` passes or `repair` restores the
  repository to the MST root the plan recorded, and the relay model
  converges.
- No committed write is lost after a successful response.
- The firehose delivers every sequenced event exactly once to a subscriber
  that resumes correctly, in `seq` order.
- Invariant assertions compiled into release builds where they are cheap:
  `rev` monotonic, `seq` monotonic, MST root matches after commit, no blob
  served without a reference.

### 19.5 Repair ladder

`repair <did>` restores a repository, in this order, reporting which source
it used: the sequencer log's block slices; the most recent backup; a relay
or mirror through `com.atproto.sync.getRepo`. After repair the server emits
`#sync`.

### 19.6 Reporting

Nightly seed runs publish their count, the seeds that failed and the
scenarios that found them. The limits of the simulator are stated in the
README: which faults are modelled, which are not, and which parts of time
remain real.

## 20. Audit Log

### 20.1 Scope

Account and operator actions (decision of 2026-10-08): signup, sign-in
method link and unlink, password and email changes, handle renames, PLC
operations, deactivation, deletion, takedown, suspension, invite
administration, every admin and console action, every MCP write
(§22.2), and extension hooks that change account state. Ordinary reads and
ordinary repository writes are not audited: repository data is already
signed, and reads are a traffic log.

### 20.2 Entries

Each entry is DAG-CBOR with a fixed field order: sequence number, time,
actor (DID or operator identity), action, subject, credential kind, client
id, device id, request id, outcome, reason (for operator actions), the
acting sidecar if any, and the hash of the previous entry. The `AuditStore`
is insert-only; the serving credential has no update or delete path.

### 20.3 Checkpoints

Every five minutes, and at shutdown, the server builds a Merkle tree over
the entries since the last checkpoint and signs a checkpoint containing the
range, the root, the time, the key id and the signature, with a dedicated
Ed25519 key whose succession is itself logged. The checkpoint is published as
a `io.linkjar.pds.audit.checkpoint` record in the operator's own repository
on this server, so it travels the firehose and is mirrored outside the
operator's control. A witness that compares checkpoints over time detects
any rewrite.

### 20.4 Verification

`audit verify` and the standalone verifier binary recompute the chain, the
Merkle roots and the signatures from an export bundle, offline, sharing no
code with the writer beyond the entry schema. `audit export` produces the
bundle: entries in a range, chain hashes, inclusion proofs, covering
checkpoints and public keys. History before cutover is sealed as a one-off
signed snapshot and marked as such.

### 20.5 State after unit 7

The [unit record](research/2026-10-13-audit-log.md) gives the evidence.

- **Entry.** A DAG-CBOR array of thirteen fields in the order of §20.2. A
  field that does not apply is `null`. The hash of an entry is the SHA-256
  of its bytes, and the first entry names 32 zero bytes.
- **Store.** `audit.sqlite` has the tables `entry`, `checkpoint` and
  `key`. A trigger refuses every update and every delete on each.
- **What is recorded.** Every method under `com.atproto.admin`, the
  account, credential, identity and invite methods of §20.1, `importRepo`,
  and on the pages a sign-up and a link or unlink of a sign-in method. An
  entry is written after the method answered, with `ok` or the name of
  the refusal. A method that was refused is recorded too.
- **Checkpoint.** RFC 6962 Merkle tree over the bytes of the entries of
  the range. The signature is Ed25519 over a DAG-CBOR array of the text
  `linkjar-pds-audit-checkpoint-v1`, the first and the last number, the
  root, the hash of the last entry, the time and the key id. The key is
  `PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX`, a 32-byte seed. A key id is
  `ed25519:` and the first 16 hexadecimal digits of the SHA-256 of the
  public key. A key that is not the one of the last checkpoint gets an
  entry, `audit.key`, before it signs. Without a key the log chains its
  entries and signs nothing.
- **Publication.** With `PDS_AUDIT_CHECKPOINT_REPO` set to the DID of an
  account on this server, each checkpoint is written into that repository
  as a record of `lexicons/io/linkjar/pds/audit/checkpoint.json`, and the
  commit travels the firehose.
- **Bundle.** A JSON object with `version`, `anchor` (the hash before the
  first entry), `entries`, `keys`, `checkpoints` and `proofs`. Bytes are
  base64url without padding.
- **Verifier.** `linkjar-pds audit verify <file>` and the binary
  `pds-audit-verify` need no configuration. The verifier builds the tree
  with a stack and writes the signed bytes by hand, so it shares the
  entry format with the writer and nothing else.
- **Seal.** `linkjar-pds audit seal` writes the first entry of a log:
  `audit.sealed`, with the number of accounts and the SHA-256 of their
  DIDs and repository heads. It is refused on a log that has an entry.
- **Not built:** the entries of the ops console and of the MCP bridge
  (units 9 and 10), the reason of an operator action, the request id, a
  key in a KMS or an HSM (D11), and the metric of the checkpoint age.

## 21. Deployment Topologies

### 21.1 Single node

One process, one data directory, Litestream to object storage, a rehearsed
restore with the RPO and RTO targets of the hosting study. Version 1 and the
cutover target.

### 21.2 Shared-tier alternative

Several processes over one Postgres `AccountStore`, `SequencerStore`,
`DidCache` and `AuditStore` and one blob store, with a single sequencer lease
held through a session-scoped advisory lock, Postgres compare-and-swap for
DPoP replay and rate limits, and per-actor files pinned to one process by
routing. Documented as an alternative, not the recommended path, because the
per-actor file placement under several processes has no production evidence
in the surveyed implementations.

### 21.3 Entryway and member hosts

The recommended way to scale (decision of 2026-10-08). The `pds-account` and
`pds-oauth` crates run in one of two modes selected at startup:

- **Embedded**: inside `linkjar-pds` on a single node, as in §21.1.
- **Entryway**: as a standalone service that owns email, passwords, app
  passwords, invites, sessions, OAuth, service handles, PLC rotation keys
  and the account-to-host map, and routes signup to a member host. Member
  hosts run `linkjar-pds` in entryway mode with the Reference's
  `PDS_ENTRYWAY_*` contract, keep repositories, blobs, signing keys and the
  firehose, answer 404 on `/.well-known/atproto-did`, and point their
  protected-resource metadata at the entryway.

Exactly one entryway instance runs; its account database is the single copy
and is protected by §21.1's backup discipline. The entryway records every
multi-step workflow in a durable operation log and resumes after a crash.
Adoption of an existing single node into an entryway keeps every DID
document unchanged. Entryway mode is a post-cutover unit.

## 22. Sidecars

The two companion containers are specified in [sidecars.md](sidecars.md).
The normative contracts they depend on are:

### 22.1 Ops listener

`PDS_OPS_ADDRESS` serves JSON over HTTP and one WebSocket stream of live
state, authenticated by an operator session (§12.11) or the break-glass
admin credential. Methods are `internal.*` NSIDs and are excluded from
`describeServer`. The surface: live subscribers with cursor and lag and a
disconnect action; upstream health; sessions per account with revocation;
repository cards; job state; abuse lists; rate multipliers; `verify`,
`repair`, garbage collection and token purge triggers; `#sync` emission;
invite controls. Every mutating call carries an operator identity and a
reason and produces an audit entry naming the console.

### 22.2 MCP bridge

The MCP server is a confidential atproto OAuth client of this server with a
`private_key_jwt` assertion and a DPoP key per session, requesting the
permissions its scope map declares. Its own tokens never reach the PDS.
Writes it performs carry both its client id and the AI client's id into the
audit log. The hosted server exposes public records and the account only;
it never attempts to decrypt LinkJar private data. The private-data mode is
a TypeScript process in `linkjar.io` (decision of 2026-10-08).

### 22.3 Rules

Sidecars hold no database connection, no signing key and no PDS secret
beyond their own OAuth client key. They are deployable and removable without
touching the data directory.

## Appendix A. XRPC Inventory

Classes: **Core** (MUST, as the Reference), **Deprecated** (MUST while the
pin serves it, marked removable under Sync 1.1), **Proxy** (MUST forward per
§4.4), **Local** (SHOULD, read-after-write per §4.5), **Unhandled** (no handler at
the pin, so the catch-all proxy of §4.4 answers: 401 without a session,
forwarded with one), **Ext** (`linkjar` profile only),
**Project** (project lexicons, both profiles unless noted).

| Namespace | Core | Other |
|---|---|---|
| `com.atproto.server` | `describeServer`, `createAccount`, `createSession`, `getSession`, `refreshSession`, `deleteSession`, `createAppPassword`, `listAppPasswords`, `revokeAppPassword`, `createInviteCode`, `createInviteCodes`, `getAccountInviteCodes`, `requestEmailConfirmation`, `confirmEmail`, `requestEmailUpdate`, `updateEmail`, `requestPasswordReset`, `resetPassword`, `requestAccountDelete`, `deleteAccount`, `deactivateAccount`, `activateAccount`, `checkAccountStatus`, `getServiceAuth`, `reserveSigningKey` | |
| `com.atproto.repo` | `applyWrites`, `createRecord`, `putRecord`, `deleteRecord`, `getRecord`, `listRecords`, `describeRepo`, `uploadBlob`, `importRepo`, `listMissingBlobs` | |
| `com.atproto.sync` | `getRepo` (with `since`), `getRecord`, `getLatestCommit`, `getRepoStatus`, `listRepos`, `listBlobs`, `getBlob`, `getBlocks`, `subscribeRepos` | Deprecated, served at the pin ([VP-5](../parity/verify-at-pin.md)), removable under Sync 1.1: `getHead`, `getCheckout`, the `commit` parameter of `getRecord`. Unhandled ([VP-7](../parity/verify-at-pin.md)): `listReposByCollection`, `getHostStatus`, `listHosts`, `notifyOfUpdate`, `requestCrawl`. |
| `com.atproto.identity` | `resolveHandle`, `updateHandle`, `getRecommendedDidCredentials`, `requestPlcOperationSignature`, `signPlcOperation`, `submitPlcOperation` | Unhandled: `resolveDid`, `resolveIdentity`, `refreshIdentity`. |
| `com.atproto.admin` | `deleteAccount`, `disableAccountInvites`, `enableAccountInvites`, `disableInviteCodes`, `getInviteCodes`, `getAccountInfo`, `getAccountInfos`, `getSubjectStatus`, `updateSubjectStatus`, `sendEmail`, `updateAccountEmail`, `updateAccountHandle`, `updateAccountPassword`, `updateAccountSigningKey` | `searchAccounts`: as the Reference. |
| `com.atproto.moderation` | `createReport` (forwarded) | |
| `com.atproto.temp` | `checkSignupQueue` | Others: Unhandled. |
| `com.atproto.lexicon` | | Unhandled ([VP-6](../parity/verify-at-pin.md)): `resolveLexicon`. `schema` records are resolved per §5.5. |
| `com.atproto.label` | | Unhandled; header passthrough per §4.4. |
| `app.bsky.actor` | | Local: `getPreferences`, `putPreferences`, `getProfile`, `getProfiles`. |
| `app.bsky.feed` | | Local: `getActorLikes`, `getAuthorFeed`, `getFeed`, `getPostThread`, `getTimeline`. |
| `app.bsky.notification` | | Local: `registerPush`, `unregisterPush`. |
| other `app.bsky.*`, `chat.bsky.*`, labelers | | Proxy. |
| `io.linkjar.account` | | Ext: `getSignupReceipt`, `acknowledgeSignupReceipt`. |
| `io.linkjar.pds.audit` | | Project: the `checkpoint` record type (§20.3). |
| `internal.*` | | Project: ops-listener methods (§22.1), private listener only. |

Unit 0 reconciled this table against the Reference's handler tree. Unit 1
replaced the 501 classifications after the Harness showed that the pin
answers them through the catch-all proxy.

## Appendix B. Extension Traits (Informative)

```rust
pub trait SignupGate: Send + Sync {
    fn evaluate(&self, req: &SignupRequest, abuse: &AbuseLists) -> Result<GateDecision, GateError>;
}

pub enum GateDecision { Allow, Deny(DenyReason), Challenge(ChallengeKind) }

pub trait HandlePolicy: Send + Sync {
    fn suggest(&self, ctx: &SignupContext, taken: &dyn Fn(&Handle) -> bool) -> Option<Handle>;
    fn check(&self, handle: &Handle) -> Result<(), HandleRejection>;
    fn authorize_rename(&self, tx: &mut AccountTx, did: &Did, from: &Handle, to: &Handle)
        -> Result<RenameTicket, RenameRefused>;
    fn handle_host(&self, host: &str, path: &str) -> Option<HostResponse>;
    fn on_unverifiable(&self, did: &Did) -> WritePolicy;
}

pub trait IdentityProviders: Send + Sync {
    fn list(&self) -> Vec<ProviderButton>;
    fn start(&self, provider: &str, device: &DeviceId, pending: &PendingAuth) -> Result<Redirect, FlowError>;
    fn callback(&self, provider: &str, params: &CallbackParams, device: &DeviceId)
        -> Result<VerifiedIdentity, FlowError>;
    fn store(&self) -> &dyn ExternalIdentityStore;
}

pub trait OperatorRoles: Send + Sync {
    fn authorize(&self, operator: &OperatorIdentity, action: &OpsAction) -> Decision;
}

pub trait StorageIo: Send + Sync {
    fn open(&self, path: &Path, opts: OpenOptions) -> io::Result<Box<dyn IoFile>>;
    fn sync_dir(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove(&self, path: &Path) -> io::Result<()>;
}

pub struct Profile {
    pub signup_gate: Box<dyn SignupGate>,
    pub handle_policy: Box<dyn HandlePolicy>,
    pub identity_providers: Box<dyn IdentityProviders>,
    pub signin_methods: Box<dyn SignInMethods>,
    pub notices: Box<dyn Notices>,
    pub mail: Box<dyn MailTemplates>,
    pub branding: Box<dyn Branding>,
    pub receipt: Box<dyn CreationReceipt>,
    pub hooks: Box<dyn AccountHooks>,
    pub operator_roles: Box<dyn OperatorRoles>,
    pub migrations: Vec<ExtensionMigration>,
}
```

Names and signatures are illustrative. The normative content is §12.

## Appendix C. Configuration

Accepted with the Reference's defaults and meaning:

- **Service**: `PDS_HOSTNAME`, `PDS_PORT`, `PDS_SERVICE_DID`, `PDS_SERVICE_NAME`, `PDS_VERSION`, `PDS_HOME_URL`, `PDS_SERVICE_HANDLE_DOMAINS`, `PDS_DEV_MODE`, `PDS_CONTACT_EMAIL_ADDRESS`, `PDS_PRIVACY_POLICY_URL`, `PDS_TERMS_OF_SERVICE_URL`, `PDS_SUPPORT_URL`.
- **Storage**: `PDS_DATA_DIRECTORY`, `PDS_ACCOUNT_DB_LOCATION`, `PDS_SEQUENCER_DB_LOCATION`, `PDS_DID_CACHE_DB_LOCATION`, `PDS_ACTOR_STORE_DIRECTORY`, `PDS_ACTOR_STORE_CACHE_SIZE`, `PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT`, `PDS_BLOBSTORE_DISK_LOCATION`, `PDS_BLOBSTORE_DISK_TMP_LOCATION`, `PDS_BLOBSTORE_S3_*`, `PDS_BLOB_UPLOAD_LIMIT`, `PDS_MAX_REPO_IMPORT_SIZE`, `PDS_ACCEPTING_REPO_IMPORTS`.
- **Identity**: `PDS_DID_PLC_URL`, `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`, `PDS_PLC_ROTATION_KEY_KMS_KEY_ID`, `PDS_RECOVERY_DID_KEY`, `PDS_DID_CACHE_STALE_TTL`, `PDS_DID_CACHE_MAX_TTL`, `PDS_ID_RESOLVER_TIMEOUT`, `PDS_HANDLE_BACKUP_NAMESERVERS`, `PDS_ENABLE_DID_DOC_WITH_SESSION`.
- **Auth and OAuth**: `PDS_JWT_SECRET`, `PDS_DPOP_SECRET`, `PDS_ADMIN_PASSWORD`, `PDS_OAUTH_TRUSTED_CLIENTS`, `PDS_LEXICON_AUTHORITY_DID`.
- **Entryway (member mode)**: `PDS_ENTRYWAY_URL`, `PDS_ENTRYWAY_DID`, `PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX`, `PDS_ENTRYWAY_PLC_ROTATION_KEY`, `PDS_ENTRYWAY_ADMIN_TOKEN`. Refused until the entryway unit ships.
- **Invites and abuse**: `PDS_INVITE_REQUIRED`, `PDS_INVITE_EPOCH`, `PDS_INVITE_INTERVAL`, `PDS_HCAPTCHA_SITE_KEY`, `PDS_HCAPTCHA_SECRET_KEY`, `PDS_HCAPTCHA_TOKEN_SALT`, `PDS_RATE_LIMITS_ENABLED`, `PDS_RATE_LIMIT_BYPASS_KEY`, `PDS_RATE_LIMIT_BYPASS_IPS`.
- **Firehose and proxy**: `PDS_CRAWLERS`, `PDS_MAX_SUBSCRIPTION_BUFFER`, `PDS_REPO_BACKFILL_LIMIT_MS`, `PDS_BSKY_APP_VIEW_URL`, `PDS_BSKY_APP_VIEW_DID`, `PDS_BSKY_APP_VIEW_CDN_URL_PATTERN`, `PDS_PROXY_*`, `PDS_FETCH_MAX_RESPONSE_SIZE`, `PDS_DISABLE_SSRF_PROTECTION`.
- **Moderation and mail**: `PDS_MOD_SERVICE_URL`, `PDS_MOD_SERVICE_DID`, `PDS_REPORT_SERVICE_URL`, `PDS_REPORT_SERVICE_DID`, `PDS_EMAIL_SMTP_URL`, `PDS_EMAIL_FROM_ADDRESS`, `PDS_EMAIL_DISABLE_CONFIRMATION_LINK`, `PDS_MODERATION_EMAIL_ADDRESS`, `PDS_MODERATION_EMAIL_SMTP_URL`.
- **Branding**: `PDS_LOGO_URL`, `PDS_PRIMARY_COLOR`, `PDS_ERROR_COLOR`, `PDS_WARNING_COLOR`, `PDS_SUCCESS_COLOR`, `PDS_INFO_COLOR`, `PDS_BACKGROUND_LIGHT_URL`, `PDS_BACKGROUND_DARK_URL`.
- **Refused in version 1**: `PDS_REDIS_SCRATCH_*`.

`PDS_BLOBSTORE_S3_*` are `BUCKET`, `REGION`, `ENDPOINT`, `FORCE_PATH_STYLE`,
`ACCESS_KEY_ID`, `SECRET_ACCESS_KEY`, `UPLOAD_TIMEOUT_MS` and
`REQUEST_TIMEOUT_MS`. The bucket and `PDS_BLOBSTORE_DISK_LOCATION` together
are refused, and so is one half of the credentials, both as at the pin. The
Candidate needs the region and the credentials at the start and refuses to
start without them; the Reference's client looks for them at the first
request. Without `REGION` the Candidate reads `AWS_REGION` and
`AWS_DEFAULT_REGION`, and without both halves of the credentials it reads
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN`. It
reads no credentials from a profile file, an instance role or a web
identity, which the Reference's client does. The bucket is the first
segment of the path when `FORCE_PATH_STYLE` is set or the endpoint is an
address, and a label of the host otherwise.

New in the Candidate:

- `PDS_FAIL_POINTS`: fail points to arm (§19.2). Read in dev mode only.
- `PDS_PROFILE`: `stock` (default) or `linkjar`.
- `PDS_METRICS_ADDRESS`: the private metrics listener; unset disables it.
- `PDS_OPS_ADDRESS`: the private ops listener (§22.1); unset disables it.
- `PDS_OTLP_ENDPOINT`: OTLP/HTTP export target (§14.4); unset disables it.
- `PDS_AUDIT_KEY_*`: the audit checkpoint key reference (§20.3). As of
  unit 7 the Candidate reads `PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX`, the
  32-byte seed as 64 hexadecimal digits.
- `PDS_AUDIT_CHECKPOINT_REPO`: the DID of the operator's account on this
  server whose repository publishes the checkpoints (§20.3). Unset, the
  checkpoints stay in `audit.sqlite`.
- `PDS_EXTERNAL_<PROVIDER>_CLIENT_ID` and `_CLIENT_SECRET` for Apple, Google and GitHub (`linkjar` profile).

## Appendix D. Open Decisions

| Id | Decision | Owner unit |
|---|---|---|
| D1 | SQLite driver: `rusqlite` with bundled SQLite on a blocking pool, opened through `StorageIo`. Default: `rusqlite`. | 0 |
| D2 | Resolved 2026-10-12: the repository engine is written in this repository, in `pds-repo`. `rsky-repo` was run on the same vectors and workload as published (0.0.6) and from its main branch (0.1.4); the [decision record](research/2026-10-12-repository-crate.md) has the evidence. | 2 |
| D3 | HTML templating crate for §11.4. | 5 |
| D4 | Resolved 2026-10-08: the Cargo workspace is at the repository root. The reference image build is under `legacy/`; it stays buildable for security fixes until cutover and is removed afterwards. | 0 |
| D5 | Resolved 2026-10-08: dual MIT and Apache-2.0. | 0 |
| D6 | Resolved 2026-10-08: the Reference's OAuth key set is one HS256 key from `PDS_JWT_SECRET`. Moving to an asymmetric key set with `jwks` publication is a post-cutover change. | 5 |
| D7 | Whether handle-host serving (§13.2) replaces the Caddy snippet on the host or sits behind it. | 8 |
| D8 | Metric names and the `linkjar/infra` alert rules that depend on them. | 8 |
| D9 | Resolved 2026-10-08: permission sets and lexicon resolution (§5.5) are version 1. | 5 |
| D10 | Resolved 2026-10-08: `verify` recomputes blob reference integrity (§18.2). | 3 |
| D11 | Audit checkpoint key custody: KMS, HSM or file under the host's secret store. | 7 |
| D12 | Resolved 2026-10-08: entryway mode is a post-cutover unit (§21.3). | post-cutover |
| D13 | Atproto Spaces: out of scope for version 1; the `ActorStore` seam reserves room for per-space tables so the upstream per-actor design can be adopted when schemas stabilise. | post-cutover |
| D14 | OAuth lifetimes where the Reference at the pin exceeds the specification's bounds: follow the pin through the rollback window, tighten afterwards. | 5 |
| D15 | Resolved 2026-10-09: where the Reference answers 500 to a client error, the Candidate answers 400 `InvalidRequest`. Four cases: `createRecord` and `applyWrites` on a key that exists, `createAppPassword` with a name in use, an `atproto-proxy` value that does not start with a DID (§2.3, DD-1 to DD-4). 2026-10-13: three more, `importRepo` with bytes that are not a CAR and `createAccount` with the handle or the email of a deactivated or taken-down account (DD-15 to DD-17). | 4, 5 |
| D16 | Resolved. Hardenings that the pin does not have and a client could observe. 2026-10-09: the Candidate refuses a replayed service token (§5.4; §2.3, DD-5). 2026-10-12: the Candidate collects an upload that no record references after one hour (§6.4; §2.3, DD-10), and it keeps no grace period before a dereferenced blob is deleted, as the pin. | 4 |
| D17 | Resolved 2026-10-12: where the Candidate is stricter than the Reference at the pin on data from elsewhere, it keeps its check, and each case is a deliberate difference with a scenario that records what the pin does (§2.3, DD-6 to DD-9 and DD-11 to DD-13). 2026-10-13: the import also checks that the commit names the account and that the account's key signed it (DD-14). The pin stays at 0.5.34 and the legacy build carries upstream's shared-node refusal of 0.5.37 as a ninth patch (§13). | 4, 5, 8 |

## Appendix E. Changes in Revision 1

- Decisions of 2026-10-08 applied throughout (home, licence, NSID authority,
  cluster shape, cutover bar, simulation, audit scope, patch pin, pages,
  shadow runs, reports, local MCP, work mode).
- C6 limited to protocol responses. Candidate-only state moved to
  `audit.sqlite` and `ops.sqlite` (§8.1).
- Storage seams (§8.4, §12.10), two-mode account crate (§21.3), simulation
  (§19), audit log (§20), topologies (§21), sidecars (§22), operator roles
  (§12.11), ops and OTLP listeners (§4.1, §14.4), gate list (§14.5).
- From the atproto.com gap report: commit and frame size limits, diff CAR
  contents, deprecated `tooBig` and `blobs`, key-rotation commits, MST and
  CBOR decode limits, import hygiene, path and key syntax, TID rules, three
  validation modes, data-model rules, blob upload and lifecycle rules and
  headers, no transcoding, legacy JWT `typ`, service-auth `kid`, claims and
  `aud` fragment, proxy preconditions and labeler headers, status
  conventions, admin scope of invite creation, CORS, `Atproto-Repo-Rev`,
  `cursor=0`, connection errors, `seq` range and reset margin, `#sync` at
  lifecycle moments, lifecycle event ordering, wall-clock `rev`,
  `handle.invalid`, handle TLDs and resolution rules, DID document contract
  and limits, low-S for P-256, OAuth metadata fields, token response fields,
  client metadata and localhost and redirect rules, confidential-client
  binding, PKCE and code hygiene, untrusted-client display, transitional
  scope semantics, permission semantics and set resolution, domain
  separation, key custody, hostname rule, hosting statuses; the five
  contradictions resolved; the fifteen under-specified items given numbers
  or marked "verify at pin".
- Appendix A marks Sync 1.1 removals and adds project lexicons; Appendix C
  adds the new variables; Appendix D resolves D5, D6, D9, D10, D12 and adds
  D11, D13, D14.

## Appendix F. Changes in Revision 2

Revision 2 follows unit 1, the parity harness. Every change states what the
reference image did under a Harness scenario; the scenario for each is named
in [verify-at-pin.md](../parity/verify-at-pin.md).

- "Verify at pin" marks resolved: CORS (§4.2), the per-route rate limiters
  (§4.3), the legacy session lifetimes and refresh grace period (§5.1), the
  blob lifecycle (§6.4), and the lifetime of a pushed request (§11.3). The
  term's definition in §0 is joined by "at the pin".
- A method with no handler goes to the catch-all proxy and is never 501
  (§4.2, §4.4, Appendix A); the class "Relay-only" is replaced by
  "Unhandled".
- Status conventions stated as observed: `AuthMissing`, `InvalidToken`,
  `ExpiredToken`, the wrong HTTP method, `PayloadTooLarge` (§4.2).
- Overlaid reads carry `Atproto-Upstream-Lag`, and when the overlay applies
  (§4.2, §4.5).
- The proxy serves a deactivated account; the labeler header is not parsed;
  upstream errors and failures (§4.4).
- Service auth: the error names, the two methods that accept it, no `jti`
  replay check, and the expiry rules of `getServiceAuth` (§5.4).
- Blob uploads and `getBlob` errors as observed (§6.4).
- `subscribeRepos` without an upgrade, the close code, `cursor=0`, and a
  non-integer cursor (§7.2, §7.3). The Reference's one-second poll (§7.4).
  When `requestCrawl` is sent (§7.6).
- The lifecycle event sequences of account creation, reactivation, takedown
  and PLC operations (§10.4).
- OAuth: the refusals of PAR, the lifetimes from the pinned constants, the
  effects of code replay and refresh reuse, DPoP errors, the client metadata
  cache, and the hostnames a client may not use (§11.2, §11.3).
- The patch stack at the patch pin has eight patches: `102-signup-journey`
  and `103-invite-handoff` are added (§13).
- The Harness as built: one stack per target, fixtures, transcripts (§17.1).
- Appendix D adds D15 and D16.

## Appendix G. Changes in Revision 3

Revision 3 applies two owner's decisions of 2026-10-09.

- §2.3 is new: the deliberate differences between the Candidate and the
  Reference, DD-1 to DD-5. §2.2 names them as the second permitted kind of
  divergence, C6 excepts them, and the definition of "at the pin" in §0
  refers to them.
- D15 is resolved: 400 `InvalidRequest` where the Reference answers 500 to a
  client error, in four cases (§4.4, §5.2, §6.3). The fourth case is
  restated as observed: at the pin a proxy value that does not start with a
  DID answers 500, and a DID that does not resolve answers 400.
- D16 is resolved in part: the Candidate refuses a replayed service token,
  and §5.4 says when a token is spent. The two blob hardenings stay open.
- The comparison of the Reference with the Candidate may record the
  deliberate differences and no others (§17.3).

## Appendix H. Changes in Revision 4

Revision 4 follows unit 2, the repository crate decision.

- D2 is resolved: the repository engine is written in this repository
  (§3.1, Appendix D). §3.2 names the DAG-CBOR reader and writer of
  `pds-types` in place of an IPLD codec crate.
- §6.1 gives the bounds of a tree node and of tree depth as numbers, the
  key length, and the rules for a tree the server did not build.
- §16.2 points to the engine's micro-benchmarks for S1 to S3. The targets
  are unchanged.
- §17.2 adds the fixtures written by `@atproto/repo` at the pin.
- Appendix D adds D17, open: what the server answers to an imported tree
  that is not valid.

## Appendix I. Changes in Revision 5

Revision 5 applies three decisions that the owner took on 2026-10-12.

- The pin stays at 0.5.34. The legacy build carries upstream's shared-node
  fix (#5584) as a ninth patch (§13).
- D17 is resolved: the Candidate keeps its checks on data from elsewhere.
  §2.3 gains DD-6 to DD-9 and DD-11 to DD-13 in a second table, for
  differences that are decided and that no scenario has yet recorded on the
  Reference. §6.1 names the decision.
- D16 is resolved: the Candidate collects an upload that no record
  references after one hour (§6.4, DD-10) and keeps no grace period for a
  dereferenced blob.

## Appendix J. Changes in Revision 6

Revision 6 follows unit 3, the read-only server.

- §4.2 states how the method name is read from a path and which HTTP
  methods an `/xrpc/` path answers. It states the two phases of parameter
  validation, which run before the credential check. It states the answer
  to a path that is not an XRPC path and to an HTTP method that is not
  served.
- §6.6 states the block order of `sync.getRepo`, `sync.getRecord` and
  `sync.getBlocks`, and that the export is the Reference's bytes for the
  same data directory.
- §14.2 adds the `health` command.
- §17.1 describes the restore run.
- §18.2 lists what `verify` checks, drops the comparison of the root's
  revision with the commit's, and names the restore run as the oracle. It
  states that a break in `repo_seq` is reported and is not a difference,
  and that a read of a database in WAL mode creates the `-shm` and `-wal`
  files beside it.
- §2.3: the scenarios of DD-11 and DD-12 move from unit 3 to unit 5. DD-6
  moves from the second table into the table of observed differences, with
  the Reference's answers that `default/15-import` recorded.
- Corrections of fact, which change no requirement. §0 names
  `legacy/Dockerfile` as the build of the Reference; the root `Dockerfile`
  builds the Candidate. §3.1 and D4 state the workspace placement as
  resolved on 2026-10-08, with the reference image build under `legacy/`.
  §13 links the document of patch 104. §14.5 says that devenv defines the
  rustfmt and clippy hooks and does not install them. O7 in §17.3 names
  `legacy/tests/*.mjs`. §17.4 names plan.md as the place of the delivery
  units. D17 in Appendix D has the owner units 4, 5 and 8.

## Appendix K. Changes in Revision 7

Revision 7 follows unit 4 and applies the owner's decisions of 2026-10-13.

- §2.3: DD-7 to DD-10 move into the table of observed differences, with
  what the Reference answers. DD-14 and DD-15 are new and observed; DD-16
  and DD-17 are new and wait for unit 5. The section names the scenarios
  that record each difference, and states that two refusals of one tree
  may carry different messages.
- §3.1 and §3.2: the crates that own the write path, the notes of commits
  in flight and the fail points; what `pds-sim` holds; `aws-lc-rs` for
  outbound TLS.
- §6.3: a batch that updates or deletes a missing key is 500 in both
  servers. A refused write carries the Reference's message.
- §6.4: the content types the Candidate recognises; a blob is served only
  when its row is permanent.
- §6.5: the import checks the account and the signature. As at the pin it
  stores a new revision, appends no event, and does not write the account
  database's root. The earlier text required `#sync` after an import.
- §7.2: the server sends no ping.
- §7.4: how the Candidate orders a commit and its event, the note in
  `ops.sqlite`, and the repair at startup.
- §7.5: a page of the log ends after 500 events or 4 MiB; when a subscriber
  is too slow.
- §8.1: what `ops.sqlite` holds.
- §10.4: an import emits nothing; which rows unit 4 holds.
- §17.1 describes the writes run.
- §18.2: a root in the account database that is behind the store is counted
  and is not a difference.
- §19.2 states what of the simulation is written and what is not, and
  defines `PDS_FAIL_POINTS`. Appendix C lists the variable.
- Appendix D: D15 and D17 carry the decisions of 2026-10-13.

## Appendix L. Changes in Revision 8

Revision 8 follows the first part of unit 4b, the S3-compatible blob store.

- §2.3: DD-18 is new and observed in the Harness. DD-19 is new; the
  Candidate's side is held by `crates/linkjar-pds/tests/writes.rs`, and no
  Harness scenario reads the firehose after an import yet.
- §6.5: an import into an active account appends `#sync` (decision of
  2026-10-13). The question that revision 7 left open is closed.
- §6.4: what both servers answer to `getBlob` of an unreferenced upload on
  each store.
- §8.1: the object names of the S3-compatible store, how the Candidate
  uploads, retries and times out.
- §17.1 describes the stage of the writes run on the S3-compatible store.
- §19.2: the seam below SQLite and the VFS that loses power; the network
  runs on `turmoil`; what each models and what it does not.
- §10.1: what of account creation the first part of unit 5 built. Legacy
  sessions and app passwords (§5.1, §5.2) are built as specified.
- Appendix C lists the `PDS_BLOBSTORE_S3_*` variables, what the Candidate
  needs at the start, and which sources of credentials it reads.

## Appendix M. Changes in Revision 9

Revision 9 follows the ninth part of unit 5: OAuth and read-after-write.

- §9.2: a handle under `.test` is accepted in every mode, as at the pin.
  Revisions 1 to 8 accepted it only with `PDS_DEV_MODE`. The owner has not
  confirmed the change.
- §11.1: `dpop_signing_alg_values_supported` contains `ES256`; the
  Candidate lists `ES256` and `ES256K`. The CORS answers of the OAuth
  routes are stated.
- §11.5 is new: how the Candidate's authorization server differs from the
  Reference's, and what of §11 is not built.
- §4.5 is built as specified, for the six methods of the Reference, and
  `registerPush` goes to the service that the request names.
- Of the variables of Appendix C the Candidate now reads
  `PDS_DPOP_SECRET`, `PDS_OAUTH_TRUSTED_CLIENTS`,
  `PDS_DISABLE_SSRF_PROTECTION` and `PDS_BSKY_APP_VIEW_CDN_URL_PATTERN`.

## Appendix N. Changes in Revision 10

Revision 10 follows unit 6, the `linkjar` profile.

- §13.2: which headers the server sets on a handle host when the Caddy
  snippet may be in front.
- §13.5 is new: how the profile is built and what is not built.
- §12: the `Profile` trait has the handle policy (§12.4), the identity
  providers (§12.3), the creation receipt (§12.8), the sign-up guard
  (§12.2) and the notices (§12.6). The stock profile answers each as the
  Stock Reference behaves.
- §11.5: the account pages are `/account` and `/account/<did>/manage`. A
  sign-up on the pages opens no legacy session.
- §10: `com.atproto.admin.disableInviteCodes`, `disableAccountInvites` and
  `enableAccountInvites` are built.

## Appendix O. Changes in Revision 11

Revision 11 follows unit 7, the audit log.

- §20.5 is new: the entry, the store, what is recorded, the checkpoint,
  its publication, the bundle, the verifier and the seal as built, and
  what is not built.
- Appendix C: `PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX` is the form of the
  key that the Candidate reads, and `PDS_AUDIT_CHECKPOINT_REPO` is new.
- §14.2: the commands `audit seal`, `audit export` and `audit verify`.

# LinkJar PDS Specification

Status: Draft, revision 0, 2026-10-08
Normative keywords: **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, **MAY** ([RFC 2119](https://www.rfc-editor.org/rfc/rfc2119))

LinkJar PDS is an AT Protocol Personal Data Server written in Rust. It is
compatible with the reference TypeScript implementation that this repository
builds and patches today. The account-handling behaviour that LinkJar applies
as source patches becomes native extension points, so the same server serves a
stock operator and LinkJar without a fork. This document is the normative
definition of that server. The decision record, delivery units and status are
in [README.md](README.md).

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
- [Appendix A. XRPC Inventory](#appendix-a-xrpc-inventory)
- [Appendix B. Extension Traits (Informative)](#appendix-b-extension-traits-informative)
- [Appendix C. Configuration](#appendix-c-configuration)
- [Appendix D. Open Decisions](#appendix-d-open-decisions)

## 0. Conventions and Terminology

- **Reference**: the pinned upstream PDS, `@atproto/pds@0.5.34` at commit
  `7ca16cc6989f8247637615aca17c5abb911b8fb1` ([upstream.json](../../upstream.json)),
  built by this repository's `Dockerfile` with the `production` patch profile.
- **Stock Reference**: the same build with `PATCH_PROFILE=none`.
- **Candidate**: a build of LinkJar PDS under test.
- **Profile**: the set of extension implementations a Candidate runs with.
  Two profiles are defined: `stock` (§12) and `linkjar` (§13).
- **Harness**: the parity harness in §17.
- **Data directory**: the on-disk state in `PDS_DATA_DIRECTORY` (§8).
- **Operator**: whoever runs the server. LinkJar is one operator.
- Section references of the form "SPEC §n" point to the LinkJar protocol
  specification in the `linkjar.io` repository. This document never changes
  that specification: hosted accounts are ordinary AT Protocol accounts and the
  client-side key model is unchanged.

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
  Crash boundaries that the Reference leaves open are closed (§7.4).
- **G4 Public.** The `stock` profile is a complete PDS for any operator, with
  a licence and build an outside operator can use.
- **G5 In-place cutover.** The hosted LinkJar PDS moves from the Reference to
  the Candidate without changing hostnames, DIDs, signing keys, sessions or
  firehose cursors, and can roll back during a defined window (§18).

Non-goals for version 1:

- Entryway deployments (`PDS_ENTRYWAY_*`). The Candidate MAY refuse to start
  when these are set.
- Redis scratch storage. Rate limiters are in-process.
- Running a relay, AppView, feed generator or labeler.
- Multi-process or multi-replica service of one data directory. One process
  owns a data directory, as with the Reference.
- Changing LinkJar's client-side encryption, key or record model.

## 2. Compatibility Baseline

### 2.1 Pins

- The lexicons under `lexicons/com/atproto/**` at the Reference commit, and
  the `app.bsky` lexicons the Reference serves locally or munges (§4.5), are
  vendored into the repository. Request and response types MUST be generated
  from those files. Hand-written types MUST NOT diverge from them.
- LinkJar's own lexicons (`io.linkjar.account.*`, §13.3) are vendored from
  the `099b-signup-receipt.patch` source.
- Upstream bumps change the pin in one commit, regenerate the types, and rerun
  the Harness against the new Reference image.

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
  distinguish Candidate from Stock Reference except by the version string.

Divergence from the Reference is permitted only through §12 extension points,
and only in a non-stock profile.

## 3. Architecture

### 3.1 Crates

The repository root becomes a Cargo workspace (Appendix D, D4). Crate names
are provisional.

| Crate | Owns |
|---|---|
| `pds-types` | DID, handle, NSID, TID, record key, CID, AT URI syntax and validation; lexicon-generated request and response types. |
| `pds-lexicon` | Lexicon loading, record validation (strict for known, permissive for unknown), schema resolution for permission sets. |
| `pds-repo` | MST, commits, signing, CAR read and write, block stores, diffs. Unit 2 decides between the `rsky-repo` crate and an in-house implementation. |
| `pds-store` | SQLite access for the account, sequencer, DID-cache and actor databases; migrations; the actor LRU. |
| `pds-blob` | Blob stores: disk and S3-compatible; upload staging, hashing, quarantine. |
| `pds-identity` | DID resolution with cache, PLC client, handle resolution and verification, signing-key custody. |
| `pds-xrpc` | axum routing, XRPC request parsing, error mapping, rate limiting, proxying, read-after-write. |
| `pds-sequencer` | The event log, firehose subscriptions, backfill, crawler notification. |
| `pds-account` | Account lifecycle, sessions, app passwords, email tokens, invites, admin, mail. |
| `pds-oauth` | The authorization server: metadata, PAR, authorize, token, DPoP, client metadata, device sessions, server-rendered UI, account pages. |
| `pds-ext` | The extension traits of §12 and the `stock` implementations. |
| `pds-ext-linkjar` | The `linkjar` profile (§13). Compiled in behind the Cargo feature `linkjar`. |
| `linkjar-pds` | The binary: configuration, wiring, CLI (§14.2), telemetry. |
| `parity/` | The Harness (§17), a TypeScript project driven by the official SDK. |

### 3.2 Runtime

- Tokio multi-threaded runtime; axum on hyper for HTTP/1.1 and WebSocket.
  TLS termination is the reverse proxy's job, as with the Reference.
- SQLite through a synchronous driver with the bundled library, run on a
  dedicated blocking pool. Each actor database has one writer at a time.
- Cryptography from RustCrypto: `k256`, `p256`, `sha2`; DAG-CBOR through an
  IPLD codec crate that produces canonical encoding; `cid` and `multihash`.
- No `unsafe` outside vetted dependencies: `#![forbid(unsafe_code)]` in every
  workspace crate.
- Background work (sequencer crawl notifications, scheduled account deletion,
  blob garbage collection, mail outbox, DID-cache refresh) runs as supervised
  tasks in the same process and is observable through metrics.

### 3.3 Determinism hooks

Dev mode (`PDS_DEV_MODE=true`) MUST allow injecting the clock and the TID
source, so the Harness can obtain identical `rev` values from Reference and
Candidate. Production builds MUST ignore these hooks.

## 4. HTTP and XRPC Surface

### 4.1 Non-XRPC routes

| Route | Behaviour |
|---|---|
| `GET /` | The Reference's plain-text server banner. |
| `GET /robots.txt` | As the Reference. |
| `GET /xrpc/_health` | `{"version": "<PDS_VERSION>"}`; 503 with `{"version", "error"}` when the account database is unavailable. |
| `GET /.well-known/atproto-did` | Resolves the `Host` header to a hosted actor and returns its DID as `text/plain`; 404 otherwise. |
| `GET /.well-known/oauth-authorization-server`, `GET /.well-known/oauth-protected-resource` | §11.1. |
| `/oauth/*`, `/account/*` | §11. |
| `GET /metrics` | Prometheus text format, served on a separate private listener (§14.4), never on the public port. |

### 4.2 XRPC semantics

- Queries are `GET`, procedures are `POST`, subscriptions are WebSocket
  upgrades, all under `/xrpc/<nsid>`.
- Parameters and bodies are validated against the lexicon. Unknown query
  parameters are ignored. Invalid input returns 400 `InvalidRequest` with the
  Reference's message where the Harness compares messages.
- Errors are `{"error": "<Name>", "message": "<text>"}`. The Candidate MUST
  use the Reference's error names, including `AuthenticationRequired`,
  `ExpiredToken`, `InvalidToken`, `AccountTakedown`, `AccountDeactivated`,
  `RepoNotFound`, `RecordNotFound`, `BlobNotFound`, `InvalidSwap`,
  `MethodNotImplemented`, `RateLimitExceeded`, `UpstreamFailure`,
  `UpstreamTimeout`, `InternalServerError`.
- Methods with no handler and no proxy target return 501
  `MethodNotImplemented`.
- Response bodies are `application/json` unless the lexicon declares another
  encoding (`getBlob`, `getRepo`, `getRecord` and `getBlocks` stream CAR or
  raw bytes).

### 4.3 Rate limits

Rate limits default on (`PDS_RATE_LIMITS_ENABLED`). The Candidate MUST
reproduce the Reference's limiters and their `RateLimit-Limit`,
`RateLimit-Remaining`, `RateLimit-Reset` and `RateLimit-Policy` headers:

| Limiter | Window | Points | Keyed by |
|---|---|---|---|
| `global-ip` | 5 minutes | 3,000 | client IP |
| `repo-write-hour` | 1 hour | 5,000 (create 3, put 2, delete 1) | DID |
| `repo-write-day` | 24 hours | 35,000 (same weights) | DID |
| per-route limiters | as declared on each Reference handler | | IP or DID |

`PDS_RATE_LIMIT_BYPASS_KEY` and `PDS_RATE_LIMIT_BYPASS_IPS` bypass as in the
Reference. Client IP comes from the configured trusted-proxy policy, never
from an unverified header.

### 4.4 Proxying

Requests for methods the Candidate does not serve locally are forwarded as the
Reference's pipethrough does:

- The target is the `atproto-proxy` header (`<did>#<service id>`), resolved
  through the DID document, or the configured AppView for `app.bsky.*`.
- Forwarding requires an authenticated session and mints a service-auth JWT
  (§5.4) for the target, with `lxm` set to the method.
- Header filtering, `atproto-accept-labelers` passthrough, timeouts
  (`PDS_PROXY_*`), response size caps, retry policy and HTTP/2 preference
  follow the Reference configuration.
- Procedures are proxied only where the Reference proxies them.

### 4.5 Read-after-write

The `app.bsky` methods the Reference serves locally or munges (preferences,
push registration, profile and feed reads with local record overlay) are
classified in Appendix A. The Candidate SHOULD implement them in version 1
and MUST implement them before the official Bluesky app is claimed compatible.

## 5. Authentication and Authorization

### 5.1 Legacy sessions

- Access and refresh tokens are HS256 JWTs signed with `PDS_JWT_SECRET`,
  with `scope` in `com.atproto.access`, `com.atproto.refresh`,
  `com.atproto.appPass`, `com.atproto.appPassPrivileged`,
  `com.atproto.signupQueued`, `com.atproto.takendown`; `sub` the DID; `aud`
  the service DID; `jti` on refresh tokens.
- Lifetimes, refresh rotation and the grace period for a just-rotated refresh
  token (`used_refresh_token`) match the Reference.
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

### 5.3 Admin

HTTP Basic authentication as `admin` with `PDS_ADMIN_PASSWORD` grants the
admin role for `com.atproto.admin.*` and the privileged account methods.

### 5.4 Service auth

- Inbound: an ES256K or ES256 JWT signed by the caller's repository signing
  key, `iss` the caller DID, `aud` our service DID, `jti` checked against a
  replay cache, and the Reference's maximum age. When `lxm` is present it
  MUST equal the method NSID; tokens without `lxm` are accepted only where
  the Reference accepts them. The key comes from the resolved DID document
  (§9).
- Outbound: the same shape, signed with the actor's signing key, for proxying
  and for `getServiceAuth`, with the Reference's expiry cap and `lxm` rules.

### 5.5 OAuth sessions

DPoP-bound access tokens issued by §11 are verified by the resource-server
half of the same crate: signature against the server key set, `cnf.jkt`
against the DPoP proof, nonce, scope. Scope grammar and enforcement follow the
Reference's `auth-scope.ts` at the pin: `atproto`, `transition:*`, and the
permission-set and granular scopes it accepts.

## 6. Repository Engine

### 6.1 Data model

- Commits are version 3: `did`, `version`, `data` (MST root), `rev` (TID),
  `prev` (null), `sig`.
- The MST follows the AT Protocol repository specification and the upstream
  interop vectors: keys are `<collection>/<rkey>`, layer by leading zero bits
  of SHA-256, canonical DAG-CBOR nodes.
- CIDs are CIDv1, `dag-cbor` with SHA-256 for nodes and records, `raw` with
  SHA-256 for blobs.
- Signatures are secp256k1 (deterministic, low-S) or P-256 depending on the
  actor's key.

### 6.2 Record keys and revisions

- Default record keys are TIDs from a monotonic clock. NSID key types declared
  in the lexicon are enforced. Record key syntax is validated.
- `rev` is a TID strictly greater than the previous commit's `rev` for that
  repository, even across restarts.

### 6.3 Writes

- `createRecord`, `putRecord`, `deleteRecord`, `applyWrites` (at most 200
  operations) with `swapCommit` and `swapRecord` returning `InvalidSwap`, and
  the `validate` flag with the Reference's semantics: known lexicons validate
  strictly, unknown collections validate structurally unless `validate=true`.
- A commit writes the new blocks, record rows, blob references and the
  sequencer event, then publishes. The commit path MUST update the MST
  incrementally and MUST NOT rewrite unchanged records or blocks.
- Writes are refused for deactivated, taken-down and deleted repositories with
  the Reference's errors.

### 6.4 Blobs

- Uploads stream to staging while hashing; the declared MIME type is
  reconciled with sniffed content as in the Reference; `PDS_BLOB_UPLOAD_LIMIT`
  (default 5 MiB) caps size.
- A blob becomes durable when a record references it. Unreferenced blobs are
  garbage-collected after the Reference's grace period. `listMissingBlobs`
  reports referenced blobs the store lacks.
- `getBlob` serves by CID with the stored content type and honours takedowns.

### 6.5 Preferences and imports

- `app.bsky.actor.getPreferences` and `putPreferences` store in the actor
  database and apply the Reference's scope restrictions for app passwords and
  OAuth scopes.
- `importRepo` accepts a CAR up to `PDS_MAX_REPO_IMPORT_SIZE` when
  `PDS_ACCEPTING_REPO_IMPORTS` is set, with the Reference's structural checks.

### 6.6 Determinism

Given the same starting repository and the same ordered writes with explicit
record keys, the Candidate MUST produce the same MST root CID as the
Reference. Given also the same `rev` (§3.3), it MUST produce the same commit
CID. The Harness checks both after every write scenario.

## 7. Sequencer and Firehose

### 7.1 Event log

Events are appended to `repo_seq` with a monotonically increasing `seq`, the
DID, the event type, the DAG-CBOR body and the sequencing time. Types:
`#commit` (with `ops`, the CAR slice of new blocks, `blobs`, `since`, `rev`,
`prevData`, `tooBig`), `#sync`, `#identity`, `#account` and `#info`.

### 7.2 Framing

Each message is two DAG-CBOR objects: a header `{"op": 1, "t": "#<type>"}`
and the body. Errors are `{"op": -1, "error": "<Name>", "message": "<text>"}`
followed by close.

### 7.3 Cursor semantics

| Request | Behaviour |
|---|---|
| no `cursor` | live events only |
| `cursor` at or below the latest `seq` | backfill from `cursor` exclusive, then switch to live without gaps or duplicates |
| `cursor` above the latest `seq` | error `FutureCursor`, close |
| `cursor` older than `PDS_REPO_BACKFILL_LIMIT_MS` (default 1 day) | `#info` with `OutdatedCursor`, then stream from the earliest retained event |
| subscriber falls behind `PDS_MAX_SUBSCRIPTION_BUFFER` events (default 500) | error `ConsumerTooSlow`, close |

### 7.4 Durability and crash boundaries

- A commit's sequencer row MUST become durable in the same step as the commit,
  or the Candidate MUST detect on startup every repository whose head is ahead
  of its last sequenced event and emit a `#sync` event for it. The Reference
  can lose an event at this boundary; the Harness injects a crash here and
  requires the relay to converge.
- Account status changes, handle changes and deletions emit their events in
  the same transaction as the state change.

### 7.5 Fan-out

- Each event is encoded once and shared across subscribers by reference.
- Each subscriber has a bounded queue. Backfill reads the log in pages; it
  never materialises the whole range in memory.
- Keepalive pings and idle timeouts match the Reference.

### 7.6 Crawlers

On startup and after the first commit of a new repository the Candidate calls
`com.atproto.sync.requestCrawl` on every host in `PDS_CRAWLERS`, with
backoff. `notifyOfUpdate` is accepted and treated as the Reference treats it.

## 8. Storage and Data Directory

### 8.1 Layout

The Candidate MUST use the Reference's layout so that the backup, recovery
and monitoring tooling in `linkjar/infra` keeps working:

| Path | Content |
|---|---|
| `$PDS_DATA_DIRECTORY/account.sqlite` | accounts, sessions, OAuth, invites, email tokens, extension tables |
| `$PDS_DATA_DIRECTORY/sequencer.sqlite` | `repo_seq` |
| `$PDS_DATA_DIRECTORY/did_cache.sqlite` | DID document cache |
| `$PDS_ACTOR_STORE_DIRECTORY/<first two hex of sha256(did)>/<did>/store.sqlite` | the actor's repository, records, blob index, preferences |
| the sibling `key` file | the actor's signing key, byte-identical to the Reference's format |
| `$PDS_ACTOR_STORE_DIRECTORY/reserved_keys/<did>` | reserved signing keys |
| `$PDS_BLOBSTORE_DISK_LOCATION/<did>/<cid>` | stored blobs; uploads stage under `tempt/<did>/` (`PDS_BLOBSTORE_DISK_TMP_LOCATION` overrides) and taken-down blobs move to `quarantine/<did>/<cid>`, both siblings of the blob root |

The individual database locations are overridable by the same variables as
the Reference (Appendix C).

### 8.2 Schema

- **Schema version 1 is the Reference schema** for the actor, sequencer and
  DID-cache databases, and the Reference schema plus the `linkjar` profile's
  migrations (`007a` to `007d`, §13) for the account database. The Candidate
  MUST serve a data directory the Reference wrote, and the Reference MUST be
  able to serve a data directory the Candidate wrote, until §18 closes the
  rollback window.
- Migrations are forward-only and run by the `migrate` command (§14.2), never
  implicitly at serve time in production. The Candidate records applied
  migrations in the Reference's migration table with the Reference's names.
- Candidate-only migrations MUST require the explicit operator flag of §18.3.
- Later schema work (for example moving blocks out of SQLite rows) is a
  versioned change after the window closes and is out of scope here.

### 8.3 SQLite behaviour

- WAL journal mode; `PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT` hands
  checkpointing to the backup owner (Litestream), as documented in the
  hosting study.
- Actor databases are opened on demand and kept in an LRU of
  `PDS_ACTOR_STORE_CACHE_SIZE` entries (default 100). One writer per actor is
  serialised in-process; readers use separate connections.
- Busy timeouts and `synchronous` level match the Reference.
- The process takes an exclusive lock file in the data directory at startup
  and refuses to start if another server holds it (§18.4).

## 9. Identity

- **DID methods.** `did:plc` for new accounts: the genesis operation is
  signed with the service rotation key (`PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`
  or the KMS variant) and includes `PDS_RECOVERY_DID_KEY` and any
  user-supplied recovery key in the Reference's order. `did:web` accounts are
  accepted for migration only.
- **PLC operations.** `requestPlcOperationSignature` (email token),
  `signPlcOperation`, `submitPlcOperation`, `getRecommendedDidCredentials`
  and `updateAccountSigningKey` behave as the Reference.
- **Handles.** Syntax validation; the Reference's reserved and explicit-slur
  lists are vendored; `PDS_SERVICE_HANDLE_DOMAINS` defines hosted suffixes;
  custom domains verify through DNS `_atproto` TXT with
  `PDS_HANDLE_BACKUP_NAMESERVERS` or HTTPS well-known; `updateHandle` writes
  the PLC operation first and then the local state, and emits `#identity`.
  `resolveHandle` answers for hosted handles and falls back to the AppView.
- **DID cache.** `did_cache.sqlite` with `PDS_DID_CACHE_STALE_TTL` and
  `PDS_DID_CACHE_MAX_TTL`; stale entries refresh in the background.
- **Keys.** Per-actor secp256k1 signing keys in the `key` file; reserved keys
  via `reserveSigningKey`; keys are zeroised when dropped.

## 10. Accounts

- **Creation.** `createAccount` with handle, email, password, invite code,
  optional `did` and `plcOp` for migrations, and the `signupQueued` path
  (`temp.checkSignupQueue` always reports activated). Creation of an existing
  DID through service auth creates a deactivated account. The §12 gate runs
  before any PLC or storage write.
- **Email tokens.** Confirm email, reset password, update email, delete
  account and PLC operation tokens use the Reference's format, lifetimes and
  single-use rules.
- **Status.** `active`, `deactivated` (with `deleteAfter`), `takendown`,
  `suspended`, `deleted`. `checkAccountStatus`, `deactivateAccount`,
  `activateAccount` (verifying the published DID document), and
  `getRepoStatus` follow the Reference.
- **Deletion.** `requestAccountDelete` and `deleteAccount` schedule the
  purge of the actor directory, blobs, sessions and extension rows, and emit
  `#account` with the Reference's status and tombstone behaviour. Admin
  `deleteAccount` does the same without a token.
- **Invites.** `PDS_INVITE_REQUIRED`, `PDS_INVITE_EPOCH`,
  `PDS_INVITE_INTERVAL`, creation, listing, disabling and per-account
  allowances as the Reference.
- **Admin.** `getAccountInfo(s)`, `getSubjectStatus`, `updateSubjectStatus`
  (takedown with blob quarantine), `updateAccountEmail`, `updateAccountHandle`,
  `updateAccountPassword`, `sendEmail`, and invite administration.
- **Moderation.** `createReport` forwards to `PDS_REPORT_SERVICE_*` as the
  Reference does.
- **Mail.** SMTP from `PDS_EMAIL_SMTP_URL` and `PDS_EMAIL_FROM_ADDRESS`; the
  moderation mailer uses its own variables. Token mails that gate a user flow
  are sent synchronously and fail the request on SMTP failure, as the
  Reference does. Notices (§12.6) go through the durable outbox.

## 11. OAuth Authorization Server

### 11.1 Metadata

Both well-known documents carry the Reference's fields and values at the pin,
including `require_pushed_authorization_requests: true`,
`dpop_signing_alg_values_supported: ["ES256"]`,
`code_challenge_methods_supported: ["S256"]`,
`client_id_metadata_document_supported: true`, the supported scopes and the
`protected_resources` list.

### 11.2 Clients

Client metadata is fetched from the `client_id` URL through safe fetch (§15.1),
validated per the AT Protocol OAuth specification, and cached with the
Reference's TTLs. Loopback clients follow the specification's localhost rules.
Confidential clients authenticate with `private_key_jwt` against their JWKS.
`PDS_OAUTH_TRUSTED_CLIENTS` lists metadata URLs whose grants get the
Reference's trusted treatment.

### 11.3 Flows

- **PAR** is mandatory. Requests expire as the Reference's do.
- **Authorize** runs on a device session (cookie, `device` and
  `device_account` tables) with remembered accounts, `prompt`, `login_hint`,
  sign-in (password, or §12.3 providers), sign-up (§12.2 gate), consent, and
  reactivation gates for deactivated accounts.
- **Token**: `authorization_code` with PKCE S256 and DPoP binding
  (`cnf.jkt`), `refresh_token` with rotation and reuse detection, the DPoP
  nonce rotated with `PDS_DPOP_SECRET`, a `jti` replay store, and
  `AccountDeactivated` enforcement.
- **Revoke** and **introspect** per their RFCs with the Reference's
  constraints.
- **Account pages** under `/account` for sessions, devices, sign-out, and the
  §12.5 sign-in methods.
- The Reference's provider key set is one HS256 key built from
  `PDS_JWT_SECRET`, and the DPoP nonce secret is `PDS_DPOP_SECRET`. The
  Candidate MUST sign and verify OAuth access tokens with the same key so
  tokens issued before cutover verify after it. Asymmetric keys for the key
  set are a post-cutover change (Appendix D, D6).

### 11.4 User interface

- Pages are server-rendered from templates compiled into the binary; there is
  no client-side framework bundle. Progressive enhancement scripts MAY be
  inlined with a strict CSP.
- Branding (name, logo, colours, policy links, light and dark backgrounds)
  comes from the Reference's variables and §12.7.
- Pages are usable at 390 px width and meet WCAG 2.1 AA for contrast and
  keyboard operation.
- The UI's JSON routes keep the Reference's CSRF, `Origin`, fetch-metadata,
  ephemeral-token and device checks.

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
  change they accompany, and roll back with it.
- **EX-5** Hooks have bounded time and fail closed for signup, sign-in,
  linking and unlinking.
- **EX-6** Extension tables live in the account database under extension-
  owned migration names and are backed up with it; account deletion removes
  their rows through §12.9.
- **EX-7** Extension routes carry their own lexicons and appear in
  `describeServer` only when the profile enables them.

### 12.2 Signup gate

`evaluate(request) -> Allow | Deny(reason) | Challenge(kind)` sees the invite
code, CAPTCHA token, client, IP, device and any verified external identity.
It runs before PLC and storage writes for both OAuth sign-up and legacy
`createAccount`. Stock: the Reference's invite and hCaptcha behaviour.

### 12.3 Identity providers

Lists providers for UI hydration (labels, icons, start routes, never secrets);
`start(provider, device, pending request)` and `callback(...)` produce a
`VerifiedIdentity` (issuer, subject, asserted email and its verification,
display-name suggestion); a store keyed by `(provider, subject)` with atomic
link, unlink and list. Flow state is one-use, device-bound and bounded. Stock:
no providers, no routes.

### 12.4 Handle policy

`suggest(context)` for fresh signups, `check(handle)` extending the reserved
lists, `authorize_rename(tx, did, from, to)` for allowances, and
`handle_host(host, path)` for responses on handle hostnames. Stock: the
Reference's lists, no suggestion, no allowance, well-known only.

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

## 13. The LinkJar Profile

The `linkjar` profile implements the six patches with their documented
invariants. The patch documents remain the detailed source until the
extension crate's own documentation replaces them.

| Patch | Extension point | Invariants the Candidate MUST keep |
|---|---|---|
| [`093-handle-policy`](../handle-policy.md) | §12.4 | Suggested handles from display name, email local part, then `reader`; NFKD, ASCII, 3 to 18 characters, six-hex suffix on collision, at most 32 attempts; the extra infra, brand, numbered and lookalike reservations; one hosted rename within 30 days, same-target retry after an ambiguous PLC write; handle hosts serve only `/.well-known/atproto-did`, 404 for other well-known and XRPC paths, 302 to `https://linkjar.io/jar/<did>` for other GET and HEAD, no cookies, strict CSP, `nosniff`. |
| [`099-external-providers`](../external-providers.md) | §12.3 | Apple (`form_post`, nonce, confidential secret), Google and GitHub (PKCE S256); fixed issuer and JWKS endpoints; GitHub needs a primary verified email; bounded flow store (1,000 flows, three per device, ten minutes); 20 starts and 10 completions per IP per minute; `external_identity` keyed by provider and subject; email collisions refused, never linked; passwordless accounts carry the `!external-identity` sentinel; fresh-signup preauthorization only for explicitly trusted clients. |
| [`099b-signup-receipt`](../signup-receipt.md) | §12.8 | `io.linkjar.account.getSignupReceipt` and `acknowledgeSignupReceipt` over the session's DPoP transport; receipt bound to request, client, device and the created DID; survives rotation, cleared by acknowledgment, removed by revocation; fails closed. |
| [`099c-signup-policy`](../provider-policy.md) | §12.2 | All three hCaptcha variables or startup failure; fresh legacy `createAccount` refused when hCaptcha is configured, with migrations via service auth still allowed. |
| [`100-signin-methods`](../signin-methods.md) | §12.5, §12.6 | Device-bound linking from the account page; idempotent self-link; a foreign identity is refused with only that account's handle; last-method protection in the transaction; notices enqueued in the same transaction, 100 per minute, retry from one minute to one hour, stable `Message-ID`. |
| [`101-mail-templates`](../mail-templates.md) | §12.7 | Six templates in LinkJar's email style with a text alternative and no tracking. |

### 13.1 Configuration

The profile reads the same variables the patches read (`PDS_EXTERNAL_*`,
`PDS_HCAPTCHA_*`, `PDS_OAUTH_TRUSTED_CLIENTS`) so the host's secrets file does
not change at cutover.

### 13.2 Handle hosts

The server itself answers handle hostnames according to §12.4, keyed by the
`Host` header. The Caddy snippet in `staging/Caddyfile.handles` remains valid
and MAY stay in front of it.

### 13.3 Lexicons

`io.linkjar.account.getSignupReceipt` and
`io.linkjar.account.acknowledgeSignupReceipt` are vendored from the patch and
served only in this profile.

## 14. Operations

### 14.1 Configuration

- Configuration is environment variables. The Candidate MUST accept every
  variable in Appendix C with the Reference's default, MUST fail to start on
  an invalid value, and SHOULD warn on an unknown `PDS_*` variable.
- `check-config` validates without starting.

### 14.2 Command line

`linkjar-pds serve`, `migrate` (with `--allow-candidate-only`, §18.3),
`check-config`, `verify` (§18.2), `export-car <did>`, `version`.

### 14.3 Health and shutdown

`/xrpc/_health` reflects database availability. On `SIGTERM` the server stops
accepting connections, finishes in-flight writes, sends close frames to
firehose subscribers, checkpoints what it owns, and exits within a bounded
time.

### 14.4 Telemetry

- Metrics in Prometheus text format on `PDS_METRICS_ADDRESS` (private):
  request counts and latencies by method and status, rate-limit decisions,
  commit latency, sequencer lag, subscriber counts and queue depth, actor LRU
  hits, SQLite busy events, mail outbox depth, background task state.
- Metric names are documented in the README and the `linkjar/infra` alert
  rules are updated in the cutover unit (Appendix D, D8).
- Logs are JSON lines with request ids. They never contain tokens, passwords,
  email addresses or provider subjects.

### 14.5 Build and distribution

- Reproducible builds pinned by `rust-toolchain.toml` and `Cargo.lock`;
  `cargo deny` for licences and advisories in CI.
- A static binary per Linux architecture, a container image with the same
  entrypoint contract as the Reference image, and a Nix package for the
  NixOS host.
- Dual-licensed MIT and Apache-2.0, as upstream (Appendix D, D5).

## 15. Security

### 15.1 Safe fetch

Every outbound HTTP request (DID resolution, client metadata, JWKS, handle
verification, identity providers, crawler notification) resolves the host,
rejects loopback, private, link-local and multicast addresses, connects to
the resolved address, re-checks on each redirect with a redirect cap, applies
timeouts (`PDS_ID_RESOLVER_TIMEOUT`, provider-specific limits) and response
size caps (`PDS_FETCH_MAX_RESPONSE_SIZE`). `PDS_DISABLE_SSRF_PROTECTION` is
honoured only with `PDS_DEV_MODE`.

### 15.2 Secrets and comparison

Secrets are read once, never logged, zeroised on drop, and compared in
constant time. Signing keys never leave the process except through the
Reference's export paths.

### 15.3 Browser surfaces

Cookies are `HttpOnly; Secure; SameSite=Lax`. HTML responses carry a strict
CSP and `X-Content-Type-Options: nosniff`. The UI JSON routes keep the
Reference's CSRF and fetch-metadata checks.

### 15.4 Takedowns

Taken-down accounts stop serving records and blobs, blobs are quarantined,
and sync methods answer as the Reference does.

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

### 16.2 Targets

Initial targets, to be revised after unit 2 publishes measurements:

- **T1** No scenario slower than the Reference at p50 or p99.
- **T2** S1 p99 at 100,000 records at most one third of the Reference.
- **T3** S6 RSS at most one quarter of the Reference.
- **T4** S3 RSS growth under 50 MiB.
- **T5** S6 time to healthy under one second.
- **T6** S4 lag p99 under 100 ms with 1,000 subscribers.

Results are published in the README with hardware, versions and image
digests.

## 17. Parity and Conformance

### 17.1 Harness

`parity/` is a TypeScript project using `@atproto/api`,
`@atproto/oauth-client-node`, `@atproto/repo` and `@atproto/xrpc` at the
pin. A compose file boots the Reference image by digest, the Candidate, a
local PLC directory and, for relay scenarios, a local relay, all with the
same configuration. Each scenario runs against both servers and the oracles
below compare the outcomes.

### 17.2 Interop vectors

The upstream `interop-test-files` (syntax, crypto, repository and MST, CAR)
run as Rust unit tests in `pds-types`, `pds-repo` and `pds-identity`.

### 17.3 Oracles

- **O1 Response diff.** Status, `error`, and the JSON body normalised: field
  order ignored; server-generated identifiers aliased on first sight; times
  compared when the dev clock is injected.
- **O2 Repository CIDs.** MST root CID after every write scenario; commit CID
  when `rev` is injected (§3.3).
- **O3 Firehose.** Frames decoded and compared as events: type, `ops`,
  block set, `blobs`, `since` and `rev` relationships, cursor behaviour in
  every row of §7.3, and the crash injection of §7.4.
- **O4 OAuth.** Full flows for two fixture users, after a server restart,
  plus negative cases: duplicate code redemption, refresh reuse, wrong DPoP
  key, cross-user access, deactivated account.
- **O5 Migration.** The account migration procedure in both directions with
  a repository, blobs and preferences, verified by CID and byte comparison.
- **O6 Stock equivalence.** The whole suite with `PDS_PROFILE=stock` against
  Stock Reference (C6).
- **O7 LinkJar journeys.** The browser gate and the kit self-test from
  `linkjar.io` pointed at the Candidate, and this repository's `tests/*.mjs`
  suites ported to run against an HTTP endpoint.
- **O8 Official app.** A recorded manual acceptance: the Bluesky app signs in
  through the Candidate and posts.

### 17.4 Gates

Each delivery unit in the README names the oracles and scenarios that must
pass before it merges. A unit does not merge on a partial oracle.

## 18. Cutover and Rollback

### 18.1 Preconditions

- The Harness passes with the `linkjar` profile against a restored copy of
  the production data directory on staging.
- `verify` (§18.2) reports zero differences on that copy.
- Secrets are unchanged: `PDS_JWT_SECRET`, `PDS_DPOP_SECRET`, the rotation
  key, the admin password, and the SMTP and provider credentials.
- The `linkjar/infra` service definition, alert rules and recovery runbook
  are updated for the new binary and metrics.

### 18.2 Verification

`verify` opens the data directory read-only and, for every actor, loads the
repository, recomputes the MST root from the records, and compares it with
`repo_root`; checks every `key` file loads; checks `repo_seq` continuity; and
prints a report. The same command run with the Reference serving the same
copy is the oracle.

### 18.3 Procedure

1. Announce maintenance on the status page.
2. Stop the Reference container. Run `verify`.
3. Start the Candidate on the same hostname and data directory. Check health,
   `describeServer`, OAuth metadata, a firehose connection from the relay
   resuming at its last cursor.
4. Smoke: sign in from the web app, iOS and the extension; write a record;
   observe it on the firehose; send a test mail.
5. Watch for 48 hours. The rollback window stays open until the operator
   applies the first Candidate-only migration with `--allow-candidate-only`.

Rollback within the window: stop the Candidate, start the Reference image on
the same directory. Nothing else changes.

### 18.4 Fencing

Reference and Candidate MUST NOT serve the same data directory at the same
time. The Candidate's lock file (§8.3) and SQLite locking refuse a second
writer; the runbook stops one before starting the other.

## Appendix A. XRPC Inventory

Classes: **Core** (MUST, as the Reference), **Deprecated** (MUST, same
response as the Reference), **Proxy** (MUST forward per §4.4), **Local**
(SHOULD, read-after-write per §4.5), **Relay-only** (MUST return
`MethodNotImplemented`), **Ext** (`linkjar` profile only).

| Namespace | Core | Other |
|---|---|---|
| `com.atproto.server` | `describeServer`, `createAccount`, `createSession`, `getSession`, `refreshSession`, `deleteSession`, `createAppPassword`, `listAppPasswords`, `revokeAppPassword`, `createInviteCode`, `createInviteCodes`, `getAccountInviteCodes`, `requestEmailConfirmation`, `confirmEmail`, `requestEmailUpdate`, `updateEmail`, `requestPasswordReset`, `resetPassword`, `requestAccountDelete`, `deleteAccount`, `deactivateAccount`, `activateAccount`, `checkAccountStatus`, `getServiceAuth`, `reserveSigningKey` | |
| `com.atproto.repo` | `applyWrites`, `createRecord`, `putRecord`, `deleteRecord`, `getRecord`, `listRecords`, `describeRepo`, `uploadBlob`, `importRepo`, `listMissingBlobs` | |
| `com.atproto.sync` | `getRepo`, `getRecord`, `getBlocks`, `getLatestCommit`, `getRepoStatus`, `listRepos`, `listBlobs`, `getBlob`, `subscribeRepos` | Deprecated: `getCheckout`, `getHead`. Relay-only: `getHostStatus`, `listHosts`, `listReposByCollection`, `notifyOfUpdate`, `requestCrawl` (accepted as the Reference accepts them). |
| `com.atproto.identity` | `resolveHandle`, `updateHandle`, `getRecommendedDidCredentials`, `requestPlcOperationSignature`, `signPlcOperation`, `submitPlcOperation` | Proxy or 501 as the Reference: `resolveDid`, `resolveIdentity`, `refreshIdentity`. |
| `com.atproto.admin` | `deleteAccount`, `disableAccountInvites`, `enableAccountInvites`, `disableInviteCodes`, `getInviteCodes`, `getAccountInfo`, `getAccountInfos`, `getSubjectStatus`, `updateSubjectStatus`, `sendEmail`, `updateAccountEmail`, `updateAccountHandle`, `updateAccountPassword`, `updateAccountSigningKey` | `searchAccounts`: as the Reference. |
| `com.atproto.moderation` | `createReport` | |
| `com.atproto.temp` | `checkSignupQueue` | Others: as the Reference (proxy or 501). |
| `com.atproto.label`, `com.atproto.lexicon` | | Proxy or 501 as the Reference; lexicon resolution for permission sets per §5.5. |
| `app.bsky.actor` | | Local: `getPreferences`, `putPreferences`, `getProfile`, `getProfiles`. |
| `app.bsky.feed` | | Local: `getActorLikes`, `getAuthorFeed`, `getFeed`, `getPostThread`, `getTimeline`. |
| `app.bsky.notification` | | Local: `registerPush`, `unregisterPush`. |
| other `app.bsky.*`, `chat.bsky.*`, labelers | | Proxy. |
| `io.linkjar.account` | | Ext: `getSignupReceipt`, `acknowledgeSignupReceipt`. |

Unit 0 reconciles this table against the Reference's handler tree and
pipethrough rules and records any correction here.

## Appendix B. Extension Traits (Informative)

```rust
pub trait SignupGate: Send + Sync {
    fn evaluate(&self, req: &SignupRequest) -> Result<GateDecision, GateError>;
}

pub enum GateDecision { Allow, Deny(DenyReason), Challenge(ChallengeKind) }

pub trait HandlePolicy: Send + Sync {
    fn suggest(&self, ctx: &SignupContext, taken: &dyn Fn(&Handle) -> bool) -> Option<Handle>;
    fn check(&self, handle: &Handle) -> Result<(), HandleRejection>;
    fn authorize_rename(&self, tx: &mut AccountTx, did: &Did, from: &Handle, to: &Handle)
        -> Result<RenameTicket, RenameRefused>;
    fn handle_host(&self, host: &str, path: &str) -> Option<HostResponse>;
}

pub trait IdentityProviders: Send + Sync {
    fn list(&self) -> Vec<ProviderButton>;
    fn start(&self, provider: &str, device: &DeviceId, pending: &PendingAuth) -> Result<Redirect, FlowError>;
    fn callback(&self, provider: &str, params: &CallbackParams, device: &DeviceId)
        -> Result<VerifiedIdentity, FlowError>;
    fn store(&self) -> &dyn ExternalIdentityStore;
}

pub trait Notices: Send + Sync {
    fn enqueue(&self, tx: &mut AccountTx, did: &Did, notice: Notice) -> Result<(), StoreError>;
}

pub trait MailTemplates: Send + Sync {
    fn render(&self, kind: MailKind, data: &MailData) -> RenderedMail;
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
- **Invites and abuse**: `PDS_INVITE_REQUIRED`, `PDS_INVITE_EPOCH`, `PDS_INVITE_INTERVAL`, `PDS_HCAPTCHA_SITE_KEY`, `PDS_HCAPTCHA_SECRET_KEY`, `PDS_HCAPTCHA_TOKEN_SALT`, `PDS_RATE_LIMITS_ENABLED`, `PDS_RATE_LIMIT_BYPASS_KEY`, `PDS_RATE_LIMIT_BYPASS_IPS`.
- **Firehose and proxy**: `PDS_CRAWLERS`, `PDS_MAX_SUBSCRIPTION_BUFFER`, `PDS_REPO_BACKFILL_LIMIT_MS`, `PDS_BSKY_APP_VIEW_URL`, `PDS_BSKY_APP_VIEW_DID`, `PDS_BSKY_APP_VIEW_CDN_URL_PATTERN`, `PDS_PROXY_*`, `PDS_FETCH_MAX_RESPONSE_SIZE`, `PDS_DISABLE_SSRF_PROTECTION`.
- **Moderation and mail**: `PDS_MOD_SERVICE_URL`, `PDS_MOD_SERVICE_DID`, `PDS_REPORT_SERVICE_URL`, `PDS_REPORT_SERVICE_DID`, `PDS_EMAIL_SMTP_URL`, `PDS_EMAIL_FROM_ADDRESS`, `PDS_EMAIL_DISABLE_CONFIRMATION_LINK`, `PDS_MODERATION_EMAIL_ADDRESS`, `PDS_MODERATION_EMAIL_SMTP_URL`.
- **Branding**: `PDS_LOGO_URL`, `PDS_PRIMARY_COLOR`, `PDS_ERROR_COLOR`, `PDS_WARNING_COLOR`, `PDS_SUCCESS_COLOR`, `PDS_INFO_COLOR`, `PDS_BACKGROUND_LIGHT_URL`, `PDS_BACKGROUND_DARK_URL`.
- **Refused in version 1**: `PDS_ENTRYWAY_*`, `PDS_REDIS_SCRATCH_*`.

New in the Candidate:

- `PDS_PROFILE`: `stock` (default) or `linkjar`.
- `PDS_METRICS_ADDRESS`: the private metrics listener; unset disables it.
- `PDS_EXTERNAL_<PROVIDER>_CLIENT_ID` and `_CLIENT_SECRET` for Apple, Google and GitHub (`linkjar` profile).

## Appendix D. Open Decisions

| Id | Decision | Owner unit |
|---|---|---|
| D1 | SQLite driver: `rusqlite` with bundled SQLite on a blocking pool, or an async driver. Default: `rusqlite`. | 0 |
| D2 | Repository crate: the Apache-2.0 `rsky-repo` crate or an in-house implementation. Decided by the interop vectors and S1 to S3. | 2 |
| D3 | HTML templating crate for §11.4. | 5 |
| D4 | Workspace placement: the Cargo workspace at this repository's root with the image build moved under `image/`, or a `rust/` subdirectory. Default: root, image build under `image/` once the Candidate is the deployed server. | 0 |
| D5 | Licence: dual MIT and Apache-2.0 for the Candidate's own code; upstream notices retained for vendored lexicons and lists. | 0 |
| D6 | Resolved 2026-10-08: the Reference's OAuth key set is one HS256 key from `PDS_JWT_SECRET` (`context.ts` at the pin). Moving to an asymmetric key set with `jwks` publication is a post-cutover change. | 5 |
| D7 | Whether handle-host serving (§13.2) replaces the Caddy snippet on the host or sits behind it. | 7 |
| D8 | Metric names and the `linkjar/infra` alert rules that depend on them. | 7 |
| D9 | Permission sets and lexicon resolution (§5.5): version 1 or before public launch. | 5 |
| D10 | Whether `verify` also recomputes blob reference integrity. | 3 |

# How other systems solve what LinkJar PDS needs

Research record, 2026-10-08. Companion to [SPEC.md](SPEC.md) and the
[decision record](README.md). It answers one question per section: what do
the best implementations do for extensible storage, clustering, a tamper-
evident audit trail, fault-injection testing, and supply-chain security, and
what does that mean for the LinkJar PDS specification.

**Method.** Exa web search and page extraction for discovery, Firecrawl for
full reads of three design documents, and the pinned reference PDS source
already in the scratch checkout. Every claim below names its source. Where a
project states something about itself (scale, readiness), it is reported as
a self-description, not as a verified result. Nothing was built, benchmarked
or run during this pass.

## 1. Summary of findings

1. **The ecosystem has converged on per-actor SQLite.** The reference server
   introduced it in 2023 and Blacksky's rsky-pds, after years on Postgres,
   moved its account, sequencer and DID-cache databases to three SQLite files
   mirroring the upstream layout. atproto-pds and Aurora Locus keep per-actor
   SQLite even when the shared tier is Postgres. The one project that argues
   against it, tranquil-pds, proposes a custom embedded engine, not Postgres.
2. **Nobody clusters a single PDS hostname across writers.** Bluesky scales
   by running many PDS hosts under separate hostnames behind an entryway.
   Aurora Locus runs several stateless processes over one Postgres but elects
   one leader to write the sequencer, and its documented upgrade is a full
   restart. The protocol's own scaling unit is the host.
3. **The sequencer is the hard part of any multi-node design.** Postgres
   sequences are not commit-ordered, so a naive shared log loses events. The
   working answers are a single sequencer lease, an `xid8` watermark, or an
   exclusive lock. The reference and atproto-pds both treat the durable log as
   the only source of frames and use the broadcast bus as a wakeup only.
4. **Deterministic simulation is practical in Rust and well charted.**
   TigerBeetle, S2, Turso and the tranquil-store RFC document the method, the
   crates and the traps. The recurring trap is dependencies that read the
   clock, randomness or hash seeds behind your back.
5. **Tamper-evident audit logs are a solved design.** Hash chain plus Merkle
   checkpoints plus signed tree heads plus an independent verifier, as in
   Certificate Transparency, Trillian, Rekor v2 and Go's module log. The
   atproto firehose itself is a natural witness channel for checkpoints.
6. **The security gates are commodity.** cargo-deny, cargo-audit, cargo-vet,
   continuous fuzzing, SLSA provenance, SHA-pinned actions, and devenv for a
   pinned toolchain. The only judgment calls are policy, not tooling.

## 2. PDS implementations compared

| Implementation | Language | Actor store | Shared tier | Multi-node | OAuth server | Notes |
|---|---|---|---|---|---|---|
| Reference `@atproto/pds` | TypeScript | SQLite per actor, `actors/<2 hex>/<did>/store.sqlite` plus sibling key file | SQLite: account, sequencer, DID cache | No; entryway mode hands accounts to an external service | Yes, bundled React UI | 30,000 open file handles and keys in an LRU; WAL for streaming replication; shipped with Litestream in mind [PR-1705] |
| rsky-pds (Blacksky) | Rust | SQLite per actor since the 2026 refactor | Three SQLite files mirroring upstream; sequencer is a notify-driven poll loop | No | Tables created, "oauth phase" pending at that commit | Was Postgres, S3, Mailgun; moved to the upstream layout [RSKY-SQLITE] |
| Aurora Locus | Rust, axum, sqlx `Any` | SQLite per actor, always | SQLite or Postgres for accounts, sequencer, DID cache, admin, OAuth state | Yes on Postgres: leader election by session advisory lock, LISTEN/NOTIFY cache invalidation, Postgres compare-and-swap for DPoP replay and rate limits | OAuth 2.1, PKCE, DPoP, Argon2id | Self-described production-ready; no canonical benchmarks; upgrades are a coordinated full restart; dual MIT/Apache [AURORA-ARCH] [AURORA-MULTI] |
| atproto-pds (Gerakines) | Rust | SQLite per actor, `sha256(did).sqlite` | `accounts.sqlite` holds cross-account state and the firehose log | No; entryway member mode; Postgres accounts adapter compiles but refuses to boot | OAuth 2.1 with HS256 tokens over the JWT secret; server-rendered portal with no JavaScript | Every read and write through one `PublicRealmBackend` trait; conformance harness drives the real `@atproto/oauth-client-node`; migrations append-only, no down step [ATPROTO-PDS] |
| tranquil-pds | Rust | Postgres required today | Postgres, Valkey, S3 | Via Postgres | Yes, granular scopes, passkeys, 2FA, SSO, delegation | RFC for an embedded engine: Bitcask block log, Fjall metadata, segmented event log, group commit, intent log, simulation-tested [TRANQUIL-RFC] |
| cocoon | Go | SQLite block store | SQLite default, Postgres optional | Not stated | Custom OAuth with DPoP | Hourly SQLite snapshot to S3; self-described experimental [COCOON] |
| Bluesky fleet | TypeScript | Many hosts such as `morel.us-east.host.bsky.network` | Entryway at `bsky.social` owns accounts, sessions, handles, signup routing | Horizontal by adding hosts | Entryway | Each account has its own signing key; "a deployment pattern we chose to scale out for millions of accounts", not a protocol requirement [BSKY-MIGRATION] [ENTRYWAY] |

Two findings deserve emphasis. First, the storage layout the SPEC already
chose (§8.1) is not only reference-compatible, it is where independent
implementations ended up after trying alternatives. Second, the only Rust
server with a working OAuth authorization server and a conformance harness
against the official client is atproto-pds, and its author reports that the
harness "caught two OAuth bugs that every Rust test missed". The SPEC's
harness (§17) is the right investment.

## 3. Clustering

### 3.1 What the protocol scales with

Bluesky's answer to "millions of accounts" is many PDS hosts, each a
complete single-node server with its own hostname, signing keys, sequence
space and backups, fronted by an entryway that owns the account side: email,
password, app passwords, sessions, OAuth, handles, PLC rotation keys, and
which host each account lives on [ENTRYWAY] [BSKY-MIGRATION]. Relays crawl
each host separately. The entryway is one instance by design; two copies
"will issue tokens and sign PLC operations from different account tables"
[ATPROTO-ENTRYWAY]. atproto-entryway implements the same contract the
reference PDS expects in entryway mode, with a durable operation log for
multi-step signup and periodic reconciliation against every member, PLC and
the handle backend.

This maps onto the SPEC in a way that was not visible before this pass: the
§12 extension points (signup gate, identity providers, handle policy, sign-in
methods, notices, mail, receipt) are exactly the entryway's responsibilities.
An account layer designed as a crate that can run embedded in a single-node
PDS or as a standalone entryway gives LinkJar the protocol's own scaling
model without a shared database.

### 3.2 Shared-database clusters

Aurora Locus is the worked example. Several stateless processes share one
Postgres and one blob store. One process holds a session-scoped
`pg_try_advisory_lock` and is the only writer of sequencer events; standbys
retry every two seconds; a dropped connection releases the lock. A dedicated
maintenance pool backs Postgres compare-and-swap tables for DPoP `jti`
replay and rate-limit buckets, with a ten-second acquire timeout that
degrades to per-instance limits when saturated. Cache invalidation is one
LISTEN/NOTIFY channel with TTL fallback for missed notifications. Upgrades
drain, stop every instance, migrate, start every instance, about thirty
seconds of downtime, because expand-and-contract migrations were judged not
worth their complexity [AURORA-MULTI].

The document does not say where per-actor SQLite files live when more than
one instance serves requests. It calls per-actor storage "local I/O" and
requires shared blob storage on NFS or object storage. SQLite over a network
filesystem is unsafe for writers, so either each actor is pinned to one
instance by routing, or the files sit on shared storage with the risks that
implies. Treat this as an open question in that design, not a solved one.

### 3.3 The sequencer under concurrent writers

The failure is well documented. A Postgres `BIGSERIAL` is assigned before
commit, so transaction B with position 2 can commit before transaction A
with position 1; a subscriber that advances its cursor past 2 never sees 1
[EVENT-DRIVEN] [NONCRAB]. Three working answers:

- **Single sequencer lease.** One process assigns `seq` and writes the log.
  Aurora Locus does this with an advisory lock. Simple and keeps the
  reference's cursor semantics exactly; the handover is a drill to rehearse.
- **Transaction-id watermark.** Store `pg_current_xact_id()` as `xid8` with
  each event and read only rows below `pg_snapshot_xmin(pg_current_snapshot())`,
  ordered by `(transaction_id, position)`. Lock-free on the write path;
  a long-running transaction stalls readers [EVENT-DRIVEN] [MIRE].
- **Exclusive table lock** on insert. Gapless and simple, serialises every
  write behind a disk flush [EVENTIUM].

Independent of the choice, the reference and atproto-pds agree on one
invariant worth copying: frames are read from the durable log in `seq`
order, and the in-process broadcast carries only a wakeup. "A lost wakeup
costs the poll interval and a duplicated one costs an empty query; neither
can cost an event" [ATPROTO-PDS]. The reference's outbox does backfill,
then a cutover query that dedupes against a buffer, then live streaming
[PDS-OUTBOX].

Sync 1.1 raises the bar for what a PDS emits. `#commit` carries `prevData`
and the ops list so non-archival relays verify by MST inversion; `#sync`
resets a repository's state; relays will move from lenient to strict
validation and can ratchet by host [SYNC-SPEC] [RELAY-SYNC]. The §7.4
crash-boundary repair (emit `#sync` for any repository whose head is ahead
of its last sequenced event) is therefore not only correct but the
protocol's intended recovery path.

### 3.4 SQLite replication options for one host

| Tool | Model | Failover | Write cost | Fit |
|---|---|---|---|---|
| Litestream | Async WAL shipping to object storage; new LTX format, compaction, read-replica VFS, lease via conditional writes; directory replication of thousands of databases now feasible | Restore, minutes | Negligible | Disaster recovery, already in use at LinkJar [LITESTREAM-REVAMP] |
| LiteFS | FUSE or VFS interception, LTX changesets, Consul lease, async | Automatic, lease TTL 10 s | FUSE caps writes near 100 tx/s; "tens or hundreds" of databases tested, thousands untested | Hot standby for the shared tier, not for thousands of actor files [LITEFS] [LITEFS-FAQ] |
| rqlite, dqlite | Raft over SQLite, HTTP or C API | Automatic | Consensus per write | Wrong shape for per-actor files [LITESTREAM-ALT] |

### 3.5 Recommendation for the SPEC

- **Version 1** stays single node, as specified. HA is Litestream plus a
  rehearsed restore, with the RPO and RTO targets the hosting study set.
- **The account and OAuth crate is designed to run in two modes**: embedded
  in the PDS binary, and as a standalone entryway service with the reference
  entryway contract. This is the clustering story: add member hosts.
- **The shared-database cluster is documented as the alternative**, with
  the sequencer lease as its mechanism and the per-actor file placement
  question stated as unresolved. It is not the recommended path.
- Keep the durable-log-is-truth invariant and the §7.4 repair.

## 4. Storage extensibility and switching databases

Evidence on how far an abstraction can go:

- Aurora Locus routes every shared-tier query through `sqlx::Any` and runs
  both backends in CI. The cost shows in the details it lists: placeholder
  syntax, boolean decoding, per-backend migration sets, and a few
  "backend-specific spots" [AURORA-ARCH].
- atproto-pds defines `PublicRealmBackend` and `AtomicCommitWriter` so a
  slot such as blob storage swaps without a second code path. Its Postgres
  accounts adapter is finished and tested but refuses to boot, because "a
  documented mode that does not work is worse than an absent one; a mode
  that fails loudly at startup is neither" [ATPROTO-PDS].
- tranquil-pds plans "snapshot-n-switch" between Postgres and its embedded
  engine [TRANQUIL-RFC]. rsky-pds switched the other way, Postgres to
  SQLite, in one commit that rewrote the sequencer and DID cache
  [RSKY-SQLITE].

Recommendation for the SPEC (new §8.4 and §12.10):

- Define the seams at the domain level: `ActorStore`, `AccountStore`,
  `SequencerStore`, `BlobStore`, `DidCache`, `AuditStore`, each with a
  transaction type the extension hooks join (EX-4 already requires this).
- Per-actor SQLite is not swappable in version 1. The swappable surfaces
  are the shared tier and the blob store. This matches every implementation
  surveyed.
- Switching is a command, not a live dual-write: `migrate-storage` copies
  the shared tier under a maintenance window and verifies row counts and
  checkpoints; actor stores move one actor at a time under a short
  per-actor write lease with the MST root as the oracle. The SPEC already
  has the oracle (§18.2).
- A backend combination that is compiled but not supported refuses to
  start with a message naming the gap. No silent fallbacks.

## 5. Deterministic simulation and fault injection

### 5.1 What the practitioners agree on

- **Determinism first.** TigerBeetle's VOPR stubs clock, network and disk,
  seeds every fault from one number, and reproduces any failure from the
  seed and the commit. Assertions stay on in production: "far better to stop
  operating than to continue in an incorrect state" [VOPR]. Its 2026 work
  checks invariants inside each replica, not only at the API [TB-PROTO].
- **The hard part is dependencies.** S2 found CI failures it could not
  reproduce until it controlled `getrandom`, `clock_gettime` and Rust's
  randomised `HashMap`, then added a meta-test that reruns a seed and
  compares TRACE logs byte for byte. Seventeen notable bugs found [S2-DST].
- **Partial determinism is common and must be stated.** Turso's simulator
  seeds its wall clock but returns the real monotonic clock, and its
  `fsync` faults are currently disabled because they caused false positives,
  even though the profile says they are on [TURSO-READING]. Turso pairs the
  simulator with Antithesis in CI and with formal "intents" that caught an
  `fsync` bug the simulator missed [TURSO-FORMAL].
- **Differential testing and fault injection are separate modes.** Turso
  disables faults in differential mode because they cannot be mirrored on
  the comparison SQLite [TURSO-READING]. The same split applies to the
  LinkJar harness: parity scenarios against the Reference run without
  faults; fault scenarios run against the Candidate alone with its own
  oracles.

### 5.2 Crash consistency specifically

SQLite's own suite is the model: a test VFS that fails after N operations,
transiently or persistently, and a crash VFS that simulates power loss by
replaying, reordering and trashing unsynced sectors according to declared
device characteristics, then checks `integrity_check` [SQLITE-TESTING]
[SQLITE-TEST6]. The ALICE and CrashMonkey work adds two empirical rules:
most application crash bugs reproduce with three or fewer operations after
an `fsync`, and forgetting to `fsync` the directory is a recurring cause
[ALICE] [CRASHMONKEY].

Rust crates for a custom VFS: `sqlite-plugin` (0.10, 2026, centralised
`Vfs` trait with WAL and shared-memory hooks, MIT or Apache) is the
maintained option; `sqlite-vfs` lacks WAL support; a safe VFS wrapper is
proposed for rusqlite itself and covers all three file API versions
[SQLITE-PLUGIN] [SQLITE-VFS] [RUSQLITE-VFS]. For the network and process
layer, `turmoil` now ships `turmoil-fs` with a durability model (pending
writes become durable on `sync_all` or by probability, crashes discard
pending writes) and execution barriers for suspending code at chosen
points [TURMOIL]; `mad-turmoil` adds the libc overrides [MAD-TURMOIL].

### 5.3 Recommendation for the SPEC (new §19)

1. **One `StorageIo` seam** under SQLite and the blob store, as the
   tranquil-store RFC does, with an in-memory implementation that injects
   partial writes, bit flips, `fsync` failures and misdirected writes from
   one seed [TRANQUIL-RFC]. Implemented as a `sqlite-plugin` VFS for the
   SQLite files and a trait for blobs.
2. **Fail points** on the commit, sequencing, mail-outbox and PLC paths so
   the harness can kill the process between any two steps.
3. **turmoil plus mad-turmoil** for the firehose fan-out, crawler
   notification, identity resolution, mail and, later, the entryway
   protocol, with a seed per run and a meta-test for determinism.
4. **Model tests** for the MST against the upstream vectors; the vectors
   repository is CC0 and covers crypto, data model, firehose, lexicon, MST
   and syntax [INTEROP].
5. **Oracles**: `verify` after every simulated crash; the repair ladder
   (sequencer log, backup, relay `getRepo`) must restore a repository to
   the MST root the simulation recorded.
6. **Nightly seeds**, every failure filed with its seed and commit, and
   assertions compiled into release builds for invariants that are cheap.
7. Antithesis is a later option, not a dependency.

## 6. Audit and provenance

The transparency-log design is settled: a hash chain gives per-entry
linkage; a Merkle tree over batches gives short inclusion proofs; a signed
tree head gives a dated commitment; consistency proofs show one head extends
another; witnesses or publication outside the operator's control defeat
re-signing [RFC6962] [TLOG] [TRILLIAN] [REKOR2]. Rekor v2 serves the tree as
cacheable tiles and co-signs checkpoints with witnesses. The practitioner
guide adds the operational rules: an insert-only write path, canonical
serialisation written down as a spec, checkpoints every few minutes with
the key in a KMS, a verifier that shares no code with the writer and runs
offline, and a signed one-off snapshot for history before the cut-over
[SIGILBASE].

Two facts make this cheap for a PDS. Repository data already is a signed
Merkle structure, so provenance of records is intrinsic to the protocol;
the audit log only needs to cover account and operator actions. And the
server already has a canonical encoding, DAG-CBOR, and a signing key.

Recommendation for the SPEC (new §20):

- `AuditStore` receives insert-only entries encoded as DAG-CBOR: actor,
  action, subject, credential kind, client id, device id, request id,
  outcome, time, and the previous entry's hash.
- A checkpoint every five minutes: Merkle root over the entries since the
  last checkpoint, signed with a dedicated key, with the key id recorded
  and key succession logged.
- **Publish each checkpoint as a record in the operator's own repository.**
  It then travels the firehose, is mirrored by relays and indexers, and
  becomes a witness nobody at LinkJar can rewrite. This is the one idea in
  this document that uses the protocol itself as the witness network.
- A standalone verifier binary and an export format that bundles entries,
  chain hashes, proofs and checkpoints.
- Admin and support actions, sign-in method changes, handle renames, PLC
  operations, takedowns and deletions are in scope. Ordinary authenticated
  reads are not; that is a traffic log, not an audit log.

## 7. Security and supply chain

The consensus gate set, with the specific traps reported by teams running
it [FJORD] [MS-RUST] [SIMARD] [GH-GUARD] [SOTA-RUST]:

- `cargo deny` on every PR for advisories, licences, bans and sources with
  `unknown-registry` and `unknown-git` denied; `cargo audit` on a schedule
  against a pinned advisory database so a new advisory cannot fail
  unrelated PRs; every `ignore` entry carries a reason and an expiry.
- `cargo vet` for new dependencies, with imports from Mozilla, Google and
  the Bytecode Alliance, and a standing review of every crate that runs at
  build time (`build.rs`, proc-macros).
- Pinned tool versions in CI, SHA-pinned actions with version comments,
  `permissions: read-all`, `persist-credentials: false`, no
  `pull_request_target`, `--locked` everywhere.
- Continuous fuzzing of every parser of untrusted bytes with `cargo fuzz`
  on a schedule; for this server that is DAG-CBOR, CAR, MST, XRPC input,
  JWT, DPoP and client metadata.
- `cargo auditable` and an SBOM in every release; SLSA level 3 provenance
  through the official generator, which must be referenced by tag; signed
  tags; releases built from tags in CI.
- `zeroize` and `secrecy` for key material, with the honest caveat that
  `Drop` is not guaranteed to run; constant-time comparison for every
  token; manual `Debug` on secret-bearing types; body size limits at every
  ingest; `deny_unknown_fields` on auth and config types.

devenv covers the toolchain: `languages.rust` with `channel = "stable"` or
a `rust-toolchain.toml` through `toolchainFile`, `git-hooks` for rustfmt and
clippy, `services.postgres` for the Postgres backend tests, `processes` for
the parity compose, and `languages.rust.import` to package the binary with
crate2nix for the NixOS host [DEVENV-RUST] [DEVENV-BLOG]. devenv replaced
fenix with rust-overlay for toolchains, so "latest stable" is a one-line
setting.

## 8. Performance evidence worth keeping

- The reference server holds up to 30,000 open actor databases and keys in
  an LRU and runs each in WAL mode for streaming replication [PR-1705].
- The reference fsyncs once per actor per mutation; tranquil-store's main
  throughput idea is group commit across users, which per-actor files
  prevent for the actor store but not for the shared sequencer file
  [TRANQUIL-RFC].
- atproto-pds builds the `getRepo` CAR in memory and returns it in one
  response; pds-workers rebuilds the whole MST per commit. Both are the
  anti-patterns the SPEC's §6.3 and §16 targets already exclude.
- The new Bluesky relays validate signatures and MST inversion for the
  whole network on about two vCPUs and 12 GB [RELAY-SYNC]. Relay-side
  limits for a new PDS host started at ten accounts, 1,500 events per hour
  and 10,000 per day [BSKY-FED].
- The relay desync repair is a manual bump of `sqlite_sequence` for
  `repo_seq` [PDS-README]; the Candidate's `#sync` repair should make that
  unnecessary.

## 9. What changes in the SPEC

| Section | Change |
|---|---|
| §3.1 | Add `pds-audit` and `pds-sim` crates; name the `StorageIo` seam. |
| §7.4 | Keep; cite Sync 1.1 as the reason `#sync` is the repair. Add: frames are served from the durable log; the broadcast carries wakeups only. |
| §8.4 (new) | Storage seams at the domain level; swappable surfaces are the shared tier and blobs; `migrate-storage` with per-actor leases; unsupported combinations refuse to start. |
| §12.10 (new) | The account and OAuth crate runs embedded or as an entryway; the entryway contract is the reference's. |
| §16 | Add group commit for the sequencer file and a target for the entryway handover time once that mode exists. |
| §19 (new) | Simulation and fault injection: the seam, fail points, turmoil, nightly seeds, determinism meta-test, repair ladder oracle. |
| §20 (new) | Audit log: DAG-CBOR entries, hash chain, five-minute signed checkpoints, checkpoint record published to the operator repository, standalone verifier, scope. |
| §21 (new) | Deployment topologies: single node, single node with Litestream restore, entryway plus members, shared-database cluster as documented alternative. |
| §14.5 | The gate list from §7 above, and devenv as the toolchain definition. |
| Appendix D | D2 note: `rsky-repo` remains the candidate crate; rsky's own PDS now uses the upstream layout. New D11: audit checkpoint key custody. New D12: entryway mode in version 2 or version 3. |

Units: add a simulation unit between writes and OAuth, an audit-log unit
after OAuth, and move "entryway mode" into the post-cutover units next to
Postgres. The allowance of ten to fourteen extra weeks given in
conversation stands, low confidence.

## 10. Unresolved

- Whether Aurora Locus serves one actor from several instances, and how.
- Whether rsky-pds has shipped its OAuth provider since the SQLite commit.
- Real numbers for any Rust PDS; none of the surveyed projects publishes
  benchmarks. Unit 2 produces the first.
- The relay's strict-validation timeline, which decides how soon the
  `#sync` repair must be exercised against a real relay.

## Sources

- [PR-1705] [PDS SQLite refactor](https://github.com/bluesky-social/atproto/pull/1705)
- [RSKY-SQLITE] [rsky-pds: move account manager, sequencer, and did cache to sqlite](https://github.com/blacksky-algorithms/rsky/commit/6ba2eb7435a1819005b2e04c71abf01bff0bc514)
- [AURORA-ARCH] [Aurora Locus architecture](https://github.com/dollspace-gay/Aurora-Locus/blob/main/docs/architecture.md)
- [AURORA-MULTI] [Aurora Locus multi-instance deployment](https://github.com/dollspace-gay/Aurora-Locus/blob/main/docs/operator/multi-instance-deployment.md)
- [ATPROTO-PDS] [atproto-pds operator guide](https://atproto-crates.com/pds.html)
- [ATPROTO-ENTRYWAY] [atproto-entryway](https://atproto-crates.com/entryway.html)
- [TRANQUIL-RFC] [tranquil-store RFC](https://tangled.org/matchai.dev/tranquil-pds/blob/main/TRANQUIL_OWN_DB_RFC.txt), read in full with Firecrawl
- [COCOON] [cocoon](https://github.com/haileyok/cocoon)
- [BSKY-MIGRATION] [Migrating bsky.social to multiple PDS instances](https://github.com/bluesky-social/atproto/discussions/1832)
- [ENTRYWAY] [PDS Entryway](https://docs.bsky.app/docs/advanced-guides/entryway)
- [BSKY-FED] [Early access federation for self-hosters](https://docs.bsky.app/blog/self-host-federation)
- [PDS-README] [bluesky-social/pds README](https://github.com/bluesky-social/pds/blob/main/README.md)
- [PDS-OUTBOX] [Reference sequencer outbox](https://github.com/bluesky-social/atproto/blob/2f2cd3fd/packages/pds/src/sequencer/outbox.ts)
- [SYNC-SPEC] [AT Protocol sync specification](https://atproto.com/specs/sync)
- [RELAY-SYNC] [Relay updates for Sync v1.1](https://docs.bsky.app/blog/relay-sync-updates)
- [EVENT-DRIVEN] [How Postgres sequences issues can impact your messaging](https://event-driven.io/en/ordering_in_postgres_outbox/)
- [NONCRAB] [Avoiding lost updates in PostgreSQL logs](https://noncrab.net/posts/postgres-logs/)
- [MIRE] [mire: xid8 ordering commit](https://git.kjuulh.io/kjuulh/mire/commit/2e31d4935649b75d8b59ddcef952c93e5a3c32e8.patch)
- [EVENTIUM] [Eventium design and internals](https://www.sidorenko.me/blog/2026/04/eventium-design-and-internals/)
- [LITESTREAM-REVAMP] [Litestream revamped](https://fly.io/blog/litestream-revamped.md)
- [LITESTREAM-ALT] [Litestream alternatives](https://litestream.io/alternatives/)
- [LITEFS] [LiteFS architecture](https://github.com/superfly/litefs/blob/main/docs/ARCHITECTURE.md)
- [LITEFS-FAQ] [LiteFS FAQ](https://fly.io/docs/litefs/faq/)
- [VOPR] [TigerBeetle deterministic simulation testing](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vopr.md)
- [TB-PROTO] [Protocol-aware deterministic simulation testing](https://tigerbeetle.com/blog/2026-08-20-protocol-aware-dst)
- [S2-DST] [Deterministic simulation testing for async Rust](https://s2.dev/blog/dst)
- [MAD-TURMOIL] [mad-turmoil](https://github.com/s2-streamstore/mad-turmoil)
- [TURMOIL] [turmoil](https://docs.rs/turmoil/latest/turmoil/)
- [TURSO-READING] [turso's simulator: every failure is a u64 seed](https://aviavni.github.io/database-learning-path/topics/16-testing-correctness/reading-turso-simulator.html)
- [TURSO-FORMAL] [The final boss of reliability: formal verification](https://turso.tech/blog/the-final-boss-of-reliability)
- [SQLITE-TESTING] [How SQLite is tested](https://www.sqlite.org/testing.html)
- [SQLITE-TEST6] [SQLite crash test VFS](https://github.com/sqlite/sqlite/blob/397a3c4a/src/test6.c)
- [ALICE] [All file systems are not created equal](https://pages.cs.wisc.edu/~remzi/Classes/739/Fall2018/Papers/alice-osdi14.pdf)
- [CRASHMONKEY] [Finding crash-consistency bugs with bounded black-box crash testing](https://www.usenix.org/system/files/osdi18-mohan.pdf)
- [SQLITE-PLUGIN] [sqlite-plugin](https://crates.io/crates/sqlite-plugin)
- [SQLITE-VFS] [sqlite-vfs](https://github.com/rkusa/sqlite-vfs)
- [RUSQLITE-VFS] [rusqlite: add VFS safe wrapper](https://github.com/rusqlite/rusqlite/pull/1797/files)
- [INTEROP] [atproto interop test files](https://github.com/bluesky-social/atproto-interop-tests), read with Firecrawl
- [RFC6962] [Certificate Transparency](https://datatracker.ietf.org/doc/html/rfc6962)
- [TLOG] [Transparent logs for skeptical clients](https://research.swtch.com/tlog.pdf)
- [TRILLIAN] [Trillian](https://www.github.com/google/trillian)
- [REKOR2] [Rekor v2 specification](https://github.com/sigstore/architecture-docs/blob/main/rekor-v2-spec.md)
- [SIGILBASE] [How to implement provable audit logs](https://sigilbase.io/writing/implementing-provable-audit-logs/)
- [FJORD] [Supply-chain CI gate](https://docs.fjord.sh/operations/supply-chain-ci)
- [MS-RUST] [Dependency management and supply chain security](https://microsoft.github.io/RustTraining/engineering-book/ch06-dependency-management-and-supply-chain-s.html)
- [SIMARD] [Simard supply-chain audit](https://github.com/rysweet/Simard/blob/main/docs/reference/supply-chain-audit.md)
- [GH-GUARD] [gh-guard](https://github.com/sbom-tool/gh-guard)
- [SOTA-RUST] [sota-rust security and supply chain rules](https://github.com/martinholovsky/sota-skills/blob/main/skills/sota-rust/rules/05-security-supply-chain.md)
- [DEVENV-RUST] [devenv Rust](https://devenv.sh/languages/rust/)
- [DEVENV-BLOG] [devenv: from environments to packaged applications for Rust](https://devenv.sh/blog/2025/08/22/closing-the-nix-gap-from-environments-to-packaged-applications-for-rust/)
- [Aurora Locus README](https://github.com/dollspace-gay/Aurora-Locus), read with Firecrawl: "this README does not carry canonical benchmark numbers"; dual MIT or Apache-2.0.

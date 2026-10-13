# LinkJar PDS in Rust: decision record and plan

Started 2026-10-08. The normative definition is [SPEC.md](SPEC.md). This page
records why the work exists, what it is built on, how it is delivered, and
where it stands. Nothing here changes the deployed server until the cutover
unit runs.

## Decision

Build LinkJar PDS, a Rust implementation of the AT Protocol PDS that is
compatible with the pinned reference server this repository builds today, and
that expresses LinkJar's account behaviour as native extension points instead
of source patches. Publish it for other operators. Replace the hosted
reference server with it in place, once the parity harness passes on a copy
of production data.

Reasons, in the order they weigh:

1. **The patch stack is the cost.** Nine source patches, about 11,100 lines
   (six and about 7,500 when this was decided; the sign-up journey, the
   invitation hand-off and upstream's shared-node fix came after), reapplied on every upstream bump and
   verified by image builds. Each new
   upstream release risks a conflict in the OAuth provider, the part upstream
   changes most. Native extension points remove the conflict surface.
2. **Ownership of the account and OAuth layer.** External sign-in, the
   creation receipt, handle policy and sign-in methods are LinkJar's product
   surface. Owning them in a typed server with a test harness is cheaper to
   evolve than patching a compiled React bundle.
3. **Measured performance and reliability.** A single static binary, lower
   memory per idle account, bounded firehose fan-out, and closed crash
   boundaries, each measured against the reference on the same scenarios
   (SPEC §16, §17).
4. **Give it back.** A stock profile that any operator can run, with the
   extension points documented for upstream discussion.

What this decision does not change: the September 2026 hosting decision in
`linkjar.io` (`docs/research/serverless-pds-2026-09/`) stands. The reference
server stays in production and the LinkJar launch does not depend on this
work. The client-side encryption model, the SPEC in `linkjar.io`, and the
account hosting design in `docs/research/linkjar-accounts-2026-09/` are
unchanged.

## Research

[How other systems solve what LinkJar PDS needs](research/2026-10-08-state-of-the-art.md)
compares the PDS implementations, clustering shapes, storage seams,
simulation testing, audit-log designs and supply-chain gates, with sources,
and lists the SPEC changes they imply.

[Gap report against atproto.com](research/2026-10-08-atproto-doc-sweep.md)
compares SPEC revision 0 with every specification and guide page: 53
missing requirements, 5 contradictions, 15 under-specified items, 12 things
changing in 2026. Revision 1 applies sections A to C and tracks D.

[The repository crate decision](research/2026-10-12-repository-crate.md)
records D2: the in-house repository engine and `rsky-repo`, as published
and from its main branch, on the same interop vectors, the same fixtures
from `@atproto/repo` at the pin and the same S1 to S3 workload.

[Upstream 0.5.35 to 0.5.37 against the frozen pin](research/2026-10-10-upstream-0.5.35-to-0.5.37.md)
classifies each upstream change after the pin, examines the shared-node fix
(#5584) for LinkJar's deployment, and prices the three pin options.

[Writes, the event log and the firehose](research/2026-10-13-writes-and-firehose.md)
records unit 4: the write path and the firehose, the writes run that holds
them to the Reference, what the Reference accepts from elsewhere, the
boundary between a commit and its event, the simulation, and what the unit
did not build.

[The read-only server](research/2026-10-13-read-only-server.md) records
unit 3: what the server serves, the restore run that holds it to the
Reference on one data directory, eleven things the comparison found, what
`verify` checks, and what the unit leaves out.

[Sidecars](sidecars.md) designs the two companion containers: an operations
and moderation console with OpenTelemetry built in, and an MCP server that
lets a person reach their own data from any AI client after signing in
through the PDS.

## Starting points examined

- **The reference** (`@atproto/pds@0.5.34`, commit `7ca16cc6`): the oracle.
  Its handler tree, databases, configuration and crash boundaries are the
  baseline the SPEC cites.
- **rsky-pds** (Blacksky): the most complete Rust PDS and in production for
  Blacksky accounts, on Postgres, S3 and Mailgun rather than the reference's
  SQLite-per-actor layout. Its repository crate, `rsky-repo`, was the
  candidate for SPEC D2; unit 2 ran it beside an in-house engine and chose
  the in-house engine ([decision record](research/2026-10-12-repository-crate.md)).
  Whether it ships an authorization server was not confirmed from public
  sources on 2026-10-08.
- **bluepds**: axum and SQLite on rsky's repository crate, self-described as
  not operable. Useful as a reading reference only.
- **millipds** (Python): a clear from-scratch architecture document, useful
  for the sequencer and MST design notes.

## Where it lives

- This repository. Each unit is a branch from `main` that merges back when
  its gate is green.
- The Cargo workspace is at the repository root. The reference image build
  moved under `legacy/` on 2026-10-08 (owner's request) with its workflows
  repointed; it stays buildable for security fixes until cutover and is
  removed afterwards.
- The `linkjar.io` repository gets only a pointer from its research index and
  a PDS URL setting for its browser gate and kit self-test (unit 1).
- The `linkjar/infra` repository changes in unit 8: service definition, alert
  rules, recovery runbook.

## Delivery units

Each unit is bounded, ends in a merge to `main` of this repository, and names
its gate. Units run one at a time in one worktree (decision of 2026-10-08).
Estimates are planning allowances for one engineer, low confidence until
units 2 and 5 land. Units 0 to 8 ship before cutover (the cutover bar of
2026-10-08); 9 onward follow it.

| Unit | Delivers | Gate | Allowance |
|---|---|---|---|
| 0 | Cargo workspace, licence files, devenv, CI gates (fmt, clippy, deny, vet, audit, fuzz skeleton), vendored lexicons and code generation, interop vectors wired, the "verify at pin" scenarios enumerated, Appendix A reconciled against the handler tree | Workspace builds; generated types compile for every vendored lexicon; every "verify at pin" item has a scenario id | 3 to 4 days |
| 1 | The parity harness (`parity/`) against the reference image only: compose with a local PLC and a Sync 1.1 relay, scenario runner, oracles O1 to O4 and O9, the port of `legacy/tests/*.mjs` to an HTTP endpoint, and the `linkjar.io` PDS URL setting | Harness green against Reference; O9 results recorded; stock-versus-production image differences documented | 1 to 2 weeks |
| 2 | Repository crate decision: interop vectors, MST and CAR benchmarks for `rsky-repo` versus in-house; `pds-types`, `pds-lexicon` and `pds-identity` with their vectors and the limits of SPEC §6.1 and §9 | Vectors pass; S1 to S3 micro-benchmarks published; D2 decided | 1 week |
| 3 | Read-only server: `sync.*`, `repo.get*`, `describeRepo`, `describeServer`, identity reads, `verify`, served from a staging restore of a reference data directory | O1 and O2 on read scenarios; `verify` zero differences on a staging restore | 2 weeks |
| 4 | Writes, sequencer, firehose, blob store, crawler notification, lifecycle event sequences, and the simulation seam: `StorageIo`, fault-injecting VFS, fail points, turmoil, seed runner, determinism meta-test, nightly runs | O1 to O3 on write and firehose scenarios including the crash injection; a local Sync 1.1 relay in strict mode converges; first nightly seed run green | 4 weeks |
| 5 | Accounts, legacy sessions, app passwords, admin, mail, permission sets, and the OAuth authorization server with the extension traits and the stock profile, designed for embedded and entryway modes | O4 and O6; the Bluesky app signs in (O8 recorded) | 6 weeks |
| 6 | The `linkjar` extension crate replacing the eight account patches at the patch pin (the ninth, 104, is engine behaviour in `pds-repo`, SPEC §6.1); handle hosts; `io.linkjar.account.*`; operator roles | O7 green; the patch docs' invariants have tests in the crate | 2 to 3 weeks |
| 7 | Audit log: entries, chain, checkpoints, checkpoint record, verifier, export; the pre-cutover snapshot procedure | Every §20.1 action produces an entry; verifier accepts an export and rejects a tampered one; checkpoint record appears on the firehose | 2 weeks |
| 8 | Migration endpoints, performance targets, metrics and OTLP export, alert rules, Nix package, restore drill on staging, cutover runbook, cutover | SPEC §16 targets met or revised with evidence; §18.1 preconditions met; cutover done and the 48-hour watch passed | 3 weeks |
| 9 | Operations console sidecar: ops listener, live connections and topology, moderation and cleanup actions, embedded OTLP pane ([sidecars.md](sidecars.md) §1) | Every action lands in the audit log with operator and reason; console runs with no database access | 3 weeks |
| 10 | MCP server sidecar: OAuth bridge to the PDS, scope map, tools and resources from the vendored lexicons ([sidecars.md](sidecars.md) §2) | Official MCP client and Inspector complete sign-in through the PDS page, refresh and revocation; writes audited | 3 weeks |
| 11 | Postgres implementation of the shared tier and `migrate-storage` | Harness green on both backends; a staging switch SQLite to Postgres and back verifies clean | 3 weeks |
| 12 | Entryway mode: the standalone account service, member mode, adoption of the single node, reconciliation | A member host behind the entryway passes the Harness; adoption changes no DID document | 4 weeks |

Unit 1 pays for itself alone: it guards the current patch stack across
upstream bumps, which nothing does today beyond the image smoke gate.

## Status

| Date | Event |
|---|---|
| 2026-10-13 | Unit 4b, second part, on `feat/unit-4b-vfs`. **The simulation loses power below SQLite** ([unit record](research/2026-10-13-power-loss-vfs.md)). Every database opens through `pds_storage::io`, and the simulation names a VFS there that it registers through `sqlite-plugin`, without `unsafe`. Inside a write of the plan the power goes: each file loses a seeded suffix of its unsynced writes, a kept write may be torn, the server restarts and repairs, and the repository must hold the records from before the write or those after it. 550 seeds passed with 5,440 losses of power; with SQLite's syncing turned off 30 of 30 seeds fail. Not modelled: I/O errors that the process survives, bit flips, misdirected writes, a full disk, and the blob store. The first part merged as pull request #31. Next: `turmoil`. |
| 2026-10-13 | Unit 4b, first part, on `feat/unit-4b-s3-store`. **The Candidate keeps blobs in an S3-compatible store** ([unit record](research/2026-10-13-s3-blob-store.md)). The owner chose on 2026-10-13 to build the three missing parts of unit 4 before unit 5, the store first. The object names are the Reference's, requests are signed with Signature Version 4, and an upload goes to the store in parts of 8 MiB as the client sends it. The writes run has an `s3` stage on a store that checks signatures: the Reference stores a blob and stages an upload, the Candidate takes over, serves and removes them, and `verify` checks the bucket with 0 differences. The stage showed that the Reference answers 500 to `getBlob` of an unreferenced upload on this store; the Candidate answers 400 (DD-18). The owner answered the open questions on 2026-10-13: an import into an active account now appends `#sync` (DD-19). A spike showed that `sqlite-plugin` 0.11.0 registers a VFS with the SQLite that `rusqlite` bundles ([unit record](research/2026-10-13-s3-blob-store.md) §8). SPEC revision 8. The first nightly seed run of unit 4 was dispatched by hand after `sim.yml` was repaired (pull request #30). Next: the fault-injecting VFS, then `turmoil`. |
| 2026-10-13 | Unit 4 on `feat/unit-4-writes`. **The Candidate writes, sequences and serves the firehose on a data directory that the Reference started** ([unit record](research/2026-10-13-writes-and-firehose.md)). `createRecord`, `putRecord`, `deleteRecord`, `applyWrites`, `uploadBlob`, `importRepo`, `listMissingBlobs` and `subscribeRepos` answer as the Reference does. The Harness has a writes run, `pnpm parity writes`: the Reference creates a pool of accounts, the Candidate takes over the volume, and eight scenarios run on each server; the differences are the deliberate ones of SPEC §2.3 and one wording, 37 entries in all. A relay in strict mode re-emits every commit of the Candidate. The Candidate stores a commit before it appends the event and notes the commit in `ops.sqlite`; stopped between the two, it states the head with `#sync` at the next start, and the relay converges. The run recorded what the Reference at the pin does with data from elsewhere: `importRepo` accepts a commit that any key signed and a commit for another account, a tree that is not a Merkle search tree, and record blocks that are not canonical. `pds-sim` runs seeded plans with stops at the fail points and five oracles; 550 seeds passed before the merge, and a seed run twice gives the same trace. SPEC revision 7 carries the owner's decisions of 2026-10-13 and what the run showed. **Not built: the fault-injecting VFS, the `turmoil` network and the S3-compatible blob store** ([below](#open-questions-for-the-owner)). Left for unit 5: the account events, rate limits, OAuth and service tokens, the proxy. Next: unit 5, from `main`. |
| 2026-10-13 | After unit 3, an audit and a cleanup (pull request #28). Nine readers audited the repository and a second reader checked each finding; 111 were confirmed and most are fixed. A streamed answer ends with an error when its producer fails or the client stalls for 60 seconds, and shutdown is bounded. `verify` reports a break in the event log without counting it as a difference, because the Reference deletes the events of a deleted account; it names fifteen kinds of damage in tests. The restore run has its own workflow. The reads of both servers are measured on one data directory of 100,000 records ([perf-reads.md](../parity/results/perf-reads.md)): the export takes 1,385 ms on the Reference and 167 ms on the Candidate, for the same bytes. What was not fixed, and why, is in the pull request. |
| 2026-10-13 | Unit 3 done on `feat/unit-3-read-server`. **The Candidate serves the reads of a data directory that the Reference wrote** ([unit record](research/2026-10-13-read-only-server.md)): the sync and repository reads, `describeServer`, `resolveHandle` and the plain routes. The Harness has a restore run, `pnpm parity restore`: the Reference seeds six accounts and answers four read scenarios, the Candidate replaces it on the same volume and answers them again. 331 requests, no difference in status, compared headers or body, and 328 bodies equal byte for byte, repository exports among them. `verify` reports zero differences on that directory and names fourteen kinds of damage in tests. `pds-storage` reads the Reference's SQLite files, `pds-identity` resolves DIDs through the cache, `pds-xrpc` validates parameters in the Reference's order and with its messages. The image is built by `./Dockerfile`, and CI runs the restore run on every change to the crates. SPEC revision 6 carries what the comparison showed. **Not met: `verify` on a staging restore**, which needs the owner's copy ([below](#open-questions-for-the-owner)). Left for later units: the proxy, the read-after-write methods, rate limits, OAuth tokens, handle resolution over DNS. Next: unit 4, from `main`. |
| 2026-10-13 | The ninth patch is in production's image stream: `104-mst-shared-node` merged (pull request #25) and revision 12 of the reference image is published. The Harness pins it and has an `importRepo` scenario, `default/15-import`. It shows what the patch closes: the unpatched build answers 200 to a tree that references a node more than once and replaces the repository with it; revision 12 answers 400 and keeps the repository. DD-6 has moved into the table of observed differences (SPEC §2.3). The scenario also found that `importRepo` answers 500 to bytes that are not a CAR, on both builds. |
| 2026-10-12 | Between units 2 and 3. The owner decided the pin, D17 and the rest of D16 ([below](#decisions-taken-by-the-owner-2026-10-12)); SPEC revision 5 carries them. The upstream 0.5.37 note is on `main` (pull request #20). `devenv shell` installs no git hook any more, and the weekly upstream check reports releases after the pin in a run summary and an issue (pull request #23). The Monday audit reads CVSS 4.0 advisories (pull request #22). Open before unit 3: the ninth patch of the legacy build. Next: unit 3, from `main`. |
| 2026-10-12 | Unit 2 done on `feat/unit-2-repository` (pull request #21). **D2 is decided: the repository engine is written in this repository** ([decision record](research/2026-10-12-repository-crate.md)). `pds-repo` holds the tree, commits, CAR, export and the checks on a repository from elsewhere, synchronous and without I/O. It is held to fixtures that a tool in `parity/` writes with `@atproto/repo` at the pin: thirteen commits replay to the same tree roots, commit CIDs, block sets, firehose CARs and export, byte for byte, and its firehose CAR sizes and block counts equal what unit 1 measured on the reference image. `rsky-repo` ran on the same vectors and workload in `bench/`, as published (0.0.6) and from its main branch (0.1.4). S1 to S3 micro-benchmarks for all three and for the Reference's library are published with the method and the host. `pds-types` applies the decode limits of SPEC §6.1 in a strict DAG-CBOR reader; `pds-lexicon` validates records and passes the record-data and data-model vectors; `pds-identity` has keys, low-S signatures on both curves, `did:key`, the DID-document contract and the handle rules, and passes the crypto vectors. No vector is a known failure beyond the three CID strings of unit 0. Five fuzz targets are new. SPEC revision 4 records D2 and opens D17. Also in the unit: `devenv shell` starts on devenv 2.4 again, and `scripts/vendor-agent-sources.sh` mirrors the pinned upstream sources for reading. Next: unit 3, from `main`. |
| 2026-10-09 | Units 0 and 1 merged to `main` (pull requests #14 and #15) with every gate green in CI. Their first CI runs showed four faults, all repaired: the Rust workflow gave its pinned toolchain action no `toolchain` input; a `main` push that touches `legacy/` tried to publish revision 11 again, so a `release` job now publishes only when `upstream.json` names an unused tag; and the nightly fuzz workflow could not build, first for the workspace manifest and then for the target of the prebuilt cargo-fuzz (#16, #17). The parity workflow runs its four targets side by side, the fuzz corpus is kept between nights, and the Rust gates run every Monday (#18). The owner decided D15 and the service-token part of D16 ([below](#decisions-taken-2026-10-09)); SPEC revision 3 carries them as §2.3. `linkjar.io` has the PDS URL setting on `main`. Next: unit 2, from `main`. |
| 2026-10-09 | Unit 1 done on `feat/unit-1-parity`, stacked on unit 0. The parity harness (`parity/`) runs 28 scenarios in five profiles against the reference image `ghcr.io/linkjar/pds` revision 11 by digest, a second boot of it, and the stock build, each in its own compose stack: the production hostnames behind a TLS edge, a PLC directory, a strict Sync 1.1 relay, a mail catcher, and fixtures for everything else. Oracles O1 to O4 and O9 are in place: two Reference boots compare equal, the difference between the stock and the production build is recorded by patch ([differences](../parity/differences/README.md)), every commit is verified against the PLC key and its MST root rebuilt, the relay re-emits every commit, and the OAuth flows run through the official client and Chromium. VP-8 to VP-12 are answered from the pinned source and confirmed by scenario ([verify-at-pin.md](../parity/verify-at-pin.md)). The legacy suites are ported to HTTP scenarios, with the assertions HTTP cannot observe listed. SPEC revision 2 carries the results and seventeen corrections that the scenarios forced, among them: a method without a handler is proxied and never 501, a deleted blob has no grace period, and the patch stack has eight patches, not six. Reference baselines for S1 to S5, S8 and S9 are in [parity/results](../parity/results/perf-reference.md). The `linkjar.io` browser gate takes `E2E_PDS_URL`. |
| 2026-10-08 | Unit 0 in progress on `feat/rust-pds`: Cargo workspace with fifteen crates and `xtask`; the legacy image build moved under `legacy/` with its workflows repointed; docs reorganised with a root README and an architecture page whose Mermaid sources render through beautiful-mermaid; `pds-types` passes every interop syntax and data-model vector (three contrived CID strings recorded as known failures); `pds-lexicon` parses and validates all 263 vendored lexicons with every reference resolved; `cargo xtask codegen` generates the API types (about 10,000 lines) and they compile and round-trip real records; CI gates (fmt, clippy with the deny-panic set, tests, codegen check, deny, vet, scheduled audit and fuzz) and devenv are in place. Seven of twelve "verify at pin" items answered from the pinned source ([parity/verify-at-pin.md](../parity/verify-at-pin.md)). |
| 2026-10-08 | Decision taken. Branch `feat/rust-pds` created from `main`. SPEC revision 0 written from the reference source at the pin and the six patch documents. No code yet. |
| 2026-10-08 | Documentation sweep against atproto.com completed; SPEC revision 1 written: decisions applied, the research's sections added (storage seams, simulation, audit, topologies, sidecars), every gap-report item folded in, units renumbered 0 to 12 with the cutover bar after unit 8. Patch pin recorded. No code yet. |
| 2026-10-08 | Scope widened by the owner: storage and auth extensibility, clustering, audit and provenance, simulation testing, a database switch, devenv. [State-of-the-art research](research/2026-10-08-state-of-the-art.md) recorded; its §9 lists the SPEC sections to add in revision 1. Headline: the account and OAuth crate should run embedded or as an entryway, which is how the protocol itself scales; audit checkpoints can be published as repository records so the firehose witnesses them. |

## Decisions taken 2026-10-08

Answered by the owner after the research record and the sidecar design.
Revision 1 of the SPEC applies them.

| Topic | Decision |
|---|---|
| Home | `linkjar/pds` is the project. The public name is LinkJar PDS. |
| Licence | Dual MIT or Apache-2.0 for the project's own code. |
| NSID authority | Project lexicons (ops methods, audit checkpoint record, MCP scopes) live under `io.linkjar.pds.*`. |
| Cluster shape | Entryway plus member hosts. The account and OAuth crate is designed from unit 5 to run embedded in one node or as a standalone entryway. The shared-Postgres cluster is documented as an alternative only. |
| Cutover bar | Units 0 to 8, the audit log (unit 7) among them, must ship before the Candidate replaces the reference. Sidecars, Postgres and entryway mode come after. |
| Simulation | The `StorageIo` seam, a fault-injecting SQLite VFS, fail points and turmoil enter in unit 4; nightly seeds from then on. |
| Audit scope | Account and operator actions, including console and MCP writes. Reads and ordinary repository writes are not audited. |
| Patch stack | Frozen except security fixes while the Rust server is built. The `linkjar` profile targets the patches at the **patch pin**. A security fix moves the pin and adds the mirrored behaviour to SPEC §13. The pin was `main` of this repository at `08b1235` (2026-10-08, "Let the app hand the invite code to the OAuth sign-up page", revision 11) and has been at `0c34a1b` since 2026-10-12, when patch 104 joined the stack (revision 12). |
| OAuth pages | Behaviour parity with new server-rendered pages to the LinkJar brand guidelines. No pixel parity. |
| Shadow runs | Unit 3 verifies against staging restores only. Production is touched only at cutover. |
| Reports | `createReport` keeps forwarding to a configured external moderation service as the reference does. The console links out; it holds no report queue. |
| Local MCP | The private-data MCP mode is a TypeScript process in `linkjar.io` beside the kit. This repository ships the hosted, public-data MCP server only. |
| Work mode | Sequential units, one worktree at a time, each merged to `main` with its gate green. |

## Decisions taken 2026-10-09

Answered by the owner after unit 1. Revision 3 of the SPEC applies them.

| Topic | Decision |
|---|---|
| D15, client errors that the Reference answers with 500 | The Candidate answers 400 `InvalidRequest`. Four cases: `createRecord` on a key that exists, `applyWrites` with such a create, `createAppPassword` with a name in use, an `atproto-proxy` value that does not start with a DID. They are deliberate differences DD-1 to DD-4 in SPEC §2.3, and the Harness records them. |
| D16, replay of a service token | A service token is good for one successful request. The Candidate refuses a spent token until it expires: 401 `BadJwt` (SPEC §5.4, DD-5). A request that fails does not spend its token, so a caller can retry with the same one. |

## Decision taken 2026-10-12

Taken by unit 2, as the plan assigns it. Revision 4 of the SPEC records it.

| Topic | Decision |
|---|---|
| D2, the repository crate | The engine is written in this repository, in `pds-repo`. It gives the same tree roots, commit CIDs, commit blocks and export bytes as `@atproto/repo` at the pin, refuses trees that reference a node more than once, and is synchronous with no I/O of its own. `rsky-repo` as published (0.0.6) fails four of six commit-proof vectors; its main branch (0.1.4, not published) passes them. Neither form checks an imported tree as SPEC §6.1 requires; both take the revision from the wall clock and are asynchronous over a locked store. Evidence and numbers: [decision record](research/2026-10-12-repository-crate.md). |

## Decisions taken by the owner 2026-10-12

Answered after unit 2. Revision 5 of the SPEC applies them.

| Topic | Decision |
|---|---|
| Upstream pin | Keep 0.5.34 and carry upstream's shared-node fix (#5584) as a ninth patch, revision 12. Evidence and cost: [the upstream note](research/2026-10-10-upstream-0.5.35-to-0.5.37.md) §5. |
| D17, data from elsewhere | Where the Candidate is stricter than the Reference at the pin, it keeps its check. Each case gets a scenario that records what the pin does and a row in SPEC §2.3: DD-6 to DD-9 and DD-11 to DD-13. The cases are those of [the decision record](research/2026-10-12-repository-crate.md) §7, items 1 to 4, 6 and 7. Item 5 is no difference: the Candidate validates records as the pinned server does. |
| D16, blobs | The Candidate collects an upload that no record references after one hour (DD-10). It keeps no grace period: a dereferenced blob is deleted at once, as at the pin. |

## Decisions taken by the owner 2026-10-13

Answered after unit 3: the owner took the recommendation on each open
question. Revision 7 of the SPEC applies them.

| Topic | Decision |
|---|---|
| The commit signature on import | The Candidate keeps the check of SPEC §6.5: `importRepo` verifies the commit signature against the account's key. The Reference at the pin verifies nothing. It is deliberate difference DD-14, with a scenario in unit 4. |
| Three more cases for D15 | The Candidate answers 400 `InvalidRequest` to each. `importRepo` with bytes that are not a CAR is DD-15 (unit 4). `createAccount` with the handle, and with the email, of a deactivated or taken-down account are DD-16 and DD-17 (unit 5). |
| Outbound TLS | `aws-lc-rs` is accepted as the TLS provider of outbound requests. SPEC §3.2 names RustCrypto for the protocol's own cryptography only. |
| The licence of the root list | `deny.toml` keeps the CDLA-Permissive-2.0 exception for `webpki-root-certs`. |
| The three missing parts of unit 4 | Built before unit 5, as unit 4b: the S3-compatible blob store first, then the fault-injecting VFS, then `turmoil`. |
| The S3 client of the pin (#5575) | Not carried now. A tenth patch publishes an image, and the evidence that decides it is a measurement on staging. Confirmed on 2026-10-13, second round: it stays out until staging shows the count of open connections to R2 growing under repeated `getBlob`. |
| DD-18, `getBlob` of an unreferenced upload on the S3-compatible store | Second round, 2026-10-13. The Candidate answers 400 `InvalidRequest` on both stores, as one more case of D15. |
| `#sync` after an import into an active account | Second round. The Candidate emits it; it is deliberate difference DD-19 (SPEC §6.5). |
| The `importRepo` finding | Second round. It is reported upstream as a public issue. The draft is in [the unit record](research/2026-10-13-s3-blob-store.md#7-draft-of-the-report-upstream); the owner sends it. |
| Imports in production | Second round. `PDS_ACCEPTING_REPO_IMPORTS=false` on the production host until revision 12 runs there, unless move-in is offered to people. The owner sets it. |

## Open questions for the owner

Four, each an action on a system that is the owner's. Unit 5 needs none
of them.

- **`verify` on a staging restore.** Unit 3's gate asks for zero
  differences on a staging restore of production data. The unit had no such
  copy. Run `linkjar-pds verify` on it
  ([unit record](research/2026-10-13-read-only-server.md) §5) and send the
  report; a finding there is a fact about production data that the cutover
  runbook has to handle. Production blobs are in R2: with the
  `PDS_BLOBSTORE_S3_*` variables of a copy of the bucket, or of the bucket
  itself, the command lists the objects of each account and writes
  nothing. Without a blob store configured it checks everything else and
  says that it skipped the blob store.

- **Imports in production until revision 12 is deployed.** Revision 12 of
  the reference image is published. Until it runs in production, the
  upstream note recommends `PDS_ACCEPTING_REPO_IMPORTS=false` on the host,
  unless move-in is already offered to people. The Harness showed what the
  unpatched build does with a tree that shares nodes: it answers 200 and
  replaces the repository (`default/15-import`). The setting is on the host
  and is yours to change.

- **Send the report on `importRepo`.** The owner said yes on 2026-10-13.
  The draft is in
  [the unit record](research/2026-10-13-s3-blob-store.md#7-draft-of-the-report-upstream).

- **Measure the S3 client of the pin on staging.** The fix (#5575) stays
  out of the legacy build until the count of open connections from the PDS
  to R2 grows under repeated `getBlob`
  ([the upstream note](research/2026-10-10-upstream-0.5.35-to-0.5.37.md) §4).

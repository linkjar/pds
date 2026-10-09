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

1. **The patch stack is the cost.** Eight source patches, about 10,400 lines
   (six and about 7,500 when this was decided; the sign-up journey and the
   invitation hand-off came after), reapplied on every upstream bump and
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
  SQLite-per-actor layout. Its repository crate is the first candidate for
  SPEC D2. Whether it ships an authorization server was not confirmed from
  public sources on 2026-10-08.
- **bluepds**: axum and SQLite on rsky's repository crate, self-described as
  not operable. Useful as a reading reference only.
- **millipds** (Python): a clear from-scratch architecture document, useful
  for the sequencer and MST design notes.

## Where it lives

- This repository, branch `feat/rust-pds`, until the first unit merges.
- The Cargo workspace is at the repository root. The reference image build
  moved under `legacy/` on 2026-10-08 (owner's request) with its workflows
  repointed; it stays buildable for security fixes until cutover and is
  removed afterwards.
- The `linkjar.io` repository gets only a pointer from its research index and
  a PDS URL setting for its browser gate and kit self-test (unit 1).
- The `linkjar/infra` repository changes in unit 7: service definition, alert
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
| 1 | The parity harness (`parity/`) against the reference image only: compose with a local PLC and a Sync 1.1 relay, scenario runner, oracles O1 to O4 and O9, the port of `tests/*.mjs` to an HTTP endpoint, and the `linkjar.io` PDS URL setting | Harness green against Reference; O9 results recorded; stock-versus-production image differences documented | 1 to 2 weeks |
| 2 | Repository crate decision: interop vectors, MST and CAR benchmarks for `rsky-repo` versus in-house; `pds-types`, `pds-lexicon` and `pds-identity` with their vectors and the limits of SPEC §6.1 and §9 | Vectors pass; S1 to S3 micro-benchmarks published; D2 decided | 1 week |
| 3 | Read-only server: `sync.*`, `repo.get*`, `describeRepo`, `describeServer`, identity reads, `verify`, served from a staging restore of a reference data directory | O1 and O2 on read scenarios; `verify` zero differences on a staging restore | 2 weeks |
| 4 | Writes, sequencer, firehose, blob store, crawler notification, lifecycle event sequences, and the simulation seam: `StorageIo`, fault-injecting VFS, fail points, turmoil, seed runner, determinism meta-test, nightly runs | O1 to O3 on write and firehose scenarios including the crash injection; a local Sync 1.1 relay in strict mode converges; first nightly seed run green | 4 weeks |
| 5 | Accounts, legacy sessions, app passwords, admin, mail, permission sets, and the OAuth authorization server with the extension traits and the stock profile, designed for embedded and entryway modes | O4 and O6; the Bluesky app signs in (O8 recorded) | 6 weeks |
| 6 | The `linkjar` extension crate replacing the eight patches at the patch pin; handle hosts; `io.linkjar.account.*`; operator roles | O7 green; the patch docs' invariants have tests in the crate | 2 to 3 weeks |
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
| 2026-10-09 | Unit 1 done on `feat/rust-pds`, stacked on unit 0. The parity harness (`parity/`) runs 28 scenarios in five profiles against the reference image `ghcr.io/linkjar/pds` revision 11 by digest, a second boot of it, and the stock build, each in its own compose stack: the production hostnames behind a TLS edge, a PLC directory, a strict Sync 1.1 relay, a mail catcher, and fixtures for everything else. Oracles O1 to O4 and O9 are in place: two Reference boots compare equal, the difference between the stock and the production build is recorded by patch ([differences](../parity/differences/README.md)), every commit is verified against the PLC key and its MST root rebuilt, the relay re-emits every commit, and the OAuth flows run through the official client and Chromium. VP-8 to VP-12 are answered from the pinned source and confirmed by scenario ([verify-at-pin.md](../parity/verify-at-pin.md)). The legacy suites are ported to HTTP scenarios, with the assertions HTTP cannot observe listed. SPEC revision 2 carries the results and seventeen corrections that the scenarios forced, among them: a method without a handler is proxied and never 501, a deleted blob has no grace period, and the patch stack has eight patches, not six. Reference baselines for S1 to S5, S8 and S9 are in [parity/results](../parity/results/perf-reference.md). The `linkjar.io` browser gate takes `E2E_PDS_URL`. |
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
| Cutover bar | Units 0 to 7 plus the audit log must ship before the Candidate replaces the reference. Sidecars, Postgres and entryway mode come after. |
| Simulation | The `StorageIo` seam, a fault-injecting SQLite VFS, fail points and turmoil enter in unit 4; nightly seeds from then on. |
| Audit scope | Account and operator actions, including console and MCP writes. Reads and ordinary repository writes are not audited. |
| Patch stack | Frozen except security fixes while the Rust server is built. The `linkjar` profile targets the patches at the **patch pin**: `main` of this repository at `08b1235` (2026-10-08, "Let the app hand the invite code to the OAuth sign-up page"). A security fix moves the pin and adds the mirrored behaviour to SPEC §13. |
| OAuth pages | Behaviour parity with new server-rendered pages to the LinkJar brand guidelines. No pixel parity. |
| Shadow runs | Unit 3 verifies against staging restores only. Production is touched only at cutover. |
| Reports | `createReport` keeps forwarding to a configured external moderation service as the reference does. The console links out; it holds no report queue. |
| Local MCP | The private-data MCP mode is a TypeScript process in `linkjar.io` beside the kit. This repository ships the hosted, public-data MCP server only. |
| Work mode | Sequential units, one worktree at a time, each merged to `main` with its gate green. |

## Open questions for the owner

Two, from unit 1, recorded as SPEC Appendix D, D15 and D16. Neither blocks
unit 2.

- **D15.** The Reference answers 500 to four client errors (a record key
  that exists, a batch with one, an app-password name in use, a proxy target
  that is not a resolvable DID). Keep them in the Candidate for C1, or
  answer 400 and record the difference.
- **D16.** Three hardenings the pin does not have and a client could
  observe: a replay cache for service-auth `jti`, collection of uploads no
  record ever referenced, and a grace period before a dereferenced blob is
  deleted. Adopt any of them, or follow the pin.

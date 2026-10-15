# LinkJar PDS: plan

LinkJar PDS is a Rust implementation of the AT Protocol personal data server.
It is compatible with the reference server that production runs today
(`@atproto/pds@0.5.34` with LinkJar's patches), and it expresses LinkJar's
account behaviour as extension points instead of source patches. The
normative definition is [SPEC.md](SPEC.md).

**Where it stands on 2026-10-15.** Every unit of the plan has code on `main`,
and the Candidate is ready to test by hand: the
[test-host runbook](runbooks/test-host.md) puts the published image on a
host with a name of its own, and `pnpm parity smoke <url>` checks a running
server from outside. Production still runs the reference server. The next
step is the cutover, and the steps before it are the owner's: they need
staging, production and `linkjar/infra` (see [Owner's steps](#owners-steps)).

## Why

1. **The patch stack is the cost.** Nine source patches, about 11,100 lines,
   are reapplied on every upstream release. Each release risks a conflict in
   the OAuth provider, the part upstream changes most. Extension points
   remove that conflict surface.
2. **LinkJar owns its account layer.** External sign-in, the creation
   receipt, the handle policy and sign-in methods are LinkJar's product. A
   typed server with a test harness is cheaper to change than a patched
   React bundle.
3. **Measured performance and reliability.** One static binary, less memory
   per idle account, bounded firehose fan-out and closed crash boundaries,
   each measured against the reference server on the same scenarios
   (SPEC §16, §17).
4. **A server other operators can run.** The `stock` profile is a complete
   PDS, dual-licensed MIT or Apache-2.0.

This work does not change the September 2026 hosting decision in
`linkjar.io`, the client-side encryption model, or the LinkJar launch, which
does not depend on it.

## Units

Each unit is a branch from `main` that merged when its gate passed. A
checked box is met and recorded; an open box says who or what it waits on.
The unit records under [research/](research/) have the evidence and the
full list of what each unit left out.

### Before the cutover

- [x] **0. Workspace.** Cargo workspace, licence files, CI gates (format,
  lint, `cargo deny`, `cargo vet`, advisories, fuzz targets), vendored
  lexicons with generated types, interop vectors.
- [x] **1. Parity harness.** `parity/` drives the reference image and the
  Candidate through the same scenarios behind a TLS edge, with a PLC
  directory, a strict Sync 1.1 relay and fixtures
  ([record](research/2026-10-09-parity-harness.md)).
- [x] **2. Repository engine.** `pds-repo`, written in this repository after
  a comparison with `rsky-repo`; same tree roots, commits and exports as the
  reference ([record](research/2026-10-12-repository-crate.md)).
- [ ] **3. Read-only server.** Serves the reads of a data directory that the
  reference wrote; `verify` checks a data directory
  ([record](research/2026-10-13-read-only-server.md)).
  - [x] Response comparison and repository CIDs on the read scenarios.
  - [ ] `verify` with no difference on a staging restore of production data
    (owner's step).
- [x] **4. Writes and firehose.** Writes, the event log, the firehose, blob
  stores on disk and S3, crawler notices, and the simulation: a file system
  that loses power below SQLite, and a simulated network
  ([record](research/2026-10-13-writes-and-firehose.md),
  [S3 store](research/2026-10-13-s3-blob-store.md),
  [simulation](research/2026-10-13-power-loss-vfs.md)). The nightly
  simulation runs green.
- [ ] **5. Accounts and OAuth.** Accounts, legacy sessions, app passwords,
  admin methods, mail, rate limits, the proxy and the OAuth authorization
  server with server-rendered pages
  ([accounts](research/2026-10-13-accounts-and-sessions.md),
  [OAuth](research/2026-10-13-oauth.md)).
  - [x] The OAuth flows and the stock-equivalence run of the Harness.
  - [ ] The Bluesky app signs in through the Candidate and posts, recorded
    by hand (owner's step).
- [ ] **6. The `linkjar` profile.** The behaviour of the eight account
  patches as one extension crate: external sign-in, the creation receipt,
  the handle policy and handle hosts, sign-in methods, the sign-up journey
  ([record](research/2026-10-13-linkjar-profile.md)).
  - [x] The LinkJar scenarios of the Harness pass.
  - [ ] The browser gate and kit self-test of `linkjar.io` pass against a
    Candidate (`E2E_PDS_URL`; owner's step).
- [x] **7. Audit log.** A hash chain of account and operator actions, with
  signed checkpoints published as records and an offline verifier
  ([record](research/2026-10-13-audit-log.md)).
- [ ] **8. Cutover.** Account migration in, metrics, the data-directory
  lock, and the [cutover runbook](runbooks/cutover.md)
  ([record](research/2026-10-13-migration-and-telemetry.md)).
  - [x] Migration in, metrics, the runbook, and the server and sidecar
    images published from `main`.
  - [x] A test host for hand tests: a compose file with Caddy and
    on-demand certificates (`GET /tls-check`), its
    [runbook](runbooks/test-host.md), and a smoke test of a running
    server. The smoke test passes against the published image and the
    Reference in the Harness's stacks.
  - [ ] The performance targets of SPEC §16 measured on production
    hardware (owner's step).
  - [ ] The restore drill, the cutover and the 48-hour watch (owner's
    steps).

### After the cutover

- [x] **9. Operations console.** An ops listener on the server and a
  console sidecar; every action is audited with its operator and reason
  ([record](research/2026-10-13-ops-console.md)).
- [x] **10. MCP server.** A sidecar through which an AI client reads and
  writes a person's repository after sign-in on the PDS pages. The official
  MCP TypeScript client and the MCP Inspector run against it in CI
  ([record](research/2026-10-13-mcp-server.md)).
- [x] **11. Postgres.** The shared databases on Postgres, and
  `migrate-storage` to copy them there and back. The Harness passes on
  Postgres ([record](research/2026-10-13-postgres.md)).
  - [ ] The switch on staging, SQLite to Postgres and back (owner's step).
- [x] **12. Entryway.** A server can be an entryway that keeps the accounts,
  or a member host that keeps the repositories. A single node is adopted
  without a change of DID document. The Harness passes with a member host
  behind an entryway; `linkjar-pds reconcile` repairs a sign-up that a crash
  stopped ([record](research/2026-10-14-entryway.md),
  [runbook](runbooks/entryway.md)).

## Owner's steps

Actions on systems that are the owner's. The order follows the
[cutover runbook](runbooks/cutover.md).

- [ ] Set `PDS_ACCEPTING_REPO_IMPORTS=false` on the production host until
  revision 12 of the reference image runs there, unless move-in is offered.
  The unpatched build replaces a repository with an imported tree that
  shares nodes.
- [ ] Send the `importRepo` report upstream. The draft is in
  [the S3 store record](research/2026-10-13-s3-blob-store.md#7-draft-of-the-report-upstream).
- [ ] Measure the reference's S3 client on staging: does the count of open
  connections to R2 grow under repeated `getBlob`? The answer decides
  whether upstream's fix (#5575) becomes a tenth patch
  ([upstream note](research/2026-10-10-upstream-0.5.35-to-0.5.37.md) §4).
- [ ] Restore production to staging and run `linkjar-pds verify` on the
  copy, with the blob store of the copy. Resolve every finding.
- [ ] Run the Harness against the staging copy with the `linkjar` profile.
- [ ] Start a Candidate on a test host
  ([test-host runbook](runbooks/test-host.md)) and run
  `pnpm parity smoke <url> --write` against it.
- [ ] Point the `linkjar.io` browser gate and kit self-test at a Candidate.
- [ ] Record the Bluesky app signing in through a Candidate and posting.
- [ ] Measure the SPEC §16 targets on production hardware.
- [ ] Update `linkjar/infra`: the service definition, the alert rules and
  the recovery runbook.
- [ ] Create the audit checkpoint key and name the operator's repository.
- [ ] Run the cutover and the 48-hour watch.

Two rules follow the reference server and stay as built unless the owner
says otherwise:

- [ ] Confirm: a handle under `.test` is accepted in every mode, as the
  reference does. A stricter rule protects nothing, because such a handle
  cannot verify on the public internet.
- [ ] Confirm: the OAuth metadata lists the two signing algorithms the
  server verifies, `ES256` and `ES256K`. Every atproto client signs DPoP
  proofs with `ES256`.

## Not built

None of these blocks the cutover. Each record's "What is not built" section
has the detail.

Accounts and OAuth

- [ ] The reference's list of disposable email domains, and the slur check
  on handles.
- [ ] Permission sets: an `include:` scope is accepted and grants nothing.
- [ ] The settings of the account page (email address, password), the
  reactivation step for a deactivated account, hCaptcha on the sign-up page,
  and the password reset from the sign-in page.
- [ ] The periodic re-fetch of a confidential client's keys.
- [ ] `didDoc` in session answers (`PDS_ENABLE_DID_DOC_WITH_SESSION`).

Operations

- [ ] Export over OTLP, and metrics of the commit path, the sequencer and
  the subscribers.
- [ ] A Nix package.
- [ ] The audit checkpoint key in a KMS or an HSM, and a witness that
  compares checkpoints over time.
- [ ] The console's telemetry pane, a console under a host name, and forms
  for record and blob takedowns.

Scale

- [ ] Several server processes over one Postgres tier (the DPoP replay set,
  the rate limits and the head of the event log are in one process's
  memory), TLS to Postgres, and the Harness on Postgres in CI.
- [ ] More than one member host behind an entryway, with a map from account
  to host.
- [ ] An MCP session store that outlives the process, and tool schemas from
  the lexicons.

Harness

- [ ] A scenario for a migration between two servers, and one for the ops
  listener.

## Decisions

Decisions that shaped the work, by date. SPEC
[Appendix D](SPEC.md#appendix-d-decisions) lists every decision, with the
ones still open.

| Date | Decision |
|---|---|
| 2026-10-08 | The project is `linkjar/pds`, named LinkJar PDS, dual-licensed MIT or Apache-2.0. The Cargo workspace is at the repository root; the reference image build moved to `legacy/` and is removed after the cutover. |
| 2026-10-08 | Units 0 to 8, the audit log among them, ship before the cutover. The sidecars, Postgres and the entryway come after. |
| 2026-10-08 | The patch stack is frozen except for security fixes; the `linkjar` profile targets the patches at the patch pin. |
| 2026-10-08 | Scaling follows the protocol: an entryway over member hosts. A shared-Postgres cluster is an alternative only. |
| 2026-10-08 | The OAuth pages are new and server-rendered to the LinkJar brand; behaviour matches the reference, pixels do not. |
| 2026-10-08 | Account and operator actions are audited, console and MCP writes among them. Reads and ordinary repository writes are not. |
| 2026-10-08 | Verification uses staging restores; production is touched only at the cutover. |
| 2026-10-08 | `createReport` forwards to an external moderation service; the console holds no report queue. The private-data MCP mode lives in `linkjar.io`. |
| 2026-10-09 | Where the reference answers 500 to a client error, the Candidate answers 400 `InvalidRequest`. Each case is a deliberate difference (SPEC §2.3). |
| 2026-10-09 | A service token is good for one successful request. |
| 2026-10-12 | The repository engine is written in this repository (`pds-repo`). |
| 2026-10-12 | The pin stays at 0.5.34 and carries upstream's shared-node fix as a ninth patch. |
| 2026-10-12 | Where the Candidate is stricter than the reference on data from elsewhere, it keeps the check, and each case is a deliberate difference with a scenario. |
| 2026-10-12 | An upload that no record references is collected after one hour. |
| 2026-10-13 | `importRepo` verifies the commit signature; `#sync` follows an import into an active account. |
| 2026-10-13 | `aws-lc-rs` is the TLS provider of outbound requests; RustCrypto stays for the protocol's own cryptography. |
| 2026-10-13 | Upstream's S3 client fix (#5575) stays out until staging shows the connection growth. |

## Research

- [How other systems solve what LinkJar PDS needs](research/2026-10-08-state-of-the-art.md):
  PDS implementations, clustering, storage seams, simulation testing, audit
  logs and supply-chain gates.
- [Gap report against atproto.com](research/2026-10-08-atproto-doc-sweep.md):
  the first SPEC compared with every specification and guide page.
- [Upstream 0.5.35 to 0.5.37 against the pin](research/2026-10-10-upstream-0.5.35-to-0.5.37.md):
  each upstream change after the pin, and the cost of moving it.
- [Sidecars](sidecars.md): the design of the operations console and the MCP
  server.
- [Architecture](architecture/README.md): the crates and how a request moves
  through them.

## Ground rules

- One unit at a time, each merged to `main` with its gate green.
- The reference image build in `legacy/` stays buildable for security fixes
  until the cutover.
- `linkjar.io` gets only a pointer and a PDS URL setting for its browser
  gate; `linkjar/infra` changes at the cutover.

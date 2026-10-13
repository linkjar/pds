# Parity harness

[Specification §17](../docs/SPEC.md#17-parity-and-conformance) · [Verify at pin](verify-at-pin.md) · [Differences between targets](differences/README.md) · [Architecture](../docs/architecture/README.md#parity-harness)

The harness holds a PDS to the behaviour of the reference image. It drives a
target with the official atproto SDK at the pin and a headless browser,
records what the target answered, and compares two targets on the record.

The Candidate reads but does not write yet. It is held by the
[restore run](#the-restore-run): the Reference writes a data directory, and
both servers answer the same read scenarios on it. When the Candidate writes
(unit 4), the other scenarios run against it as one more target.

## Run it

Requirements: Docker with Compose, Node 24 and pnpm 12 (`devenv shell`
provides both), and a login to `ghcr.io` that can read `linkjar/pds`.

```sh
cd parity
pnpm install
pnpm exec playwright install chromium   # once

pnpm parity images verify               # the pinned Reference is the one upstream.json describes
pnpm parity run reference               # default profile, on a fresh stack
pnpm parity run reference --profile invites
pnpm parity run reference --profile firehose
pnpm parity run reference --profile captcha

pnpm parity run repeat                  # a second boot of the same image
pnpm parity compare reference repeat    # must show no difference

pnpm parity images build-stock          # PATCH_PROFILE=none build of ../legacy
pnpm parity run stock
pnpm parity compare reference stock     # must show exactly the recorded differences

pnpm parity run official                # the upstream distribution, by digest
pnpm parity compare stock official      # must show no difference
```

```sh
pnpm parity images build-candidate      # ../Dockerfile, tagged linkjar-pds:candidate
pnpm parity restore                     # must show no difference
pnpm parity writes                      # must show the recorded differences only
```

Each `run` builds a fresh stack, runs the scenarios of one profile in file
order, and removes the stack. `--keep` leaves it up; `--reuse --only <text>`
reruns part of a profile against a stack that is already up. When a run
fails, the logs of the PDS, relay, PLC directory and edge are saved under
`.run/<target>/logs/`.

Other commands: `pnpm typecheck`, `pnpm test` (unit tests of the normaliser),
`pnpm parity stack up|down|logs <target>`.

## How it is built

| Part | Where | What it does |
|---|---|---|
| Targets | [`src/stack/targets.ts`](src/stack/targets.ts), [`images.json`](images.json) | A target is an image. `reference` is `ghcr.io/linkjar/pds` by digest, `stock` the `PATCH_PROFILE=none` build, `official` the upstream distribution by digest, `repeat` a second boot of `reference`, `candidate` the image in `PARITY_CANDIDATE_IMAGE`, by default `linkjar-pds:candidate`. |
| Stack | [`compose/compose.yml`](compose/compose.yml), [`src/stack/`](src/stack/) | One compose project per target: the PDS, a PLC directory with its Postgres, a Sync 1.1 relay in strict mode, a mail catcher and a Caddy edge. |
| Fixtures | [`src/fixtures/`](src/fixtures/) | The outside world, served from the harness process: the AppView and report service the PDS forwards to, OAuth client metadata, a custom-domain handle, and stand-ins for Apple, Google, GitHub and hCaptcha. |
| Scenarios | [`scenarios/<profile>/`](scenarios/), [`src/scenario.ts`](src/scenario.ts) | One `node:test` test per file. Assertions state what the SPEC requires. Every exchange and observation goes into a transcript. |
| Oracles | [`src/oracles/`](src/oracles/), [`src/compare.ts`](src/compare.ts) | O1 response comparison, O2 repository CIDs, O3 firehose and relay, O4 OAuth, O9 verify at pin. |
| Profiles | [`src/stack/env.ts`](src/stack/env.ts) | A profile changes a few environment variables for the scenarios that need them. |

### The stack

Every target runs under the production hostnames, `pds.linkjar.social` with
`.linkjar.social` handles, behind a TLS edge that uses the production Caddy
snippet [`legacy/staging/Caddyfile.handles`](../legacy/staging/Caddyfile.handles).
The base environment is the production configuration with the outside
services replaced by the stack's own. Because each target has its own stack,
two targets differ in the PDS image and in nothing else, and transcripts need
no hostname rewriting.

The stack's main network is internal: the PDS cannot reach the internet. Names
of outside services resolve to the edge, which forwards them to the fixture
server. Patch 099 fixes the Apple, Google and GitHub endpoints in code, so
those names are among them. No public identity is created and no real
provider is contacted.

A CA generated per run signs the edge certificate. Only the stack's
containers and the harness's own HTTP client trust it; it is never installed
in a system or browser trust store, and the run directory that holds its key
is ignored by git.

Three things in the stack exist because a component has no test switch:

- **The relay refuses a host on a private address**, and has no development
  flag for that check. The edge therefore also sits on a second internal
  network whose only subnet is IPv6 in the documentation prefix
  `2001:db8::/32`, which the relay's check accepts and which is never routed.
  The relay reaches `pds.linkjar.social` there.
- **The PLC directory's in-memory store no longer starts** at the pinned
  commit, so the directory gets a disposable Postgres. Its published image is
  private, so the stack builds it from the pinned commit.
- **The relay image is published for amd64 only.** An arm64 host runs it under
  emulation.


### The restore run

`pnpm parity restore` holds the Candidate to the Reference on a data
directory that the Reference wrote ([`src/restore.ts`](src/restore.ts),
SPEC §8.2, §17.1 and §18.2). It uses one stack, named `candidate`, and two servers:

1. The reference image starts on a fresh stack. The scenarios of
   `scenarios/restore/` whose names start with `0` write: six accounts in
   every status a read can meet, a repository of 700 records, records that
   were deleted and replaced, blobs, and a record and a blob taken down.
   What they wrote goes to `.run/candidate/restore-seed.json`.
2. The other scenarios of the profile only read. They run against the
   Reference; the transcripts go to `transcripts/restore-reference/`.
3. The PDS container is replaced by the Candidate image
   (`PARITY_CANDIDATE_IMAGE`, default `linkjar-pds:candidate`) on the same
   volume. Nothing else in the stack changes.
4. The same scenarios run against the Candidate; the transcripts go to
   `transcripts/restore-candidate/`.
5. `linkjar-pds verify --json` runs in the Candidate's container. It must
   report zero differences; the report is saved as `verify.json`.
6. The two groups of transcripts are compared. No difference is allowed.

Because both servers read the same bytes, a read scenario records each body
twice: normalised, and as the SHA-256 of its bytes
([`src/lib/reads.ts`](src/lib/reads.ts)). A repository export must
therefore be the Reference's export byte for byte.

The stack's health check depends on the image that runs: Node in the
reference image, the binary's own `health` command in the Candidate.
`--keep` leaves the stack up with the Candidate serving. `--perf` runs the
[read measurements](#read-measurements) instead of the read scenarios. The
command refuses `--update`: a restore run allows no difference.

### The writes run

`pnpm parity writes` holds the Candidate to the Reference on writes and the
firehose ([`src/writes.ts`](src/writes.ts), SPEC §17.1). The Candidate
cannot create an account before unit 5, so every pass starts on the
Reference. A pass is one fresh stack, named `candidate`:

1. The reference image starts and `scenarios/writes/00-pool` creates 24
   accounts. They go to `.run/candidate/account-pool.json`.
2. In the Candidate's pass, the PDS container is replaced by the Candidate
   image on the same volume.
3. The scenarios of the stage run with `PARITY_ACCOUNT_POOL=1`: a call to
   `s.createAccount` takes the next account of the pool and records no
   exchange ([`src/lib/pool.ts`](src/lib/pool.ts)). Both passes take the
   accounts in the same order, so their transcripts name the same handles.

| Stage | Profile | Scenarios |
|---|---|---|
| `records` | `writes`: the production configuration | `default/03-repo-writes`, `04-repo-reads`, `05-blobs`, `15-import`, and `writes/20-firehose`, `21-relay`, `22-elsewhere` |
| `cursor-limits` | `writes-firehose`: a five-second backfill window and a buffer of five events | `firehose/01-cursor-limits` |
| `crash` | `writes-crash`: dev mode, and the fail point `commit.stored` armed to abort once | `writes-crash/30-crash`, on the Candidate only |

Each of the first two stages runs one pass on the Reference and one on the
Candidate. The transcripts go to `transcripts/writes-reference/` and
`transcripts/writes-candidate/`, and the two groups are compared. The
differences that may show are in
[differences/reference-vs-candidate.json](differences/reference-vs-candidate.json):
the deliberate differences of SPEC §2.3. `--update` rewrites that file;
review the result like code. After every pass on the Candidate,
`linkjar-pds verify --json` must report zero differences.

`writes/20-firehose` and `21-relay` are `default/06` and `07` without the
account events, which need the endpoints of unit 5. `22-elsewhere` records
what each server answers to data from elsewhere: a tree that is not a
Merkle search tree, a record block that is not canonical, a commit that
another key signed. It signs with the account's own key, read from its
actor store, unless the case is about the signature.

The crash stage has no pass on the Reference. The Candidate stops after it
stored a commit and before it appended the event. The scenario restarts
it and requires `#sync` for the head, no `#commit` for that revision, a
next write that succeeds, and a relay that holds the new head (SPEC §7.4).

`--only=<stage>` runs one stage. `--keep` leaves the last stack up.
`--perf` runs the [write measurements](#write-measurements) instead.

### Scenarios and transcripts

A scenario gets a `Scenario` object. `s.http`, `s.query` and `s.procedure`
send a request and record the normalised exchange; `s.note` records an
observation that is not an exchange; `s.exact` records a value that must be
byte-identical on every target. Handles and client addresses are derived from
the scenario name, so scenarios do not collide and both targets see the same
names.

The normaliser ([`src/lib/normalise.ts`](src/lib/normalise.ts)) replaces what
a server generates by an alias numbered by first sight: the first DID a
scenario sees is `did:plc:<1>` on every target. Times become a marker. A JWT
becomes its comparable claims and its lifetime. Only the response headers
that SPEC §4.2, §4.3, §4.4 and §6.4 name are compared.

Scenarios run one file at a time, in name order, so that both targets see the
same sequence of writes.

### Oracles

- **O1 response comparison.** `pnpm parity compare <left> <right>` compares
  the transcripts of two targets and checks the result against
  `differences/<left>-vs-<right>.json`. No file means no difference is
  allowed. `--update` rewrites the file; a change to it is reviewed like code.
- **O2 repository CIDs.** [`src/oracles/repo.ts`](src/oracles/repo.ts) exports
  the repository, verifies the commit signature against the key in the PLC
  document, rebuilds the MST from the records with `@atproto/repo`, and
  compares the root with the signed one. The root over fixed keys and content
  is recorded unaliased, so two targets must produce the same bytes. The
  commit CID covers the DID, the `rev` and the signature, so it differs
  between targets until a Candidate takes its `rev` and key from a Reference
  data directory (SPEC §3.3, unit 4).
- **O3 firehose.** [`src/oracles/firehose.ts`](src/oracles/firehose.ts) checks
  each commit frame: the frame limit, the CAR slice, the signature, `since`
  and `prevData` against the previous commit, and the `prev` of each
  operation. `default/06-firehose` and `firehose/01-cursor-limits` cover every
  row of SPEC §7.3 and the sequences of §10.4. `default/07-relay` requires
  the strict relay to re-emit every commit. The crash injection of §7.4 runs
  against a Candidate only and is not in this unit.
- **O4 OAuth.** `default/20-oauth-flow`, `21-oauth-negative` and
  `22-oauth-clients` use `@atproto/oauth-client-node` and Chromium for the
  flows, and a hand-written DPoP client
  ([`src/lib/oauth-raw.ts`](src/lib/oauth-raw.ts)) for the cases the official
  client will not produce.
- **O9 verify at pin.** [verify-at-pin.md](verify-at-pin.md) names the
  scenario for each item.

### The page contract

HTML is outside the comparison (SPEC C6): the Candidate renders its own
pages. The page driver ([`src/oracles/oauth.ts`](src/oracles/oauth.ts),
[`src/oracles/linkjar.ts`](src/oracles/linkjar.ts)) finds controls by name and
records what a page showed as facts. A target's pages must therefore keep
these names:

| Step | The driver needs |
|---|---|
| Welcome | Buttons "Sign in" and "Create a new account", beside the text "Please authenticate to continue". |
| Sign in | `input[name=username]`, a password field, `input[name=remember]`, a submit button. In the LinkJar journey the form sits behind "Or use email". |
| Sign up | `input[name=email]`, a password field, `input[name=inviteCode]` when a code is required, then `input[name=handle]`, each with a submit button. |
| Providers | "Continue with Apple", "Continue with Google", "Continue with GitHub". |
| Consent | Buttons "Authorize" and "Deny access". |
| Account, sign-in methods | A link to the account's `manage` page, a section "Sign-in methods", controls "Link <Provider>" and "Unlink <Provider>". The unlink control of the last method is disabled. |
| Errors | An element with the `alert` role. |

## Profiles

| Profile | Changes | Scenarios |
|---|---|---|
| `default` | Nothing: the production configuration. | `01` to `15` protocol surface, `20` to `22` OAuth, `30` to `33` LinkJar journeys. |
| `invites` | `PDS_INVITE_REQUIRED=true` | Invite administration; the invitation hand-off of patch 103. |
| `firehose` | A five-second backfill window and a subscriber buffer of five events. | `OutdatedCursor` and `ConsumerTooSlow`. |
| `captcha` | The three hCaptcha variables. | The signup policy of patch 099c. |
| `perf` | Rate limits off, no crawler. | SPEC §16.1, S1 to S5, S8 and S9. See below. |
| `restore` | Nothing: the production configuration. | The seed and the read scenarios of the [restore run](#the-restore-run). Run through `pnpm parity restore`, not `run`. |
| `restore-perf` | Nothing: the production configuration. | The read measurements: one repository of 100,000 records, and the same reads timed on both servers. Run through `pnpm parity restore --perf`. See below. |
| `writes`, `writes-firehose`, `writes-crash`, `writes-perf` | See [the writes run](#the-writes-run). | Run through `pnpm parity writes`, not `run`. |

## Performance scenarios

`pnpm parity run reference --profile perf` runs S1 to S5, S8 and S9 of SPEC
§16.1 and writes the measurements to `.run/reference/perf/`;
`pnpm parity perf report reference` prints them as the table in
[results/perf-reference.md](results/perf-reference.md). The run seeds a
repository of 100,000 records and 100,000 firehose events, so it takes tens
of minutes and is not part of the pull-request gate.
`PARITY_PERF_SCALE=smoke` runs the same code at a fraction of the size to
check the harness; only a full run is a baseline.

S6 (10,000 idle accounts) and S7 (the token endpoint under load) are account
and OAuth paths. They are measured when the Candidate has those paths, in
units 5 and 8. The sequencer fsync count of S9 cannot be read from outside
the Reference; the Candidate reports its own counter.

### Read measurements

`pnpm parity restore --perf` runs the [restore run](#the-restore-run) with
the profile `restore-perf` in place of `restore`. The Reference writes one
repository of `PARITY_RESTORE_RECORDS` records, 100,000 unless the variable
is set. Each server is started fresh before its requests. The read scenario
times five exports with `sync.getRepo` and 200 requests to each of five
point reads, one request at a time. `verify` and the comparison run as in
every restore run, so both servers must return the same export, byte for
byte. When the run passes, it prints the table and writes it to
[results/perf-reads.md](results/perf-reads.md). The table is one run on one
host, not a benchmark and not a run against the targets of SPEC §16.2.

### Write measurements

`pnpm parity writes --perf` runs `perf/01-repository`, SPEC §16.1 S1 to S3,
on each server with the profile `writes-perf`: rate limits off and no
crawler. The Reference creates the account; in the Candidate's pass the
Candidate then takes over the data directory and does every write. When
the run passes, it prints the table and writes it to
[results/perf-writes.md](results/perf-writes.md). The table is one run on
one host, not a benchmark and not a run against the targets of SPEC §16.2.

## The port of `legacy/tests`

The legacy suites import the server's compiled modules inside the container.
Each assertion that HTTP can observe is now a scenario against a running
server. The legacy suites stay in place and keep guarding the rest until the
`linkjar` extension crate has its own tests (unit 6).

| Legacy suite | Scenario | Not observable over HTTP, still guarded by the legacy suite |
|---|---|---|
| `handles.mjs` | `default/30-linkjar-external` (derivation, collision suffix), `default/31-linkjar-handles` (reservations, the hosted rename, handle hosts) | The explicit-slur filter on derived handles; the bound of 32 allocation attempts; the 30-day boundary and the same-target retry after an ambiguous PLC write; two renames racing; removal of the policy row on deletion. |
| `handle-proxy-backend.mjs`, `handle-proxy-probes.mjs` | `default/31-linkjar-handles`, through the stack's edge with the production snippet | The mail-event route of the recovery service, which the stack does not run. |
| `external-accounts.mjs` | `default/30-linkjar-external` | Rollback of the account, identity and policy rows together; the flow store bounds (1,000 flows, three per device, ten minutes); the per-address limits on starts and completions. |
| `external-ui.mjs`, `external-ui-server.mjs` | `default/30-linkjar-external`, `default/32-linkjar-signup-journey`, against the real pages of the running server | The fixture server itself, which the running server replaces. |
| `signup-receipt.mjs` | `default/30-linkjar-external`, `default/32-linkjar-signup-journey` | One account per authorization under concurrent sign-ups; removal of the receipt on revocation and on code replay. |
| `signup-policy.mjs` | `captcha/01-signup-policy` | Nothing. |
| `signin-methods.mjs`, `signin-methods-ui.mjs` | `default/33-linkjar-signin-methods` | Last-method protection under concurrent unlinks; the retry schedule of the notice outbox and its limit of 100 a minute; a stable `Message-ID` across retries. |
| `mail-templates.mjs` | `default/10-email`, `default/11-identity`, `default/33-linkjar-signin-methods`: all six mails as sent, with their text part | Escaping of markup in template inputs, which no request can supply; the HTML layout. |
| `provider-branding.mjs` | `default/20-oauth-flow` (client display), `default/32-linkjar-signup-journey` (the order of the steps) | Computed colours, the font, the logo and the policy links of the page. |
| `patched-runtime.mjs` | Not needed: no module is loaded into the container. | |

The Python suites (`recovery.py`, `recovery-mail.py`) test the backup tooling
and are not part of this port.

## Limits

- **Time.** The Reference has no clock hook. A scenario that depends on a
  lifetime asserts the lifetime the server reports and then ages the stored
  row through a SQL tool container ([`compose/sqlite-tool.cjs`](compose/sqlite-tool.cjs)),
  or mints a legacy token with the stack's own `PDS_JWT_SECRET`. Schema
  version 1 is the Reference schema, so the same statements work on a
  Candidate's data directory.
- **Commit CIDs** are not compared across targets yet (see O2).
- **Migration (O5), stock equivalence as a gate (O6) and the Bluesky app
  (O8)** belong to later units.
- **One address family for the relay.** The relay sees the PDS over IPv6
  only. Everything else in the stack is IPv4.

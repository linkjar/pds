# What the parity harness is built on

Research record, 2026-10-09. Companion to [SPEC.md](../SPEC.md) §17 and the
[harness README](../../parity/README.md). It records what was learned about
the three outside components the harness runs beside the PDS (a relay, a PLC
directory, an OAuth client), with the source of each fact, and the choices
that follow.

**Method.** Exa web search for discovery. Source files read from GitHub at a
named commit through the GitHub API. The pinned reference source read in a
sparse checkout of `bluesky-social/atproto` at
`7ca16cc6989f8247637615aca17c5abb911b8fb1`. Each "observed" statement was
produced by running the stack on 2026-10-09 with Docker Engine 29.8.2 and
Compose v5.5.1 on an arm64 host; the scenario or file that shows it is named.

## 1. The relay

**The Sync 1.1 relay is `cmd/relay` in indigo.** Bluesky's announcement
describes it as a fork and refactor of `bigsky` that is non-archival,
supports `#sync`, and validates `#commit` messages with MST inversion,
"controlled by 'lenient mode' flag". It also says the relay "can be running
locally (eg, using sqlite) and pointed at a local PDS instance".
Source: [Relay Updates for Sync v1.1](https://docs.bsky.app/blog/relay-sync-updates), 2025-05-02.

**Strict is the default.** The flag `--lenient-sync-validation`
(`RELAY_LENIENT_SYNC_VALIDATION`) reads "when messages fail atproto 'Sync
1.1' validation, just log, don't drop". Unset, a failing message is dropped.
The harness leaves it unset, so a commit that reaches the relay's own
firehose was accepted.
Source: [`cmd/relay/main.go`](https://github.com/bluesky-social/indigo/blob/5568a3799fb62b3ac92a417a03ce8f9ab8c89395/cmd/relay/main.go), flags at lines 135 to 143.

**Configuration the harness uses.** `RELAY_ADMIN_PASSWORD`, `DATABASE_URL`
(SQLite is supported "for testing"), `RELAY_PERSIST_DIR`, `RELAY_PLC_HOST`,
`RELAY_API_BIND`. Go reads the CA bundle from `SSL_CERT_FILE`.
Source: [`cmd/relay/README.md`](https://github.com/bluesky-social/indigo/blob/5568a3799fb62b3ac92a417a03ce8f9ab8c89395/cmd/relay/README.md) lines 17, 46 and 100 to 105; `main.go` lines 53 to 97.

**A published image exists, for amd64.** The workflow
`container-relay-ghcr.yaml` pushes `ghcr.io/bluesky-social/indigo:relay-<commit>`
on every push. The tag for commit `5568a3799fb6…` resolves; its manifest
list has no arm64 entry (observed: Compose reports "no matching manifest for
linux/arm64/v8"). The harness pins that image by digest and runs it under
emulation on arm64 hosts.
Source: [`.github/workflows/container-relay-ghcr.yaml`](https://github.com/bluesky-social/indigo/blob/5568a3799fb62b3ac92a417a03ce8f9ab8c89395/.github/workflows/container-relay-ghcr.yaml).

**A bare hostname means TLS.** `ParseHostname` turns a hostname without a
scheme into `https://`, except `localhost:<port>`, and refuses a port on any
other name. The PDS sends its bare `PDS_HOSTNAME` in `requestCrawl`, so the
relay connects to `wss://pds.linkjar.social`. This is why every stack has a
TLS edge.
Source: [`cmd/relay/relay/host.go`](https://github.com/bluesky-social/indigo/blob/5568a3799fb62b3ac92a417a03ce8f9ab8c89395/cmd/relay/relay/host.go) lines 143 to 180; `packages/pds/src/crawlers.ts` at the atproto pin.

**The relay refuses a host on a reserved address and has no switch for
it.** The host check and the identity directory use
`ssrf.PublicOnlyTransport()`. The list of refused IPv4 networks covers the
private ranges, CGNAT, loopback, link-local, the three documentation
networks and the benchmarking range. For IPv6 the check accepts global
unicast `2000::/3` except 6to4 `2002::/16`. The IPv6 documentation prefix
`2001:db8::/32` ([RFC 3849](https://www.rfc-editor.org/rfc/rfc3849)) is
inside `2000::/3` and is not excluded.
Observed: on a private IPv4 network `requestCrawl` answers 400, "unsafe
network address: 172.18.0.3 is not a public IP address". With the edge on an
IPv6-only internal network in `2001:db8::/32`, the relay subscribes.
Source: [`util/ssrf/ssrf.go`](https://github.com/bluesky-social/indigo/blob/5568a3799fb62b3ac92a417a03ce8f9ab8c89395/util/ssrf/ssrf.go) lines 36 to 92; [`cmd/relay/relay/host_checker.go`](https://github.com/bluesky-social/indigo/blob/5568a3799fb62b3ac92a417a03ce8f9ab8c89395/cmd/relay/relay/host_checker.go) line 38; `main.go` lines 333 to 336.

**Consequence.** If a later relay pin excludes the documentation prefix, the
harness needs another address for the edge. The pin is in
[`parity/images.json`](../../parity/images.json).

## 2. The PLC directory

**The server is `@did-plc/server` in `did-method-plc`.** Its `bin.ts` takes
`DATABASE_URL`, runs the migrations and starts a sequencer leader; without
the variable it uses `Database.mock()`.
Source: [`packages/server/src/bin.ts`](https://github.com/did-method-plc/did-method-plc/blob/9c8ea2fe23b89a5c1011246cbb4957dad9dbf7db/packages/server/src/bin.ts).

**The in-memory store does not start at the pinned commit.** Observed:
without `DATABASE_URL` the process exits with "Cannot read properties of
undefined (reading 'selectFrom')" in `Sequencer.curr`. The harness gives the
directory a disposable Postgres.

**The image's default entry point is the production one.** It runs
`service/index.js`, which reads `DB_CREDS_JSON`. The harness overrides the
command to run `dist/bin.js`.
Source: [`packages/server/Dockerfile`](https://github.com/did-method-plc/did-method-plc/blob/9c8ea2fe23b89a5c1011246cbb4957dad9dbf7db/packages/server/Dockerfile); [`packages/server/service/index.js`](https://github.com/did-method-plc/did-method-plc/blob/9c8ea2fe23b89a5c1011246cbb4957dad9dbf7db/packages/server/service/index.js).

**The published image is not public.** The workflow pushes
`ghcr.io/did-method-plc/did-method-plc:plc-<commit>`, and the registry
refuses an anonymous pull token for that repository (observed: 401). The
harness builds the image from the pinned commit with Compose's git build
context.
Source: [`.github/workflows/build-and-push-ghcr.yaml`](https://github.com/did-method-plc/did-method-plc/blob/9c8ea2fe23b89a5c1011246cbb4957dad9dbf7db/.github/workflows/build-and-push-ghcr.yaml).

## 3. The OAuth client and its hostnames

**The SDK at the pin.** `@atproto/api` 0.20.44, `@atproto/repo` 0.10.14,
`@atproto/xrpc` 0.8.13, `@atproto/oauth-client-node` 0.5.7, read from the
`package.json` of each package at the pinned commit. The SDK's own
dependencies are held to the versions of the same commit by overrides in
[`parity/pnpm-workspace.yaml`](../../parity/pnpm-workspace.yaml).

**A client may not live under a local top-level domain.** Observed: a pushed
request from `https://client.test/...` answers `invalid_client_id`, "The
client_id's TLD must not be a local hostname" (`default/21-oauth-negative`
keeps a case for it).

**The server's fetcher refuses the documentation domains.** Observed: client
metadata at `https://client.example.com/...` answers
`invalid_client_metadata`, "Forbidden hostname". The untrusted client of the
harness therefore has a name under `linkjar.io`.

**The server caches client metadata by `client_id`, in memory.** Observed: a
second scenario that published a different document under the same trusted
`client_id` got "Invalid redirect_uri", and a confidential client whose key
was replaced kept refreshing until the PDS restarted
(`default/22-oauth-clients`). Each trusted client of the harness therefore
publishes one fixed document.

## 4. The Reference in a stack

**Patch 093 fixes the handle domain.** `LINKJAR_HANDLE_DOMAIN` is
`'.linkjar.social'` in the patch, and the reservation and the handle-host
route test for that value. A stack that wants to exercise the patch has to
run as `pds.linkjar.social`. The harness does, for every target, inside a
network that cannot reach the internet.
Source: [`legacy/patches/093-handle-policy.patch`](../../legacy/patches/093-handle-policy.patch) lines 224, 367 and 449.

**Patch 099 fixes the provider endpoints.** The authorization, token and key
URLs of Apple, Google and GitHub are constants. The stack resolves those
names to its edge and trusts the stack's CA through `NODE_EXTRA_CA_CERTS`,
so the unmodified image talks to stand-ins.
Source: [`legacy/patches/099-external-providers.patch`](../../legacy/patches/099-external-providers.patch) lines 1417 to 1438.

**The notice outbox is drained once a minute.** The dispatcher of patch 100
runs on `setInterval(tick, 60_000)`. A scenario that reads a sign-in notice
waits up to two and a half minutes for it.
Source: [`legacy/patches/100-signin-methods.patch`](../../legacy/patches/100-signin-methods.patch) line 1751.

**The Reference feeds live firehose subscribers from a poll.** `pollDb`
backs off exponentially "with a max of a second wait" when it finds nothing.
A scenario that subscribes in mid-flight waits for a quiet stream first.
Source: `packages/pds/src/sequencer/sequencer.ts` lines 156 to 162 at the pin.

**A 502 from the Reference can be the PLC directory's.** The server maps a
PLC client error with a status of 500 or more to `UpstreamFailure`, which is
502. Observed once: with three stacks running at the same time on one host,
`updateHandle` answered 502. The scenarios of one target now run alone.
Source: `packages/pds/src/index.ts` lines 165 to 183 at the pin.

**Under a thousand subscribers the Reference falls behind.** Observed in
three full-scale runs of S4 on one host. With 1 and 10 subscribers a writer
at 50 commits a second sees a median delivery lag of about 11 ms and a 99th
percentile near half a second, which is the idle poll. With 100 the median
was between 12 and 41 ms. With 1,000 the median lag and the median commit
time were both between 11 and 14 seconds: the fan-out and the write path
share one thread. Every frame was delivered in the end, and the write
throughput of fifty writers was back to its earlier value once the
subscribers had left.
Source: [`parity/results/perf-reference.md`](../../parity/results/perf-reference.md).

**One run suggests that a large repository in the same process disturbs
writes to other repositories.** Observed once: S9 ran in the process that
had just built and exported a repository of 100,000 records, and 420 of
3,000 writes across fifty other actors came back from the edge as 502, with
a median commit time near four seconds. After a restart of the server the same scenario
answered all 3,000 with a median near 165 ms, twice. The cause was not
established. The performance scenarios now start each file from a fresh
process, and the first observation is recorded here and not in the baseline.

## 5. What this means for the Candidate

- The Candidate image needs nothing harness-specific. It must honour
  `SSL_CERT_FILE` or an equivalent for an added CA, and it must be able to
  run with outbound requests to private addresses allowed
  (`PDS_DISABLE_SSRF_PROTECTION`), which SPEC §15.1 ties to `PDS_DEV_MODE`.
  The Reference accepts the variable on its own. Unit 3 has to decide how
  the Candidate runs in the stack: honour the variable without dev mode when
  a test build is used, or run with dev mode on.
- The Reference sets its keep-alive timeout to 90 seconds
  (`packages/pds/src/index.ts` line 220 at the pin), below Caddy's default
  of two minutes for upstream connections. The Candidate should set its own
  above the proxy's, or the runbook the proxy's below it.
- The SQL tool of the harness reads and ages rows in the data directory.
  It works on a Candidate as long as schema version 1 is the Reference
  schema (SPEC §8.2).

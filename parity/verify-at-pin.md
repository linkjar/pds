# Verify at pin

Items that the public documentation leaves open and the SPEC resolves by
reading the Reference at the pin (`@atproto/pds@0.5.34`, commit
`7ca16cc6989f8247637615aca17c5abb911b8fb1`). Unit 0 read VP-1 to VP-7. Unit 1
read VP-8 to VP-12 in a sparse checkout of that commit and confirmed all
twelve with a scenario against the reference image. This is oracle O9 of
SPEC §17.3; SPEC revision 2 carries the results.

Paths are relative to the atproto repository at the pin. Scenario names are
files under [`scenarios/`](scenarios/).

| Id | Question (SPEC §) | Answer at the pin | Source | Scenario |
|---|---|---|---|---|
| VP-1 | Is firehose backfill exclusive or inclusive of `cursor`? (§7.3) | Exclusive: the first frame is `cursor + 1`. | `packages/pds/src/sequencer/sequencer.ts` lines 82 and 114 (`seq > cursor`) | `default/06-firehose` |
| VP-2 | Does the outbound proxy token carry the service fragment in `aud`? (§4.4) | No. `aud` is the bare DID, for the default AppView and for an explicit `atproto-proxy` target. | `packages/pds/src/pipethrough.ts` lines 108 to 113 | `default/08-proxy` |
| VP-3 | Is `lxm` required on inbound service auth? (§5.4) | Yes. `createAccount` answers 401 `BadJwtLexiconMethod`, "missing jwt lexicon method". `uploadBlob` tells a service token from a session token by the presence of `lxm`, so there a method-less token fails as a session token: 400 `InvalidToken`. | `packages/xrpc-server/src/auth.ts` lines 75 and 119 to 123 | `default/08-proxy` |
| VP-4 | Service-auth token lifetime (§5.4) | 60 seconds by default, for proxied calls and for `getServiceAuth`. A caller may ask `getServiceAuth` for up to one hour with `lxm`, and up to one minute without. | `packages/xrpc-server/src/auth.ts` line 33; `packages/pds/src/api/com/atproto/server/getServiceAuth.ts` lines 68 to 80 | `default/08-proxy` |
| VP-5 | Are `getHead` and `getCheckout` still served? (Appendix A) | Yes, both answer 200. | `packages/pds/src/api/com/atproto/sync/index.ts` lines 25 and 26 | `default/04-repo-reads` |
| VP-6 | Is `com.atproto.lexicon.resolveLexicon` served? (§5.5) | No handler. The catch-all proxy takes it: 401 `AuthMissing` without a session, forwarded to the AppView with one. It never answers 501. | `packages/pds/src/index.ts` line 164; `packages/pds/src/pipethrough.ts` line 687 | `default/01-server`, `default/08-proxy` |
| VP-7 | Is `com.atproto.sync.listReposByCollection` served? (Appendix A) | No handler; the same as VP-6. | as VP-6 | `default/01-server`, `default/08-proxy` |
| VP-8 | PAR request lifetime (§11.3) | Five minutes: `expires_in` is 300. Opening the authorization page extends the request by five minutes of inactivity. An expired request redirects to the client with `error=access_denied` and `error_description=This request has expired`. | `packages/oauth/oauth-provider/src/oauth-constants.ts` lines 39 and 60; `request/request-manager.ts` lines 72 and 347 to 352 | `default/21-oauth-negative` |
| VP-9 | Legacy access and refresh lifetimes (§5.1) | Access 120 minutes, refresh 90 days. A rotated refresh token stays valid for a grace period of two hours and leads to the same successor. | `packages/pds/src/account-manager/helpers/auth.ts` lines 46 and 72; `account-manager.ts` line 537 | `default/02-sessions` |
| VP-10 | Blob garbage-collection grace period (§6.4) | None. The commit that removes the last reference deletes the blob row in the same transaction and the bytes from a background queue right after it. The blob stops being served within seconds. An upload that no record ever references is never collected. | `packages/pds/src/actor-store/blob/transactor.ts` lines 147 and 219 to 275 | `default/05-blobs` |
| VP-11 | CORS headers on XRPC responses (§4.2) | The `cors` middleware with `maxAge` of one day: `Access-Control-Allow-Origin: *` on every response; a preflight answers 204 with `Access-Control-Allow-Methods: GET,HEAD,PUT,PATCH,POST,DELETE`, the requested headers echoed, and `Access-Control-Max-Age: 86400`. The OAuth routes are mounted before it and set their own: origin `*`, methods `*`, headers `Content-Type,DPoP`, max age 86400, and a preflight status of 200. | `packages/pds/src/index.ts` lines 203 and 204; `packages/oauth/oauth-provider/src/router/create-oauth-middleware.ts` lines 28 to 50 | `default/01-server` |
| VP-12 | Per-route rate limiters (§4.3) | The table below. | `packages/pds/src/rate-limits.ts`; the `rateLimit` option of each handler under `packages/pds/src/api/` | `default/09-rate-limits` |

## VP-12: the limiters

Every limiter was read off the response headers of one request and matches
the source. A response reports the limiter with the fewest points left, as
`RateLimit-Limit`, `RateLimit-Remaining`, `RateLimit-Reset` and
`RateLimit-Policy: <limit>;w=<seconds>`. A 429 adds `Retry-After`. The
bypass key removes the limiters and their headers.

| Limiter | Window | Points | Keyed by |
|---|---|---|---|
| `global-ip` (every route except `sync.getRepo`) | 5 minutes | 3,000 | client address |
| `repo-write-hour` (`createRecord` 3, `putRecord` 2, `deleteRecord` 1, `applyWrites` the sum) | 1 hour | 5,000 | DID |
| `repo-write-day` (same weights) | 24 hours | 35,000 | DID |
| `sync.getRepo` | 5 minutes | 6,000 | client address |
| `repo.uploadBlob` | 24 hours | 1,000 | client address |
| `server.createAccount` | 5 minutes | 100 | client address |
| `server.createSession` | 5 minutes, and 24 hours | 30, and 300 | identifier and client address together |
| `server.resetPassword` | 5 minutes | 50 | client address |
| `server.requestPasswordReset` | 1 hour, and 24 hours | 15, and 50 | client address |
| `server.deleteAccount` | 5 minutes | 50 | client address |
| `server.requestEmailConfirmation`, `server.requestEmailUpdate`, `server.requestAccountDelete` | 1 hour, and 24 hours | 5, and 15 | DID |
| `identity.updateHandle` | 5 minutes, and 24 hours | 10, and 50 | DID |

## What the scenarios corrected

Running the scenarios against the reference image showed where SPEC revision
1 described the protocol documents and not the Reference. Revision 2 states
the Reference's behaviour in each place; C1 holds the Candidate to it.

| SPEC § | Revision 1 said | The Reference at the pin | Scenario |
|---|---|---|---|
| 4.2, Appendix A | A method with no handler answers 501 `MethodNotImplemented`; the relay-only methods answer 501. | Every method without a handler falls to the catch-all proxy: 401 `AuthMissing` without a session, forwarded to the AppView with one. | `default/01-server`, `default/08-proxy` |
| 4.2 | Read-after-write responses carry `Atproto-Repo-Rev`. | An overlaid response carries `Atproto-Upstream-Lag`, in milliseconds, and drops the AppView's `Atproto-Repo-Rev`. A response that needed no overlay passes the AppView's header through. | `default/14-appview-local` |
| 4.4 | The proxy refuses a deactivated account. | A deactivated account still proxies, mints service tokens, reads its preferences, exports its repository and uploads blobs. Writes are refused. A taken-down account's sessions end. | `default/12-lifecycle`, `default/13-admin` |
| 4.4 | A malformed `atproto-accept-labelers` is an `InvalidRequest`. | The header is forwarded as it is. | `default/08-proxy` |
| 5.4 | Inbound `jti` is checked against a replay cache. | No replay check: the same service token is accepted twice. | `default/08-proxy` |
| 6.3 | A write that the server refuses for content is a 400. | `createRecord` on a key that exists, and a batch with such an operation, answer 500 `InternalServerError`. | `default/03-repo-writes` |
| 6.4 | A deleted blob goes after a grace period of at least one hour. | VP-10: no grace period. | `default/05-blobs` |
| 6.4 | A missing blob is `BlobNotFound`. | `getBlob` answers 400 `InvalidRequest`, "Blob not found". `BlobNotFound` is the error of a record that references a blob the account does not hold. | `default/05-blobs` |
| 7.2 | A non-GET request answers 405 and a request without `Upgrade` answers 426. | `subscribeRepos` has no HTTP route; both fall to the catch-all proxy and answer 401 `AuthMissing`. | `default/06-firehose` |
| 7.3 | `cursor=0` returns the whole retained window. | It does, and when older events exist it sends `#info` `OutdatedCursor` first, like any cursor older than the window. | `firehose/01-cursor-limits` |
| 7.4, 16.1 | (nothing) | Live subscribers are fed from a database poll that backs off to one second when idle, so an event reaches a quiet stream up to a second after its commit. | `default/13-admin`; `packages/pds/src/sequencer/sequencer.ts` line 160 |
| 7.6 | `requestCrawl` is sent on startup and after the first commit of a new repository. | It is sent with the first sequenced event after a start and then at most once in twenty minutes. Nothing is sent at startup. | `default/07-relay`; `packages/pds/src/crawlers.ts` |
| 10.4 | Account creation emits `#identity`, `#account`, `#sync`, then the first `#commit`. | `#identity`, `#account`, `#commit`, `#sync`. | `default/06-firehose` |
| 10.4 | Reactivation emits `#account`, then an empty `#commit`. | `#account`, `#identity`, `#sync`. No commit. | `default/06-firehose` |
| 11.3 | A pushed request expires with `invalid_request`. | VP-8: `access_denied`, "This request has expired". | `default/21-oauth-negative` |
| 11.3 | Access tokens live under 30 minutes. | 60 minutes (`TOKEN_MAX_AGE`). Appendix D, D14 already defers to the pin. | `default/20-oauth-flow` |
| 13 | Six patches. | Eight at the patch pin: `102-signup-journey` and `103-invite-handoff` were missing. | `default/32-linkjar-signup-journey`, `invites/02-linkjar-invite-handoff` |

Two more defects of the Reference are recorded as observed.
`createAppPassword` with a name in use answers 500 (`default/02-sessions`).
An `atproto-proxy` value that does not start with a DID answers 500, while a
DID that does not resolve answers 400 (`default/08-proxy`).

**Decided on 2026-10-09.** The Candidate does not follow the Reference in
five places: it answers 400 `InvalidRequest` in these two cases and in the
two of the §6.3 row, and it refuses a replayed service token (the §5.4
row). SPEC §2.3 lists them as DD-1 to DD-5, and
[differences/README.md](differences/README.md#the-reference-against-the-candidate)
names the scenario step of each.

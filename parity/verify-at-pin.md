# Verify at pin

Items the public documentation leaves ambiguous and the SPEC resolves by
reading the reference at the pin (`@atproto/pds@0.5.34`, commit
`7ca16cc6989f8247637615aca17c5abb911b8fb1`). Each gets a harness scenario in
unit 1 (ids `VP-n`) so the Candidate is held to the recorded behaviour. The
harness lives in this directory from unit 1.

| Id | Question (SPEC §) | Answer at the pin | Where | Scenario |
|---|---|---|---|---|
| VP-1 | Is firehose backfill exclusive or inclusive of `cursor`? (§7.3) | Exclusive: `where seq > cursor`. | `packages/pds/src/sequencer/sequencer.ts` lines 82 and 114 | subscribe with `cursor=N`, expect first frame `N+1` |
| VP-2 | Does the outbound proxy service-auth `aud` carry the service fragment? (§4.4) | No. `pipethrough.ts` resolves `did#serviceId` for scope checks and "keeps bare-DID aud" on the outbound JWT. The Spring 2026 change lands with a later pin. | `packages/pds/src/pipethrough.ts` lines 109 to 118 | proxy a call, decode the forwarded JWT, expect `aud` = bare DID |
| VP-3 | Is `lxm` required on inbound service auth? (§5.4) | When the server expects a method, a token without `lxm` fails with "missing jwt lexicon method"; the check is skipped only where the PDS passes `null`. | `packages/xrpc-server/src/auth.ts` lines 75 and 119 to 123 | call with a service JWT lacking `lxm`, expect 401 |
| VP-4 | Service-auth token lifetime (§5.4) | `exp` defaults to `iat + 60 s`. | `packages/xrpc-server/src/auth.ts` line 33 | `getServiceAuth` output `exp - iat` = 60 |
| VP-5 | Are `getHead` and `getCheckout` still served? (Appendix A) | Yes, registered from `sync/deprecated/`. | `packages/pds/src/api/com/atproto/sync/index.ts` lines 25 and 26 | call both, expect 200 |
| VP-6 | Is `com.atproto.lexicon.resolveLexicon` served? (§5.5) | No handler under `api/com/atproto/`; proxy or 501. | `packages/pds/src/api/com/atproto/` has no `lexicon/` | call, expect 501 or proxy |
| VP-7 | Is `com.atproto.sync.listReposByCollection` served? (Appendix A) | No handler; 501. | `packages/pds/src/api/com/atproto/sync/` | call, expect 501 |
| VP-8 | PAR request lifetime (§11.3) | Not yet read; the constant lives in `packages/oauth/oauth-provider/src/oauth-constants.ts` and `request/request-manager.ts`. | unit 1 reads it | PAR then wait, expect `invalid_request` after the lifetime |
| VP-9 | Legacy access and refresh lifetimes (§5.1) | Not yet read; `packages/pds/src/auth-verifier.ts` and the account manager. | unit 1 | `createSession` then decode `exp` |
| VP-10 | Blob garbage-collection grace period (§6.4) | Not yet read; actor-store blob code. | unit 1 | upload, wait, `listMissingBlobs` |
| VP-11 | CORS headers on XRPC responses (§4.2) | Not yet read; `packages/pds/src/index.ts` middleware. | unit 1 | OPTIONS and GET with `Origin`, record headers |
| VP-12 | Per-route rate limiters (§4.3) | Enumerated in unit 1 from each handler's `rateLimit` option. | `packages/pds/src/api/**` | one scenario per limiter |

Resolved rows are reflected in SPEC revision 1; the "not yet read" rows are
unit 1 work and become SPEC revision 2.

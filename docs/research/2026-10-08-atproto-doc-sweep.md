# Gap report: SPEC revision 0 against atproto.com

Research record, 2026-10-08. A background agent read every page under
`https://atproto.com/specs/` and `https://atproto.com/guides/`, the OAuth and
permission specifications in full, the data-validation guide, the Sync 1.1
proposal, the February 2026 spec-update pull request, the Spring 2026
roadmap and the Spaces alpha post, then compared them with
[SPEC.md](../SPEC.md) revision 0. Revision 1 of the SPEC applies every item in
sections A to C and tracks section D. Items the spec already covered are
omitted. "Unverified" marks claims the agent could not confirm against the
pinned reference source; unit 0 confirms them.

Pages that do not exist and were not read: `/specs/http-api`,
`/specs/permissions`, `/specs/auth-scopes`, `/specs/sync-1.1`,
`/specs/spaces`, `/specs/changelog`, `/guides/service-auth`,
`/guides/lexicon-publication`. There is no spec changelog page; the GitHub
"Dev Announcements" discussions are the closest thing.

## A. Missing requirements

### Repository, records, data model

1. Commit size limits: `#commit` `blocks` at most 2,000,000 bytes, any single record block at most 1,000,000 bytes, at most 200 operations per commit, every firehose frame at most 5 MB including framing. The `tooBig` escape is deprecated. [sync](https://atproto.com/specs/sync)
2. Producers set `tooBig: false` and `blobs: []`; both deprecated but required. [sync](https://atproto.com/specs/sync)
3. Diff CAR slice content: the new commit as `roots[0]`, every MST node new in this revision, the extra nodes needed for operation inversion, every created or updated record, no deleted record bodies, and required blocks even if they appeared earlier in history. [repository](https://atproto.com/specs/repository)
4. A new commit should be created whenever the signing key rotates; `data` need not change. [repository](https://atproto.com/specs/repository)
5. MST hardening: bound entries per node, bound tree depth, CBOR decode limits for object size, recursion depth and memory. [repository](https://atproto.com/specs/repository), [data-model](https://atproto.com/specs/data-model)
6. Working parser limits: CBOR record 1 MiB, JSON record 2 MiB, nesting depth 32, container elements 131,072, object key 8 KiB, CID binary 100 bytes, integers within plus or minus 9,007,199,254,740,991, CBOR frames parsed at 4 to 5 MB. [data-validation](https://atproto.com/guides/data-validation)
7. Import hygiene on `importRepo`: verify structural completeness, ignore unreferenced and duplicate blocks, tolerate any block order, guard against cross-account contamination. [repository](https://atproto.com/specs/repository)
8. Repo path charset `A-Za-z0-9/._-~`; record key 1 to 512 characters from `A-Za-z0-9._:~-`, `.` and `..` forbidden, `%` reserved. [record-key](https://atproto.com/specs/record-key)
9. TID: 53-bit microseconds plus 10-bit clock id, top bit zero; monotonic and never repeated within one generator; millisecond clocks pad by a thousand. [tid](https://atproto.com/specs/tid)
10. Three validation modes: `validate=true` fails if the lexicon is unknown and unresolvable; `validate=false` applies data-model validation only; unset is optimistic. The success response reports whether validation happened. [lexicon](https://atproto.com/specs/lexicon)
11. Data-model rules enforced regardless of schema: no floats; unknown `$`-prefixed keys ignored; blob `ref` must be `raw`, `size` greater than zero, `mimeType` non-empty; legacy blob form read-only; CIDv1 with dag-cbor or raw and SHA-256 only; datetime with uppercase `T`, a timezone, no `-00:00`, whole seconds. [data-model](https://atproto.com/specs/data-model)

### Blobs

12. Reject an upload whose body length differs from `Content-Length`; expect `Content-Type`; chunked encoding may be allowed. [blob](https://atproto.com/specs/blob)
13. Unreferenced uploads are not downloadable and absent from `listBlobs`. [blob](https://atproto.com/specs/blob)
14. A record referencing a blob the server does not hold is rejected; deleting the last referencing record deletes the blob; account deletion deletes all blobs within a reasonable time; deactivated, taken-down and suspended accounts serve no blobs. [blob](https://atproto.com/specs/blob)
15. `getBlob` returns `Content-Type` and `Content-Length`, a CSP such as `default-src 'none'; sandbox`, and `X-Content-Type-Options: nosniff`. [blob](https://atproto.com/specs/blob)
16. No media resizing or transcoding in the PDS; sniffing is itself an exploit surface. [blob](https://atproto.com/specs/blob)
17. Prefer account-wide quotas to per-blob caps; limits must be a superset of all supported lexicons' blob constraints. [blob](https://atproto.com/specs/blob)

### XRPC, authentication, proxying

18. Legacy JWT `typ`: access `at+jwt`, refresh `refresh+jwt`. [xrpc](https://atproto.com/specs/xrpc)
19. Service auth: honour an optional `kid` header naming the verification-method fragment (default `#atproto`); accept only expected key types; outbound `aud` is `did#service_id`; the pre-2026 `iss#fragment` form is gone. [xrpc](https://atproto.com/specs/xrpc)
20. Service-auth claims: `typ: JWT`, `iat` required, `exp` short with 60 seconds recommended, `jti` replay-checked, `lxm` required for authenticated XRPC requests. [xrpc](https://atproto.com/specs/xrpc)
21. Proxy preconditions: target DID document has a `service` whose id matches the fragment; only `/xrpc/<nsid>` paths; caller has an active account; PDS rate limits apply. [xrpc](https://atproto.com/specs/xrpc)
22. Both `atproto-accept-labelers` and `atproto-content-labelers` pass through the proxy; a malformed accept header is an error. [label](https://atproto.com/specs/label)
23. 401 with `WWW-Authenticate`; 413 for oversized bodies; 429 may carry `Retry-After`; JSON error envelopes everywhere; 501 for known but unimplemented methods. [xrpc](https://atproto.com/specs/xrpc)
24. Basic admin auth also gates `createInviteCode(s)`. [xrpc](https://atproto.com/specs/xrpc)
25. CORS on authorization-server metadata, PAR and token endpoints; XRPC CORS encouraged. [oauth](https://atproto.com/specs/oauth)
26. `Atproto-Repo-Rev` response header on read-after-write responses. [sync](https://atproto.com/specs/sync)

### Firehose and sequencing

27. `cursor=0` means start at the oldest retained event. [event-stream](https://atproto.com/specs/event-stream)
28. Connection-time errors: 405 for non-GET, 426 without `Upgrade`, 429 and 5xx with JSON bodies; client frames ignored; `wss://` on the public listener. [event-stream](https://atproto.com/specs/event-stream)
29. `seq` in 1 to 2^53 exclusive, never reused; after any reset restart with a healthy margin above the previous maximum. [event-stream](https://atproto.com/specs/event-stream)
30. Emit `#sync` after `#account` on account creation, after migration and activation, and after a CAR import. [Sync 1.1 proposal](https://github.com/bluesky-social/proposals/blob/main/0006-sync-iteration/README.md)
31. Lifecycle ordering: creation `#identity`, `#account`, `#commit`; activation after migration `#account` then an empty `#commit` signed with the new key at a higher `rev`; deactivation, deletion and takedown `#account` only; reactivation `#account` then an empty `#commit` if long inactive. [account-lifecycle](https://atproto.com/guides/account-lifecycle)
32. `rev` derives from wall-clock time; consumers drop revisions more than a few minutes in the future. [sync](https://atproto.com/specs/sync)
33. `#identity` `handle` may be `handle.invalid`. [sync](https://atproto.com/specs/sync)

### Identity

34. Disallowed handle TLDs `.alt .arpa .example .internal .invalid .local .localhost .onion`, `.test` only in development; handles effectively at most 244 characters. [handle](https://atproto.com/specs/handle)
35. Resolution: TXT values start with `did=`; several valid TXT records with different DIDs fail; well-known needs 2xx and whitespace-stripped body, HTTPS 443 in production, bounded redirects; DNS wins on conflict; a PDS may refuse writes when the handle no longer verifies. [handle](https://atproto.com/specs/handle)
36. DID document contract: first `at://` in `alsoKnownAs` is the claimed handle; signing key is the first `verificationMethod` ending `#atproto`, type `Multikey`, controller the DID; PDS is the first `service` ending `#atproto_pds`, type `AtprotoPersonalDataServer`, endpoint with scheme, host and port only; accept relative and fully qualified ids; accept the legacy key types during transition. [did](https://atproto.com/specs/did)
37. DID at most 2048 characters, case-sensitive; `did:web` at hostname level only; distinguish invalid syntax, unsupported method and resolution failure. [did](https://atproto.com/specs/did)
38. Low-S signatures for P-256 as well as secp256k1; verify only through library routines. [cryptography](https://atproto.com/specs/cryptography)

### OAuth

39. Metadata: `authorization_response_iss_parameter_supported: true`; `token_endpoint_auth_methods_supported` includes `none` and `private_key_jwt`; signing algorithms include `ES256` and never `none`; `grant_types_supported` includes `refresh_token`; `require_request_uri_registration` not `false`; `issuer` equals the fetch origin without path or default port; protected-resource metadata lists exactly one authorization server. [oauth](https://atproto.com/specs/oauth)
40. Token responses always include `scope` and `sub`. [oauth](https://atproto.com/specs/oauth)
41. Client metadata validation: `client_id` is `https://` without port except the localhost case; fetch returns exactly 200 with `application/json`; `client_uri` shares the hostname; `logo_uri`, `tos_uri`, `policy_uri` https only; `dpop_bound_access_tokens` true; `grant_types` contains `authorization_code`; confidential clients need `private_key_jwt` and exactly one of `jwks` or `jwks_uri`. [oauth](https://atproto.com/specs/oauth)
42. Localhost clients: `http://localhost` with no port and empty path; `redirect_uri` and `scope` query parameters; defaults `http://127.0.0.1/` and `http://[::1]/`; path matched, port ignored; synthesized metadata is a public native client. [oauth](https://atproto.com/specs/oauth)
43. Redirect URIs: web clients https with non-default ports only; native custom schemes are the reversed `client_id` host plus `:/path`; native https redirects share the `client_id` origin. [oauth](https://atproto.com/specs/oauth)
44. Confidential-client sessions are bound to the assertion key's `kid`, `alg` and `jkt`; refreshes use the same key; metadata is re-fetched periodically and the session revoked if the key disappears; assertion `aud` is the issuer, `jti` unique for the validity period, `iat` within the last minute. [oauth](https://atproto.com/specs/oauth)
45. Reject reuse of `code_challenge` for about 24 hours; on `code` reuse revoke every session issued from it; reject duplicate `state`. [oauth](https://atproto.com/specs/oauth)
46. Untrusted clients: do not display `client_name`, `client_uri` or `logo_uri`; always show the full `client_id`; `login_hint` restricts sign-in; silent re-authorization only for confidential or trusted clients. [oauth](https://atproto.com/specs/oauth)
47. Transitional scopes: `transition:generic` grants record writes, blobs, preferences, proxying and service-auth minting but no account management and no `chat.bsky.*`; `transition:chat.bsky` requires `transition:generic`; `transition:email` exposes email on `getSession`; all three slated for deprecation. [oauth](https://atproto.com/specs/oauth)
48. Permission semantics: `rpc` covers proxying and `getServiceAuth`; `account:email` read versus manage; `account:repo?action=manage` gates `importRepo`; `identity:handle` and `identity:*`; `blob:<accept>` gates `uploadBlob` by MIME glob; `aud` and `lxm` not both wildcard; unknown resources inside a set ignored; `include:` sets reference only their own NSID group or children. [permission](https://atproto.com/specs/permission)
49. Permission-set resolution: `_lexicon.<reversed authority>` TXT to a DID, then the `com.atproto.lexicon.schema` record keyed by NSID; no DNS hierarchy walk; cache shared across accounts; fail the authorization request if a set is unresolvable and uncached; recompute permissions on refresh. [permission](https://atproto.com/specs/permission), [lexicon](https://atproto.com/specs/lexicon)

### Operations

50. Do not host the PDS on a subdomain shared with an OAuth client app. [going-to-production](https://atproto.com/guides/going-to-production)
51. The recovery key's private half is the last-resort path and belongs offline; the rotation key belongs in a KMS or HSM. [going-to-production](https://atproto.com/guides/going-to-production)
52. Changing the PDS hostname requires a PLC update per account. [self-hosting](https://atproto.com/guides/self-hosting)
53. `getRepoStatus` known values include `desynchronized` and `throttled`. [account](https://atproto.com/specs/account)

## B. Contradictions

1. Backfill boundary. SPEC §7.3 says exclusive; the event-stream spec says greater-or-equal. The Reference's behaviour at the pin is unverified; the harness pins one and the spec cites the discrepancy. [event-stream](https://atproto.com/specs/event-stream)
2. `getHead` and `getCheckout` will be removed, `sync.getRecord`'s `commit` parameter will be removed, and `getBlocks` becomes optional under Sync 1.1. Appendix A marks them accordingly. [Sync 1.1 proposal](https://github.com/bluesky-social/proposals/blob/main/0006-sync-iteration/README.md)
3. `lxm` on inbound service auth: the XRPC page both says it is required and that it may become required. The spec states the rule it enforces. [xrpc](https://atproto.com/specs/xrpc)
4. Proxying requires an active account, not only an authenticated session. [xrpc](https://atproto.com/specs/xrpc)
5. CSP applies to `getBlob`, not only HTML. [blob](https://atproto.com/specs/blob)

## C. Under-specified in revision 0

| SPEC | Revision 0 said | Docs give |
|---|---|---|
| §4.3 | "as declared on each Reference handler" | blob uploads 1,000 per 24 hours per IP; bypass header `x-ratelimit-bypass` [account-migration](https://atproto.com/guides/account-migration) |
| §5.4 | "the Reference's maximum age" | 60 seconds recommended lifetime [xrpc](https://atproto.com/specs/xrpc) |
| §6.2 | "strictly greater than the previous" | derived from wall-clock time, then checked strictly greater [sync](https://atproto.com/specs/sync) |
| §6.3 | 200 operations | plus 2,000,000-byte `blocks`, 1,000,000-byte record, 5 MB frame [sync](https://atproto.com/specs/sync) |
| §6.4 | "the Reference's grace period" | at least one hour, several recommended [blob](https://atproto.com/specs/blob) |
| §9 | "syntax validation; reserved lists" | TLD blocklist, length limits, `did=` prefix, multi-TXT rule, redirect cap [handle](https://atproto.com/specs/handle) |
| §10 | "follow the Reference" | `deleteAfter`; the old PDS assists PLC recovery for 72 hours unless deleted [account-migration](https://atproto.com/guides/account-migration) |
| §10 | "C5 works" | `getRepo?since=` diff import; idempotent repeated imports; blobs after import is indexed; deactivated-account blobs need that account's auth; `submitPlcOperation` runs safety checks [account-migration](https://atproto.com/guides/account-migration) |
| §11.2 | "the Reference's TTLs" | metadata TTL short enough to reject a removed confidential key promptly; permission sets stale at most 24 hours, at least the access-token lifetime, expiry 90 days [oauth](https://atproto.com/specs/oauth), [permission](https://atproto.com/specs/permission) |
| §11.3 | "as the Reference's" | access tokens under 30 minutes, at most 15 if not individually revocable, 5 recommended; public-client session and refresh at most 2 weeks; confidential refresh at most 180 days [oauth](https://atproto.com/specs/oauth) |
| §11.3 | "rotated with `PDS_DPOP_SECRET`" | nonce lifetime at most 5 minutes, recently stale nonces accepted [oauth](https://atproto.com/specs/oauth) |
| §11.3 | "expire as the Reference's do" | no number published; unverified |
| §11.4 | "Reference's checks" | a session-list page with revocation is required [oauth](https://atproto.com/specs/oauth) |
| §6.3 | two validation modes | three, plus the response flag [lexicon](https://atproto.com/specs/lexicon) |

## D. New or changing in 2026

1. Sync 1.1 folded into the written specs on 2026-02-28; `bsky.network` on the Sync 1.1 relay since January 2026. Remaining: streamable CAR block ordering and partial `getRepo` by collection. The strict-validation ratchet has no published date. [spec update PR](https://github.com/bluesky-social/atproto-website/pull/510/), [Spring 2026 roadmap](https://atproto.com/blog/2026-spring-roadmap)
2. Sync 1.1 deprecations: `prev` removed from commits on the wire, `blobs` empty, `tooBig` false, `#handle`, `#tombstone`, `#migrate` gone, `getHead` and `getCheckout` removed, `getBlocks` optional, new statuses `desynchronized` and `throttled`.
3. Service auth, Spring 2026: `kid` header replaces `iss#fragment`; `aud` carries the service fragment; the reference pipethrough is being updated. Affects parity when the pin moves. [xrpc](https://atproto.com/specs/xrpc)
4. Permissions and permission sets shipped. Whether the reference at 0.5.34 serves `com.atproto.lexicon.resolveLexicon` is unverified. [permission](https://atproto.com/specs/permission)
5. Transitional scopes to be deprecated and removed.
6. Account management moving into the reference PDS: email and password change, deactivate and delete, data export, extra second factors. [Spring 2026 roadmap](https://atproto.com/blog/2026-spring-roadmap)
7. Atproto Spaces alpha, 2026-08-20: per-space permissioned repositories, `com.atproto.space.*`, `getDelegationToken`, `space:` scopes, `#atproto_space` keys and `#atproto_space_host` services, DPoP-bound space credentials, direct PDS-to-app sync. Schemas unstable. [Spaces alpha](https://atproto.com/blog/atproto-spaces-alpha), [proposal 0016](https://github.com/bluesky-social/proposals/blob/main/0016-permissioned-data/README.md)
8. IETF ATP working group approved March 2026; `draft-holmgren-at-repository` is the repository and sync draft.
9. PLC: WebSocket streaming of updates and a replica reference implementation, early 2026.
10. `internal.*` NSIDs on the reserved `.internal` TLD are the convention for operator-private XRPC methods. [installing-lexicons](https://atproto.com/guides/installing-lexicons)
11. A protocol test suite beyond the interop vectors is planned.
12. Lexicon additions: `permission` and `permission-set` types, `tid` and `record-key` string formats, tightened `datetime`.

## Unverified

Reference 0.5.34 cursor semantics, PAR request lifetime, service-auth maximum age, whether `resolveLexicon`, `getHead` and `getCheckout` are served at the pin, whether the pin includes the `aud`-fragment proxy change, and the relay strict-validation date. `/guides/glossary` and `/guides/scope-builder` were not read.

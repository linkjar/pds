# Hosted handles

Owns [linkjar.io #93](https://github.com/linkjar/linkjar.io/issues/93). The patch
`patches/093-handle-policy.patch` applies to the exact `upstream.json` source pin,
then provider patch #99 consumes its API. Keep the DID as account authority.
No web app cookies, WebAuthn RP ID, seed, or recovery behavior changes here.

## Allocation and reserved names

`handle/linkjar.ts` exports:

```ts
deriveLinkjarHandle({
  displayName?: string,
  email?: string,
  isTaken: (handle: HandleString) => Promise<boolean>,
}): Promise<HandleString>
```

The helper considers display name, email local part, then `reader`; NFKD removes
combining marks and converts unsupported characters to hyphens. It produces an
ASCII label of 3–18 characters and passes upstream syntax, reserved-name and
explicit-slur checks. Unsupported scripts safely fall back instead of producing
punycode or invalid labels. A taken name gets a six-hex-character cryptographic
suffix, up to 32 attempts. An availability/database error fails the allocation.

The lookup is advisory. #99 must handle the actor table's unique-handle constraint
and retry allocation if another signup claims the name between lookup and commit.
Provider claims are naming suggestions, never proof of ownership of an account.
The complete upstream reserved/profanity lists remain in place. Additional infra,
brand, numbered, hyphenated and common ASCII digit lookalikes are rejected for
`.linkjar.social`; the new service reservations cannot be bypassed by
`allowReserved`. This is a bounded conservative list, not a claim to detect every
possible deceptive spelling. Review it when adding DNS/service names.

## First rename

Only fresh IdP signups pass the internal `linkjarAutoHandle: true` option to
`AccountManager.createAccount`. Neither returning sign-in nor account linking
sets it, and an HTTP request field must never populate it. The account and policy
row commit in the same transaction; rollback leaves neither. Existing/password
accounts receive no retroactive restriction. #99 owns the authenticated provider
flow that sets this internal flag.

One new hosted handle may be chosen before 30 days elapse. An unchanged handle is
idempotent. `AccountManager.updateHandle` enforces this for both the OAuth account
UI and `com.atproto.identity.updateHandle`, after upstream validation and before
any PLC write. A transaction claims the target, so concurrent requests cannot
spend the allowance twice. The DID remains unchanged; custom-domain changes keep
upstream ownership verification and do not consume the hosted allowance.

PLC and SQLite cannot commit atomically. If PLC succeeds but its response or the
local write fails, the selected target remains reserved in the policy row;
retrying **that same target** works even after the 30-day boundary. Another target
is blocked. This avoids spending two renames after an uncertain external write.
An operator investigating a permanently failed attempt must check the current PLC
operation and actor handle before any manual repair. Do not automatically clear
this field on a network error or restore an older account DB over live PLC state.

The migration is `007a-linkjar-093`, ordered after pinned upstream `007` and before
future upstream `008`; #99 uses `007b-linkjar-099`. Its table contains only DID,
creation timestamp and selected rename target, and is deleted with the actor.
Backup/restore must include this table with the rest of the account DB. A rollback
to an unpatched image removes enforcement; review migrations and restore policy,
not just the image tag. Remove this patch only when an upstream equivalent covers
its behavior and persisted data has an explicit migration. There is no upstream
proposal URL for this downstream hosting policy.

## Caddy and DNS boundary

Merge `staging/handles.env.example` into #90's environment. Import
`staging/Caddyfile.handles` at the top of the configured Caddyfile, then import
`linkjar_handle_hosts` in the TLS site covering `pds.linkjar.social` and
`*.linkjar.social`. Set `PDS_UPSTREAM` to the private PDS address (default
`pds:3000`). The #90 runbook owns certificates, origin protection and explicit
reserved service records; this snippet does not provision them.

The canonical PDS hostname retains every upstream route and session cookie.
Handle hosts expose only anonymous `/.well-known/atproto-did`. Other
`/.well-known/*` and `/xrpc/*` requests return 404, without redirecting: clients
must use the canonical PDS endpoint in the DID document. This prevents serving
user blobs or OAuth pages under a handle hostname. Other GET/HEAD paths rewrite
to the PDS's `/.well-known/linkjar-profile` helper, which looks up the hosted actor
and returns 302 to `https://linkjar.io/jar/<encoded DID>`. Unknown, nested and
reserved handles get 404. Input paths and queries cannot control the target.
Handle responses drop cookies and add a restrictive CSP and `nosniff`. The
unrelated `.linkjar.io` app cookie domain cannot be set by `.linkjar.social`.

Configure Cloudflare proxied wildcard A/AAAA records on `*.linkjar.social` using
#90's provisioned origin; explicit reserved service records take precedence.
Do not create an AAAA record until that origin actually has working IPv6. Keep
app infrastructure records in the separate `linkjar.io` zone. Cloudflare is the
TLS edge; keep its origin connection authenticated with the #90 TLS policy.

Caddy semantics follow its official [request matchers](https://caddyserver.com/docs/caddyfile/matchers)
and [reverse proxy rewrites and headers](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy).

## Verification

Local verification on 12 September 2026: the full ARM64 production Docker image
built successfully, including upstream TypeScript and UI builds. All 11 focused
test groups and the real Caddy integration probes passed against that compiled
image. Official/unpatched API parity and production startup/OAuth metadata smoke
also passed. Browser UI was not rerun locally for this server-only patch; CI keeps
the existing real-PAR browser gate.

```sh
# Mandatory CI: compiled production image, actual SQLite and HTTP route code.
scripts/test-handles.sh - linkjar-pds:production
python3 scripts/test-handle-proxy.py --pds-image linkjar-pds:production

# Fast local development only; transform changed TS into an isolated container.
# This checks behavior, not TypeScript types, and never replaces the full build.
scripts/test-handles.sh "$PWD/.upstream" linkjar-pds-98:production
python3 scripts/test-handle-proxy.py .upstream --pds-image linkjar-pds-98:production
```

The tests cover all upstream reserved names, local infra/lookalikes, Unicode,
profanity, collisions/exhaustion, atomic fresh-signup registration/rollback,
rename timing, concurrency, ambiguous PLC retry, actor cleanup, actual anonymous
well-known/profile HTTP handlers and a real Caddy container. The proxy probes
prove canonical PDS protocol/session behavior survives while handle hosts serve
no user blob content or cookies. Containers and their private network are removed
on completion, with no public accounts, host ports or host DB mutation.

Still required after #90 provisions DNS/TLS and #99 provides signup: create a real
Unicode-name account, `dig` its handle, `curl` the anonymous DID endpoint, and
`curl -I` an arbitrary handle-host path; then sign in from the web app while
recording that `.linkjar.social` resolution does not contact `bsky.social`.
These live acceptance steps are not claimed by local fixtures.

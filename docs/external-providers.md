# External provider authentication

Tracks [linkjar.io #99](https://github.com/linkjar/linkjar.io/issues/99). Patch
`099-external-providers.patch` applies after the exact `093-handle-policy.patch`
against the pinned PDS source. It changes the OAuth provider, compiled UI, and
account database. Remove it only after an upstream equivalent and an explicit
migration plan preserve external identities and disabled password credentials.

Configure complete pairs in the PDS secrets environment; missing providers stay
hidden, and an incomplete pair fails startup:

| Provider | Client ID | Client secret | Callback |
| --- | --- | --- | --- |
| Apple | `PDS_EXTERNAL_APPLE_CLIENT_ID` | `PDS_EXTERNAL_APPLE_CLIENT_SECRET` | `https://pds.linkjar.social/oauth/external/apple/callback` |
| Google | `PDS_EXTERNAL_GOOGLE_CLIENT_ID` | `PDS_EXTERNAL_GOOGLE_CLIENT_SECRET` | `https://pds.linkjar.social/oauth/external/google/callback` |
| GitHub | `PDS_EXTERNAL_GITHUB_CLIENT_ID` | `PDS_EXTERNAL_GITHUB_CLIENT_SECRET` | `https://pds.linkjar.social/oauth/external/github/callback` |

Apple's secret is an operator-generated, expiring client-secret JWT. #90 owns
registration, secret custody, expiry monitoring, and rotation. These values are
never included in browser hydration. `PDS_OAUTH_TRUSTED_CLIENTS` is the stock
trusted-client allowlist; configure exact metadata URLs, never an origin wildcard.
Keep `PDS_INVITE_REQUIRED=false` for public external signup. While invites are
required, known identities can sign in but new external accounts are refused.
No provider credentials, public accounts, DNS records, or servers were created.

## Authentication boundary

A same-origin start binds a random one-use state, nonce, and PKCE verifier to the
original device and pending authorization request. Apple uses `form_post`; its
callback only records the code and redirects to a same-origin completion GET.
That restores the original SameSite=Lax device cookie before token exchange.
The callback does not change the cookie policy, create an account, or log in a
browser based on state alone. Completion rotates the device session and verifies
the pending request again. Provider cancellation is consumed after device verification and frees its flow slot.
Wrong provider/device, stale request, replay, invalid
signature/issuer/audience/nonce/time claims, and mismatched account hints fail.

Apple/Google ID tokens use fixed issuer/JWKS endpoints and RS256 verification.
Google and GitHub use S256 PKCE; Apple uses nonce and its confidential client
secret. GitHub identity comes from `/user`; only a primary **verified** address
from `/user/emails` can initialize an account. Apple first-login name data is a
bounded display-name suggestion; its user-supplied email is ignored. Every
outbound decoded JSON body is bounded and has a ten-second timeout. Redirects
from provider endpoints are refused.

The flow store is process-local, bounded to 1,000 pending flows, three per device,
and ten minutes. Per-IP limits allow 20 starts and 10 completions per minute;
the limiter keeps at most 10,000 keys. IPs come from the existing device metadata
policy. Configure trusted proxy handling deliberately so clients cannot forge
that address. Restarting this single-process PDS cancels in-flight sign-ins;
multiple independent replicas need a shared transaction store before use.

`external_identity` is keyed by `(provider, subject)`, with DID, asserted email,
verification flag, optional display name, and creation/last-use timestamps. Email
collisions are rejected, never linked. Account, verified email, external identity,
and the initial rename allowance commit in one SQLite transaction. A handle
claimed between allocation and commit is retried at most three times. Identity
uniqueness races roll back rather than labeling another flow's account as newly
created. Account deletion removes the identity rows. The store exposes list/link/
unlink primitives; authenticated linking and recovery-method lockout policy are
owned by #100 and no anonymous linking route is exposed here.

Passwordless accounts use the explicit `!external-identity` credential sentinel.
Password verification rejects it before scrypt; an ordinary password update
replaces it with a valid hash. Existing accounts, app passwords, password signup,
CAPTCHA, and email password-reset confirmation retain their upstream paths.

## Consent and UI

Both signup hooks run only after account creation. The PDS authorizes exactly the
requested scopes only for its explicitly trusted client IDs. Known external
identities never receive fresh-signup preauthorization. The normal consent check
still honors `prompt=consent`. External login resumes the original request; a
missing grant renders a request-bound ephemeral session, and deactivated accounts
retain the stock explicit reactivation gate. The compiled page canonicalizes its
URL to `/oauth/authorize` with the original client and request URI before mounting
React, preserving existing API Referrer, CSRF, device, and ephemeral-token checks.
No permanent remembered account is created by an external OAuth authorization flow.

Provider buttons are supplied by backend hydration on authorization sign-in and
signup pages. Email/password remain under an expandable “Or use email” section.
Explicit `/account` sign-in uses the same device-bound provider verification and
remembers the account through the stock device-account store before returning to
`/account`; it has no PAR and never grants OAuth client scopes. The signup disclaimer applies to provider signup as well as email.

## Authoritative fresh-account signal for #95/#96

The current patch deliberately does not change standard OAuth state or attach an
unauthenticated `created=true` flag. The server has a trustworthy local `created`
result in completion, but the stock token response does not expose its provenance.
The following is a proposal for a separate tested integration, not shipped code:

1. Add an `accountCreatedDid` marker to the **existing authorization request**.
   The account-creation transaction writes it only when that request still belongs
   to the same device and OAuth client and remains unconsumed. Both password and
   external signup use this transaction. Existing-account login never writes it.
2. When consuming the authorization code, copy the marker into the persisted
   OAuth session only if it equals the authorized DID. Bind it to that session's
   client ID and DPoP key. Do not infer it from account age, empty repository,
   `prompt=create`, or a matching email.
3. Expose that session marker through an authenticated, DPoP-verified PDS endpoint
   for the current access token, or a supported token-response extension consumed
   by the OAuth SDK. The clients can show the handle/seed transition only after
   checking that authenticated result. Consume/acknowledge it per OAuth session,
   with replay, refresh, cancellation, cross-client and concurrent-signup tests.

This requires a coordinated account/authorization/session schema change plus web
and native SDK integration; no client draft should claim it is already available.

## Verification and limits

Local verification on 12 September 2026: the full ARM64 production image built
successfully, including 102 provider tests before production dependency pruning.
The five SQLite groups and compiled browser checks also passed against that exact
image. Production health/OAuth metadata and the existing six-probe
official/unpatched parity gate passed. PDS and UI TypeScript builds passed;
compiled UI build passed; signed provider-token, callback HTTP, transaction-store,
and signup-consent tests passed (102 focused cases). Five actual SQLite runtime groups prove identity
isolation, rollback, email collision rejection, password enablement, and cleanup.
Compiled Chromium mobile checks ([signup](external-signup.png),
[sign-in](external-signin.png)) prove provider buttons/email folds and actual
consent/reactivation API guards after callback URL canonicalization, including
request URI, Referrer, and ephemeral bearer binding. The browser fixture uses a fixed verified-token payload at the signer boundary;
the actual API performs its device/request/DID comparison. Cryptographic token
verification is tested separately with signed provider fixtures. Fixtures mock
provider/token transport and account effects; they are not staging provider acceptance.

Run source-package tests using pinned pnpm from `packages/oauth/oauth-provider`:
`pnpm test src/external src/router/create-external-middleware.test.ts src/account/account-manager.external.test.ts`.
Run `tests/external-accounts.mjs` in the compiled image. Local compiled UI checks
use `UPSTREAM_DIR=/path/to/built/atproto node tests/external-ui.mjs`.

Remote CI, public provider registration, deployed callbacks, web/iOS tap counts,
account-page password setup against staging, and live encrypted seed behavior
remain acceptance work after #90. The prepared upstream proposal remains in
`docs/upstream-proposal.md`; there is no published upstream issue URL yet.

Primary protocol sources: [Apple verification](https://developer.apple.com/documentation/signinwithapple/verifying-a-user),
[Google discovery](https://accounts.google.com/.well-known/openid-configuration),
[GitHub authorization](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps).

Apple refresh/access tokens are not persisted by this patch. #100 must provide
encrypted revocation-material custody or fresh Apple authorization before it can
meet its account-deletion revocation requirement. Apple revocation is not claimed
by this implementation.

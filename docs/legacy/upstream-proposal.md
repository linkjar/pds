# External sign-in providers for self-hosted PDS operators

Target: https://github.com/bluesky-social/atproto/issues/new

We are adding optional external sign-in to a self-hosted reference PDS and would
like to keep the integration as a small upstream extension rather than a fork of
the provider. Would a provider-neutral interface at the following boundaries fit
the project's direction?

The current reference OAuth provider authenticates accounts locally and ships a
compiled UI. We propose keeping all AT Protocol client OAuth semantics intact
while allowing the PDS to act separately as an OIDC/OAuth client to configured
external identity providers. This does not add `openid` to AT Protocol client
scopes or make external email addresses AT Protocol identities.

Proposed extension points:

- A durable external-identity store keyed by provider issuer and subject, mapping
  to a local account DID. Uniqueness and account linking should be atomic. An
  equal email address alone must never link two identities.
- Explicit start/callback routes under the PDS provider origin. A one-time,
  short-lived transaction binds state, PKCE, nonce where applicable, the browser
  device session, and the original authorization request. Return locations must
  be selected from server-side state, not arbitrary callback parameters.
- A documented hook to authenticate an already-verified local account into the
  existing device session, then resume the original PAR/authorization flow with
  the same consent, client, scope and security checks as password sign-in.
- Optional provider-list entries in UI hydration data, containing display labels,
  icons and PDS-local start routes, never provider secrets. Operators with no
  external providers configured should retain the current UI and behavior.

Account creation would still pass through the PDS account store and existing
handle, CAPTCHA, invite, rate-limit and abuse controls. Linking and unlinking
require a fresh authenticated session; removing the last usable sign-in method
must be rejected. Provider failures should leave local sign-in and recovery
usable. Provider credentials belong exclusively to the PDS configuration.

We would contribute tests for unchanged default behavior, one-time callback
consumption, state/nonce/PKCE failures, issuer/subject mismatch, parallel callback
races, safe linking and recovery, and resuming the original authorization request.
External-provider implementation policy (Apple/Google/GitHub) can remain outside
the core package if a narrow adapter interface is preferred.

Source inspected: `@atproto/pds@0.5.34`, commit
`7ca16cc6989f8247637615aca17c5abb911b8fb1`, particularly
`packages/oauth/oauth-provider/src/router/assets/assets.ts` and the OAuth provider
account/device stores and UI hydration contracts. Our downstream work is tracked
in https://github.com/linkjar/linkjar.io/issues/98, with provider support in
https://github.com/linkjar/linkjar.io/issues/99 and account linking in
https://github.com/linkjar/linkjar.io/issues/100. We have prepared a pinned source-image build and
reversible UI-patch check; the external sign-in implementation is not yet shipped.

Is this extension boundary useful upstream, and are there existing plans or
preferred interfaces we should align with before implementing the adapters?

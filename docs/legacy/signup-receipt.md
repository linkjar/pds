# Account creation receipt

Issues [#95](https://github.com/linkjar/linkjar.io/issues/95) and
[#96](https://github.com/linkjar/linkjar.io/issues/96) need to distinguish an
account created during this authorization from an existing account signing in.
`099b-signup-receipt.patch` adds that distinction after the #93 and #99 patches.
A new local session, an empty repository, account age, or `prompt=create` is not
creation evidence.

## Contract

Call these methods on the authenticated account's PDS using the OAuth session's
normal DPoP transport. They require no email permission or additional account
write permission: the receipt is metadata belonging to that specific session.
App passwords and unauthenticated requests are rejected. Responses use
`Cache-Control: no-store`.

- `GET /xrpc/io.linkjar.account.getSignupReceipt` returns
  `{ "created": true, "did": "did:plc:…", "handle": "name.linkjar.social" }`
  only while the current session has a pending receipt. Otherwise it returns
  `{ "created": false, "did": "did:plc:…" }`. Read does not consume it.
- `POST /xrpc/io.linkjar.account.acknowledgeSignupReceipt` with JSON `{}` clears
  that session's pending receipt and returns `{ "created": false, "did": … }`.
  Repeated acknowledgment is safe. Another session cannot acknowledge it.

Both lexicons are in the source patch and generated through upstream codegen.
There is no additional token response field, OAuth state value, caller-supplied
DID, or modification to the standard `com.atproto.server.getSession` contract.
The handle comes from the current account record, including after a rename.

Web and iOS should accept this signal only from the configured LinkJar PDS,
validate the response DID against the authenticated session, show the handle
before existing seed setup, and acknowledge it when the user continues. Handle
changes use the existing `com.atproto.identity.updateHandle` method and #93
policy. Unsupported PDS methods must not turn a returning login into signup.
These client changes are tracked in app PRs #110 and #111; they are not shipped
by this image patch.

## Binding and lifecycle

1. Email and external-provider signup pass a validated request ID, client ID,
   and original browser device ID internally to account creation. These are
   separate arguments, never fields accepted from the signup body. Account-page
   signup without an authorization creates no receipt.
2. The account transaction checks that the request is still unexpired,
   unauthorized, bound to that client and device, and has no prior receipt. It
   writes the newly created DID or rolls back the account, credentials,
   provider identity, and handle policy together. Concurrent signup attempts
   against one authorization can create only one account.
3. Authorization-code consumption carries this marker through the standard
   client, DPoP key and PKCE checks. Token creation retains it only if the
   authorized DID is exactly the newly created DID. Selecting another account,
   signing in again, or requesting `prompt=create` does not grant a receipt.
4. Authenticated resource requests use the verifier's signed token ID and client
   ID to query the persisted token row for that DID. No unverified JWT decoding
   takes place. Rotation preserves the pending receipt in the same database row;
   an old token ID cannot retrieve it. Acknowledgment clears the marker, and
   later refresh cannot restore it. Revocation or authorization-code replay
   removes the session and receipt.

Migration `007c-linkjar-signup-receipt` adds nullable `signupDid` columns to
`authorization_request` and `token`. Existing rows remain null. There are no
new keys, credentials, external calls, or background jobs. Unconsumed requests
expire through upstream cleanup; receipt retention follows the OAuth session.
If an error after account creation forces the authorization to restart, the
next login receives no creation receipt. This intentionally fails closed.

## Verification and removal

`tests/signup-receipt.mjs` runs against the compiled PDS with SQLite, actual
account transactions, code consumption, PKCE checks, signed access tokens,
resource DPoP validation, registered XRPC handlers, refresh, acknowledgment,
concurrent signup, and replay revocation. The provider build also runs the
signup context and external callback tests. CI runs the compiled receipt test
inside the production image, alongside the existing handle, external-provider,
API parity, and browser checks.

This is a LinkJar-specific extension, not an upstream OAuth standard. No
upstream proposal has been submitted. Remove it when an upstream equivalent
provides the same authenticated creation provenance, after migrating both
clients. Removing the source patch leaves nullable columns in SQLite harmlessly;
rolling back the schema explicitly loses pending receipts. It does not undo
accounts, handle policy, provider identities, or OAuth sessions.

# Unit 6: the linkjar profile

Date: 2026-10-13 · [Plan](../plan.md) · [SPEC §12](../SPEC.md#12-extension-points) · [SPEC §13](../SPEC.md#13-the-linkjar-profile)

The Reference in production is the upstream server with eight account
patches. This unit gives the Candidate the behaviour of those patches as a
profile that `PDS_PROFILE=linkjar` selects. The server core calls a trait
and holds no branch for an operator.

## 1. Result

| Patch | State |
|---|---|
| `093-handle-policy` | Built: the extra reservations, the handle of a new provider account, one hosted rename in 30 days, handle hosts. |
| `099-external-providers` | Built: Apple, Google and GitHub, the flow store, the limits, `external_identity`, accounts without a password. |
| `099b-signup-receipt` | Built: `io.linkjar.account.getSignupReceipt` and `acknowledgeSignupReceipt`. |
| `099c-signup-policy` | Built: the three hCaptcha variables or no start, the closed legacy `createAccount`, the challenge on the sign-up page. |
| `100-signin-methods` | Built: linking and unlinking on the account page, last-method protection, notices through an outbox. |
| `101-mail-templates` | Built: the six templates, taken from the patch. |
| `102-signup-journey` | Built: the order of the steps and the "Or use email" disclosure. The pages are the Candidate's own. |
| `103-invite-handoff` | Built: the code in the fragment, on the form and on the provider links. |

`pnpm parity run candidate` passes all 22 default scenarios, the `captcha`
profile and the `firehose` profile, and both scenarios of the `invites`
profile. The Harness runs the Candidate with `PDS_PROFILE=linkjar`,
because the Candidate stands in for the production build.

## 2. How it is built

- **`pds-ext`** has the `Profile` trait and the `Stock` profile. Each
  method has a default that is the Stock Reference's behaviour: no
  reservation, no rename rule, no provider, no receipt, no notice.
- **`pds-ext-linkjar`** has `Linkjar`. Its rules are functions without
  input or output of their own: the reserved labels and their lookalikes,
  the label of a name, the rename decision, the address a browser is sent
  to, and the checks on what a provider answers. An ID token is verified
  with RS256 against the provider's key set, with `ring`.
- **Requests to a provider** go through a `Fetch` that the server gives:
  10 seconds, 128 KiB, no redirect.
- **Tables.** The profile applies the four migrations of the patch stack
  that a database lacks, under the patch stack's names
  (`007a-linkjar-093` to `007d-linkjar-signin-notices`). Each server opens
  a database that the other migrated.
- **Transactions.** The identity, the confirmed address, the handle
  policy row and the creation receipt are written in the transaction that
  creates the account (EX-4). A link, an unlink and a password change
  write their notice in the transaction of the change.
- **No request field** selects an extension flag (EX-3). The server passes
  the identity and the authorization of a sign-up as arguments of account
  creation; the JSON of `createAccount` cannot set them.

## 3. How it is checked

- `crates/pds-ext-linkjar`: the reservations, the labels, the rename
  rule, each provider's parameters, a GitHub identity, and an ID token
  that a test key signs, with each wrong claim refused.
- `crates/linkjar-pds/tests/accounts.rs`: the reservations and the rename
  rule over HTTP with the migrations applied, a handle host, and the
  closed legacy sign-up.
- `crates/pds-xrpc`: the flow store and its limits, the retry delays of a
  notice, the request to hCaptcha.
- The Harness: `30-linkjar-external`, `31-linkjar-handles`,
  `32-linkjar-signup-journey`, `33-linkjar-signin-methods`,
  `invites/02-linkjar-invite-handoff` and `captcha/01-signup-policy`. The
  providers are the fixture server's stand-ins, and the edge of the stack
  runs the production Caddy snippet.

## 4. What differs from the patches

1. **Handle hosts.** The server answers a handle host itself (SPEC
   §13.2). On the two answers that the Caddy snippet forwards, the DID and
   the profile redirect, the server sets none of the three headers that
   the snippet sets, because a second copy joins the first. Without the
   snippet in front, the DID of a handle host comes without them.
2. **Start of a provider flow.** The patch compares the request URI of
   the start with the one of the referring page. A page of the Candidate
   that answers a form has no query, so the Candidate checks the
   referring page's origin and path, and checks the request against the
   device.
3. **Session rotation at completion.** The patch rotates the device
   session when a flow completes. The Candidate rotates a session after
   five minutes, as for every request.
4. **The account page** is the Candidate's own: `/account` lists the
   accounts of the device, and `/account/<did>/manage` has the sign-in
   methods, the sessions and the devices.

## 5. What is not built

1. Setting a password from the account page. An account that an identity
   created gets a password through the reset mail.
2. Operator roles (SPEC §13.4). They belong to the ops listener of unit 9.
3. The reactivation step for a deactivated account in a provider flow. It
   gets the consent page, and the authorization then answers
   `access_denied`.
4. Apple token revocation, which the patch does not claim either.
5. The explicit-slur check on a suggested handle. The reserved lists are
   applied.

## 6. Dependencies

`ring` and the two `icu` crates were in the lock file already; the
profile now uses them directly. `cargo vet` needs no new exemption.

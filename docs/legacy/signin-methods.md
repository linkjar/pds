# Sign-in methods

Revision 7 implements the explicit linking, unlinking, account settings and
security notification portions of [linkjar.io #100](https://github.com/linkjar/linkjar.io/issues/100).
It follows the provider, signup receipt and CAPTCHA patches. Provider credentials
remain opt-in; use [the provider setup](external-providers.md).

## Account boundary

A signed-in account starts a fresh Apple, Google or GitHub authorization from its
account page. The server stores the originating DID and device with the one-use
flow. It checks that the account session is active before exchange, after the
provider response and after the policy hook. Callback parameters cannot select a
new target account. Linking neither creates an account nor grants application
OAuth permissions or an account-creation receipt.

An identity already owned by another account is refused with that account's
handle. Its email and DID are not included in the error. Linking an identity
already owned by this account is idempotent. The account contact email is never
replaced by the provider's asserted address, including an Apple private relay
address when Google is added later.

The account page lists linked providers and asserted emails, offers configured
providers to link, and reports whether a password is set. Its methods API requires
the existing permanent device session, the requested account and account-page
context. It retains the provider's CSRF, Origin and fetch-metadata checks and
rejects bearer/ephemeral authentication and authorization-page context.

Unlink checks the password and remaining configured providers in the same SQLite
transaction as deletion. A contact email alone is not a sign-in method, and a
disabled provider cannot keep an account recoverable. Concurrent requests cannot
each remove the other's last method. The UI disables that action and explains
how to add another method; the database check remains authoritative.

## Security email delivery

Link, unlink and password updates enqueue a notice in the account database in
the same transaction as the change. A failed insert rolls back the change.
Migration `007d-linkjar-signin-notices` adds the outbox. Existing accounts and
provider links remain intact; no messages are backfilled for old changes.

One dispatcher per PDS process starts after the HTTP server listens. Every minute
it processes at most 100 due notices. Failed deliveries remain in SQLite across
restarts, with exponential retry delays from one minute up to one hour. SMTP
configuration is required to deliver; missing SMTP retains notices. Use one
active PDS process for this SQLite data directory, as with the rest of the PDS.

Messages use the contact address captured when the change committed, contain the
method, DID and time, and never contain credentials, tokens or provider subjects.
Delivery is at least once: a crash after SMTP accepts a message and before its row
is deleted can result in a duplicate. Retries use the same Message-ID. Successful
rows are deleted; failed rows are retained until successful delivery or account
deletion. Include this data in the operator's retention policy and account DB
backup. SMTP errors and identity data are not logged by the retry worker.

Before opening registration, verify real SMTP delivery (including Apple's relay
sender registration), retry after a temporary outage and recovery after restart.
The local tests use captured transports and do not send mail.

## Compatibility and remaining scope

There is no automatic email-based linking in this patch. A matching provider
email is not sufficient to attach an existing account. The requested Apple/Google
auto-link policy is pending an explicit product decision: Google documents that
`email_verified` alone does not establish present ownership of an external email
address without an authoritative hosted domain.
See [Google's token verification guidance](https://developers.google.com/identity/gsi/web/guides/verify-google-id-token).

Apple credential retention/revocation and passwordless account deletion are
separate remaining work for #100. This patch must not be used as evidence that
Apple account-deletion acceptance has passed. Real provider, staging and native
settings handoff acceptance also remain outstanding. Do not close #100 based on
this subset.

## Verification and removal

```sh
docker build -t linkjar-pds:production .
docker run --rm --entrypoint node -v "$PWD/tests:/linkjar-tests:ro" \
  linkjar-pds:production --test /linkjar-tests/signin-methods.mjs
python3 scripts/test-external.py linkjar-pds:production --methods
```

The image build runs the external-provider and flow suites, including linking,
foreign identity refusal and a session lost during provider exchange. Compiled
SQLite tests cover concurrent last-method protection, disabled providers,
transaction rollback, password updates, contact preservation, retry, restart and
mail formatting. Compiled mobile-browser checks cover the rendered controls and
real API authentication/CSRF/context refusals, followed by successful unlink.
The fixture substitutes identity storage for browser tests; the separate database
suite tests the real storage implementation.

`100-signin-methods.patch` is owned by #100. The prepared upstream proposal is
[still a draft](upstream-proposal.md); no upstream issue has been filed. Remove
this patch only when upstream supplies equivalent device-bound linking,
transactional last-method protection and durable security notices, and the tests
pass against that implementation. Back up the account database before upgrading.
Do not drop the outbox during rollback while notices are pending; an older image
will not deliver them. Restore a matched image and database snapshot if a schema
rollback is necessary, accounting for changes since that snapshot.

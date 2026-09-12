# Provider configuration and hosting policy

Implementation for [LinkJar #94](https://github.com/linkjar/linkjar.io/issues/94).
The configuration and signup guard are prepared; published hosting terms,
privacy/support pages and three-client live acceptance remain pending.

## Configuration

Merge [provider.env.example](../staging/provider.env.example) into the private
raw PDS environment collected by the provisioning wizard. Retain existing
credentials when merging; the example's blank fields are not replacements for
saved secrets. Use each key once. Do not source the file or quote its values:
Compose's `format: raw` preserves quotes and hashes literally.

The name, logo and links use upstream branding configuration. Primary and
success come from the app's light palette (`heroui-theme.ts` at app commit
`fe1aef339ac407608bdbf0bbdcaa3ef6fb9e7a17`). Error and warning use the app's inherited
HeroUI 3.2.4 danger/warning tokens, converted by Chromium to sRGB:
`oklch(0.6532 0.2328 25.74)` → `#ff383c` and
`oklch(0.7819 0.1585 72.33)` → `#f5a524`. Info shares the brand accent.
The provider exposes one palette for both themes and chooses black or white
primary foreground text by contrast. Verify the compiled light/dark pages.

Trust exactly the three first-party metadata URLs in the example. Trust extends
session lifetimes; it does not grant scopes or independently skip consent.
Existing grants can skip repeat consent. #99 records the requested grant during
fresh provider signup, so that path has no redundant consent screen. Test each
client separately; a grant to the web app is not a grant to the extension or iOS.

Handle domains advertise signup availability; invites are explicitly disabled.
Keep server rate limits enabled and development mode disabled. The wizard
collects all three hCaptcha settings; configure its site for `pds.linkjar.social`.
Missing all three leaves upstream hCaptcha disabled, so all three must be present
before opening LinkJar signup. Partial configuration now prevents startup.

## Legacy signup boundary

The legacy `com.atproto.server.createAccount` lexicon cannot carry hCaptcha
proof. With hCaptcha configured, the production patch rejects fresh accounts
through that endpoint before account storage or PLC work. It directs callers to
OAuth signup, whose email path checks hCaptcha. An invite code, claimed DID,
deactivated flag or another account's service token cannot bypass this guard.

Migration remains available when verified service authentication names exactly
the DID being imported. Existing input, email, handle and DID validation still
run; the guard does not authorize a migration itself. Entryway deployments and
deployments without hCaptcha retain their existing endpoint behavior. LinkJar's
production configuration is the standalone, hCaptcha-enabled case.

`099c-signup-policy.patch` follows the provider and receipt patches. It changes no
lexicon, token format or database schema. Remove it when an upstream release
closes the same signup bypass and fails closed on partial hCaptcha config, after
running the compatibility tests. No upstream report has been filed by this work.

## Acceptance before opening signup

Record the image digest, date and redacted evidence in #94:

1. Publish the completed hosting documents; fill the three policy/support URLs
   and monitored contact email. Check successful HTTPS responses and page content.
2. Inspect the authorization and account pages in light/dark modes, including
   visible mark/name, button contrast and functional footer links.
3. Exercise web, iOS and extension PAR requests and signup independently. Check
   the requested client/scopes, first authorization and repeat authorization.
4. Submit email signup without a CAPTCHA, with an invalid token and with a valid
   site-bound challenge. Only the last should proceed. Exercise expired/replayed
   challenge handling. Provider signup uses #99's verified-provider exception.
5. Verify an anonymous legacy createAccount request is rejected. Rehearse an
   authenticated import in isolation before the move-in wizard in #103 ships.
6. Prove support mail delivery and the password-reset/deletion procedures below.

Local tests exercise the real registered createAccount handler and compiled
configuration. They do not establish live CAPTCHA verification, public page
availability, successful migration or ownership of a mailbox.

## Support procedures

Use the confirmed mailbox and assign an operator before publication. Never ask
for a password, Private Jar seed, recovery secret or provider token in a ticket.

| Request | Procedure |
| --- | --- |
| Password reset | Direct to the PDS reset flow (`requestPasswordReset`, then `resetPassword`). Check delivery status privately, including Apple relay addresses. Do not disclose whether an unrelated email has an account. |
| Handle change | Verify the signed-in DID; use the account client's `updateHandle` flow. Explain reserved names and #93's initial hosted rename limit. Custom-domain handles retain upstream validation. |
| Account deletion | Use the authenticated `requestAccountDelete` flow and its confirmation token through `deleteAccount`. Verify account/session invalidation and storage cleanup; do not delete by an unauthenticated email request. Apple token revocation remains required work under #100. |
| Export or move | Export the repository CAR and its referenced blobs; CAR alone does not contain blob bytes. Keep the Private Jar recovery material separately. Follow #103's validated migration sequence; preserve the DID and verify the destination before retiring the source. |
| Abuse or takedown | Record the reported public URI/DID and reason in the private support system. Apply the approved hosting policy, preserve a minimal audit trail, and communicate the decision and review route. Do not promise access to private plaintext. |

Do not claim immediate erasure from backups or independent relays. The actual
backup retention, deletion schedule, legal retention process and support response
commitment must be recorded in the published policy, once the operator supplies
those decisions. [Draft content inventory](hosting-policy-draft.md).

Sources checked 2026-09-12: [upstream customization](https://atproto.com/blog/pds-customization-and-metrics),
[hCaptcha verification](https://docs.hcaptcha.com/), and the pinned PDS's
`config/env.ts`, `config/config.ts`, `createAccount.ts`, `requestAccountDelete.ts`
and `deleteAccount.ts` under upstream commit `7ca16cc6989f8247637615aca17c5abb911b8fb1`.

## Local verification (2026-09-12)

The production image compiles and its 103 provider tests pass. The new five
signup-policy tests fail on the previous receipt image and pass on this compiled
image (partial configuration and legacy signup account for the two prior
failures). Eleven handle tests, external account storage/browser checks, seven
receipt groups, official/unpatched API parity and the compiled browser smoke
check pass. The branding fixture reads the committed raw env example and checks
its exact trusted-client list through `readEnv`/`envToCfg`; light/dark signup and
account pages render with the pinned app logo, palette and white-on-blue form
buttons. Screenshots were inspected at a 390px mobile width.

The branding test deliberately substitutes `policy.invalid` URLs and serves the
pinned logo fixture locally. It proves configuration/rendering, not published
policy availability. The logo fixture is copied from the app's
`apps/web/public/logo/linkjar-128.png` at the commit cited above. No real signup,
provider registration, CAPTCHA challenge or support message was performed.

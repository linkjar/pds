# Account email templates

`101-mail-templates.patch` supplies HTML for password reset, account deletion,
email confirmation, email change, PLC operations and sign-in method changes.
It follows the provider/security-mail work in #99 and `100-signin-methods.patch`.

All six templates follow LinkJar's existing welcome and alpha-invite emails:
the `#020202` canvas, blue accent, centered jar icon and heading, subtle dark panel,
and established footer. Inline CSS and presentation tables keep codes, account
details, security instructions and support links readable. Handlebars escapes
dynamic fields. The logo uses the same public `linkjar-128.png` asset as the
existing emails; there are no tracking pixels or remote fonts. The existing
Nodemailer HTML-to-text plugin supplies the text alternative.
The optional upstream Bluesky email-confirmation link retains its original URL and
configuration gate. No new expiry or account-policy claims are introduced.

CI runs `tests/mail-templates.mjs` against the compiled production image. It checks
all six renderers, escaping, removal of the oversized spacer, and HTML/text MIME
alternatives for password-reset and sign-in notices.

Remove the patch when upstream provides equivalent layouts and a template-backed
sign-in notice, or replace it with a supported upstream branding interface.

## Temporary runtime overlay

Until the full revision 9 image passes CI and is published, an operator can build
only the mail changes on the pinned, deployed revision 8 image:

```sh
sudo python3 scripts/build-mail-overlay.py
sudo docker run --rm --entrypoint node \
  -v "$PWD/tests:/linkjar-tests:ro" linkjar-pds:mail-overlay \
  /linkjar-tests/mail-templates.mjs
```

The builder extracts source from the pinned base, checks and applies the canonical
patch, compiles the templates and mailer, and prints the immutable local image ID.
It never reads the PDS environment or data volume. This is a local runtime overlay,
not a published multi-architecture release. Pin its printed ID only after testing;
retain the source and image on the host. Replace it with the verified registry
digest after the normal release pipeline completes.

The production database schema and account data do not change. Switching the image
restarts the PDS, so check health, actor inventory, security-mail queue and backups
afterward. Use synthetic preview codes when checking mail appearance.

On 2026-10-02 the initial template implementation and multipart messages passed an
isolated server overlay test. After operator feedback, all six templates were
restyled against `packages/transactional/emails/welcome.tsx` and `alpha-invite.tsx`
in the LinkJar app repository. The revised renderers pass local escaping checks;
the dark reset preview was inspected in the browser. Full image CI, the canonical
overlay and live deployment remain pending. Browser previews use `FIXTURE-CODE`,
not an account code.

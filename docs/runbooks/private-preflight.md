# Private launch preflight: 2026-10-01

Public launch remains closed. `enablePds = false`; the host has no production
accounts or PDS container. The Hetzner public HTTPS gate remains closed.

Completed configuration checks:

- Blob storage, backup credentials, online PLC rotation key and all three
  hCaptcha inputs are present in the protected runtime environment.
- Apple and Google credentials are present. GitHub setup remains deferred.
- SQLite automatic WAL checkpoints are disabled for Litestream replication.
  The provisioning wizard now sets this value for subsequent installations.
- LinkJar branding, palette and the three first-party trusted clients are set.
- The published privacy and support pages loaded in the browser. The privacy
  page identifies the confirmed operator, minimum age 16, Cloudflare account
  email and the approved initial backup retention policy.
- Runtime privacy/support links use `https://linkjar.io/privacy/` and
  `https://linkjar.io/support/`; contact uses the published `hello@linkjar.io`.

Open launch checks:

- Hosting terms are not configured. A reviewed, published terms URL is required.
- The configured production image is not present on the host. Registry access
  to the revision 8 release was denied, including with the existing GitHub CLI
  credential. Verify package access, image digest, architecture, source revision
  and telemetry label before starting it. Do not substitute an unverified image.
- Operator email alert delivery and independent external uptime monitoring are
  not yet accepted. Self-hosted rules cannot detect a complete host outage.
- Cloudflare delivery-event ingestion is not implemented; the existing signed
  receiver supports Resend. SMTP acceptance alone cannot establish bounce health.
- Real accounts, provider callbacks, hCaptcha challenges, SMTP and Apple relay
  delivery, blob lifecycle, PLC identity, handles, signed writes and full account
  recovery still need the [live acceptance checks](pds.md#live-acceptance-record-all-pending).
- The live R2 restore rehearsal used synthetic data. It does not establish
  production account recovery or strict backup deletion during an outage.

Do not request relay crawl or open signup from these configuration checks alone.

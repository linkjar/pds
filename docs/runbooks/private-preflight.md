# Private launch preflight: 2026-10-01

Public launch remains closed. `enablePds = true` with `enablePdsIngress = false`;
the persistent PDS runs on loopback with zero accounts. Caddy and the host HTTPS
rule are disabled. The Hetzner public HTTPS gate remains closed. An operator-approved
Cloudflare Tunnel now provides HTTPS only through an IP allowlist for acceptance.

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

- GitHub package access is verified after granting the CLI `read:packages`.
  The host pulled the configured index digest
  `sha256:65439dafd5c431e86400a877cc33d91a932ec931aef52223dc44972535080d07`.
  Its ARM64 manifest is
  `sha256:edbb9c9a548e55202666071772e3f3e9f2baab95f07f115ec992a83b1d735a1b`;
  labels identify source revision `e1b9043b74d31f00c3d41badb52bc60695fd8fc1`
  and `social.bsky.pds.telemetry=otel`. Temporary registry login files were removed.
- An isolated container started with the protected runtime configuration, tmpfs
  data storage and loopback-only port 3002. Health returned version `0.5.34`;
  OAuth authorization metadata and describeServer returned HTTP 200.
  describeServer advertised `.linkjar.social`, the published privacy URL and
  `hello@linkjar.io`. No account was created. The container was stopped and
  temporary storage discarded. This does not prove real provider callbacks,
  persistent storage, public routing or account recovery.

- The restricted HTTPS route passed an allowed-source HTTP 200 and a controlled
  denied-source HTTP 403 test. The real app completed PAR and opened the provider
  signup page. No account has been created; signup agreement confirmation is pending.

Open launch checks:

- Hosting terms are not configured. A reviewed, published terms URL is required.
- Recovery acceptance with real accounts remains pending. Litestream and recurring
  encrypted host-material backups now run. An isolated R2 restore of the current
  empty PDS recovered three databases and matching online configuration, passed
  integrity checks in 13.473 seconds, and was removed. This does not prove actor
  key recovery, blob reads, signed writes or OAuth on a replacement host.
- Grafana SMTP alert delivery is accepted: the contact-point test succeeded and
  the operator confirmed inbox receipt on 2026-10-01. Independent external uptime
  monitoring remains pending; self-hosted rules cannot detect a complete host outage.
- Cloudflare delivery-event ingestion is not implemented; the existing signed
  receiver supports Resend. SMTP acceptance alone cannot establish bounce health.
- Real accounts, provider callbacks, hCaptcha challenges, SMTP and Apple relay
  delivery, blob lifecycle, PLC identity, handles, signed writes and full account
  recovery still need the [live acceptance checks](pds.md#live-acceptance-record-all-pending).
- The live R2 restore rehearsal used synthetic data. It does not establish
  production account recovery or strict backup deletion during an outage.

Do not request relay crawl or open signup from these configuration checks alone.

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
  signup page. Subsequent private acceptance results are recorded below.

Private acceptance update, 2026-10-02 (Europe/Berlin):

- The operator approved signup and created `uros-karic.linkjar.social` through
  Google. Clean sign-out and Google sign-in returned the same account.
- The operator's saved Recovery Kit unlocked the Private Jar after sign-in.
  The synthetic private bookmark and its encrypted snapshot opened successfully.
  This verifies client recovery; replacement-host OAuth and signed writes remain
  unverified. The app still displays the DID instead of the handle.
- A labelled test sent through the PDS's configured SMTP transport was accepted
  for one recipient, with no rejections. The operator confirmed Gmail receipt
  from `accounts@linkjar.io`; the message appeared in Archive / All Mail.
  This proves receipt of the transport test, not inbox placement for every message.
- Ordinary Google signup and login do not enqueue security notices. Current
  notices cover provider linking, unlinking and password changes. Automatic
  outbox delivery, outage retries and Apple Hide My Email delivery remain untested.

Further acceptance, 2026-10-02:

- Apple was explicitly linked to the existing Google account using Hide My Email.
  The account page lists both providers; the contact email remains Gmail.
  The security-notice queue was empty after linking. The operator confirmed
  receipt of the automatic Apple-linked notice at the existing Gmail contact
  address. The recorded change time was `2026-10-01T22:07:47.072Z`.
- A labelled SMTP test to the Apple relay address was accepted for one recipient,
  with no rejections. The operator confirmed receipt of the forwarded test.
  A real password-reset message and bounce handling remain unverified.
- The account-bearing R2 restore recovered four databases and one actor in
  23.12 seconds. SQLite integrity checks passed; restored data contains two
  bookmarks, one archive record and one blob record.
- An isolated restored PDS, with mail and crawlers disabled, returned health 200
  and served the two bookmarks and archive record. It read the 12,733-byte blob
  from existing R2 storage with SHA-256
  `599d122335b08a4eac9e9ac188b87cde903101100ddd0479afc209ab40df676c`,
  matching the original. This tests restored metadata and existing blob storage,
  not restoration of a lost blob bucket.
- An earlier account restore recovered the actor signing key; a fresh offline
  signature verified against the PLC document's public signing key. Replacement
  host OAuth, authenticated signed writes and sequencer continuity remain pending.
- The isolated container and both disposable restore directories were removed.
  Production traffic was unchanged; the operator's work Tailscale was restored.

Additional reset, retry and write acceptance, 2026-10-02:

- The live `com.atproto.server.requestPasswordReset` request returned HTTP 200.
  The operator confirmed receipt at Gmail. No password was changed.
- A fresh isolated restore changed only its copied account contact address to
  the approved Apple relay address. Its real password-reset endpoint returned
  HTTP 200; the operator confirmed forwarded reset-email receipt. Its reset token
  was not valid on production. The live contact address remained unchanged.
- The deployed `deliverSignInNotices` implementation retained a synthetic notice
  after an actual SMTP connection refusal, with attempts 1 and retry delay 60,000
  ms. A separate process reopened the database and skipped delivery before the
  deadline. At the deadline, a successful captured transport delivered once and
  removed the row. This proves persistent retry state across process restarts;
  it does not test the minute timer during a real provider outage.
- A temporary credential existed only in the disposable restored account. That
  instance accepted a session and an authenticated synthetic record write.
  Readback CID matched; its exported CAR commit signature verified against the
  original PLC public signing key. This proves authenticated restored-instance
  writes, not full OAuth recovery on a replacement machine.
- The disposable container, copied data, reset token and temporary credential
  were removed. The work Tailscale account was restored.
- Remaining: full OAuth on a routed replacement instance, sequencer continuity,
  Cloudflare delivery-event ingestion and bounce acceptance. No public launch
  approval follows from these private tests.

The earlier configuration-only checks below retain their original limitations;
the dated acceptance results above supersede their pending status where stated.

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

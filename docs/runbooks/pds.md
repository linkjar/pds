# PDS provisioning and acceptance

Preparation for [linkjar/linkjar.io#90](https://github.com/linkjar/linkjar.io/issues/90).
The service has **not** been provisioned or accepted. A generated env file is an
input inventory, not evidence of a working account host. #98 owns the image,
#93 handle routing, #94 branding/hosting policy, #99 provider authentication,
#91 recovery drills and #92 relay ingestion/capacity.

## Collect the human-only inputs

Run from a trusted interactive terminal with Bash, Git and OpenSSL installed:

```sh
./scripts/provision-wizard.sh
```

The default output is `$HOME/.config/linkjar-pds/setup.env`. To choose another
private location, set `PDS_SETUP_FILE` to an absolute path outside any Git checkout.
The wizard uses hidden secret input, creates files with mode 0600, and preserves
existing values when you press Enter. It never sources the file, purchases a
host, changes DNS itself, uploads GitHub secrets or starts a container.

The ten stages collect host/DNS plans, R2, Resend mail, hCaptcha, online secrets
and offline recovery custody, Apple, Google, GitHub, telemetry/relay inventory,
and the deployment handoff. Provider/telemetry stages can be deferred and are
listed as incomplete. Re-running preserves generated PDS secrets; rotation must
be a deliberate operation with a backup and session-impact review.

The output is **raw Docker env-file data**, including literal dollars, hashes,
quotes and backslashes. Never `source` or `eval` it, pass it as Compose's global
`--env-file`, or paste it into an issue. `staging/compose.yml` uses `format: raw`
and therefore requires Docker Compose 2.30 or later. Use a separate image file
based on `staging/example.env` for Compose interpolation.

## Topology and installation handoff

The #89 decision targets an approved Hetzner CX33 in `nbg1`. Check current stock,
capacity and the full quote before purchase. Record the chosen host identifier,
IP addresses and billing owner in the private operations inventory.

```text
app.linkjar.io / linkjar.io (application pages and cookies)
                         ↓ OAuth
Cloudflare: pds.linkjar.social, *.linkjar.social
                         ↓ TLS, Full (strict)
Caddy on the origin      ↓ 127.0.0.1:3000
PDS container → /var/lib/linkjar-pds/staging + private R2 bucket
              → Resend SMTP, PLC, relay and existing Grafana
```

Install Docker/Compose and Caddy through the host's supported package procedure.
Keep port 3000 bound to loopback, permit origin HTTPS only from the intended
proxy path, and restrict SSH to operator access. Disable Watchtower; every image
change uses the tested GHCR digest from #98. Prepare the persistent directory
with ownership matching the pinned image's runtime user before starting it.

For origin TLS, use a certificate covering `pds.linkjar.social` and the wildcard.
Keep its private key in the host secret store and configure Caddy to load it.
A Cloudflare Origin CA certificate is suitable only behind Cloudflare; a public
ACME wildcard certificate needs DNS validation and a protected DNS credential.
Record which path was chosen and its renewal owner before publishing DNS.
Do not weaken Full (strict) to make a failing origin certificate work.

Add proxied A records for `pds` and `*`; add AAAA only for working public IPv6.
Import the reviewed [handle routing snippet](../../staging/Caddyfile.handles) into
the TLS site. For the host-installed Caddy topology above, set
`PDS_UPSTREAM=127.0.0.1:3000` in Caddy's service environment; its `pds:3000`
default is for Caddy on the Compose network. The canonical PDS host
must preserve protocol routes. Handle discovery must serve
`/.well-known/atproto-did`; handle pages must redirect to the DID profile without
serving app pages, app cookies or user-controlled blobs on handle hosts.

Transfer the collected env file through the approved secret channel to
`/etc/linkjar-pds/staging.env`, owner root, mode 0600. Keep the selected image
file separate and point `PDS_ENV_FILE` at that raw secrets file. Before starting,
validate without printing resolved secrets:

```sh
docker compose --env-file /etc/linkjar-pds/image.env -f staging/compose.yml config --quiet
```

The compose service intentionally contains only the PDS. Installing the proxy,
certificates, firewall, backup agent and monitoring is still required. Start it
only after these inputs and the pinned image digest have been reviewed.

## Environment and custody inventory

The committed inventory records names and intended locations only. Real vault
paths, owners and rotation dates belong in the private operations record. The
wizard's `LINKJAR_*` entries are operator notes; the PDS ignores them.

| Names | Source / storage |
| --- | --- |
| `PDS_HOSTNAME`, `PDS_SERVICE_HANDLE_DOMAINS`, `PDS_PORT`, `PDS_DATA_DIRECTORY` | Fixed topology; host env |
| `PDS_DEV_MODE`, `PDS_RATE_LIMITS_ENABLED`, `PDS_INVITE_REQUIRED` | false / true / false per D3; host env |
| `PDS_BLOBSTORE_S3_BUCKET`, `_ENDPOINT`, `_REGION`, `_FORCE_PATH_STYLE` | Private R2 bucket configuration; host env |
| `PDS_BLOBSTORE_S3_ACCESS_KEY_ID`, `_SECRET_ACCESS_KEY` | Bucket-scoped object read/write token; operator vault + host env |
| `PDS_BLOB_UPLOAD_LIMIT` | Initial 50 MiB ceiling; confirm archive requirements |
| `PDS_EMAIL_SMTP_URL`, `PDS_MODERATION_EMAIL_SMTP_URL` | Dedicated Resend key embedded in SMTPS URL; vault + host env |
| `PDS_EMAIL_FROM_ADDRESS`, `PDS_MODERATION_EMAIL_ADDRESS` | Verified sender / monitored mailbox; host env |
| `PDS_HCAPTCHA_SITE_KEY`, `_SECRET_KEY`, `_TOKEN_SALT` | Site-restricted hCaptcha credentials; secrets in vault + host env |
| `PDS_JWT_SECRET`, `PDS_DPOP_SECRET`, `PDS_ADMIN_PASSWORD` | Stable generated 32-byte secrets; vault + host env |
| `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` | Online PDS rotation key; protected host store + encrypted backup |
| `PDS_RECOVERY_DID_KEY` | Public counterpart of offline higher-priority recovery key; host env |
| `PDS_EXTERNAL_APPLE_CLIENT_ID`, `_CLIENT_SECRET` | #99 Services ID and signed client-secret JWT; vault + host env |
| `PDS_EXTERNAL_GOOGLE_CLIENT_ID`, `_CLIENT_SECRET` | #99 Web client; vault + host env |
| `PDS_EXTERNAL_GITHUB_CLIENT_ID`, `_CLIENT_SECRET` | #99 organization OAuth App; vault + host env |
| `OTEL_EXPORTER_OTLP_ENDPOINT`, `_HEADERS` | Existing Grafana collector and scoped authentication; vault + host env |
| `PDS_CRAWLERS` | Intended relay URLs; host env, then explicit crawl request |
| `LINKJAR_HOST_IPV4`, `LINKJAR_OFFLINE_RECOVERY_CUSTODY`, `LINKJAR_APPLE_KEY_CUSTODY`, `LINKJAR_APPLE_SECRET_EXPIRES` | Private operator inventory only |

Keep the **offline recovery private key off the PDS host**. Keep Apple's `.p8`
key in the operator vault; #99 consumes a signed JWT and needs renewal before
its expiration. A JWT is not the `.p8` contents. Provider pairs are optional only
as complete pairs; partial configuration must fail. Until #99 lands, these
settings do not enable external-provider routes.

Callback URLs are exactly
`https://pds.linkjar.social/oauth/external/{apple,google,github}/callback`, using
the literal provider name in place of the braces. Apple uses a Services ID
associated with the primary app and a form POST callback. Google uses a Web
application client. GitHub uses an OAuth App with only identity/email access.
Register the exact outgoing mail domain/address in Apple's Sign in with Apple
for Email Communication service, with aligned SPF/DKIM. Keep bounce notifications
enabled and test password-reset delivery to a Hide My Email address.
No provider is linked to an existing DID based on matching email.

#94 must supply the approved branding, support and published policy URLs, plus
`PDS_OAUTH_TRUSTED_CLIENTS` for the web, iOS and extension metadata URLs. Stock
trust extends sessions; consent handling remains a separate mechanism. Do not
label these values or the hosting policy complete from this wizard.

## Live acceptance record (all pending)

Record date, operator, image digest and redacted evidence for each check:

1. Public health and authorization metadata succeed through Cloudflare/Caddy;
   direct-origin port 3000 is unreachable. Inspect wildcard TLS and DNS.
2. Create a designated test account through a real web PAR with `prompt=create`.
   Verify the PLC document and reverse-checked `.linkjar.social` handle.
3. Upload, download and delete a test blob through the authenticated PDS and
   verify the R2 object lifecycle. Keep the bucket private and test limits.
4. Deliver and use a verification email; inspect Resend delivery evidence and
   sender SPF/DKIM. Exercise the email-signup hCaptcha and rate limits.
5. Verify handle discovery and profile redirect paths; no app cookies or user
   HTML is served on handle hosts. Test reserved names and custom handles (#93).
6. Verify Grafana receives metrics and the external uptime check alerts on a
   controlled failure. Never place authorization tokens in dashboard URLs.
7. Restore encrypted backups into an isolated replacement host: all shared and
   actor SQLite state, actor signing-key files under `actors/<hash>/<DID>/key`,
   `actors/reserved_keys`, operator config and online PLC material. Reconcile
   state and prove signed writes, blob reads and OAuth before traffic resumes.
   #91 owns the complete drill; SQLite replication alone is insufficient.
8. Request relay crawl with the supported pdsadmin command against the intended
   relay and confirm consumption. #92 owns capacity coordination; do not raise
   limits or contact relay operators implicitly.
9. After #99, exercise web and iOS Apple/Google/GitHub authorization with real
   credentials. Verify fresh signup versus existing sign-in, recorded consent,
   passwordless login restrictions, Hide My Email password-reset delivery and
   expired/invalid provider callbacks.

Keep #90 open until the required live account, R2, email, hCaptcha, custody and
runbook acceptance is recorded. This repository's tests cannot establish those
external results.

## Sources checked 2026-09-12

- [Pinned PDS environment names](https://github.com/bluesky-social/atproto/blob/7ca16cc6989f8247637615aca17c5abb911b8fb1/packages/pds/src/config/env.ts).
- [AT Protocol production guidance](https://atproto.com/guides/going-to-production).
- [Cloudflare DNS records](https://developers.cloudflare.com/dns/manage-dns-records/how-to/create-dns-records/) and [R2 tokens](https://developers.cloudflare.com/r2/api/tokens/).
- [Resend SMTP](https://resend.com/docs/send-with-smtp) and [hCaptcha](https://docs.hcaptcha.com/).
- [Apple web configuration](https://developer.apple.com/help/account/capabilities/configure-sign-in-with-apple-for-the-web/) , [private keys](https://developer.apple.com/help/account/capabilities/create-a-sign-in-with-apple-private-key/) and [private email relay](https://developer.apple.com/help/account/capabilities/configure-private-email-relay-service/).
- [Google Web clients](https://developers.google.com/identity/protocols/oauth2/web-server) and [GitHub OAuth Apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app).
- [Docker raw env files](https://docs.docker.com/reference/compose-file/services/#env_file).

## Local preparation checks

`bash -n` and ShellCheck pass. The shared wizard library matches its source
template unchanged. A disposable PDS-image container received a test env value
containing literal dollars, hashes, quotes and backslashes byte-for-byte through
Compose's raw env-file mode; the test container and network were removed.
The three existing pipeline checks pass. The interactive wizard itself was not
run end to end, and no real credential or external setup action was exercised.

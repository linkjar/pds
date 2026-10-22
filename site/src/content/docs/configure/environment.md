---
title: Environment variables
description: Every setting a self-hoster is likely to touch, grouped by what it does, with the reference server's defaults.
sidebar:
  order: 1
---

The server reads configuration from its process environment. Compose loads
`pds.env`; native systemd examples load `/etc/linkjar-pds/pds.env`. The binary
does not load either file itself. It uses the reference server's variable names.
The tables identify LinkJar defaults and safety restrictions; new settings
are at the end.

An invalid value stops the server at start with a message that names the
variable. An unknown `PDS_*` variable is a warning. `check-config` runs
the same checks without starting anything:

For native services, use the [CLI helper](/operate/cli/#native-services):

```bash
pds check-config
```

For Compose:

```bash
docker compose run --rm pds linkjar-pds check-config
```

## The server

| Variable | Meaning | Default |
|---|---|---|
| `PDS_HOSTNAME` | The server's public name. **Required.** | |
| `PDS_PORT` | The port the public listener binds. | `2583` natively; `3000` in the image |
| `PDS_SERVICE_DID` | The server's own DID. | `did:web:` and the hostname |
| `PDS_SERVICE_HANDLE_DOMAINS` | The suffixes of hosted handles, each with a leading dot, separated by commas. | `.` and the hostname |
| `PDS_VERSION` | Shown by the health route. | unset |
| `PDS_CONTACT_EMAIL_ADDRESS` | Shown by `describeServer`. | unset |
| `PDS_PRIVACY_POLICY_URL`, `PDS_TERMS_OF_SERVICE_URL` | Linked by `describeServer` and on the sign-up page. | unset |
| `PDS_HOME_URL`, `PDS_SUPPORT_URL` | Links on the OAuth pages. | unset |
| `PDS_DEV_MODE` | Relaxes checks for local development. Never on a public host. | `false` |

## Secrets

| Variable | Meaning |
|---|---|
| `PDS_JWT_SECRET` | Signs sessions. **Required.** Changing it signs everyone out. |
| `PDS_ADMIN_PASSWORD` | The password of the `admin` user for the admin API and invite codes. **Required.** |
| `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` | The key that signs PLC operations for new accounts and handle changes. Without it the server creates no account. Keep it: it is the control of every account's DID. |
| `PDS_DPOP_SECRET` | Signs the nonces of OAuth proofs. Optional; set it so that nonces survive a restart. |
| `PDS_RECOVERY_DID_KEY` | A `did:key` added to each new account's DID document as a further rotation key, for recovery with a key kept elsewhere. Optional. |

## Storage

| Variable | Meaning | Default |
|---|---|---|
| `PDS_DATA_DIRECTORY` | The data directory. Native examples use `/var/lib/linkjar-pds`; Compose mounts it at `/app/data`. | |
| `PDS_BLOBSTORE_DISK_LOCATION` | Where blobs go on disk, usually `blocks` under the data directory. | |
| `PDS_BLOBSTORE_S3_*` | An S3-compatible store in place of the disk. [Blob storage](/configure/blob-storage/) has the list. | |
| `PDS_BLOB_UPLOAD_LIMIT` | The largest blob, in bytes. The example file sets 100 MiB. | 5 MiB |
| `PDS_ACTOR_STORE_CACHE_SIZE` | How many accounts' databases stay open at once. | `100` |
| `PDS_MAX_REPO_IMPORT_SIZE` | The largest repository import body, in bytes. Must be positive; parsing uses additional memory. | 64 MiB (67,108,864 bytes) |
| `PDS_ACCEPTING_REPO_IMPORTS` | Whether accounts may import a repository, as in a migration from another server. | `true` |
| `PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT` | Hands SQLite checkpoints to a backup tool such as Litestream. | `false` |

## Identity

| Variable | Meaning | Default |
|---|---|---|
| `PDS_DID_PLC_URL` | The PLC directory where new DIDs are registered. | `https://plc.directory` |
| `PDS_HANDLE_BACKUP_NAMESERVERS` | Nameservers to ask when the system resolver cannot verify a handle. | unset |
| `PDS_ID_RESOLVER_TIMEOUT` | How long a handle or DID lookup may take, in milliseconds. | `3000` |
| `PDS_DID_CACHE_STALE_TTL`, `PDS_DID_CACHE_MAX_TTL` | How long a cached DID document is served before and after a refresh is due. | 1 hour, 1 day |

## The network

| Variable | Meaning | Usual value |
|---|---|---|
| `PDS_BSKY_APP_VIEW_URL`, `PDS_BSKY_APP_VIEW_DID` | The AppView that serves feeds, notifications and preferences for the Bluesky app. Without them the app cannot load its settings. | `https://api.bsky.app`, `did:web:api.bsky.app` |
| `PDS_REPORT_SERVICE_URL`, `PDS_REPORT_SERVICE_DID` | Where `createReport` is forwarded. | `https://mod.bsky.app`, `did:plc:ar7c4by46qjdydhdevvrndac` |
| `PDS_CRAWLERS` | The relays the server asks to crawl it, separated by commas. | `https://bsky.network` |
| `PDS_MAX_SUBSCRIPTION_BUFFER` | How many events a slow firehose subscriber may fall behind before the server drops it. | `500` |
| `PDS_REPO_BACKFILL_LIMIT_MS` | How far back a firehose subscriber may ask for events, in milliseconds. | 3 days |
| `PDS_PROXY_HEADERS_TIMEOUT` | Total upstream proxy deadline, including the response body, in milliseconds. | `10000` |
| `PDS_DISABLE_SSRF_PROTECTION` | Allows private OAuth metadata destinations only with `PDS_DEV_MODE=true`. Production startup rejects it. | `false` |

Untrusted DID, proxy, handle and client-metadata destinations must use
public HTTPS on port 443. The connector checks the actual destination
addresses, refuses redirects and ignores environment proxies. Configured
service URLs, including PLC and AppView, remain trusted and can use private
networks. Keep development mode off on a public host.

Response caps are fixed per operation: DID/PLC 512 KiB, OAuth metadata
256 KiB, MCP metadata 64 KiB, external handle files 4 KiB, and proxy errors
64 KiB. `PDS_FETCH_MAX_RESPONSE_SIZE` is not an implemented setting.

## Invites and limits

| Variable | Meaning | Default |
|---|---|---|
| `PDS_INVITE_REQUIRED` | Whether sign-up needs an invite code. | `true` in the example file |
| `PDS_INVITE_EPOCH`, `PDS_INVITE_INTERVAL` | The reference server's per-account invites: from the epoch on, each account earns one code per interval, in milliseconds. | unset, no codes earned |
| `PDS_RATE_LIMITS_ENABLED` | The reference server's rate limits, by client address and by account. | `true` |
| `PDS_RATE_LIMIT_BYPASS_KEY`, `PDS_RATE_LIMIT_BYPASS_IPS` | A header value, and addresses, that bypass the limits. | unset |
| `PDS_HCAPTCHA_SITE_KEY`, `PDS_HCAPTCHA_SECRET_KEY`, `PDS_HCAPTCHA_TOKEN_SALT` | hCaptcha on the sign-up page. All three or none. | unset |

The process also admits at most 8 uploads, 2 imports, 8 streamed exports,
32 firehose connections and 64 ordinary blocking jobs. Exhaustion returns
503 `ServerBusy`; a rate-limit bypass does not bypass these limits. Audit
writes use four separate workers and wait for capacity. These limits are
fixed in the implementation.

Request-body reads allow 15 seconds without progress and 300 seconds
total; expiry returns 408 `RequestTimeout`. Exports have the same total
deadline and a 15-second queue stall limit. Firehose sends may stall for
15 seconds; incoming frames and messages are capped at 64 KiB. Clients
must discard an incomplete export and retry later.
Refused bodies are drained for at most five seconds or the byte cap;
an oversized body still returns 413 if draining times out.

## Mail

| Variable | Meaning |
|---|---|
| `PDS_EMAIL_SMTP_URL` | The SMTP server, as a URL with the credentials. [Email](/configure/email/) has the shape. |
| `PDS_EMAIL_FROM_ADDRESS` | The sender of the server's mail. |
| `PDS_MODERATION_EMAIL_SMTP_URL`, `PDS_MODERATION_EMAIL_ADDRESS` | The sender of mail a moderation service sends to accounts through the admin API. Optional. |

## Branding

`PDS_LOGO_URL`, `PDS_PRIMARY_COLOR`, `PDS_ERROR_COLOR`, `PDS_WARNING_COLOR`,
`PDS_SUCCESS_COLOR`, `PDS_INFO_COLOR`, `PDS_BACKGROUND_LIGHT_URL` and
`PDS_BACKGROUND_DARK_URL` style the sign-in and sign-up pages, as on the
reference server.

## New in LinkJar PDS

| Variable | Meaning | Default |
|---|---|---|
| `PDS_PROFILE` | `stock` for the reference server's behaviour, `linkjar` for LinkJar's sign-in providers and policies. [Profiles](/configure/profiles/). | `stock` |
| `PDS_METRICS_ADDRESS` | The private listener for Prometheus metrics. [Logs, health and metrics](/operate/monitoring/). | unset, no listener |
| `PDS_OPS_ADDRESS`, `PDS_OPS_CLIENT_IDS` | The private listener of the operations console, and the console's client id. [Console and MCP server](/scale/sidecars/). | unset |
| `PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX` | The key that signs audit checkpoints. [Audit log](/operate/audit-log/). | unset, checkpoints unsigned |
| `PDS_AUDIT_CHECKPOINT_REPO` | The DID of the account on this server that publishes the checkpoints. | unset |
| `PDS_POSTGRES_URL`, `PDS_POSTGRES_SCHEMA_PREFIX` | The shared databases on Postgres instead of SQLite files. [Postgres](/scale/postgres/). | unset, `pds` |
| `PDS_ENTRYWAY_*`, `PDS_MEMBER_*` | A member host, or an entryway. [Entryway](/scale/entryway/). | unset |
| `PDS_MCP_CLIENT_IDS`, `PDS_OAUTH_KNOWN_CLIENTS_FILE` | The MCP servers of this server, and OAuth clients known without a fetch. | unset |
| `LOG_LEVEL` | The log filter: `info`, `debug`, or a filter such as `info,pds_xrpc=debug`. | `info` |

The [specification's Appendix C](https://github.com/linkjar/pds/blob/main/docs/SPEC.md#appendix-c-configuration)
lists every accepted variable, including the ones this page leaves out.

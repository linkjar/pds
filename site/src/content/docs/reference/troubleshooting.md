---
title: Troubleshooting
description: What goes wrong most, what it looks like, and what fixes it.
sidebar:
  order: 2
---

| You see | Cause | Fix |
|---|---|---|
| `check-config` names a variable | The value is missing or malformed. | Fix the setting in the runtime environment file and run the check again. The message says what shape it expects. |
| `docker-compose: command not found`, or an error about the Compose file's version | Compose version 1. | Install the Compose plugin and use `docker compose`, with a space. |
| The server exits at start and the log says another server holds the lock | Another process still runs, or the lock file is left from a copy. | Check `systemctl status linkjar-pds` or `docker compose ps`; stop the process. A stale `linkjar-pds.lock` in a copied directory can be removed when no server runs there. |
| `curl` to the server fails with a certificate error on the first request | Caddy fetches the certificate on the first connection, which takes a few seconds. | Try again. If it keeps failing, DNS does not point at the host, or port 80 or 443 is closed. |
| A handle's name gets no certificate | The wildcard record is missing, or the handle is not an active account. | Check `dig +short anything.pds.example.com`, and the `/tls-check` route in [the checks](/start/verify/#the-certificate-guard). |
| The Bluesky app signs in but cannot load settings, and the log shows `getPreferences` failing | `PDS_BSKY_APP_VIEW_URL` and `PDS_BSKY_APP_VIEW_DID` are unset. | Set both, as in the example file, and restart. |
| Sign-up fails with an invite code error | `PDS_INVITE_REQUIRED` is on and the code is wrong or used up. | Create a code with the [admin API](/operate/accounts/#invite-codes). |
| Sign-up fails and the log mentions the rotation key | `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` is unset. | Generate one as in [the install](/start/install/) and restart. Without it the server creates no account. |
| Posts do not appear for people on other servers | The relay has not crawled the host yet, or cannot reach it. | Wait for the first write after a start, then check `getHostStatus` as in [the first account](/start/first-account/#when-posts-reach-other-people). The firehose must be reachable at `wss://pds.example.com`. |
| Password reset mail never arrives | SMTP is not set, or the URL is wrong. | [Email](/configure/email/). A wrong URL shows in the log of the request. |
| The firehose subscriber count stays at 0 | The relay is not connected. | Same as the posts above. A proxy in front of DNS that drops long WebSockets is a common cause. |
| `verify` prints differences after a restore | The backup may be inconsistent, or blobs may be missing. | Restore a tested cold backup or consistent snapshot, including its WAL and external storage. Copying live files separately is insufficient. |
| A second host started on a copy of the directory | Two servers served one set of accounts. | Stop one at once. The relay and the apps saw two histories; the one that keeps running is the truth, and the other's writes since the copy are lost. |

For native services, read `sudo journalctl -u linkjar-pds`; for Compose, read
`docker compose logs pds`. Set `LOG_LEVEL=debug` and restart for more detail.
A foreground native binary needs its settings exported; a nearby `.env` file
is not loaded automatically. Check CA certificates if outbound HTTPS fails.

Open an issue at [github.com/linkjar/pds](https://github.com/linkjar/pds/issues)
with the relevant log lines, installation method, and source commit. Review
logs and configuration excerpts for private information before sharing them.

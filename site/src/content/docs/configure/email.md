---
title: Email
description: The mail the server sends, the SMTP URL it needs, and how to test it.
sidebar:
  order: 2
---

The server sends mail for five things: to confirm an email address, to
reset a password, to change the address, to delete the account, and to
sign a PLC operation. Each mail carries a token, and the flow waits for
it.

Without SMTP the server sends nothing. A server of one can go without;
before you invite other people, set it up, or nobody can reset a
forgotten password.

## Set the SMTP URL

Add these settings to your runtime environment file:

```ini
PDS_EMAIL_SMTP_URL=smtps://user:password@smtp.example.com/
PDS_EMAIL_FROM_ADDRESS=pds@example.com
```

The URL names the scheme, the credentials and the host. `smtps://` is
TLS on port 465; `smtp://` on port 587 upgrades to TLS. Add a port after
the host when the provider uses another one.

:::caution[Percent-encode the password]
A password with a character that means something in a URL, such as `@`,
`:`, `/` or `#`, must be percent-encoded. `p@ss` becomes `p%40ss`.
:::

A sender address at the server's own domain, or at a domain whose SPF and
DKIM records name the provider, keeps the mail out of spam folders.

Restart after the change. For NixOS or the native systemd example:

```bash
sudo systemctl restart linkjar-pds
```

For Compose:

```bash
docker compose up -d
```

## Test it

Ask for a password reset for an account whose address you can read:

```bash
curl -s -H 'content-type: application/json' -d '{"email":"alice@example.com"}' https://pds.example.com/xrpc/com.atproto.server.requestPasswordReset
```

The answer is empty with status 200, and the mail arrives within a
minute. A mail that gates a flow is sent before the request is answered,
so an SMTP failure fails the request, with the reason in the logs:

For a native service, use `sudo journalctl -u linkjar-pds`. For Compose,
use `docker compose logs pds`. See [monitoring](/operate/monitoring/#logs).

## Mail from a moderation service

A moderation service can send mail to an account through the admin API.
It goes out with `PDS_MODERATION_EMAIL_SMTP_URL` and
`PDS_MODERATION_EMAIL_ADDRESS` when they are set, and with the server's
own sender otherwise.

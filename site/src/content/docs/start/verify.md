---
title: Check that it works
description: Four requests from outside the host, each with the answer to expect, and the smoke test that makes them all.
sidebar:
  order: 6
---

Run these from your workstation, not from the host, so that DNS, the
firewall and the certificate are all in the path.

## The health route

```bash
curl -s https://pds.example.com/xrpc/_health
```

It answers `{}` with status 200, or `{"version":"…"}` when `PDS_VERSION`
is set. A 503 with `"error":"Service Unavailable"` means the account
database did not answer: read the logs.

## What the server says about itself

```bash
curl -s https://pds.example.com/xrpc/com.atproto.server.describeServer
```

The answer names the server's DID, `did:web:pds.example.com`, lists the
handle suffix `.pds.example.com` under `availableUserDomains`, and says
`"inviteCodeRequired":true`.

## The OAuth metadata

```bash
curl -s https://pds.example.com/.well-known/oauth-authorization-server
```

A JSON document whose `issuer` is `https://pds.example.com`. Apps that
sign in with OAuth, the Bluesky app among them, read it first.

## The certificate guard

The supplied Compose and native Caddy configurations ask the server before
requesting a certificate for a name, so
that only the server's name and the handles of its accounts get one. Ask
the same question:

```bash
curl -s "https://pds.example.com/tls-check?domain=pds.example.com"
```

It answers `{"success":true}`. A name that nobody hosts gets a 404, and a
stranger who points a name at your host gets no certificate from it.

## Everything at once

The repository's smoke test makes the requests above, subscribes to the
firehose, and reports one line per check. It needs Node 24 and pnpm on
your workstation.

```bash
cd pds/parity && pnpm install
```

```bash
pnpm parity smoke https://pds.example.com
```

Every line starts with `ok`, and the last says how many checks passed.

With `--write` and `PDS_ADMIN_PASSWORD` in the environment, the test also
creates an account, posts, uploads an image, makes an app password, and
deletes the account again. The account's DID stays in the PLC directory
after the deletion, as every DID does.

```bash
read -s PDS_ADMIN_PASSWORD && export PDS_ADMIN_PASSWORD
```

```bash
pnpm parity smoke https://pds.example.com --write
```

Next: [create the first account](/start/first-account/).

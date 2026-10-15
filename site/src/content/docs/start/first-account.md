---
title: Create the first account
description: An invite code, the Bluesky app, the first post, and when the rest of the network sees it.
sidebar:
  order: 5
---

Sign-up needs an invite code, so that strangers cannot fill the server.
The admin password creates codes.

## Create an invite code

On the host, with `PDS_ADMIN_PASSWORD` from `pds.env` in the environment:

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"useCount": 1}' https://pds.example.com/xrpc/com.atproto.server.createInviteCode
```

It answers `{"code":"pds-example-com-xxxxx-xxxxx"}`. A code with a
`useCount` of 5 admits five accounts.

## Sign up from the Bluesky app

1. Open the app and choose **Create account**.
2. Under **Hosting provider**, choose **Custom** and enter
   `https://pds.example.com`.
3. Enter the invite code, an email address, a password and a handle. The
   handle becomes `alice.pds.example.com`.
4. Post something.

The first request for `alice.pds.example.com` makes Caddy fetch its
certificate, which takes a few seconds.

## Or sign up from the terminal

```bash
curl -s -H 'content-type: application/json' -d '{"email":"alice@example.com","handle":"alice.pds.example.com","password":"…","inviteCode":"pds-example-com-xxxxx-xxxxx"}' https://pds.example.com/xrpc/com.atproto.server.createAccount
```

The answer holds the account's `did`, its `handle` and a session. Sign in
with the handle and the password from any app.

## Check the handle

```bash
curl -s https://alice.pds.example.com/.well-known/atproto-did
```

It prints the account's DID, `did:plc:…`. Apps resolve handles this way.

## When posts reach other people

A post appears in other people's timelines once the relay at
`bsky.network` crawls the host. The server asks it to with the first write
after a start, and then at most once in twenty minutes. The relay's own
status shows the connection:

```bash
curl -s "https://bsky.network/xrpc/com.atproto.sync.getHostStatus?hostname=pds.example.com"
```

The answer has `"status":"active"` and a `seq` that moves with each
write.

:::tip[Keep invites on]
`PDS_INVITE_REQUIRED=false` opens sign-up to anyone who finds the server.
Leave it on and hand out codes; [accounts and invites](/operate/accounts/)
has the rest of the invite commands.
:::

Next: set up [email](/configure/email/) so that confirmation and
password-reset mail arrives, and [back up](/operate/backup/) the data
directory and `pds.env`.

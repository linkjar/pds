---
title: Accounts, invites and moderation
description: "The admin API with the admin password: invite codes, account details, password and email changes, takedowns and deletion."
sidebar:
  order: 4
---

The admin API is the reference server's. Every call authenticates with
HTTP basic auth as `admin` with `PDS_ADMIN_PASSWORD`, and every call is an
entry in the [audit log](/operate/audit-log/).

The commands below assume `PDS_ADMIN_PASSWORD` in the environment:

```bash
read -s PDS_ADMIN_PASSWORD && export PDS_ADMIN_PASSWORD
```

## Invite codes

One code for one account:

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"useCount": 1}' https://pds.example.com/xrpc/com.atproto.server.createInviteCode
```

Ten codes of five uses each:

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"codeCount": 10, "useCount": 5}' https://pds.example.com/xrpc/com.atproto.server.createInviteCodes
```

Disable a code that leaked:

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"codes": ["pds-example-com-xxxxx-xxxxx"]}' https://pds.example.com/xrpc/com.atproto.admin.disableInviteCodes
```

`PDS_INVITE_REQUIRED=false` opens sign-up to everyone. Leave it on unless
you want a public server, and then set up hCaptcha and watch the sign-ups.

## Look up an account

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" "https://pds.example.com/xrpc/com.atproto.admin.getAccountInfo?did=did:plc:…"
```

The answer has the handle, the email address and whether it is confirmed,
the invite code the account used, and the codes it has.

## Reset a password or change an address

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"did": "did:plc:…", "password": "…"}' https://pds.example.com/xrpc/com.atproto.admin.updateAccountPassword
```

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"account": "did:plc:…", "email": "alice@example.net"}' https://pds.example.com/xrpc/com.atproto.admin.updateAccountEmail
```

A person who still has access resets their own password from the app,
through [email](/configure/email/).

## Take an account down

A takedown hides the repository and quarantines its blobs. The account
cannot sign in; its data stays until you lift the takedown or delete the
account.

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"subject": {"$type": "com.atproto.admin.defs#repoRef", "did": "did:plc:…"}, "takedown": {"applied": true, "ref": "spam"}}' https://pds.example.com/xrpc/com.atproto.admin.updateSubjectStatus
```

Lift it with `"applied": false`. The same method takes down one record,
with a `com.atproto.repo.strongRef` subject, or one blob.

## Delete an account

```bash
curl -s -u "admin:$PDS_ADMIN_PASSWORD" -H 'content-type: application/json' -d '{"did": "did:plc:…"}' https://pds.example.com/xrpc/com.atproto.admin.deleteAccount
```

The repository, the blobs and the sessions are removed. The DID stays in
the PLC directory, as every DID does; the account can move to another
server later with the same DID. A person deletes their own account from
the app, with a token sent by mail.

## Reports

A report from the app goes to the moderation service named by
`PDS_REPORT_SERVICE_URL`, Bluesky's by default. The server keeps no
report queue of its own.

## A console

The [operations console](/scale/sidecars/) does the above from a page,
with a reason recorded for each action, and shows the live firehose
subscribers and the audit log.

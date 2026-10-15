---
title: Entryway and member host
description: Split one server into an entryway that keeps the accounts and a member host that keeps the repositories, without changing a DID document.
sidebar:
  order: 2
---

An entryway is the way to grow past one host. It keeps the accounts, the
sessions, the OAuth server and the PLC rotation key, and routes everything
else to a member host, which keeps the repositories, the blobs, the
signing keys and the firehose. The old server becomes the member host and
keeps its address, so no DID document changes.

Today an entryway serves one member host. Rehearse the split on a copy
first, and tell the people on the server before the day: every OAuth
client must sign in again, because the member host is no longer an
authorization server.

## What to know first

- `PDS_ENTRYWAY_URL` of the member host is the public address of the
  entryway. OAuth clients read it, and tokens name it as their issuer.
- Sessions with a password or an app password carry on: the client
  refreshes once and gets tokens of the entryway. An OAuth session that
  the entryway ends is refused at the member host when its access token
  expires, at most 60 minutes later.
- The entryway passes uploads, exports and the firehose through to the
  member host as streams. Only the member host asks the relay to crawl it.
- A member host answers no handle. `/.well-known/atproto-did` on the
  handle names must reach the entryway, so the wildcard DNS and the
  Caddyfile move to it.

## Prepare

1. Choose the entryway's name, such as `entryway.example.com`, and point
   it at the new host.
2. Create the key of the tokens, 32 random bytes as 64 hexadecimal
   digits:

   ```bash
   openssl rand -hex 32
   ```

   It goes into the entryway's secrets. The member host needs the public
   key, which the entryway logs at start as
   `jwt_verify_key_k256_public_key_hex`, with its DID.
3. Note the `did:key` of the old server's PLC rotation key. The entryway
   takes the private key; the member host gets the `did:key`.

## Split

1. Stop the old server.
2. Copy `account.sqlite` from its data directory to the entryway's.
3. Configure the entryway with the old server's variables, and:

   | Variable | Value |
   |---|---|
   | `PDS_HOSTNAME` | the entryway's name |
   | `PDS_JWT_SECRET` | the old server's; it takes the refresh tokens from before |
   | `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` | the old server's |
   | `PDS_MEMBER_URL` | the old server's public address |
   | `PDS_MEMBER_INTERNAL_URL` | where the entryway reaches it |
   | `PDS_MEMBER_ADMIN_PASSWORD` | the old server's admin password |
   | `PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX` | the new key |

4. Configure the old server as the member host. Keep its variables, and:

   | Variable | Value |
   |---|---|
   | `PDS_ENTRYWAY_URL` | the entryway's address |
   | `PDS_ENTRYWAY_DID` | `did:web:` and the entryway's name |
   | `PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX` | the public key |
   | `PDS_ENTRYWAY_PLC_ROTATION_KEY` | the `did:key` of the rotation key |
   | `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` | remove it |
   | `PDS_JWT_SECRET` | keep it; the member host answers `ExpiredToken` to a token from before, and the client refreshes |

5. Send `/.well-known/atproto-did` of the handle names to the entryway.
6. Start the entryway, then the member host.

## Check

1. Sign in at the member host's address with a password. The entryway
   answers, and the session writes a record.
2. Refresh a session from before the split.
3. Create an account at the entryway. Its DID document names the member
   host.
4. Change a handle. Both servers show it.

## Reconcile

A crash during a sign-up can leave a repository on the member host with no
account at the entryway. At the entryway:

```bash
docker compose exec pds linkjar-pds reconcile
```

It lists each repository without an account and each account without a
repository, and exits 1 when it finds one. Run it again with
`--remove-orphans` to remove the repositories without an account; one
with a commit in the last ten minutes stays, because it may be a sign-up
in progress.

## Back to one server

Stop both. Remove the `PDS_ENTRYWAY_*` variables from the old server and
give it its rotation key back. Copy `account.sqlite` from the entryway
back: it has the accounts and the sessions since the split. A repository
the member host created since then is in its directory already. Start the
old server; clients refresh once more.

The [runbook](https://github.com/linkjar/pds/blob/main/docs/runbooks/entryway.md)
in the repository has the same procedure with the specification's
references.

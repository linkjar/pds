# A single node becomes an entryway and a member host

[SPEC §21.3](../SPEC.md#213-entryway-and-member-hosts) · [Unit record](../research/2026-10-14-entryway.md) · [Plan](../plan.md)

This runbook splits one server into two: an entryway that keeps the
accounts, and a member host that keeps the repositories. The old server
becomes the member host and keeps its address, so no DID document
changes.

Rehearse this on staging first. An OAuth client must be authorized
again after the split: the member host is no authorization server, and a
session of the old server has no token of the entryway. Tell the people
who use the apps before the day.

## 1. What to know first

- The entryway serves one member host.
- `PDS_ENTRYWAY_URL` of the member host is the public address of the
  entryway: OAuth clients read it, and tokens name it as their issuer.
- An OAuth session that the entryway ends is refused at the member host
  when its access token ends, at most 60 minutes later.
- Sessions with a password and with an app password go on: the client
  refreshes once and gets tokens of the entryway.
- The entryway passes uploads, exports and the firehose on to the member
  host as streams. Only the member host asks the relay to crawl it.
- A member host passes `activateAccount` and `deactivateAccount` on to
  the entryway. The entryway keeps the status and asks the member host to
  announce it.
- A member host answers no handle: `/.well-known/atproto-did` on a
  handle host must reach the entryway.
- The entryway limits requests by the address of the client. A member
  host sends that address in `X-Forwarded-For`, and the entryway takes it
  only from a private address. When the member host reaches the entryway
  over the internet, the limits count the member host as one client.

## 2. Prepare

1. Choose the host name of the entryway, such as `entryway.example.com`,
   and point it at the new server.
2. Create the key of the tokens: 32 random bytes as 64 hexadecimal
   digits.

   ```bash
   openssl rand -hex 32
   ```

   Keep it in the secret store of the entryway. The member host needs
   the public key. The entryway logs it at start, as
   `jwt_verify_key_k256_public_key_hex`, with its DID.
3. Note the `did:key` of the PLC rotation key of the old server. The
   entryway takes the private key; the member host gets the `did:key`.

## 3. Split

1. Stop the old server.
2. Copy `account.sqlite` of the old data directory to the data directory
   of the entryway. With the shared tier on Postgres, copy the
   `<prefix>_account` schema.
3. Configure the entryway with the variables of the old server, and:
   - `PDS_HOSTNAME`: the host name of the entryway.
   - `PDS_JWT_SECRET`: the secret of the old server. The entryway takes
     the refresh tokens from before with it.
   - `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`: the key of the old
     server.
   - `PDS_MEMBER_URL`: the public address of the old server.
   - `PDS_MEMBER_INTERNAL_URL`: where the entryway reaches it.
   - `PDS_MEMBER_ADMIN_PASSWORD`: the admin password of the old server.
   - `PDS_ENTRYWAY_JWT_SIGNING_KEY_K256_PRIVATE_KEY_HEX`: the new key.
4. Configure the old server as the member host. Keep its variables, and:
   - `PDS_ENTRYWAY_URL`: the address of the entryway.
   - `PDS_ENTRYWAY_DID`: `did:web:` and the host name of the entryway.
   - `PDS_ENTRYWAY_JWT_VERIFY_KEY_K256_PUBLIC_KEY_HEX`: the public key.
   - `PDS_ENTRYWAY_PLC_ROTATION_KEY`: the `did:key` of the rotation key.
   - Remove `PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX`.
   - Keep `PDS_JWT_SECRET`: the member host answers `ExpiredToken` to a
     token from before, and the client then refreshes.
5. Send `/.well-known/atproto-did` of the handle hosts to the entryway.
   In `legacy/staging/Caddyfile.handles` this is the `@did` route.
6. Start the entryway, then the member host.

## 4. Check

1. Sign in at the address of the member host with a password. The
   entryway answers, and the session writes a record.
2. Refresh a session from before the split.
3. Create an account at the entryway. Its DID document names the member
   host.
4. Change a handle. Both servers show it.

## 5. Reconcile

A crash during a sign-up can leave a repository on the member host
without an account at the entryway. At the entryway, run:

```bash
linkjar-pds reconcile
```

It lists each repository without an account and each account without a
repository, and exits 1 when it finds one. To remove the repositories
without an account, run it again with `--remove-orphans`. A repository
with a commit of the last ten minutes stays: it may be a sign-up in
progress.

## 6. Back to one server

Stop both. Remove the `PDS_ENTRYWAY_*` variables from the old server and
give it its rotation key again. Copy `account.sqlite` of the entryway
back: it has the accounts and the sessions since the split. A repository
that the member host created since then is in its data directory
already. Start the old server. Clients refresh once more: tokens of the
entryway end at a server that has no entryway.

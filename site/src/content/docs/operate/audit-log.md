---
title: Audit log
description: What the server records about accounts and operators, how checkpoints make the record tamper-evident, and how to verify a bundle offline.
sidebar:
  order: 6
---

Every action on an account or by an operator is an entry in a log the
server cannot rewrite without it showing: a sign-up, a password or email
change, a handle change, a deactivation, a deletion, a takedown, an invite
created or disabled, and every call to the admin API, with its outcome.
Ordinary reads and ordinary writes to a repository are not in it:
repository data is already signed, and reads are a traffic log.

Each entry carries the hash of the one before it, and the store refuses
updates and deletes. Every five minutes, and at shutdown, the server
builds a Merkle tree over the new entries and signs a checkpoint over its
root. A checkpoint published into an account's repository travels the
firehose, so a copy exists outside the host.

## Turn on signing

Without a key the log chains its entries and signs nothing. Create a key,
32 random bytes as 64 hexadecimal digits:

```bash
openssl rand -hex 32
```

In the runtime environment file:

```ini
PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX=…
PDS_AUDIT_CHECKPOINT_REPO=did:plc:…
```

The repository is your own account on the server. Each checkpoint becomes
a record in it, under `io.linkjar.pds.audit.checkpoint`. Without
`PDS_AUDIT_CHECKPOINT_REPO` the checkpoints stay in `audit.sqlite`, and
nobody outside can compare them.

A key that differs from the one of the last checkpoint is itself logged
before it signs, so a key change is visible in the record.

## Seal history

A data directory that existed before the log, one the reference server
wrote, has history the log did not see. Seal it once, before the first
start of the server on it:

For native services, stop `linkjar-pds` and use the
[CLI helper](/operate/cli/#native-services):

```bash
pds audit seal
```

For Compose, stop the main container first:

```bash
docker compose run --rm pds linkjar-pds audit seal
```

It prints `sealed <n> accounts as sha256:…` and `signed by ed25519:…`.
Keep both lines. The command is refused on a log that has entries.

## Export and verify

For native services:

```bash
umask 077
pds audit export > audit-bundle.json
```

For Compose:

```bash
umask 077
docker compose exec -T pds linkjar-pds audit export > audit-bundle.json
```

The bundle holds the entries, the chain hashes, the inclusion proofs, the
checkpoints that cover them and the public keys. Verify it away from the
host, with any build of the binary:

```bash
linkjar-pds audit verify audit-bundle.json
```

It recomputes the chain, the trees and the signatures and exits 0, or
names the first entry that does not fit and exits 1. The verifier shares
nothing with the writer but the entry format.

## Watch it

Two metrics say whether signing works:
`pds_audit_unsigned_entries` returns to 0 within five minutes of an entry,
and `pds_audit_checkpoint_age_seconds` stays under 600 while there are
unsigned entries. [Logs, health and metrics](/operate/monitoring/) has
the listener.

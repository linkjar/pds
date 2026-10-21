---
title: Back up and restore
description: What is in the data directory, what must be kept, a cold backup in three commands, and the restore.
sidebar:
  order: 2
---

Two things hold everything: the data directory, and `pds.env`. Lose the
directory and the accounts are gone. Lose `pds.env` and you lose the
rotation key, which is the control of every account's DID, and the JWT
secret, which signs every session.

## The data directory

| Path | Content | Keep |
|---|---|---|
| `account.sqlite` | accounts, passwords, sessions, invites, OAuth state, credential recovery epochs | yes |
| `sequencer.sqlite` | the event log the firehose serves | yes |
| `did_cache.sqlite` | cached DID documents | no, rebuilt |
| `audit.sqlite` | the [audit log](/operate/audit-log/) | yes |
| `ops.sqlite` | commits in flight, operator roles, blocked domains | yes |
| `actors/<xx>/<did>/store.sqlite` | an account's repository, records and blob index | yes |
| `actors/<xx>/<did>/key` | the account's signing key | yes |
| `actors/reserved_keys/` | signing keys of sign-ups in progress | no |
| `blocks/<did>/<cid>` | blobs | yes |
| `tempt/`, `quarantine/` | uploads in progress; blobs of taken-down accounts | quarantine, yes |
| `linkjar-pds.lock` | held by the running server | no |

Each SQLite file has a `-wal` and a `-shm` sibling while the server runs.
A copy made while the server runs must take them too, or it may miss the
last writes. A cold backup avoids the question.

Keep the whole account database, including its `credential_epoch` table.
That table prevents tokens issued before password recovery from working
again. A restore of an older backup also restores its older credentials,
sessions and recovery state. If those credentials were compromised,
repeat recovery before reopening the server. On
[Postgres](/scale/postgres/), the table is part of the account schema.

## A cold backup

Stop the server, copy, start. The pause is a few seconds; Caddy keeps
answering, with a 502, meanwhile.

```bash
docker compose stop pds
```

```bash
sudo tar -czf ../pds-backup-$(date +%F).tar.gz data pds.env .env
```

```bash
docker compose start pds
```

Move the archive off the host. It holds the secrets of `pds.env`, so
treat it as a secret.

## Continuous replication

LinkJar's own hosts replicate the SQLite files with
[Litestream](https://litestream.io) to object storage and sync the blobs
beside them. With Litestream in charge of checkpoints, set
`PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT=true`. Rehearse the restore before
you rely on it.

## One account

`export-car` writes an account's repository as a CAR file, the format
every AT Protocol tool reads and the one an account takes to another
server:

```bash
docker compose exec -T pds linkjar-pds export-car did:plc:… > alice.car
```

Blobs are not in the CAR; they are the files under `blocks/<did>/`.

## Restore

On a fresh host with the Compose files in place:

1. Unpack the archive where the Compose file is, and give the data
   directory to the server's user:

   ```bash
   sudo chown -R 1000:1000 data
   ```

2. Check the directory. `verify` reads every account, compares each
   repository with its records and blobs, and prints a summary that ends
   with `differences 0`:

   ```bash
   docker compose run --rm pds linkjar-pds verify
   ```

3. Start the server, point DNS at the new host, and
   [check it from outside](/start/verify/).

The relay resumes at its cursor. A restore from a backup older than the
last events the relay saw leaves the relay ahead of the server; the relay
then re-crawls the repositories at their next commit.

:::danger[One server per data directory]
Never run two servers on one data directory, on one host or two. The
server takes `linkjar-pds.lock` at start and refuses to start when another
holds it, but a copy on a second host has its own lock file.
:::

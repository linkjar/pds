---
title: Back up and restore
description: Back up native and Docker installations, preserve authentication state, and restore with the correct paths and service user.
sidebar:
  order: 2
---

Keep the complete data directory, all blobs, configuration, and every secret.
The Compose deployment uses `data/` and `pds.env`. The native examples use
`/var/lib/linkjar-pds` and `/etc/linkjar-pds/pds.env`. Include any storage or key
files configured outside those paths, and record the exact source commit or
image digest. With S3 or Postgres, back up those stores at the same consistent
point too; a local archive cannot include remote data.

The PLC rotation key controls the accounts' DIDs. The JWT secret signs sessions.
For secrets provisioned under `/run/secrets`, preserve their encrypted source
and recovery keys so you can reconstruct the runtime file on a fresh host.

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

SQLite can keep recent writes in `-wal` files while the server runs.
Copying live database, WAL, and SHM files individually does not ensure a
consistent backup. Use a cold backup, a coordinated filesystem snapshot,
or a supported SQLite backup tool. Do not omit WAL files from a snapshot.

Keep the whole account database, including its `credential_epoch` table.
That table prevents tokens issued before password recovery from working
again. A restore of an older backup also restores its older credentials,
sessions and recovery state. If those credentials were compromised,
repeat recovery before reopening the server. On
[Postgres](/scale/postgres/), the table is part of the account schema.

## A cold backup

Stop the server, archive its state and configuration, then start it.
The TLS proxy can return an upstream error during the pause. Keep the server
stopped until the archive has completed successfully.

### Native and NixOS examples

For the paths in [native installation](/start/native/):

```bash
sudo systemctl stop linkjar-pds &&
sudo sh -c 'umask 077; tar -czf /root/linkjar-pds-backup-$(date +%F-%H%M%S).tar.gz -C / var/lib/linkjar-pds etc/linkjar-pds' &&
sudo systemctl start linkjar-pds
```

The archive is root-readable only and contains the data plus the runtime file
under `/etc/linkjar-pds`. If you use another secret path, also back up its
recoverable source. Preserve the host flake and lockfile or the systemd unit,
TLS proxy configuration, and deployment record. Keep a recoverable copy of the
selected binary or its pinned build inputs.

### Docker Compose

From the directory containing `compose.yml`:

```bash
docker compose stop pds &&
sudo sh -c 'umask 077; tar -czf /root/linkjar-pds-backup-$(date +%F-%H%M%S).tar.gz data pds.env .env compose.yml Caddyfile' &&
docker compose start pds
```

Move the archive to protected storage off the host. Encrypt backups and restrict
access: they contain account data, passwords, signing keys, and session state.
Rehearse a restore before relying on them.

## Continuous replication

LinkJar's own hosts replicate the SQLite files with
[Litestream](https://litestream.io) to object storage and sync the blobs
beside them. With Litestream in charge of checkpoints, set
`PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT=true`. Rehearse the restore before
you rely on it.

## One account

`export-car` writes an account's repository as a CAR file, the format
every AT Protocol tool reads and the one an account takes to another
server.

For native services, use the [command helper](/operate/cli/#native-services):

```bash
umask 077
pds export-car did:plc:REPLACE_WITH_ACCOUNT_DID > alice.car
```

For Docker:

```bash
umask 077
docker compose exec -T pds linkjar-pds export-car did:plc:REPLACE_WITH_ACCOUNT_DID > alice.car
```

Blobs are not in the CAR; they are the files under `blocks/<did>/`.

## Restore

Restore on a private host with the PDS service stopped. Install the reviewed
build and create its service user before restoring data. Inspect the archive
paths, restore them to their intended locations, and keep the secret file
restricted to root. Restore external blobs and databases consistently too.

### Native or NixOS

Restore the native data to `/var/lib/linkjar-pds` and the runtime file to
`/etc/linkjar-pds/pds.env`, or reconstruct it through your secret manager.
Do not start an empty NixOS service against the public hostname during recovery.
After restoring, check ownership and run the [native helper](/operate/cli/#native-services):

```bash
sudo chown -R linkjar-pds:linkjar-pds /var/lib/linkjar-pds
sudo chmod 0700 /var/lib/linkjar-pds
sudo chmod 0600 /etc/linkjar-pds/pds.env
pds check-config
pds verify
```

Keep native paths in the environment. A restore from Docker needs `/app/data`
paths changed to their host equivalents. Do not generate new account secrets.

### Docker Compose

Unpack the Compose backup into its deployment directory. Give the restored data
to the container's user and check it before starting:

```bash
sudo chown -R 1000:1000 data
docker compose run --rm pds linkjar-pds verify
```

For either installation, `verify` reads every repository and compares its records
and blobs. Expect `differences 0`. Resolve differences before reopening traffic.
Start the selected service, move DNS if needed, and
[check it from outside](/start/verify/).

The relay resumes at its cursor. A restore from a backup older than the
last events the relay saw leaves the relay ahead of the server; the relay
then re-crawls the repositories at their next commit.

:::danger[One server per data directory]
Never run two servers on one data directory, on one host or two. The
server takes `linkjar-pds.lock` at start and refuses to start when another
holds it, but a copy on a second host has its own lock file.
:::

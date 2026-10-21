---
title: Postgres for the shared tier
description: Move the accounts, the event log, the DID cache and the audit log from SQLite files to Postgres, and back.
sidebar:
  order: 1
---

Four of the server's databases are shared by every account: the accounts,
the event log, the DID cache and the audit log. They can live in Postgres
instead of SQLite files. The repositories and the blobs stay in the data
directory either way.

Most hosts never need this. Postgres is for an operator whose backups and
replication are already Postgres, or who plans an
[entryway](/scale/entryway/) later.

## What to know first

- **One server.** The tier serves one `linkjar-pds` process. A second one
  on the same schemas exits 2 at start.
- **No TLS.** The server reaches Postgres without TLS. Run the two on one
  host or one private network. The URL takes no option but
  `sslmode=disable`.
- **Sign-in** with `scram-sha-256` or a clear-text password. `md5` is not
  supported.
- **Backups change.** A restore must bring back Postgres and the data
  directory from the same moment, because the repositories and the event
  log refer to each other. Preserve the account schema's `credential_epoch`
  table so password recovery continues to reject old tokens. Storage
  migration copies this table in both directions.

## Move to Postgres

1. Create a database and a role that owns it. The server creates four
   schemas in it: `pds_account`, `pds_sequencer`, `pds_did_cache` and
   `pds_audit`.
2. Stop the server.

   ```bash
   docker compose stop pds
   ```

3. Add the URL to `pds.env`. Set `PDS_POSTGRES_SCHEMA_PREFIX` when `pds`
   is taken. Remove `PDS_ACCOUNT_DB_LOCATION`, `PDS_SEQUENCER_DB_LOCATION`
   and `PDS_DID_CACHE_DB_LOCATION` if they are set, so that the copy finds
   the files at their default places.

   ```ini
   PDS_POSTGRES_URL=postgres://pds:password@db:5432/pds
   ```

4. Copy.

   ```bash
   docker compose run --rm pds linkjar-pds migrate-storage --to postgres
   ```

   It prints one line per table and ends with a line that says the row
   counts, the head of the event log and the last checkpoint are the same
   on both sides. An exit code of 2 means nothing was switched: read the
   message.

5. Check the directory against the new tier, then start.

   ```bash
   docker compose run --rm pds linkjar-pds verify
   ```

   ```bash
   docker compose up -d
   ```

6. Keep the SQLite files until the first backup of Postgres exists. Then
   move them out of the data directory: a later copy back needs the
   places empty.

## Back to SQLite

1. Stop the server.
2. Move `account.sqlite`, `sequencer.sqlite`, `did_cache.sqlite` and
   `audit.sqlite` out of the data directory, with their `-wal` and `-shm`
   files. The copy refuses a target that exists.
3. Copy, with `PDS_POSTGRES_URL` still set:

   ```bash
   docker compose run --rm pds linkjar-pds migrate-storage --to sqlite
   ```

4. Remove `PDS_POSTGRES_URL` from `pds.env`, run `verify`, and start.

## When the copy fails

The copy writes to the target only, so a copy that fails leaves the source
as it was. Remove what the target got, the four schemas in Postgres or
the four files on disk, and run the copy again.

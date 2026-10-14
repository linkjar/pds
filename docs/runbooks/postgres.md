# The shared tier on Postgres

[SPEC §21.2](../SPEC.md#212-shared-tier-alternative) · [SPEC §8.4](../SPEC.md#84-storage-seams) · [Plan](../plan.md)

This runbook moves the accounts, the event log, the DID cache and the
audit log of a data directory from SQLite files to Postgres, and back.
The actor stores and the blobs stay where they are. Rehearse each
direction on staging before production.

## 1. What to know first

- **One server.** The tier serves one `linkjar-pds` process. A second one
  on the same schemas exits 2.
- **No TLS.** The server reaches Postgres without TLS. Put the two on one
  host or one private network.
- **Sign-in.** The role signs in with `scram-sha-256` or a clear-text
  password. `md5` is not supported.
- **Backups change.** Litestream covers the SQLite files. For Postgres,
  the backup and the restore are those of the Postgres installation. A
  restore must bring back Postgres and the data directory from the same
  moment: the actor stores and the event log refer to each other.

## 2. To Postgres

1. Create a database and a role that owns it. The server creates four
   schemas in it: `pds_account`, `pds_sequencer`, `pds_did_cache` and
   `pds_audit`.
2. Stop the server.
3. Set `PDS_POSTGRES_URL`, and `PDS_POSTGRES_SCHEMA_PREFIX` when the
   prefix `pds` is taken. Remove `PDS_ACCOUNT_DB_LOCATION`,
   `PDS_SEQUENCER_DB_LOCATION` and `PDS_DID_CACHE_DB_LOCATION` when they
   are set: the copy reads the files from the default places of the data
   directory.
4. Copy.

   ```bash
   linkjar-pds migrate-storage --to postgres
   ```

   It prints one line for each table and ends with a line that says the
   row counts, the head of the event log and the last checkpoint are the
   same. An exit code of 2 means nothing was switched: read the message.
5. Check the data directory against the new tier.

   ```bash
   linkjar-pds verify
   ```

6. Start the server. It reads `PDS_POSTGRES_URL` and uses Postgres.
7. Keep the SQLite files until the first backup of Postgres exists. Then
   move them out of the data directory: a later copy back needs the
   places empty.

## 3. Back to SQLite

1. Stop the server.
2. Move the old `account.sqlite`, `sequencer.sqlite`, `did_cache.sqlite`
   and `audit.sqlite` out of the data directory, with their `-wal` and
   `-shm` files. The copy refuses a target that exists.
3. Copy, with `PDS_POSTGRES_URL` still set.

   ```bash
   linkjar-pds migrate-storage --to sqlite
   ```

4. Remove `PDS_POSTGRES_URL` from the configuration.
5. Check and start.

   ```bash
   linkjar-pds verify
   ```

## 4. When the copy fails

The copy writes to the target only. A copy that fails leaves the source
as it was. Remove what the target got, then run the copy again:

- Postgres: drop the four schemas.
- SQLite: remove the four files it created.

---
title: Migrate from the reference PDS
description: Plan a switch from the reference PDS using the existing data directory, with backup and rollback safeguards.
sidebar:
  order: 7
---

LinkJar PDS can serve a data directory the reference server wrote, with
the existing `pds.env`. Check a restored copy before switching the running
server. Database compatibility alone does not make an older image a safe
rollback target.

This page assumes the reference server's installer: the data at `/pds`,
the settings in `/pds/pds.env`, and a systemd unit named `pds` that runs
its Compose file.

## Before the switch

1. Take a [cold backup](/operate/backup/#a-cold-backup) of `/pds`.
2. Get the Compose files onto the host, as in
   [the install](/start/install/), and write `.env` with the image and
   `PDS_DATA_PATH=/pds`.
3. Make `pds.env` from the reference server's:

   ```bash
   sudo cp /pds/pds.env pds.env && sudo chown $USER pds.env && chmod 600 pds.env
   ```

   Change the two paths to where the Compose file mounts the directory:

   ```ini
   PDS_DATA_DIRECTORY=/app/data
   PDS_BLOBSTORE_DISK_LOCATION=/app/data/blocks
   ```

   Leave every other line as it is. The JWT secret, the rotation key,
   the admin password and the SMTP settings must stay: sessions and the
   DIDs of the accounts depend on them.

4. Check the configuration:

   ```bash
   docker compose run --rm pds linkjar-pds check-config
   ```

## The switch

1. Stop the reference server. Two servers must never serve one data
   directory.

   ```bash
   sudo systemctl disable --now pds
   ```

2. Check the data directory. `verify` reads every account and prints a
   summary that ends with `differences 0`:

   ```bash
   docker compose run --rm pds linkjar-pds verify
   ```

   A difference is a fact about the data, not about the switch. Stop here
   and start the reference server again until you understand it.

3. Seal the history for the [audit log](/operate/audit-log/#seal-history),
   if you have set its key:

   ```bash
   docker compose run --rm pds linkjar-pds audit seal
   ```

4. Start.

   ```bash
   docker compose up -d
   ```

   Caddy starts with an empty certificate store and fetches a certificate
   for the server's name at once, and for each handle at its first
   request.

5. [Check it from outside](/start/verify/). The relay resumes at its
   cursor; its status for the host goes back to `active` with the first
   write.

## Plan for rollback

Prefer a forward fix or a validated build that retains LinkJar's recovery
checks. This release adds a `credential_epoch` table to the shared account
database and a blob-reference index to actor databases. The reference
migration ledger stays unchanged, and older servers can read the layout.

The reference server and older LinkJar images do not enforce credential
epochs or the member host's live session check. They can accept revoked,
unexpired legacy tokens after password recovery. Read the
[update and rollback guidance](/operate/update/#roll-back) before choosing
an older image.

Keep the current account database and `credential_epoch` in backups and
copies. Do not restore a pre-recovery account database to undo the switch:
that can restore old passwords, credentials, and access. Preserve
`audit.sqlite` and `ops.sqlite` as well.

:::note[The relay and the AppView]
The reference server's `pds.env` already names `PDS_BSKY_APP_VIEW_URL`,
`PDS_BSKY_APP_VIEW_DID` and `PDS_CRAWLERS`. Keep them: without the
AppView the Bluesky app cannot load an account's settings on either
server.
:::

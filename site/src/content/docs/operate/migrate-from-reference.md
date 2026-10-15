---
title: Migrate from the reference PDS
description: Switch a host that runs the reference PDS to LinkJar PDS on the same data directory, and switch back if you must.
sidebar:
  order: 7
---

LinkJar PDS serves a data directory the reference server wrote, with the
same `pds.env`, and the reference server serves what LinkJar PDS wrote. A
switch is a stop and a start, and so is the way back.

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

## Back to the reference server

```bash
docker compose down
```

```bash
sudo systemctl enable --now pds
```

LinkJar PDS leaves three files of its own in the directory, `audit.sqlite`,
`ops.sqlite` and `linkjar-pds.lock`. The reference server ignores them. A
session that an app opened on LinkJar PDS goes on at the reference server,
because the rows are the reference server's.

:::note[The relay and the AppView]
The reference server's `pds.env` already names `PDS_BSKY_APP_VIEW_URL`,
`PDS_BSKY_APP_VIEW_DID` and `PDS_CRAWLERS`. Keep them: without the
AppView the Bluesky app cannot load an account's settings on either
server.
:::

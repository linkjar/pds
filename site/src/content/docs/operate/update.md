---
title: Update
description: Move to a newer image, and what the server does while it stops.
sidebar:
  order: 1
---

An update is a new image digest in `.env` and a restart. A newer server
reads the existing data directory. This release adds a credential recovery
table to the account database and an index to actor databases on writable
open. The first index build takes a SQLite write lock and extra disk space.

## Find the new image

Resolve the `main` tag to its digest:

```bash
docker buildx imagetools inspect ghcr.io/linkjar/pds-server:main --format '{{.Manifest.Digest}}'
```

Or build it yourself from a fresh checkout, as in
[the install](/start/install/#choose-the-image).

## Restart on it

Set `PDS_IMAGE` in `.env` to the new digest, then:

```bash
docker compose pull
```

```bash
docker compose up -d
```

Compose replaces the server's container and leaves Caddy running. On
`SIGTERM` the server stops taking connections, finishes the writes in
flight, closes the firehose with a close frame so that the relay resumes
at its cursor, and exits. The Compose file gives it 60 seconds.

Check that the new server is up:

```bash
docker compose logs --since 2m pds
```

```bash
curl -s https://pds.example.com/xrpc/_health
```

## Roll back

Put the previous digest back in `.env` and run `docker compose up -d`
again, subject to the authentication limit below. The additive table and
index leave the reference layout readable by older servers.

The reference server and older Candidate images do not enforce credential
epochs or the member host's live session check. They can accept revoked,
unexpired legacy tokens after a password recovery. If recovery must remain
effective, rotate the token-signing secrets and keys and require sign-in
again before serving with an older image. Keep the account database and
its recovery table in every [backup](/operate/backup/).

:::note[Pin the digest, not a tag]
`:main` moves with every merge. A digest names one build, so two hosts
with the same digest run the same bytes, and a rollback is exact.
:::

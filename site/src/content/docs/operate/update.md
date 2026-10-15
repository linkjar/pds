---
title: Update
description: Move to a newer image, and what the server does while it stops.
sidebar:
  order: 1
---

An update is a new image digest in `.env` and a restart. The data
directory needs nothing: the server writes the reference server's layout,
and a newer server reads what an older one wrote.

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
again. No release so far changes the layout of the data directory, so a
rollback is a restart; a release that does will say so in its notes and
in the [migration page](/operate/migrate-from-reference/).

:::note[Pin the digest, not a tag]
`:main` moves with every merge. A digest names one build, so two hosts
with the same digest run the same bytes, and a rollback is exact.
:::

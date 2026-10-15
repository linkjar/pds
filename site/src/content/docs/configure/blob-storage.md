---
title: Blob storage
description: Images and videos on the disk, or in an S3-compatible bucket.
sidebar:
  order: 3
---

A blob is an image, a video or any other file an account uploads. By
default blobs are files under the data directory. For a host with a small
disk, or for a bucket that is backed up and replicated for you, the server
can keep them in an S3-compatible store instead.

## On the disk

`PDS_BLOBSTORE_DISK_LOCATION` names the directory, `blocks` under the
data directory in the Compose deployment. The layout is the reference
server's:

| Path | Content |
|---|---|
| `blocks/<did>/<cid>` | a stored blob |
| `tempt/<did>/` | an upload that waits for the record that refers to it |
| `quarantine/<did>/<cid>` | a blob of a taken-down account or record |

`tempt` and `quarantine` sit beside `blocks`, in the data directory.
`PDS_BLOB_UPLOAD_LIMIT` caps one upload, in bytes; the example file sets
100 MiB.

## In an S3-compatible bucket

Set the bucket and remove `PDS_BLOBSTORE_DISK_LOCATION`: the server
refuses both at once.

```ini
PDS_BLOBSTORE_S3_BUCKET=pds-blobs
PDS_BLOBSTORE_S3_REGION=auto
PDS_BLOBSTORE_S3_ENDPOINT=https://<account>.r2.cloudflarestorage.com
PDS_BLOBSTORE_S3_FORCE_PATH_STYLE=true
PDS_BLOBSTORE_S3_ACCESS_KEY_ID=…
PDS_BLOBSTORE_S3_SECRET_ACCESS_KEY=…
```

The example is Cloudflare R2. MinIO, Backblaze B2, Wasabi and AWS S3 work
the same way; AWS needs no endpoint. The rules:

- The region and both halves of the credentials are needed at start.
  Without a region the server reads `AWS_REGION` or `AWS_DEFAULT_REGION`;
  without credentials it reads `AWS_ACCESS_KEY_ID`,
  `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN`. It reads no profile
  file and no instance role.
- With `FORCE_PATH_STYLE`, or an endpoint that is an IP address, the
  bucket is the first segment of the path. Otherwise it is a label of the
  host.
- `PDS_BLOBSTORE_S3_UPLOAD_TIMEOUT_MS` bounds one upload from its first
  byte to its last, 20 seconds by default.

Objects are named as the reference server names them, `blocks/<did>/<cid>`,
`tmp/<did>/<key>` and `quarantine/<did>/<cid>`, so a bucket the reference
server wrote is served as it is. An upload goes to the bucket as the
client sends it, in one request below 8 MiB and in parts of 8 MiB above,
so the server holds one part in memory and writes nothing to the disk. A
request the store refuses with 429 or a 5xx is sent three times at most.

## Check a store

`verify` lists the objects of each account and compares them with the
blob index of its repository. It changes nothing in the bucket.

```bash
docker compose run --rm pds linkjar-pds verify
```

Moving blobs between the disk and a bucket is a copy with a tool of your
choice while the server is stopped; the names are the same on both sides.

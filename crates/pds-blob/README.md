# pds-blob

Blob bytes: on disk in the Reference server's layout, or in an
S3-compatible bucket with SigV4-signed requests. It stages an upload while
it hashes it to a raw CID, then makes it permanent, deletes, quarantines
and lists blobs, and guesses a media type from the first bytes. The
metadata rows of a blob live in pds-storage, not here.

## Modules

| Module | Holds |
|---|---|
| `lib` | `BlobStore` (`stage`, `make_permanent`, `open`, `has`, `delete`, `quarantine`, `stored`), `Staging`, `DiskBlobStore`. |
| `s3` | `S3BlobStore`, `S3Config`, multipart uploads, retries. |
| `sniff` | The media type from the first 64 bytes. |

## Place in the workspace

Depends on pds-types. Used by pds-xrpc, linkjar-pds and pds-sim.

## What it needs

On disk, a writable root that holds `<did>/<cid>`, `tempt/` (the
Reference's name for the staging directory) and `quarantine/`; the server
reads `PDS_BLOBSTORE_DISK_LOCATION` and `PDS_BLOBSTORE_DISK_TMP_LOCATION`
for it. On S3, the endpoint of the `PDS_BLOBSTORE_S3_*` variables, by
default `s3.<region>.amazonaws.com`, with their credentials or the `AWS_*`
ones. The S3 store makes blocking calls on a tokio handle.

## Tests

`cargo test -p pds-blob`. `tests/s3.rs` runs against a fake S3 on
127.0.0.1 that checks no signatures. The test against a store that checks
them runs only with `PDS_BLOB_TEST_S3` set.

## Specification

SPEC §6.4, §8.1 and §8.3.

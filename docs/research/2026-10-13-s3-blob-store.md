# Unit 4b, first part: the S3-compatible blob store

Date: 2026-10-13 · [Plan](../plan.md) · [SPEC §8.1](../SPEC.md#81-layout) ·
[Unit 4](2026-10-13-writes-and-firehose.md)

Unit 4 left three parts unbuilt. This is the first: production blobs are in
Cloudflare R2, so the Candidate cannot serve a restore of production
without it. The fault-injecting VFS and the network simulation follow.

## 1. Result

- The Candidate keeps blobs in an S3-compatible store when
  `PDS_BLOBSTORE_S3_BUCKET` is set. The object names are the Reference's:
  `blocks/<did>/<cid>`, `tmp/<did>/<key>`, `quarantine/<did>/<cid>`.
- The writes run has an `s3` stage. The Reference stores one blob and
  stages one upload in a store that checks request signatures. The
  Candidate takes over, serves the blob, makes the staged upload permanent
  and removes both with their last references. The blob scenario of the
  default profile passes on that store on both servers, and `verify`
  reports 0 differences with the bucket checked.
- The stage recorded one difference, in two scenarios (§4).
- The store adds no crate to the build. It uses `reqwest`, `hmac` and
  `sha2`, which the server already had; `bytes` and `url` were in the tree
  and are now named. `cargo vet` still has 286 exemptions.

## 2. What is built

`crates/pds-blob/src/s3.rs` holds the store and `BlobStore` in
`crates/pds-blob/src/lib.rs` is the store that the configuration names,
disk or S3. The callers did not change: every call blocks and runs on the
blocking pool, as before.

| Part | How |
|---|---|
| Signing | AWS Signature Version 4 with the SHA-256 of the body. The unit tests hold the signer to three examples of the Amazon S3 API reference. |
| Upload | The body goes to the store as the client sends it. Below 8 MiB it is one `PutObject`. Above, it is a multipart upload in parts of 8 MiB. The server holds one part in memory and writes nothing to disk. |
| An upload that ends early | A dropped upload aborts its multipart upload in a task of its own, because the value can be dropped where blocking is not allowed. |
| Staged to permanent | `CopyObject`, then `DeleteObject`, as at the pin. When the staged object is gone and the blob is in place, the call succeeds. |
| Read | `GetObject`, streamed to the client in the chunks the store sends. |
| `verify` | `ListObjectsV2` with the account's prefix: one request for each thousand blobs of an account. |
| Retries | A request that fails to connect, or that the store answers with 429, 500, 502, 503 or 504, is sent three times at most. |
| Timeouts | 5 seconds to connect; the upload timeout for one upload from first byte to last; the request timeout for a read that stalls. The defaults are the Reference's. |

Not built: quarantine. The disk store has none yet either; both get it
with takedowns in unit 7. `DeleteObjects` is not used; account deletion in
unit 5 can delete one object at a time or add it.

## 3. How it is held to the Reference

- **The signer** matches the published examples (`GET` with a range, a
  listing with a query, `?lifecycle`).
- **The calls** run against a server in the test process that speaks the
  part of the S3 API the store uses: staging, a move, a second move, a
  read, a listing over three pages, deletes, a multipart upload of three
  parts, a dropped upload, and a store that answers 503.
- **A store that checks signatures.** The same calls ran against the
  Versity S3 gateway 1.8.0, by hand through the test that
  `PDS_BLOB_TEST_S3` enables, and in the Harness.
- **The Harness.** `pnpm parity writes --only=s3`, described in
  [parity/README.md](../../parity/README.md#the-writes-run). The store is
  configured as production configures R2: an endpoint, path style and
  static credentials.

MinIO was the first choice for the Harness. Its images are no longer
published on Docker Hub or Quay, so the stack uses the Versity gateway,
pinned by digest.

## 4. What the run showed about the Reference

With blobs in the S3-compatible store, the Reference answers 500
`InternalServerError` to `com.atproto.sync.getBlob` for an upload that no
record references yet. On the disk store it answers 400 `InvalidRequest`,
"Blob not found". The cause is in `packages/aws/src/s3.ts` at the pin:
`getObject` reads `blocks/<did>/<cid>`, the object is still at
`tmp/<did>/<key>`, and the client's `NoSuchKey` is not mapped to
`BlobNotFoundError`. The disk store maps a missing file.

Production runs on R2, so production answers 500 here today. The Candidate
answers 400 on both stores. This is SPEC §2.3, DD-18, a client error that
the Reference answers with 500, as in decision D15. The owner confirmed it
on 2026-10-13.

## 5. What the Candidate needs that the Reference does not

The Candidate refuses to start without a region and credentials. The
Reference's client looks for them at the first request, and also reads a
profile file, an instance role and a web identity. The Candidate reads the
`PDS_BLOBSTORE_S3_*` variables, then `AWS_REGION`, `AWS_DEFAULT_REGION`,
`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN`.
Production sets the `PDS_BLOBSTORE_S3_*` variables
(`legacy/scripts/provision-wizard.sh`), so it needs no change.

## 6. Not measured

No latency or memory was measured on the S3-compatible store. The Harness
store is a gateway on a memory file system in the same Docker network, and
its numbers would say nothing about R2. The measurement that matters is a
`getBlob` and an `uploadBlob` against R2 from the staging host.

## 7. Draft of the report upstream

For `bluesky-social/atproto`, as a public issue. The owner sends it.

> **`com.atproto.repo.importRepo` accepts a commit that the account's key
> did not sign, and `getBlob` answers 500 for a staged blob on S3**
>
> Observed on `@atproto/pds` 0.5.34 (`7ca16cc6`).
>
> 1. `importRepo` verifies neither the DID of the commit nor its
>    signature. With a full-access session of account A, a CAR whose
>    commit is signed by another key, or whose commit names account B, is
>    answered 200 and replaces A's repository. The caller can only write
>    to its own account, so this gives no access to another account. The
>    result is a repository whose head does not verify against the
>    account's signing key, which a relay in strict mode rejects.
>    Expected: 400 `InvalidRequest`.
> 2. `importRepo` into an active account appends no event. The account's
>    next commit has a `since` and `prevData` that the relay's copy does
>    not match, and a strict relay drops it until something emits `#sync`.
> 3. With `S3BlobStore`, `com.atproto.sync.getBlob` for a blob that was
>    uploaded and is not referenced by a record yet answers 500
>    `InternalServerError`. `DiskBlobStore` answers 400 "Blob not found".
>    `S3BlobStore.getObject` reads `blocks/<did>/<cid>` while the object is
>    at `tmp/<did>/<key>`, and the SDK's `NoSuchKey` is not mapped to
>    `BlobNotFoundError`.
>
> Each was reproduced with a scripted run against a local stack. I can
> share the scenarios.

The scenarios are `parity/scenarios/writes/22-elsewhere.scenario.ts`,
`default/15-import.scenario.ts` and `default/05-blobs.scenario.ts`.

## 8. The next part: can `sqlite-plugin` carry the VFS

SPEC §19.2 names `sqlite-plugin` for the fault-injecting VFS. A spike
outside the workspace checked version 0.11.0 with `rusqlite` 0.40.2 and its
bundled SQLite 3.53.2:

- `register_static` is a safe function. A VFS written against the crate's
  `Vfs` trait needs no `unsafe` in this repository.
- It links. The crate declares the SQLite symbols itself and the linker
  resolves them to the SQLite that `rusqlite` bundles. `rusqlite` opened a
  database through the registered VFS, in WAL mode.
- A write that the VFS failed reached SQLite as `SQLITE_IOERR_WRITE`, the
  statement failed, and after a reopen the database held the rows from
  before and passed `integrity_check`.
- The spike ran WAL with `locking_mode=EXCLUSIVE`. WAL between two
  connections needs the shared-memory methods, which the trait has
  (`shm_map`, `shm_lock`, `shm_barrier`, `shm_unmap`). The spike did not
  write them.

Two costs. The crate builds with `bindgen`, so the build needs libclang:
the spike failed until `LIBCLANG_PATH` named Xcode's copy, and `devenv` and
the CI job need the same. `bindgen` brings about 30 build-time crates for
`cargo vet`. Both stay out of the server's binary and image when only
`pds-sim` depends on the crate.

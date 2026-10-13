# Unit 4b, second part: a file system below SQLite that loses power

Date: 2026-10-13 · [Plan](../plan.md) ·
[SPEC §19.2](../SPEC.md#192-seams) ·
[First part](2026-10-13-s3-blob-store.md)

Until now a stop in the simulation was a process stop between two steps of
a commit: the files held what SQLite had committed. This part takes writes
away below SQLite.

## 1. Result

- Every database of the data directory opens through one seam,
  `pds_storage::io`. The simulation names a SQLite VFS there.
- The VFS loses power inside a write of the plan. Each file then loses a
  seeded suffix of its writes since its last sync, and the last kept write
  may be torn at 512 bytes.
- 550 seeds passed in two shapes: 39,459 commits, 6,413 stops at fail
  points and 5,440 losses of power, with 0 failures.
- With SQLite's syncing turned off in the server, 30 of 30 seeds fail. The
  oracle sees the fault it is built for.
- The repository still has no `unsafe`: `sqlite-plugin` registers the VFS
  through a safe function.

## 2. What is built

| Part | Where | What |
|---|---|---|
| The seam | `crates/pds-storage/src/io.rs` | One function opens every database: with the operating system's files, or through the VFS that `use_vfs` names. |
| The VFS | `crates/pds-sim/src/vfs.rs` | Passes each operation to a file of the host. Keeps, for each file, what every write and truncation replaced since the last sync. Holds the write-ahead log's shared memory and all locks in the process. |
| The plan | `crates/pds-sim/src/lib.rs` | 5 writes in 100 arm a loss of power after 0 to 39 more writes and syncs below SQLite. A commit takes fewer, so some losses come after the answer and do not happen. |
| The check | the same | After the loss the server starts again and repairs. The repository must hold the records from before the write or those after it. A write that was answered 200 must be there. Then every oracle of SPEC §19.4 runs. |

SPEC §19.2 asked for a `StorageIo` trait. SQLite's own VFS interface is
that trait, so the seam is one function and the SPEC now says so.

The VFS declares a 512-byte sector and `SQLITE_IOCAP_POWERSAFE_OVERWRITE`
only: no atomic write and no safe append. A sync does not ask the disk of
the host. What a sync makes durable is decided by the VFS, so the real
flush adds time and no information.

## 3. Two faults in the first version

- It called `File::sync_data` on every sync. On macOS that is
  `F_FULLFSYNC`, and a seed took about 6.5 seconds in place of 1.3.
- It kept the default device characteristics of `sqlite-plugin`, which
  tell SQLite that every write is atomic. SQLite may then skip protection
  that a torn write needs. The first 20 seeds ran with that default; the
  550 seeds of §1 ran without it.

## 4. What the runs showed about the Candidate

Nothing failed. Three behaviours were exercised that no run had reached
before:

- A commit that the server stored and could not answer, because the power
  went after the store was synced. The note in `ops.sqlite` survives, the
  repair emits `#sync`, and the relay model converges.
- A note in `ops.sqlite` that comes back. The server clears a note without
  a sync, so a loss of power can undo the clearing. The repair then finds
  the event in the log and settles the note.
- A commit whose transaction was lost in full. The repository holds the
  records from before, and the note names a revision that was never
  stored.

In 60 seeds with 30 losses in 100 writes, 1,317 losses of power ended as
follows:

| Answer | Repository after the restart | Repair | Count |
|---|---|---|---|
| 500 | the records from before | settled a note of a commit that was never stored | 846 |
| 500 | the records from before | nothing to do | 83 |
| 500 | the records after the write | `#sync` | 336 |
| 500 | the records after the write | settled a note: the event was in the log | 1 |
| 200 | the records after the write | settled a note | 51 |

A kept write was torn in 354 of them.

## 5. Not modelled

- An I/O error that the process survives, and a failed `fsync` that is
  not a loss of power.
- Bit flips, a write at another offset, and a full disk.
- The loss of a delete: a deleted file is gone at once.
- Reordering between files. Each file loses a suffix of its own writes;
  the choice for one file does not depend on another.
- The blob store and the signing-key files, which do not go through the
  seam.
- A loss of power outside a commit: during an upload, a read or the
  startup repair.

## 6. Cost

- The build of `pds-sim` needs libclang, because `sqlite-plugin` generates
  its bindings with `bindgen`. `devenv.nix` sets `LIBCLANG_PATH`. The
  server's binary and image do not depend on the crate.
- `cargo vet` has 296 exemptions, 10 more than before: `sqlite-plugin`
  and the build-time crates of `bindgen`.
- `the_first_seeds_pass_every_oracle` and the nightly seeds now run with
  power losses. The second nightly shape runs with 25 in 100.

# Writes, the event log and the firehose: unit 4

Research record, 2026-10-13. Companion to [SPEC.md](../SPEC.md) §6, §7,
§17 and §19, and to [plan.md](../plan.md), unit 4. It records what the unit
built, how it is held to the Reference, what the comparison found, how the
server behaves when it stops inside a commit, and what the unit leaves out.

**Result.** The Candidate takes over a data directory that the Reference
started, accepts writes on it, and serves the firehose. On 2026-10-13 the
writes run of the Harness ran the same eight scenarios on each server, and
all passed on both. The comparison showed 37 differences: 36 belong to the
deliberate differences of SPEC §2.3, and one is the wording of a refusal
that both servers give.
`verify` reported zero differences on every directory the Candidate had
written. A relay in strict mode re-emitted every commit of the Candidate.
The Candidate was stopped once between a commit and its event; at the next
start it stated the head with `#sync`, and the relay converged.

**Three things of the unit's list are not built.** The plan names them
under "Delivers", and the owner's decision of 2026-10-08 placed the first
two in this unit:

1. **The fault-injecting VFS and the `StorageIo` trait.** The simulation
   stops the server between two steps of a commit. It does not lose or tear
   a write below SQLite.
2. **The `turmoil` network.** The simulation has no socket, no partition and
   no slow consumer.
3. **The S3-compatible blob store.** The Candidate writes blobs to disk.
   Production keeps them in R2, so the cutover needs this store.

Each needs a place in the plan; [plan.md](../plan.md) asks the owner.

The first nightly seed run is the last item of the gate. It can run only
from `main`; §5 has what ran before the merge.

## 1. What the unit built

| Crate | Now holds |
|---|---|
| `pds-storage` | The write side of an actor store: one transaction with the Reference's statements for the root, blocks, the record index, backlinks and the blob index. The event log: append, the cursor queries, pages bounded by bytes. `ops.sqlite` with the notes of commits in flight. The fail points. The Reference's schema for the actor store, the event log and the DID cache. |
| `pds-blob` | Uploads staged to `tempt/<did>/` while they are hashed, the move into place, deletion, and content sniffing for the image, video and audio types the vendored lexicons accept. |
| `pds-lexicon` | The vendored lexicons compiled into the binary (`cargo xtask codegen` writes the list), and the Reference's wording of a validation error with its JSON path. |
| `pds-sequencer` | The encoding of `#commit`, `#sync`, `#identity`, `#account`, `#info` and the error frame. The sequencer: one writer, a wakeup for subscribers, and a cache of encoded frames bounded at 16 MiB. A subscription with the cursor rules of SPEC §7.3. Crawler notification. |
| `pds-xrpc` | Request bodies read and checked in the Reference's order. The write path: prepare a record, build the commit, store it, append its event. `createRecord`, `putRecord`, `deleteRecord`, `applyWrites`, `uploadBlob`, `importRepo`, `listMissingBlobs` and `subscribeRepos` over a WebSocket. The repair at startup and the collection of unreferenced uploads. |
| `pds-sim` | Seeded plans against the server in process, stops at the fail points, a relay model, the oracles of SPEC §19.4, a seed runner and the determinism meta-test. |
| `linkjar-pds` | The settings of the write path, the repair before the first request, and the upload sweep every ten minutes. `verify` counts a stale root of the account database without calling it a difference. |

The server keeps the write connection of an actor store open between
writes, up to `PDS_ACTOR_STORE_CACHE_SIZE` stores. It still opens a
connection for each read.

## 2. How it is held to the Reference

The Candidate cannot create an account before unit 5. So every pass of the
writes run, `pnpm parity writes`, starts on the Reference, which creates a
pool of 24 accounts. In the Candidate's pass the PDS container is then
replaced by the Candidate on the same volume, and the scenarios take their
accounts from the pool. A session that the Reference minted is valid on the
Candidate, because both check it with the same secret.

| Stage | Scenarios | What runs |
|---|---|---|
| records | `default/03-repo-writes`, `04-repo-reads`, `05-blobs`, `15-import`, and `writes/20-firehose`, `21-relay`, `22-elsewhere` | Each on the Reference and on the Candidate, on a stack of its own |
| cursor-limits | `firehose/01-cursor-limits` | The same, with a five-second backfill window and a buffer of five events |
| crash | `writes-crash/30-crash` | On the Candidate only |

Four scenarios are the ones unit 1 wrote for the Reference, unchanged.
`20-firehose` and `21-relay` are `default/06` and `07` without the account
events, which need the endpoints of unit 5. In the Candidate's pass the log
already holds the events the Reference wrote, and a cursor reads through
both; the relay first followed the Reference and then finds another server
behind the same name.

The comparison allows the differences recorded in
[`reference-vs-candidate.json`](../../parity/differences/reference-vs-candidate.json)
and fails on any other, and on a recorded one that no longer shows
([differences](../../parity/differences/README.md)). CI runs the writes run
on every change to the crates (`.github/workflows/restore.yml`, job
`writes`).

Without a Reference, the crates hold the rest: 235 tests in the
workspace, among them the write path through a real listener
([`writes.rs`](../../crates/linkjar-pds/tests/writes.rs)), the cursor table
on a log in a temporary directory
([`subscription.rs`](../../crates/pds-sequencer/tests/subscription.rs)) and
the three fail points ([`crash.rs`](../../crates/linkjar-pds/tests/crash.rs)).

## 3. What the comparison found

About the Reference at the pin:

1. **`importRepo` checks nothing about the commit.** It accepts a commit
   that another key signed, and a commit that names another account, and
   replaces the repository with it. The caller must hold a full session of
   the account, so this is not a way into another account. It does mean
   that the Reference stores a repository whose commit its own key did not
   sign, and serves it. The Candidate refuses both (DD-14).
2. **A tree that is not a Merkle search tree is accepted.** A key at the
   wrong layer and keys out of order across nodes are both stored. An empty
   node below the root is 500 (DD-7).
3. **A record block that is not canonical DAG-CBOR is accepted**: map keys
   out of order, the float `1.0`, and `undefined` (DD-8).
4. **A record may reference a blob with size zero, and with an empty media
   type** (DD-9). The second was not expected: the Reference compares the
   record's media type with the stored one only while the blob still waits
   for its first record.
5. **An import appends no event and does not write the account database's
   root.** SPEC revision 6 required `#sync` after an import. The Candidate
   now follows the pin (SPEC §6.5), and the question whether an import into
   an active account should emit `#sync` is the owner's
   ([plan.md](../plan.md)). `sync.listRepos` reads that root, so after an
   import it names the commit before.
6. **A stale root in the account database is a state the Reference
   leaves**, after an import and after a root update that failed, which it
   logs and does not retry. `verify` called it a difference. It now counts
   it (`staleAccountRoots`) and calls only a root ahead of the store a
   difference.
7. **A batch that updates or deletes a key with no record is 500.** The
   owner's decision D15 names the create on a key that exists; the
   Candidate answers 500 to the other two, as the pin does.

About the Candidate, found by its own tests and by the Harness, and fixed:

1. **A subscriber could be called too slow without being slow.** The mark
   that says "this subscriber waited at the head" was set after the wait.
   The WebSocket handler drops the wait when the client sends a frame, so
   the mark was lost and the next burst of events ended the stream with
   `ConsumerTooSlow`. The mark is now set before the wait.
2. **A backfill of large commits could hold hundreds of megabytes.** A page
   of the log was 500 events whatever their size, and the frame cache held
   1,024 frames whatever their size. The cursor-limit scenario writes 60
   commits of 900 kB. A page now ends after 4 MiB, and the frame cache is
   bounded at 16 MiB. One run of that stage ended with an empty `verify`
   output, before this change; it did not recur in three later runs, and
   its cause is not established.
3. **A refusal could reach the client as 502.** The server answered an
   upload over the limit, and any request it refused before the body, while
   the proxy still sent the body; the server then closed the connection.
   One of the two runs before this change showed 502 from the proxy in
   place of 413. The server
   now reads a refused body to its end, up to 16 MiB, as the Reference's
   HTTP server does.
4. **A blob could be served without a reference.** A commit that stopped
   after it moved an upload into place left the file where `getBlob` reads
   it, with an index row that still waited for a record. The simulation's
   oracle named this state. `getBlob` now serves a permanent row only, and
   the upload sweep removes the file.

One difference in wording stays recorded. The tree of `default/15-import`
shares nodes and also hangs a subtree below layer zero. Both servers answer
400 `InvalidRequest`; the Reference names the first defect and the
Candidate the second.

## 4. The boundary between a commit and its event

A commit is in the actor's `store.sqlite` and its event in
`sequencer.sqlite`. No transaction covers both.

The Reference appends the event inside the actor transaction and commits
afterwards. When it stops between the two, the log holds an event for a
commit that does not exist, and a relay that took it is ahead of the
server.

The Candidate orders the steps the other way (SPEC §7.4):

1. It notes the account and the revision in `ops.sqlite`.
2. It commits the actor transaction.
3. It appends the event.
4. It removes the note.

At the next start, before the first request, it reads the notes. Where the
repository's head is the noted revision and the log has no event of that
revision, the server appends `#sync` with the head. Sync 1.1 defines
`#sync` for this state: a consumer fetches the repository again.

Three things hold this:

- [`crash.rs`](../../crates/linkjar-pds/tests/crash.rs) fails each of the
  three steps in turn and checks the head, the log, the note and what the
  repair did.
- The crash stage of the writes run arms the fail point `commit.stored` to
  abort the process once. The write gets no answer. After the restart the
  head has moved, the log has `#sync` and no `#commit` for that revision,
  the next write succeeds, and the relay holds the new head.
- Every seed of the simulation stops the server at these points several
  times (§5).

The note names a commit that stopped in this process. When a power failure
takes the tail of one file and not of the other, no note says so; that
needs the storage faults the simulation does not have yet.

## 5. The simulation

A run is a function of its seed (`cargo run -p pds-sim -- --from <seed>
--seeds 1 --trace`). The seed draws a plan against one server over one
data directory: creates, updates, deletes, batches and uploads on several
accounts; subscribers that connect with a cursor, read for a while and
leave; restarts; and stops at the three fail points. The clock, the
revision clock and the server's random source belong to the run. Requests
enter the router in process.

After every stop and at intervals a run checks:

- `verify` finds no difference;
- every repository holds exactly the records of the acknowledged writes: a
  write that was refused, or stopped before its commit, is absent, and a
  write that stopped after its commit is present;
- each subscriber received every event after its start once, in order;
- a relay model that checks the signature, `since`, `prevData` and the
  `prev` of every operation, and fetches a repository again only on
  `#sync`, holds the server's repositories;
- a blob is served when a record references it, and only then.

On 2026-10-13, before the merge: 400 seeds of 200 steps with the default
plan, and 150 seeds of 250 steps with five accounts and 30 stops in 100
writes. Together they made 51,283 commits and stopped the server 9,241
times. No seed failed.

Two checks on the simulator itself:

- **Determinism.** A seed run twice gives the same trace, byte for byte,
  commit revisions and CIDs included
  ([`seeds.rs`](../../crates/pds-sim/tests/seeds.rs)).
- **It fails when the server is wrong.** With the `#sync` of the repair
  removed by hand, each of the first three seeds failed at the relay
  oracle.

`.github/workflows/sim.yml` runs 500 new seeds of each shape every night
and puts the totals and any failed seed in the run summary.

What the simulator does not model is in the module documentation of
[`pds-sim`](../../crates/pds-sim/src/lib.rs) and in SPEC §19.2: storage
faults below SQLite, the network, concurrent operations, and two operations
on one key in one batch.

## 6. What the unit leaves out

Besides the three items at the top:

| Left out | Where it goes |
|---|---|
| The account events of SPEC §10.4. The encoders exist and the simulation uses them to create its accounts; no endpoint emits them yet. | Unit 5 |
| OAuth access tokens and service tokens. A write takes a legacy session. `uploadBlob` with a service token is refused. | Unit 5 |
| Rate limits (SPEC §4.3). The Harness sends its bypass key, so it does not see their absence. | Unit 5, with the account endpoints that share the limiters |
| The slur check on a record key. The Reference refuses a key from its list. | Unit 5, with the handle policy that owns the list |
| The proxy and read-after-write. A method without a handler is still 501 for a valid session. | Unit 5 |
| A cache of read connections to actor stores (SPEC §8.3). Writes have one. | Unit 8, with the performance targets |
| `repair <did>` (SPEC §19.5). The startup repair covers the boundary of §4. | Unit 8 |
| The Reference's message for a body that is not JSON, which repeats its JSON parser. | Not planned; SPEC §6.3 says so |
| An upper bound on an import when `PDS_MAX_REPO_IMPORT_SIZE` is unset. The Reference has none either. | Unit 8, with the migration endpoints |

Record validation follows the pin's validator, and its messages carry the
Reference's wording. Where a record breaks several rules, the two servers
may name different ones (SPEC §6.3).

## 7. Measurements

[perf-writes.md](../../parity/results/perf-writes.md), measured on 2026-10-13
by `pnpm parity writes --perf`: SPEC §16.1, S1 to S3, on each server, on a
stack of its own. One run on one host, sequential requests. It is not a
benchmark and not a run against the targets of SPEC §16.2.

| | Reference | Candidate |
|---|---|---|
| S1, one `createRecord`, 1,000 records in the repository, p50 / p99 ms | 14.4 / 43.4 | 4.9 / 10.8 |
| S1, 100,000 records, p50 / p99 ms | 14.5 / 53.9 | 11.5 / 37.4 |
| S2, `applyWrites` of 200, 1,000 records, p50 ms | 190.3 | 14.3 |
| S2, 100,000 records, p50 ms | 274.7 | 31.4 |
| S3, export of 101,100 records, ms | 1,529 | 201 |

The bytes of new blocks per write are the same on both servers at every
size, and the export has the same 128,312 blocks.

A single write on the Candidate costs more at 100,000 records than at
10,000: 11.5 ms against 5.1 ms at p50. The cause is not measured. The
Candidate syncs four times for one write: the note, the commit, the event,
and the root in the account database, each in a file of its own; the
removal of the note is not synced. It keeps the write connection of an
actor store open between writes (`PDS_ACTOR_STORE_CACHE_SIZE`), and opens a
connection for each read.

## 8. Dependencies

The firehose is a WebSocket: `axum` with its `ws` feature brings
`tokio-tungstenite` and `sha1`. The simulation drives the router through
`tower`. The server binary now has 264 crates in its tree, up from 248, and
`cargo vet` passes on 286 exemptions, up from 276.

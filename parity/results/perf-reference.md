# Performance baseline: reference

SPEC §16.1 scenarios S1 to S5, S8 and S9, measured by the parity harness from outside the server. Latencies are wall-clock times at the client, through the TLS edge. Memory is the anonymous memory of the PDS container, without the page cache.

One run on the host below. It places the Reference for the comparison with the Candidate on the same host (SPEC §16.2, T1); it is not a benchmark of the Reference.

|  |  |
|---|---|
| Image | `ghcr.io/linkjar/pds@sha256:12c1de787be2dc5ffdce36e477621a3943c0fe58bf768690c0ea81a4c4b90933` |
| Upstream | `@atproto/pds@0.5.34` at `7ca16cc6` |
| Scale | full |
| Measured | 2026-10-09 |
| Host | Apple M1 Pro, 10 cores, 16 GiB, darwin 27.0.0 arm64, Docker |

## S1: one `createRecord`

| Records in the repository | p50 / p99 ms | Bytes of new blocks per write | Net block rows per write | Net block bytes per write |
|---|---|---|---|---|
| 10 | 18.9 / 167.3 | 1,548 | 1.3 | 157 |
| 1,000 | 32.3 / 91.2 | 2,801 | 1.3 | 155 |
| 10,000 | 34 / 85.6 | 4,287 | 1.1 | 146 |
| 100,000 | 12.3 / 28.6 | 3,511 | 1.2 | 157 |

The bytes of new blocks are the CAR slice of the commit on the firehose: the commit, the tree nodes on the path to the record, and the record. Net growth is what the block table keeps after the Reference deletes the nodes and the commit that the write replaced.

## S2: `applyWrites` with 200 operations

| Records in the repository | p50 / max ms |
|---|---|
| 10 | 402.4 / 762.1 |
| 1,000 | 493.8 / 752.9 |
| 10,000 | 208.4 / 244.9 |
| 100,000 | 304.5 / 316 |

## S3: `getRepo` export

| Records | Wall ms | CAR MiB | Blocks | Memory growth MiB |
|---|---|---|---|---|
| 101,100 | 2,752 | 19.8 | 128,312 | 42.8 |

## S4: one writer at 50 commits a second

| Subscribers | Writes answered 200 | Frames delivered | Lag p50 / p99 ms | Commit p50 / p99 ms | Memory MiB |
|---|---|---|---|---|---|
| 1 | 500 of 500 | 500 of 500 | 10.9 / 610.8 | 9.5 / 26.1 | 690 |
| 10 | 500 of 500 | 5,000 of 5,000 | 11.8 / 396.9 | 10.7 / 36.2 | 692 |
| 100 | 500 of 500 | 50,000 of 50,000 | 12.8 / 503.3 | 10.6 / 245.1 | 697 |
| 1,000 | 500 of 500 | 475,500 of 500,000 | 11280 / 18287.6 | 11265.7 / 18281.1 | 214 |

Lag runs from the moment the write was sent to the moment a subscriber received its commit, so it includes the commit itself. At most twenty subscribers record arrivals; the others hold a connection and count frames.

### After the subscribers left

Write throughput of 50 writers, each writing as fast as it is answered, around the S4 run that ended with 1,000 subscribers. Every subscriber had disconnected before the second and third measurement.

| When | Commits per second | Failed writes | Slowest write ms |
|---|---|---|---|
| Before any subscriber | 154 | 0 | 1,886 |
| Right after they left | 134 | 0 | 2,246 |
| Fifteen seconds later | 146 | 0 | 2,085 |

## S5: backfill from a cursor

| Events | Wall ms | Events per second | Memory growth MiB |
|---|---|---|---|
| 100,000 | 8,478 | 11,795 | 49.2 |

## S9: writers across actors, two commits a second each

| Writers | Writes answered 200 | Lag p50 / p99 ms | Commit p50 / p99 ms | Sequencer fsyncs |
|---|---|---|---|---|
| 50 | 3,000 of 3,000 | 164.8 / 1058.1 | 160.6 / 353.6 | not observable from outside the Reference |

## S8: concurrent uploads of 5 MiB

| Uploads | Wall ms | Upload p50 / max ms | Memory growth MiB |
|---|---|---|---|
| 20 | 1,113 | 1065.1 / 1101.6 | 48 |

S6 (idle accounts) and S7 (the token endpoint) are measured from unit 5 on. See the [harness README](../README.md#performance-scenarios).

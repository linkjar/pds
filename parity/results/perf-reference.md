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
| 10 | 11.9 / 29.4 | 1,548 | 1.3 | 157 |
| 1,000 | 10.8 / 23.4 | 2,801 | 1.3 | 155 |
| 10,000 | 12.5 / 30 | 4,287 | 1.1 | 146 |
| 100,000 | 12.6 / 43.8 | 3,511 | 1.2 | 157 |

The bytes of new blocks are the CAR slice of the commit on the firehose: the commit, the tree nodes on the path to the record, and the record. Net growth is what the block table keeps after the Reference deletes the nodes and the commit that the write replaced.

## S2: `applyWrites` with 200 operations

| Records in the repository | p50 / max ms |
|---|---|
| 10 | 167 / 235 |
| 1,000 | 160.9 / 184.2 |
| 10,000 | 242 / 400 |
| 100,000 | 255.5 / 453.7 |

## S3: `getRepo` export

| Records | Wall ms | CAR MiB | Blocks | Memory growth MiB |
|---|---|---|---|---|
| 101,100 | 1,642 | 19.8 | 128,312 | 49.1 |

## S4: one writer at 50 commits a second

| Subscribers | Writes answered 200 | Frames delivered | Lag p50 / p99 ms | Commit p50 / p99 ms | Memory MiB |
|---|---|---|---|---|---|
| 1 | 500 of 500 | 500 of 500 | 11.3 / 604.9 | 10.5 / 117.7 | 711 |
| 10 | 500 of 500 | 5,000 of 5,000 | 10.9 / 438.9 | 10 / 22.6 | 711 |
| 100 | 500 of 500 | 50,000 of 50,000 | 40.8 / 998.9 | 14.9 / 938.5 | 721 |
| 1,000 | 500 of 500 | 500,000 of 500,000 | 13905.9 / 21816.8 | 13901.6 / 21811.9 | 229 |

Lag runs from the moment the write was sent to the moment a subscriber received its commit, so it includes the commit itself. At most twenty subscribers record arrivals; the others hold a connection and count frames.

### After the subscribers left

Write throughput of 50 writers, each writing as fast as it is answered, around the S4 run that ended with 1,000 subscribers. Every subscriber had disconnected before the second and third measurement.

| When | Commits per second | Failed writes | Slowest write ms |
|---|---|---|---|
| Before any subscriber | 128 | 0 | 1,684 |
| Right after they left | 125 | 0 | 1,857 |
| Fifteen seconds later | 131 | 0 | 1,519 |

## S5: backfill from a cursor

| Events | Wall ms | Events per second | Memory growth MiB |
|---|---|---|---|
| 100,000 | 9,791 | 10,213 | 90.3 |

## S9: writers across actors, two commits a second each

| Writers | Writes answered 200 | Lag p50 / p99 ms | Commit p50 / p99 ms | Sequencer fsyncs |
|---|---|---|---|---|
| 50 | 3,000 of 3,000 | 205.6 / 1122.7 | 167.2 / 374.1 | not observable from outside the Reference |

## S8: concurrent uploads of 5 MiB

| Uploads | Wall ms | Upload p50 / max ms | Memory growth MiB |
|---|---|---|---|
| 20 | 1,178 | 1114 / 1160.2 | 65.5 |

S6 (idle accounts) and S7 (the token endpoint) are measured from unit 5 on. See the [harness README](../README.md#performance-scenarios).

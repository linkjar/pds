# Write measurements: the Reference and the Candidate

Measured by `pnpm parity writes --perf`: SPEC §16.1, S1 to S3. Each server runs the same scenario on a stack of its own. The Reference creates the account; in the Candidate's pass the Candidate then takes over the data directory and does every write. Latencies are wall-clock times at the client, through the TLS edge. Memory is the anonymous memory of the container.

One run on the host below. It places the two servers beside each other on that host; it is not a benchmark of either. The requests are sequential, so the numbers say nothing about throughput under load.

|  |  |
|---|---|
| Reference | `ghcr.io/linkjar/pds@sha256:78c56cd9e8b13d53cd03bd715352fe84e8deeca0595163dca2d6f1d1b215a801` |
| Upstream | `@atproto/pds@0.5.34` at `7ca16cc6` |
| Candidate | `linkjar-pds:candidate` |
| Scale | full |
| Measured | 2026-10-13 |
| Host | Apple M1 Pro, 10 cores, 16 GiB, darwin 27.0.0 arm64, Docker |

## S1: one `createRecord`, p50 / p99 ms over 100 writes

| Records in the repository | Reference | Candidate | Reference ÷ Candidate at p50 | Bytes of new blocks per write, Reference | Candidate |
|---|---|---|---|---|---|
| 10 | 11.5 / 47.6 | 4.5 / 10.5 | 2.6× | 1,548 | 1,548 |
| 1,000 | 14.4 / 43.4 | 4.9 / 10.8 | 2.9× | 2,801 | 2,801 |
| 10,000 | 12.8 / 35.9 | 5.1 / 23.7 | 2.5× | 4,287 | 4,287 |
| 100,000 | 14.5 / 53.9 | 11.5 / 37.4 | 1.3× | 3,511 | 3,511 |

The bytes of new blocks are the CAR slice of the commit on the firehose: the commit, the tree nodes on the path to the record, and the record. The two servers build the same tree, so the same writes give the same slices.

## S2: `applyWrites` with 200 operations, p50 / max ms over five batches

| Records in the repository | Reference | Candidate | Reference ÷ Candidate at p50 |
|---|---|---|---|
| 10 | 183.7 / 204 | 14.8 / 16.3 | 12.4× |
| 1,000 | 190.3 / 263.6 | 14.3 / 24.4 | 13.3× |
| 10,000 | 217.7 / 236.7 | 16.4 / 26.5 | 13.3× |
| 100,000 | 274.7 / 340.2 | 31.4 / 83.1 | 8.7× |

## S3: `getRepo` export

|  | Reference | Candidate | Reference ÷ Candidate |
|---|---|---|---|
| Records | 101,100 | 101,100 |  |
| Wall ms | 1,529 | 201 | 7.6× |
| CAR MiB | 19.8 | 19.8 |  |
| Blocks | 128,312 | 128,312 |  |
| Memory growth MiB | 34.1 | 1.8 |  |

# Read measurements: the Reference and the Candidate on one data directory

Measured by `pnpm parity restore --perf`. The Reference writes one repository and answers the requests below; the Candidate replaces it on the same volume and answers them again. Latencies are wall-clock times at the client, through the TLS edge, one request at a time. Memory is the anonymous memory of the PDS container, without the page cache. Each server is started fresh before its requests. Both servers returned the same export, byte for byte.

One run on the host below. It places the two servers beside each other on that host; it is not a benchmark of either.

|  |  |
|---|---|
| Reference | `ghcr.io/linkjar/pds@sha256:78c56cd9e8b13d53cd03bd715352fe84e8deeca0595163dca2d6f1d1b215a801` |
| Upstream | `@atproto/pds@0.5.34` at `7ca16cc6` |
| Candidate | `linkjar-pds:candidate` |
| Records in the repository | 100,000 |
| Measured | 2026-10-14 |
| Host | Apple M1 Pro, 10 cores, 16 GiB, darwin 27.0.0 arm64, Docker |

## The export: `sync.getRepo`

|  | Reference | Candidate | Reference ÷ Candidate |
|---|---|---|---|
| Median of 5 runs, ms | 1,588 | 268 | 5.9× |
| The 5 runs, ms | 1993, 1541, 1528, 1588, 1591 | 406, 254, 268, 336, 251 |  |
| CAR MiB | 22.3 | 22.3 |  |
| Memory growth over the 5 runs, MiB | 104.6 | 2.7 |  |
| Memory before the first request, MiB | 286.6 | 6.1 | 47× |

## Point reads: p50 / p99 ms over 200 requests

| Method | Reference | Candidate | Reference ÷ Candidate at p50 |
|---|---|---|---|
| `sync.getLatestCommit` | 2.8 / 9.6 | 2 / 5.9 | 1.4× |
| `repo.getRecord` | 2.7 / 5.9 | 2.2 / 6.1 | 1.2× |
| `sync.getRecord` (record with proof) | 7.2 / 17.9 | 2.3 / 7 | 3.1× |
| `repo.listRecords`, 100 records | 189.2 / 1027.2 | 3.7 / 7.7 | 51.1× |
| `repo.describeRepo` | 14 / 125.1 | 2.2 / 7.8 | 6.4× |

The requests are sequential, so the numbers say nothing about throughput under load. The Candidate opens a SQLite connection for each read in this unit; the actor store cache of SPEC §8.3 arrives with the write path.

# Read measurements: the Reference and the Candidate on one data directory

Measured by `pnpm parity restore --perf`. The Reference writes one repository and answers the requests below; the Candidate replaces it on the same volume and answers them again. Latencies are wall-clock times at the client, through the TLS edge, one request at a time. Memory is the anonymous memory of the PDS container, without the page cache. Each server is started fresh before its requests. Both servers returned the same export, byte for byte.

One run on the host below. It places the two servers beside each other on that host; it is not a benchmark of either.

|  |  |
|---|---|
| Reference | `ghcr.io/linkjar/pds@sha256:78c56cd9e8b13d53cd03bd715352fe84e8deeca0595163dca2d6f1d1b215a801` |
| Upstream | `@atproto/pds@0.5.34` at `7ca16cc6` |
| Candidate | `linkjar-pds:candidate`, release build |
| Records in the repository | 100,000 |
| Measured | 2026-10-12 |
| Host | Apple M1 Pro, 10 cores, 16 GiB, darwin 27.0.0 arm64, Docker |

## The export: `sync.getRepo`

|  | Reference | Candidate | Reference ÷ Candidate |
|---|---|---|---|
| Median of five runs, ms | 1,392 | 176 | 7.9× |
| The five runs, ms | 1791, 1426, 1388, 1357, 1392 | 259, 263, 176, 172, 154 |  |
| CAR MiB | 22.3 | 22.3 |  |
| Memory growth over the five runs, MiB | 111.2 | 2.6 |  |
| Memory before the first request, MiB | 287 | 5.2 | 55.2× |

## Point reads: p50 / p99 ms over 200 requests

| Method | Reference | Candidate | Reference ÷ Candidate at p50 |
|---|---|---|---|
| `sync.getLatestCommit` | 2.8 / 11.6 | 1.7 / 6.7 | 1.6× |
| `repo.getRecord` | 2.6 / 7 | 1.9 / 6.1 | 1.4× |
| `sync.getRecord` (record with proof) | 6.2 / 14 | 1.9 / 10.9 | 3.3× |
| `repo.listRecords`, 100 records | 126.3 / 225.1 | 109.4 / 210.6 | 1.2× |
| `repo.describeRepo` | 8.2 / 14.8 | 6.2 / 12.3 | 1.3× |

The requests are sequential, so the numbers say nothing about throughput under load. The Candidate opens a SQLite connection for each read in this unit; the actor store cache of SPEC §8.3 arrives with the write path.

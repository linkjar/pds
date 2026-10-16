# pds-storage

Synchronous access to the Reference server's data directory: its SQLite
files with its exact schema and statements, the files only this server
keeps (`ops.sqlite`, `audit.sqlite`), a Postgres backend for the shared
databases through a hand-written client, the copy between the two
backends, fail points for the simulation, and the VFS seam below SQLite.
It stores rows and bytes and does not interpret them: the audit chain is
pds-audit's, and the meaning of an account row is pds-xrpc's.

## Modules

| Module | Holds |
|---|---|
| `layout` | `DataLayout`: the paths of the data directory, the Postgres schemas, the sequencer lease, `create_missing`. |
| `account`, `oauth`, `linkjar` | `AccountDb`: `account.sqlite`, with the OAuth tables and the tables of the `linkjar` profile. |
| `actor`, `actor_txn` | `ActorDb` and `ActorTxn`: one `store.sqlite` per actor, the read side and the write side. `ActorDb` implements `pds_repo::BlockStore`. |
| `sequencer`, `did_cache`, `audit`, `ops` | `SequencerDb`, `DidCacheDb`, `AuditDb`, `OpsDb`: one database each. |
| `schema`, `schema_shared` | The Reference's DDL at the pin: the actor store and the functions that create a database, and the three shared databases with the migrations the Reference records for them. |
| `sql`, `pg` | One SQL API over SQLite and Postgres, and the Postgres client: `SharedTier`, `PgTarget`. |
| `migrate` | The copy of the shared databases between the backends, behind `linkjar-pds migrate-storage`. |
| `fail` | The fail points: armed by `PDS_FAIL_POINTS` in dev mode, hit by the write path. |
| `io` | The VFS seam that pds-sim's power-loss file system plugs into, and the switch that hands the WAL checkpoints to a backup tool (`PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT`). |
| `error` | `StorageError`. |

## Place in the workspace

Depends on pds-repo, for `BlockStore` and `BlockMap`. Used by
pds-sequencer, pds-xrpc, linkjar-pds and pds-sim.

## What it needs

A data directory in the Reference's layout: `account.sqlite`,
`sequencer.sqlite`, `did_cache.sqlite`, `ops.sqlite`, `audit.sqlite`,
`actors/<prefix>/<did>/store.sqlite` beside the actor's `key`, and
`actors/reserved_keys/`. The directory must be writable even for reads:
the databases are in WAL mode, and a reader creates the `-wal` and `-shm`
files. SQLite is compiled in (`rusqlite` with `bundled`), so a C compiler
is needed at build time. Postgres is optional, reached through
`PDS_POSTGRES_URL`, without TLS.

## Tests

`cargo test -p pds-storage`. `tests/reads.rs` and `tests/wal.rs` build a
data directory in a temporary directory. The Postgres test runs only with
`PDS_TEST_POSTGRES_URL` set.

## Specification

SPEC §8 (§8.1 to §8.4), §3.2, §3.3, §19.2, §20, §21.2 and §22.1.

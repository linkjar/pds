# LinkJar PDS

An AT Protocol Personal Data Server in Rust, compatible with the reference
implementation. One binary, one data directory, the reference server's
layout and settings. Dual-licensed MIT or Apache-2.0.

**Host your own:** [pds.linkjar.io](https://pds.linkjar.io) is the guide,
from the DNS records to the audit log. Its source is [`site/`](site/) and
the deployment it describes is [`deploy/compose/`](deploy/compose/).

## What it is

```mermaid
flowchart LR
  classDef ext fill:#eef3ff,stroke:#4a6cf7,color:#1b2b6b
  classDef pds fill:#fff7e6,stroke:#e0960f,color:#5a3b00
  classDef side fill:#eafaf1,stroke:#2e9e5b,color:#0f4d2a
  classDef store fill:#f4f4f5,stroke:#71717a,color:#27272a

  clients["Apps, extension, iOS<br/>any atproto client"]:::ext
  relay["Relays and indexers"]:::ext
  idp["Apple · Google · GitHub"]:::ext
  plc["PLC directory · DNS"]:::ext

  subgraph host["One host"]
    pds["linkjar-pds<br/>XRPC · OAuth · firehose"]:::pds
    data[("data directory<br/>SQLite per actor · blobs")]:::store
    console["ops console"]:::side
    mcp["MCP server"]:::side
  end

  clients -- "XRPC + OAuth" --> pds
  pds -- "subscribeRepos" --> relay
  pds -- "sign-in (linkjar profile)" --> idp
  pds -- "resolve · PLC ops" --> plc
  pds --- data
  console -- "ops API · OTLP" --> pds
  mcp -- "OAuth client · XRPC" --> pds
```

The server is one binary. Behaviour that varies per operator sits behind
extension traits: the `stock` profile reproduces the reference server, the
`linkjar` profile adds LinkJar's sign-in providers, handle policy, creation
receipt, sign-in methods and mail. Everything else, from the repository
engine to the firehose, is shared.

Since 2026-10-15 the server runs `pds.linkjar.social`, LinkJar's
invite-only pilot, on the data directory the reference image wrote.
[docs/plan.md](docs/plan.md) tracks each unit of the work, the owner's
steps and what is not built.

## Documentation

| Read | For |
|---|---|
| [pds.linkjar.io](https://pds.linkjar.io) | Hosting the server: requirements, install, configuration, operation, scaling |
| [docs/README.md](docs/README.md) | The map of the engineering documentation |
| [docs/SPEC.md](docs/SPEC.md) | The normative specification (RFC 2119) |
| [docs/architecture/README.md](docs/architecture/README.md) | One diagram per part of the system, kept current with the code |
| [docs/plan.md](docs/plan.md) | Status of each unit, the owner's steps, what is not built, decisions |
| [docs/sidecars.md](docs/sidecars.md) | The operations console and the MCP server |
| [docs/runbooks/](docs/README.md#runbooks) | Cutover, entryway, Postgres, and a test host |
| [docs/research/](docs/research/) | What other systems do, and the record of each unit |

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | The workspace crates, one per subsystem ([architecture](docs/architecture/README.md#crates)) |
| `crates/linkjar-pds/` | The binary: `serve`, `verify`, `export-car`, `audit`, `migrate-storage`, `reconcile`, `check-config`, `health`, `version` |
| `crates/pds-sim/` | The simulation: `pds-sim --seeds N` runs seeded plans against the server in process |
| `Dockerfile`, `Dockerfile.console`, `Dockerfile.mcp` | The images of the server and its two sidecars |
| `deploy/compose/` | The Compose deployment of one host: the server behind Caddy |
| `site/` | The self-hosting guide, [pds.linkjar.io](https://pds.linkjar.io) |
| `xtask/` | Repository tasks: `cargo xtask codegen` generates API types from the lexicons |
| `lexicons/` | Vendored upstream lexicons at the pin, LinkJar lexicons, project lexicons ([notice](lexicons/NOTICE.md)) |
| `interop/` | CC0 interop test vectors from upstream ([notice](interop/README.md)) |
| `fuzz/` | cargo-fuzz targets for every parser of untrusted bytes |
| `parity/` | The parity harness: the compose stack, scenarios and oracles that hold a target to the reference image ([README](parity/README.md)) |
| `bench/` | The repository engine beside `rsky-repo` on the same vectors and workload ([README](bench/README.md)) |
| `docs/` | Specification, plan, architecture, runbooks, research ([map](docs/README.md)) |
| `legacy/` | The reference-image build that ran until 2026-10-15, the rollback until the watch ends; frozen except security fixes ([README](legacy/README.md)) |
| `.agent_sources/` | Checkouts of the pinned upstream sources for reading: `scripts/vendor-agent-sources.sh` |
| `upstream.json` | The reference pin shared by the legacy build and the vendored lexicons |

## Build and check

Requirements: the toolchain in `rust-toolchain.toml` through rustup, or
`devenv shell`, which provides it with cargo-deny, cargo-vet, cargo-audit and
the parity harness's Node.

```sh
cargo build --workspace
cargo test --workspace
cargo xtask codegen --check     # generated types are current
cargo deny check                 # advisories, licences, bans, sources
cargo run -p pds-sim -- --seeds 20   # the simulation; its build needs libclang
cargo run -p pds-sim -- --network    # the same server on a simulated network
devenv shell -- pds:check        # the Rust gates of CI
devenv shell -- pds:hooks        # the rustfmt and clippy hooks, by hand
```

Run the server on a copy of a data directory, and check the copy:

```sh
export PDS_HOSTNAME=pds.example.com PDS_DATA_DIRECTORY=/path/to/copy \
       PDS_BLOBSTORE_DISK_LOCATION=/path/to/copy/blocks \
       PDS_JWT_SECRET=... PDS_ADMIN_PASSWORD=...
cargo run -p linkjar-pds -- verify   # reads only; exit 1 on a difference
cargo run -p linkjar-pds -- serve    # port 2583
```

The variables are the reference server's; the guide's
[environment page](https://pds.linkjar.io/configure/environment/) and SPEC
Appendix C list them. `serve` writes to the copy: records, blobs, events,
sessions and new accounts, the DID cache, and its own files `audit.sqlite`
and `ops.sqlite`. It creates an account only with
`PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX` set, and then it writes to the
PLC directory that `PDS_DID_PLC_URL` names: point that at a directory of
your own when you try it. For blobs in an S3-compatible store, set the
`PDS_BLOBSTORE_S3_*` variables in place of `PDS_BLOBSTORE_DISK_LOCATION`;
`verify` then lists the bucket and changes nothing in it.

`devenv shell` installs no git hook. Linked worktrees share one `.git/hooks`,
so an installed hook would run for every worktree with the configuration of
one.

The parity harness needs Docker and runs on its own
([parity/README.md](parity/README.md#run-it)):

```sh
cd parity && pnpm install
pnpm parity run reference        # scenarios against the reference image
pnpm parity compare reference repeat
pnpm parity images build-candidate
pnpm parity restore              # the Rust server on the reference's data
pnpm parity writes               # writes and the firehose on both servers
pnpm parity smoke https://pds.example.com   # a server that runs elsewhere
```

The guide builds on its own ([site/README.md](site/README.md)):

```sh
cd site && pnpm install && pnpm dev
```

## Measurements

Four files hold the numbers measured so far:

- [parity/results/perf-reads.md](parity/results/perf-reads.md): the reads
  of both servers on one data directory with a repository of 100,000
  records. The median `sync.getRepo` export of five runs took 1,385 ms on
  the reference server and 167 ms on the Rust server, for the same bytes.
  Memory before the first request was 280.4 MiB and 5.4 MiB.
- [parity/results/perf-writes.md](parity/results/perf-writes.md): writes
  on each server. With 1,000 records in the repository, one `createRecord`
  took 14.4 ms on the reference server and 4.9 ms on the Rust server at
  p50; a batch of 200 took 190.3 ms and 14.3 ms. With 100,000 records the
  single write took 14.5 ms and 11.5 ms.
- [parity/results/perf-reference.md](parity/results/perf-reference.md): the
  reference server's baseline for the single-write, batch-write, export,
  fan-out, backfill, concurrent-uploads and many-writers scenarios of SPEC
  §16.1.
- [bench/README.md](bench/README.md): the repository engine beside
  `rsky-repo` on the same vectors and workload, the evidence for the
  decision on the repository engine.

Each of the three result files is one run on one host, an Apple M1 Pro under
Docker, and says of itself that it is not a benchmark. The requests in the
read measurements are sequential, so they say nothing about throughput
under load.

## Contributing

Work happens in bounded units that each end in a merge with their gate green
([docs/plan.md](docs/plan.md#units)). The specification is the
source of truth; a change in behaviour starts with a change there. Every
architecture diagram in `docs/architecture/` is updated in the same change as
the code it describes, and so is the page of the guide that describes it.

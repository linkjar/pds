# pds-sim

The deterministic simulation of SPEC §19. Each seed draws a plan of writes,
batches, uploads, subscribers, restarts, stops at the fail points and power
losses, and runs it against an in-process server. It then checks the
oracles: `verify` is clean, no acknowledged write is lost, every event
arrives once and in order, the relay model converges, and a blob is served
only while a record references it. `--network` runs a server, a relay and
a client on turmoil's simulated network.

## Command

```
pds-sim [--seeds 20] [--from 1] [--steps 150] [--accounts 3] [--faults 8] [--power-loss 5] [--trace]
pds-sim --network [...]
```

Exit 1 when a seed fails and 2 on a bad argument. A failing seed is the
whole report of a failure: `--from <seed> --seeds 1 --trace` runs the same
steps again.

## Modules

| Module | Holds |
|---|---|
| `world` | The data directory, the server and the clocks. |
| `model` | The expected repositories and a strict relay model. |
| `vfs` | The SQLite VFS that loses power, below pds-storage's `io` seam. |
| `net` | The runs on turmoil's network. |
| `rng` | The seeded random source. |

## Place in the workspace

Depends on linkjar-pds, pds-types, pds-repo, pds-storage, pds-blob,
pds-identity, pds-sequencer and pds-xrpc. Nothing depends on it.

## What it needs

libclang at build time, because the VFS plugin runs bindgen; `devenv shell`
sets `LIBCLANG_PATH`, and outside it `LIBCLANG_PATH=$PWD/.devenv/profile/lib`
works. At run time only the temporary directory: no ports, no network.
`PDS_SIM_BUFFER` sets the subscriber buffer of `--network`.

## Tests

`cargo test -p pds-sim`: `tests/seeds.rs` runs seeds 1 to 6 and a
determinism check; `tests/network.rs` runs seeds 1 to 12, a small-buffer
run and a determinism check. The nightly workflow `sim.yml` runs four
shapes of seeds.

## Specification

SPEC §19 (§19.1 to §19.6).

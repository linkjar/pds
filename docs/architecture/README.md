# Architecture

[Documentation](../README.md) · [Specification](../SPEC.md) · [Plan](../plan.md)

One diagram per part of the system. Each names the crate or file that
implements it and the SPEC section that governs it. A change to the code
updates the diagram in the same commit; a diagram that drifts is a bug.

Sources are Mermaid files in [`diagrams/`](diagrams/), rendered with
[beautiful-mermaid](https://github.com/lukilabs/beautiful-mermaid) into
[`svg/`](svg/) in a light and a dark variant. Render after editing a source:

```sh
cd docs/architecture && pnpm install && pnpm render
```

Shapes carry the categories, so the diagrams read the same in any theme:
stadiums are outside the host, rectangles are the PDS process, double-edged
boxes are sidecars, cylinders are storage, hexagons are extension points,
diamonds are checks.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/01-legend.dark.svg">
  <img alt="Legend: outside the host, PDS process, sidecar, storage, extension point" src="svg/01-legend.svg">
</picture>

## Contents

- [System context](#system-context)
- [Crates](#crates)
- [Process and listeners](#process-and-listeners)
- [Write path](#write-path)
- [Sequencer and firehose](#sequencer-and-firehose)
- [Storage layout and seams](#storage-layout-and-seams)
- [Identity](#identity)
- [OAuth: stock sign-in](#oauth-stock-sign-in)
- [OAuth: LinkJar external sign-in](#oauth-linkjar-external-sign-in)
- [Extension model](#extension-model)
- [Audit log](#audit-log)
- [Simulation harness](#simulation-harness)
- [Parity harness](#parity-harness)
- [Deployment topologies](#deployment-topologies)
- [Sidecars](#sidecars)
- [Cutover and rollback](#cutover-and-rollback)
- [Repository and build](#repository-and-build)

## System context

SPEC §1, §21.1. Who talks to the PDS and over what. Source: [`02-system-context.mmd`](diagrams/02-system-context.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/02-system-context.dark.svg">
  <img alt="System context: clients and network services around the PDS host" src="svg/02-system-context.svg">
</picture>

## Crates

SPEC §3.1. Arrows point from a crate to the crates it depends on; the binary
wires everything and extension crates depend only on the trait crate. Source:
[`03-crates.mmd`](diagrams/03-crates.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/03-crates.dark.svg">
  <img alt="Crate dependency graph" src="svg/03-crates.svg">
</picture>

## Process and listeners

SPEC §3.2, §4.1, §14.3. One process, three listeners, supervised background
tasks, injectable time and randomness. Source: [`04-process.mmd`](diagrams/04-process.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/04-process.dark.svg">
  <img alt="Process layout: public, metrics and ops listeners, background tasks, providers" src="svg/04-process.svg">
</picture>

## Write path

SPEC §6.3, §6.4, §7.4. What happens on `createRecord`, and why the sequencer
row is durable with the commit. Source: [`05-write-path.mmd`](diagrams/05-write-path.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/05-write-path.dark.svg">
  <img alt="Sequence of a record write from client to firehose" src="svg/05-write-path.svg">
</picture>

## Sequencer and firehose

SPEC §7. Frames come only from the durable log; the broadcast is a wakeup.
Cursor rules: no cursor, live only; `0`, the whole window; in window,
backfill then live; above head, `FutureCursor`; older than the window,
`OutdatedCursor`; slow consumer, `ConsumerTooSlow`. Source:
[`06-firehose.mmd`](diagrams/06-firehose.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/06-firehose.dark.svg">
  <img alt="Sequencer log, wakeup bus and per-subscriber queues" src="svg/06-firehose.svg">
</picture>

## Storage layout and seams

SPEC §8. The reference layout on disk, the Candidate-only files beside it, and
the traits every subsystem goes through. Schema version 1 equals the
reference schema in the shared files, so the reference image can serve the
same directory during the rollback window. Source:
[`07-storage.mmd`](diagrams/07-storage.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/07-storage.dark.svg">
  <img alt="Storage seams over the data directory and blob store" src="svg/07-storage.svg">
</picture>

## Identity

SPEC §9. Resolution paths and what the server trusts. Source:
[`08-identity.mmd`](diagrams/08-identity.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/08-identity.dark.svg">
  <img alt="DID and handle resolution through the cache, PLC, did:web, DNS and well-known" src="svg/08-identity.svg">
</picture>

## OAuth: stock sign-in

SPEC §11. The authorization-code flow with PAR, PKCE and DPoP against the
server's own pages. Source: [`09-oauth-stock.mmd`](diagrams/09-oauth-stock.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/09-oauth-stock.dark.svg">
  <img alt="Sequence of the stock OAuth sign-in" src="svg/09-oauth-stock.svg">
</picture>

## OAuth: LinkJar external sign-in

SPEC §12.3, §13. The same flow with the `linkjar` profile's identity
providers inserted at the sign-in step; the receipt and handle policy hooks
run inside the account transaction. Source:
[`10-oauth-linkjar.mmd`](diagrams/10-oauth-linkjar.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/10-oauth-linkjar.dark.svg">
  <img alt="Sequence of an external-provider sign-up with the extension hooks" src="svg/10-oauth-linkjar.svg">
</picture>

## Extension model

SPEC §12. The core calls traits; a profile composes one implementation of
each; selection is `PDS_PROFILE`. No request field sets an internal flag;
hook writes commit in the account transaction; hooks fail closed; every hook
that changes account state produces an audit entry. Source:
[`11-extension-model.mmd`](diagrams/11-extension-model.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/11-extension-model.dark.svg">
  <img alt="Profile composing the extension traits, with the stock and linkjar implementations" src="svg/11-extension-model.svg">
</picture>

## Audit log

SPEC §20. Insert-only entries, a hash chain, signed checkpoints, and the
firehose as the witness. Source: [`12-audit.mmd`](diagrams/12-audit.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/12-audit.dark.svg">
  <img alt="Audit chain, checkpoints, checkpoint record on the firehose, offline verifier" src="svg/12-audit.svg">
</picture>

## Simulation harness

SPEC §19. One seed drives faults in storage, process and network; oracles
check verify, repair and firehose delivery. Source:
[`13-simulation.mmd`](diagrams/13-simulation.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/13-simulation.dark.svg">
  <img alt="Seeded simulation: plan, fault VFS, fail points, network simulation, oracles" src="svg/13-simulation.svg">
</picture>

## Parity harness

SPEC §17, [`parity/`](../../parity/README.md). Each target runs in its own
stack under the production hostnames: the PDS behind a TLS edge, with a PLC
directory, a strict Sync 1.1 relay and a mail catcher on a network that
cannot reach the internet. The edge forwards every outside name to the
fixture server in the harness process. Scenarios write one transcript per
target, and the comparison checks two targets against a recorded difference
list. Unit 1 runs reference builds only; the Candidate is one more target.
Fault scenarios run against the Candidate alone. Source:
[`14-parity.mmd`](diagrams/14-parity.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/14-parity.dark.svg">
  <img alt="Parity harness: one stack per target, the fixture server, transcripts and the comparison" src="svg/14-parity.svg">
</picture>

## Deployment topologies

SPEC §21. Version 1 is one node. Scale follows the protocol: an entryway
over member hosts. The shared-database cluster is documented, not
recommended. Source: [`15-topologies.mmd`](diagrams/15-topologies.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/15-topologies.dark.svg">
  <img alt="Single node with Litestream, and entryway over member hosts" src="svg/15-topologies.svg">
</picture>

## Sidecars

[sidecars.md](../sidecars.md), SPEC §22. Neither sidecar holds a database
connection or a signing key. Source: [`16-sidecars.mmd`](diagrams/16-sidecars.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/16-sidecars.dark.svg">
  <img alt="The operations console and the MCP server around the PDS" src="svg/16-sidecars.svg">
</picture>

## Cutover and rollback

SPEC §18. Same hostname, same directory, same secrets; the window stays
open until the first Candidate-only migration. Source:
[`17-cutover.mmd`](diagrams/17-cutover.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/17-cutover.dark.svg">
  <img alt="Cutover states from the reference to the Candidate and back" src="svg/17-cutover.svg">
</picture>

## Repository and build

How the repository is organised and what the gates run. The parity gate
runs the scenarios of [`parity/`](../../parity/README.md) on the published
reference image, a second boot of it, and the stock build, and then the two
comparisons. Source: [`18-repository.mmd`](diagrams/18-repository.mmd).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="svg/18-repository.dark.svg">
  <img alt="Lexicons to generated types, crates to CI gates, diagrams to SVG" src="svg/18-repository.svg">
</picture>

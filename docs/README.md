# Documentation

[Repository README](../README.md) · [Specification](SPEC.md) · [Plan](plan.md) · [Architecture](architecture/README.md)

## Normative

- [SPEC.md](SPEC.md): the LinkJar PDS specification, revision 3. Compatibility baseline, protocol surface, storage, identity, accounts, OAuth, extension points, the LinkJar profile, operations, security, performance, parity, cutover, simulation, audit log, topologies, sidecars.

## Plan and decisions

- [plan.md](plan.md): why the project exists, the owner's decisions of 2026-10-08, the thirteen delivery units with gates and allowances, and the dated status log.
- [sidecars.md](sidecars.md): the operations console and the MCP server, their contracts with the PDS, and what they may not hold.

## Architecture

- [architecture/README.md](architecture/README.md): one diagram per part of the system (context, crates, request paths, firehose, storage, OAuth flows, extension model, simulation, audit, topologies, sidecars, cutover), Mermaid sources rendered with beautiful-mermaid in light and dark variants. Updated in the same change as the code.

## Parity harness

- [../parity/README.md](../parity/README.md): how the harness is built and run, the oracles, the contract its page driver relies on, and the port of the legacy suites.
- [../parity/verify-at-pin.md](../parity/verify-at-pin.md): the twelve "verify at pin" answers with source and scenario, the rate-limiter table, and what the scenarios corrected in the specification.
- [../parity/differences/README.md](../parity/differences/README.md): how the stock build and the production build differ, by patch.
- [../parity/results/perf-reference.md](../parity/results/perf-reference.md): the Reference's baseline for the performance scenarios.

## Research

- [research/2026-10-08-state-of-the-art.md](research/2026-10-08-state-of-the-art.md): how other PDS implementations, replication tools, simulation testers, transparency logs and supply-chain tooling solve what this project needs, with sources.
- [research/2026-10-08-atproto-doc-sweep.md](research/2026-10-08-atproto-doc-sweep.md): the gap report of SPEC revision 0 against every atproto.com specification and guide page.
- [research/2026-10-09-parity-harness.md](research/2026-10-09-parity-harness.md): the relay, the PLC directory and the OAuth client the harness runs beside the PDS, with the source of each fact.

## Legacy

The reference-image build that runs in production today lives under
[`../legacy/`](../legacy/README.md). Its documentation is in
[legacy/](legacy/): the patch documents the `linkjar` profile reproduces
([handle policy](legacy/handle-policy.md), [external providers](legacy/external-providers.md),
[signup receipt](legacy/signup-receipt.md), [provider policy](legacy/provider-policy.md),
[sign-in methods](legacy/signin-methods.md), [mail templates](legacy/mail-templates.md)),
the [signup journey](legacy/signup-journey.md), the [runbooks](legacy/runbooks/pds.md),
the [upstream proposal](legacy/upstream-proposal.md) and the vendored upstream licences.

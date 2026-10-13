# Documentation

[Repository README](../README.md) · [Specification](SPEC.md) · [Plan](plan.md) · [Architecture](architecture/README.md)

## Normative

- [SPEC.md](SPEC.md): the LinkJar PDS specification, revision 6. Compatibility baseline, protocol surface, storage, identity, accounts, OAuth, extension points, the LinkJar profile, operations, security, performance, parity, cutover, simulation, audit log, topologies, sidecars.

## Plan and decisions

- [plan.md](plan.md): why the project exists, the owner's decisions, the thirteen delivery units with gates and allowances, and the dated status log.
- [sidecars.md](sidecars.md): the operations console and the MCP server, their contracts with the PDS, and what they may not hold.

## Architecture

- [architecture/README.md](architecture/README.md): one diagram per part of the system (context, crates, request paths, repository engine, firehose, storage, OAuth flows, extension model, simulation, audit, topologies, sidecars, cutover), Mermaid sources rendered with beautiful-mermaid in light and dark variants. Updated in the same change as the code.

## Parity harness

- [../parity/README.md](../parity/README.md): how the harness is built and run, the oracles, the restore run and the writes run of the Candidate, the contract its page driver relies on, and the port of the legacy suites.
- [../parity/verify-at-pin.md](../parity/verify-at-pin.md): the twelve "verify at pin" answers with source and scenario, the rate-limiter table, and what the scenarios corrected in the specification.
- [../parity/differences/README.md](../parity/differences/README.md): how the stock build and the production build differ, by patch, and how the Reference and the Candidate differ, by deliberate difference.
- [../parity/results/perf-reference.md](../parity/results/perf-reference.md): the Reference's baseline for the performance scenarios.

## Research

- [research/2026-10-08-state-of-the-art.md](research/2026-10-08-state-of-the-art.md): how other PDS implementations, replication tools, simulation testers, transparency logs and supply-chain tooling solve what this project needs, with sources.
- [research/2026-10-08-atproto-doc-sweep.md](research/2026-10-08-atproto-doc-sweep.md): the gap report of SPEC revision 0 against every atproto.com specification and guide page.
- [research/2026-10-09-parity-harness.md](research/2026-10-09-parity-harness.md): the relay, the PLC directory and the OAuth client the harness runs beside the PDS, with the source of each fact.
- [research/2026-10-12-repository-crate.md](research/2026-10-12-repository-crate.md): decision D2. The in-house repository engine and `rsky-repo` on the same interop vectors, the same fixtures from `@atproto/repo` at the pin, and the same S1 to S3 workload, with the method and the host.

- [research/2026-10-10-upstream-0.5.35-to-0.5.37.md](research/2026-10-10-upstream-0.5.35-to-0.5.37.md): what upstream released after the pin, the shared-node fix (#5584) for LinkJar's deployment, and the cost of each pin option.
- [research/2026-10-13-writes-and-firehose.md](research/2026-10-13-writes-and-firehose.md): unit 4. The write path, the event log and the firehose, the writes run that compares them with the Reference, what the Reference answers to data from elsewhere, the boundary between a commit and its event, the simulation, and what the unit leaves out.
- [research/2026-10-13-read-only-server.md](research/2026-10-13-read-only-server.md): unit 3. What the read-only server serves, the restore run that compares it with the Reference on one data directory, what the comparison found, what `verify` checks, and what the unit leaves out.

## Upstream sources

- [../.agent_sources/README.md](../.agent_sources/README.md): local checkouts of the Reference, the interop tests, the relay, the PLC directory and `rsky`, each at the commit this repository pins. `scripts/vendor-agent-sources.sh` fetches them; git ignores them.

## Legacy

The reference-image build that runs in production today lives under
[`../legacy/`](../legacy/README.md). Its documentation is in
[legacy/](legacy/): the patch documents the `linkjar` profile reproduces
([handle policy](legacy/handle-policy.md), [external providers](legacy/external-providers.md),
[signup receipt](legacy/signup-receipt.md), [provider policy](legacy/provider-policy.md),
[sign-in methods](legacy/signin-methods.md), [mail templates](legacy/mail-templates.md)),
the [signup journey](legacy/signup-journey.md), the
[shared-node fix](legacy/mst-shared-node.md) that the legacy build carries as
its ninth patch, the [runbooks](legacy/runbooks/pds.md),
the [upstream proposal](legacy/upstream-proposal.md) and the vendored upstream licences.

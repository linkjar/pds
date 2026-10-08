# LinkJar PDS in Rust: decision record and plan

Started 2026-10-08. The normative definition is [SPEC.md](SPEC.md). This page
records why the work exists, what it is built on, how it is delivered, and
where it stands. Nothing here changes the deployed server until the cutover
unit runs.

## Decision

Build LinkJar PDS, a Rust implementation of the AT Protocol PDS that is
compatible with the pinned reference server this repository builds today, and
that expresses LinkJar's account behaviour as native extension points instead
of source patches. Publish it for other operators. Replace the hosted
reference server with it in place, once the parity harness passes on a copy
of production data.

Reasons, in the order they weigh:

1. **The patch stack is the cost.** Six source patches, about 7,500 lines,
   reapplied on every upstream bump and verified by image builds. Each new
   upstream release risks a conflict in the OAuth provider, the part upstream
   changes most. Native extension points remove the conflict surface.
2. **Ownership of the account and OAuth layer.** External sign-in, the
   creation receipt, handle policy and sign-in methods are LinkJar's product
   surface. Owning them in a typed server with a test harness is cheaper to
   evolve than patching a compiled React bundle.
3. **Measured performance and reliability.** A single static binary, lower
   memory per idle account, bounded firehose fan-out, and closed crash
   boundaries, each measured against the reference on the same scenarios
   (SPEC §16, §17).
4. **Give it back.** A stock profile that any operator can run, with the
   extension points documented for upstream discussion.

What this decision does not change: the September 2026 hosting decision in
`linkjar.io` (`docs/research/serverless-pds-2026-09/`) stands. The reference
server stays in production and the LinkJar launch does not depend on this
work. The client-side encryption model, the SPEC in `linkjar.io`, and the
account hosting design in `docs/research/linkjar-accounts-2026-09/` are
unchanged.

## Research

[How other systems solve what LinkJar PDS needs](research-2026-10-08-state-of-the-art.md)
compares the PDS implementations, clustering shapes, storage seams,
simulation testing, audit-log designs and supply-chain gates, with sources,
and lists the SPEC changes they imply.

## Starting points examined

- **The reference** (`@atproto/pds@0.5.34`, commit `7ca16cc6`): the oracle.
  Its handler tree, databases, configuration and crash boundaries are the
  baseline the SPEC cites.
- **rsky-pds** (Blacksky): the most complete Rust PDS and in production for
  Blacksky accounts, on Postgres, S3 and Mailgun rather than the reference's
  SQLite-per-actor layout. Its repository crate is the first candidate for
  SPEC D2. Whether it ships an authorization server was not confirmed from
  public sources on 2026-10-08.
- **bluepds**: axum and SQLite on rsky's repository crate, self-described as
  not operable. Useful as a reading reference only.
- **millipds** (Python): a clear from-scratch architecture document, useful
  for the sequencer and MST design notes.

## Where it lives

- This repository, branch `feat/rust-pds`, until the first unit merges.
- The Cargo workspace lands at the repository root (SPEC D4). The image build
  for the reference stays where it is until cutover and then moves under
  `image/`.
- The `linkjar.io` repository gets only a pointer from its research index and
  a PDS URL setting for its browser gate and kit self-test (unit 1).
- The `linkjar/infra` repository changes in unit 7: service definition, alert
  rules, recovery runbook.

## Delivery units

Each unit is bounded, ends in a merge to `main` of this repository, and names
its gate. Estimates are planning allowances for one engineer, low confidence
until units 2 and 5 land.

| Unit | Delivers | Gate | Allowance |
|---|---|---|---|
| 0 | This record, the SPEC, the Cargo workspace skeleton, licence files, CI with `cargo deny`, vendored lexicons and code generation | Workspace builds; generated types compile for every vendored lexicon | 2 to 3 days |
| 1 | The parity harness (`parity/`) against the reference image only: compose, scenario runner, oracles O1 to O4, the port of `tests/*.mjs` to an HTTP endpoint, and the `linkjar.io` PDS URL setting | Harness green against Reference; stock-versus-production image differences documented | 1 to 2 weeks |
| 2 | Repository crate decision: interop vectors, MST and CAR benchmarks for `rsky-repo` versus in-house; `pds-types` and `pds-identity` with their vectors | O5 vectors pass; S1 to S3 micro-benchmarks published; D2 decided | 1 week |
| 3 | Read-only server: `sync.*`, `repo.get*`, `describeRepo`, `describeServer`, identity reads, `verify`, served from a copy of a reference data directory; shadow comparison on the host | O1 and O2 on read scenarios; `verify` zero differences on a production copy | 2 weeks |
| 4 | Writes, sequencer, firehose, blob store, crawler notification | O1 to O3 on write and firehose scenarios including the crash injection; a local relay converges | 3 weeks |
| 5 | Accounts, legacy sessions, app passwords, admin, mail, the OAuth authorization server with the extension traits and the stock profile | O4 and O6; the Bluesky app signs in (O8 recorded) | 5 to 6 weeks |
| 6 | The `linkjar` extension crate replacing the six patches; handle hosts; `io.linkjar.account.*` | O7 green; the patch docs' invariants have tests in the crate | 2 to 3 weeks |
| 7 | Migration endpoints, performance targets, metrics and alert rules, Nix package, restore drill on staging, cutover runbook | SPEC §16 targets met or revised with evidence; §18.1 preconditions met | 3 weeks |

Unit 1 pays for itself alone: it guards the current patch stack across
upstream bumps, which nothing does today beyond the image smoke gate.

## Status

| Date | Event |
|---|---|
| 2026-10-08 | Decision taken. Branch `feat/rust-pds` created from `main`. SPEC revision 0 written from the reference source at the pin and the six patch documents. No code yet. |
| 2026-10-08 | Scope widened by the owner: storage and auth extensibility, clustering, audit and provenance, simulation testing, a database switch, devenv. [State-of-the-art research](research-2026-10-08-state-of-the-art.md) recorded; its §9 lists the SPEC sections to add in revision 1. Headline: the account and OAuth crate should run embedded or as an entryway, which is how the protocol itself scales; audit checkpoints can be published as repository records so the firehose witnesses them. |

## Open questions for the owner

- Repository name for publication: this repository (`linkjar/pds`) becomes
  the public home, or a new one. The SPEC assumes this repository.
- Whether `verify` and the shadow comparison in unit 3 may run on the
  production host against a copy of the live data directory, or only on
  staging restores.

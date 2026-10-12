# Shared-node trees: patch 104

`104-mst-shared-node.patch` carries one upstream fix on the frozen pin. The
owner decided it on 2026-10-12 ([plan.md](../plan.md)); the evidence is in the
[upstream 0.5.35 to 0.5.37 note](../research/2026-10-10-upstream-0.5.35-to-0.5.37.md),
§3 and §5.

## What it fixes

A repository tree (MST) can be written so that two parents point at the same
node. At `@atproto/pds@0.5.34` the traversals that enter more than one child
of a node keep no record of the nodes they have entered. Upstream describes
the result: the cost of a traversal is not bounded by the number of blocks.
`com.atproto.repo.importRepo` runs such a traversal over a tree that the
caller supplies, inside the single Node process that serves every account.

Any account with a full-access session may call `importRepo`, and imports are
on unless `PDS_ACCEPTING_REPO_IMPORTS` is `false`. The effect is on
availability. Nothing in the fix suggests that another account's data can be
read or changed this way.

## What the patch changes

It is upstream commit
[`a6561d5`](https://github.com/bluesky-social/atproto/commit/a6561d54227756876a0901e4a87675df17972d32)
([#5584](https://github.com/bluesky-social/atproto/pull/5584), released in
0.5.37), cut to its source and test files:

- `packages/repo/src`: `cid-set.ts`, `error.ts`, `index.ts`, `mst/mst.ts`,
  `mst/walker.ts`. Each traversal that can enter more than one child keeps
  the set of node CIDs it has entered and throws `VisitedCidError` on a
  repeat. Record values are not marked: two records with the same content
  share one CID and stay valid.
- `packages/pds/src/api/com/atproto/repo/importRepo.ts`: the error becomes
  400 `InvalidRequest`, "invalid MST: node referenced more than once: <cid>".
- Three test files: `packages/repo/tests/mst-dag.test.ts`,
  `packages/repo/tests/cid-set.test.ts` and
  `packages/pds/tests/import-repo.test.ts`.

The changeset and the contributor documents of the upstream commit are left
out. The patch applies at the pin alone and after the other eight, and it
touches none of their files.

One effect reaches beyond the import: `sync.getRepo` streams a repository
through the same traversal. If a stored repository already has a shared node,
`getRepo` now fails for that account. Whether any account on production has
one is not known. The Candidate's `verify` command reports it as
`tree-shared-node` (SPEC §18.2).

## Tests

- The image build runs `mst-dag.test.ts` and `cid-set.test.ts` in
  `packages/repo` before it prunes development dependencies. The stock build
  applies no patch and skips them.
- `import-repo.test.ts` needs upstream's test network and does not run in the
  image build. The Harness has an `importRepo` scenario for the same case,
  `default/15-import`, which runs against revision 12 of the reference image.

## Removal

Remove the patch when `upstream.json` pins `@atproto/pds@0.5.37` or later.

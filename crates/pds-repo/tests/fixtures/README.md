# Repository fixtures from the Reference's library

Written by `@atproto/repo` 0.10.14 at the pin, through
[`parity/src/tools/repo-fixtures.ts`](../../../../parity/src/tools/repo-fixtures.ts).
The tests in [`../reference.rs`](../reference.rs) replay them and require the
same bytes.

```sh
cd parity && pnpm install --frozen-lockfile
node src/tools/repo-fixtures.ts ../crates/pds-repo/tests/fixtures
```

| File | Holds |
|---|---|
| `reference.json` | Signatures for fixed keys and messages; tree roots for five sets of keys; thirteen commits on one repository with the CIDs of their new, removed and firehose blocks; the list of the CAR files. |
| `export.car` | `getFullRepo` of that repository after the last commit. |
| `dag-diamond-<depth>.car`, `dag-shared-chain-50x50.car` | Trees that reference one node more than once: the shapes of upstream's `packages/repo/tests/mst-dag.test.ts` at `a7c8604d` (0.5.37). |

The file changes on every run: the library takes each `rev` from its clock,
and the commit CIDs follow. The tests read the `rev` values from the file.

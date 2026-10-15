# pds-types

The base crate of the workspace. It holds the AT Protocol syntax types, the
data model value with its JSON and DAG-CBOR forms, a strict DAG-CBOR reader
and writer with decode limits, the CID rules, and the API types generated
from the lexicons. It does no I/O and reads no configuration.

## Modules

| Module | Holds |
|---|---|
| `syntax` | `Did`, `Handle`, `Nsid`, `Tid` and `TidGenerator`, `RecordKey`, `AtUri`, `Datetime`, `Language`, `Uri`, `CidString`, `AtIdentifier`. Each type checks its syntax when parsed. |
| `data` | `Data`, `CidLink`, `Bytes` and `BlobRef`: the data model value, with `from_json_slice`, `from_dag_cbor_with` and `to_dag_cbor`. |
| `cbor` | The strict DAG-CBOR `Reader` and `Writer`, `decode`, `encode`, `check`, and `Limits` (`RECORD`, `FRAME`). |
| `content` | CIDv1 with SHA-256: `cid_for_cbor`, `cid_for_raw`, `repo_cid_kind`. |
| `generated` | API types written by `cargo xtask codegen` from `lexicons/`: `app.bsky`, `com.atproto` and `io.linkjar`. |

Rules beyond syntax are offered as checks, such as
`Handle::is_disallowed_tld`, and are not enforced here.

## Place in the workspace

Depends on no workspace crate. Used by pds-lexicon, pds-repo, pds-identity,
pds-blob, pds-audit, pds-sequencer, pds-xrpc, pds-ext-linkjar, linkjar-pds
and pds-sim, and by `fuzz/` and `bench/`.

## What it needs

Nothing at run time. `cargo xtask codegen` rewrites `src/generated/mod.rs`
from `lexicons/`, and CI fails when that file is stale.

## Tests

`cargo test -p pds-types`. The inline tests read the vectors under
`interop/syntax` and `interop/data-model`, and a property test compares the
codec with `serde_ipld_dagcbor`. `tests/generated.rs` round-trips generated
types. No network, no Docker.

## Specification

SPEC §6.1 to §6.3 (the data model, CBOR and CIDs) and §9 (identifiers).

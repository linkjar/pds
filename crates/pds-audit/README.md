# pds-audit

The format and the mathematics of the tamper-evident audit log: entries of
thirteen fields in DAG-CBOR chained by SHA-256, RFC 6962 Merkle roots,
checkpoints signed with Ed25519, and an offline verifier that recomputes
everything a second way. It does not store entries (pds-storage's
`AuditDb`), and it does not decide what is recorded or publish the
checkpoints (pds-xrpc's `audit`).

## Modules

| Module | Holds |
|---|---|
| `entry` | `Draft`, `Entry`, `GENESIS`: the encoding and the chain. |
| `merkle` | `root`, `proof`. |
| `checkpoint` | `Checkpoint`, `Signer::from_seed`. The signed domain is `linkjar-pds-audit-checkpoint-v1`. |
| `verify` | `verify`, `Report`: the bundle verifier. |

The binary `pds-audit-verify <bundle.json>` runs the verifier on its own
and exits 0, 1 or 2. `linkjar-pds audit verify` is the same check.

## Place in the workspace

Depends on pds-types. Used by pds-xrpc and linkjar-pds.

## What it needs

Nothing at run time. The signing seed comes from the server's
`PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX`. Ed25519 comes from `aws-lc-rs`,
so a C toolchain is needed at build time.

## Tests

`cargo test -p pds-audit`: RFC 6962 roots and proofs, and tampering
detected for every tree size.

## Specification

SPEC §20 (§20.2 to §20.4).

# pds-identity

Identity: keys and low-S signatures, `did:key`, the DID-document contract,
the supported DID methods, the handle rules beyond syntax, the building and
signing of `did:plc` operations, the reserved names, and an async resolver
that fetches DID documents through a cache the caller supplies. It makes no
DNS lookups of handles; the server does those.

## Modules

| Module | Holds |
|---|---|
| `keys` | `Keypair`, `PublicKey`, `Algorithm`: secp256k1 and P-256, `did:key`, signatures with the low-S rule on both curves. |
| `did_doc` | `DidDocument`, the contract of SPEC §9.3. |
| `method` | `SupportedDid`: `did:plc` and `did:web`. |
| `handle` | `check_handle_policy`, and the parsing of the TXT and well-known answers. |
| `plc` | `create_op`, `update_op`, `update_handle_op`: signed `did:plc` operations. |
| `reserved` | The reserved names of the Reference at the pin. |
| `resolver` | `DidResolver`, `DidCache`, `ResolverConfig`: resolution over HTTP with the Reference's two cache lifetimes. |

## Place in the workspace

Depends on pds-types. Used by pds-oauth, pds-sidecar, pds-xrpc,
pds-console, pds-mcp, pds-ext-linkjar, linkjar-pds and pds-sim, and by
`fuzz/` and `bench/`.

## What it needs

The resolver reaches the PLC directory and `did:web` hosts over the
network. The caller fills `ResolverConfig`; the server reads
`PDS_DID_PLC_URL`, `PDS_ID_RESOLVER_TIMEOUT`, `PDS_DID_CACHE_STALE_TTL`,
`PDS_DID_CACHE_MAX_TTL` and `PDS_DEV_MODE` for it. The `DidCache`
implementation is linkjar-pds's, over pds-storage's `DidCacheDb`. The
resolver does not apply the safe fetch of SPEC §15.1; its module
documentation records the gap.

## Tests

`cargo test -p pds-identity`. The inline tests of `keys` read
`interop/crypto`. `tests/resolver.rs` runs against a listener on 127.0.0.1
and an in-memory cache. No Docker.

## Specification

SPEC §9 (§9.1 to §9.4), §6.1, §15.1 and §15.2.

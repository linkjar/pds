# The crates

The workspace under `crates/` has seventeen crates, one per subsystem, and
`xtask/` for the repository tasks. Each crate's `README.md` says what it
does, what it needs and how it is tested; this page is the map. The
dependency diagram is in [architecture/README.md](architecture/README.md#crates),
and SPEC §3.1 is the target it is held to.

## Layers

| Layer | Crates | What they are |
|---|---|---|
| Base | [pds-types](../crates/pds-types/README.md) | The syntax types, the data model, DAG-CBOR, CIDs and the generated API types. No I/O. |
| Rules | [pds-lexicon](../crates/pds-lexicon/README.md), [pds-repo](../crates/pds-repo/README.md), [pds-identity](../crates/pds-identity/README.md), [pds-oauth](../crates/pds-oauth/README.md), [pds-audit](../crates/pds-audit/README.md), [pds-ext](../crates/pds-ext/README.md), [pds-ext-linkjar](../crates/pds-ext-linkjar/README.md) | Libraries the server calls: schema and validation, the repository engine, keys and DIDs, the OAuth rules and pages, the audit format, the extension boundary and the `linkjar` profile. Only pds-identity's resolver does network I/O. |
| Storage | [pds-storage](../crates/pds-storage/README.md), [pds-blob](../crates/pds-blob/README.md) | Rows in SQLite or Postgres in the Reference's schema, and blob bytes on disk or in S3. |
| Server | [pds-sequencer](../crates/pds-sequencer/README.md), [pds-xrpc](../crates/pds-xrpc/README.md) | The event log and the firehose, and the whole HTTP surface. |
| Binaries | [linkjar-pds](../crates/linkjar-pds/README.md), [pds-sim](../crates/pds-sim/README.md) | The server with its commands, and the simulation that runs it in process. |
| Sidecars | [pds-sidecar](../crates/pds-sidecar/README.md), [pds-console](../crates/pds-console/README.md), [pds-mcp](../crates/pds-mcp/README.md) | What the sidecars share, the operations console and the MCP server. They depend on pds-oauth and pds-identity only, and reach the server over HTTP. |
| Tasks | [xtask](../xtask/README.md) | `cargo xtask codegen`, which writes the generated types and the bundled lexicons. |

## Dependencies

Each row lists the workspace crates a crate depends on, as its manifest
names them.

| Crate | Depends on |
|---|---|
| pds-types, pds-ext | nothing in the workspace |
| pds-lexicon, pds-repo, pds-identity, pds-blob, pds-audit | pds-types |
| pds-oauth | pds-identity |
| pds-ext-linkjar | pds-ext, pds-identity, pds-types |
| pds-storage | pds-repo |
| pds-sequencer | pds-types, pds-repo, pds-storage |
| pds-xrpc | pds-types, pds-lexicon, pds-repo, pds-identity, pds-storage, pds-blob, pds-sequencer, pds-oauth, pds-ext, pds-audit |
| linkjar-pds | pds-types, pds-repo, pds-storage, pds-blob, pds-identity, pds-sequencer, pds-xrpc, pds-ext, pds-ext-linkjar, pds-audit, pds-sidecar; pds-console and pds-mcp for its tests |
| pds-sim | linkjar-pds, pds-xrpc, pds-sequencer, pds-storage, pds-blob, pds-identity, pds-repo, pds-types |
| pds-sidecar | pds-identity, pds-oauth |
| pds-console, pds-mcp | pds-sidecar, pds-oauth, pds-identity |
| xtask | pds-lexicon |

Outside the workspace, `fuzz/` depends on pds-types, pds-lexicon, pds-repo,
pds-identity and pds-xrpc, and `bench/` on pds-types, pds-repo and
pds-identity.

## Where the seams are joined

- pds-repo defines `BlockStore`; pds-storage's `ActorDb` implements it.
- pds-identity defines `DidCache`; linkjar-pds implements it over
  pds-storage's `DidCacheDb`.
- pds-ext defines `Profile` and `Fetch`; pds-ext-linkjar implements
  `Profile`, pds-xrpc implements `Fetch` and calls the profile, and
  linkjar-pds picks the profile from `PDS_PROFILE`.
- Blob metadata (pds-storage) and blob bytes (pds-blob) meet in pds-xrpc.
- To sign a commit, pds-xrpc reads the actor's key through pds-storage,
  builds a `Keypair` from pds-identity and hands pds-repo a closure.
- Everything below pds-xrpc is synchronous and runs on the server's
  blocking pool, except pds-identity's resolver and pds-blob's S3 store.

## What runs, and what it needs

| Binary | Crate | Needs |
|---|---|---|
| `linkjar-pds` | linkjar-pds | `PDS_JWT_SECRET`, `PDS_ADMIN_PASSWORD`, a writable data directory, a blob directory or S3, the port of `PDS_PORT`, outbound HTTPS to the PLC directory and the AppView. Every variable: SPEC Appendix C and the [environment page](https://pds.linkjar.io/configure/environment/) of the guide. |
| `linkjar-pds-console` | pds-console | `CONSOLE_PUBLIC_URL`, `CONSOLE_PDS_PUBLIC_URL`, `CONSOLE_OPS_URL`; a PDS whose `PDS_OPS_CLIENT_IDS` lists it. |
| `linkjar-pds-mcp` | pds-mcp | `MCP_PUBLIC_URL`, `MCP_PDS_PUBLIC_URL`, `MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX`; a TLS proxy; a PDS whose `PDS_MCP_CLIENT_IDS` lists it. |
| `pds-sim` | pds-sim | Only the temporary directory. |
| `pds-audit-verify` | pds-audit | One bundle file. |

## What a build needs

- A C compiler: `rusqlite` compiles SQLite in, and `aws-lc-rs` builds
  `aws-lc-sys` for Ed25519, RSA and TLS.
- libclang, for pds-sim's VFS plugin. `devenv shell` sets `LIBCLANG_PATH`.
- The Tailwind CLI, for pds-console: `build.rs` downloads the pinned
  release, or `TAILWIND_CLI` names a local one.
- `rustfmt`, for `cargo xtask codegen`.
- Node 24 and pnpm, for the parity harness, the architecture diagrams
  (`devenv shell -- docs:diagrams`), the repository fixtures and the
  self-hosting guide in `site/`.
- A nightly toolchain and cargo-fuzz, to run `fuzz/`.

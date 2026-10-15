# pds-mcp

The MCP server, a sidecar whose binary is `linkjar-pds-mcp`. Toward an AI
client it is an MCP endpoint over Streamable HTTP and an OAuth 2.1
authorization server with a consent page. Toward the PDS it is a
confidential OAuth client with `private_key_jwt` and DPoP, which reads and
writes the person's repository after they sign in on the PDS's pages. It
keeps everything in memory.

## Routes

`/mcp`; the discovery documents `/.well-known/oauth-protected-resource`,
`/.well-known/oauth-authorization-server` and `/oauth-client-metadata.json`;
the OAuth endpoints `/register`, `/token`, `/revoke`, `/authorize`,
`/authorize/decision` and `/oauth/pds-callback`; `/health` and `/mcp.css`.

## Modules

| Module | Holds |
|---|---|
| `config` | The environment. |
| `rpc` | The JSON-RPC endpoint and the tools. |
| `auth` | The authorization server for AI clients, with the consent page. |
| `clients` | Client registration, and fetching a client metadata document. |
| `pds` | The client of the PDS. |
| `store` | What it keeps in memory: clients, codes, tokens, sessions. |

## Place in the workspace

Depends on pds-sidecar, pds-oauth and pds-identity. linkjar-pds has it as
a dev-dependency for the end-to-end test.

## What it needs

- `MCP_PUBLIC_URL` (https, or http on loopback), `MCP_PDS_PUBLIC_URL` and
  `MCP_CLIENT_KEY_P256_PRIVATE_KEY_HEX`. `MCP_ADDRESS` defaults to
  `127.0.0.1:2586`; `MCP_PDS_INTERNAL_URL` and `MCP_WRITE_COLLECTIONS` (the
  NSIDs that become write scopes) are optional; `LOG_LEVEL` as for the
  server.
- A TLS proxy in front, a reachable PDS that lists
  `<MCP_PUBLIC_URL>/oauth-client-metadata.json` in `PDS_MCP_CLIENT_IDS`,
  and outbound HTTP to fetch the metadata of AI clients.

## Tests

`cargo test -p pds-mcp` runs the inline tests of `config`, `clients`,
`pds`, `rpc` and `store`. The end-to-end test, with the official MCP
TypeScript client and the MCP Inspector, is in
`crates/linkjar-pds/tests/oauth.rs`.

## Specification

SPEC §22 (§22.2) and `docs/sidecars.md` §2.

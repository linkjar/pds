# pds-sidecar

What the two sidecars share: the OAuth client of the PDS with DPoP, what
their pages read from a request, the checks on their configuration, and
the logging and the stop signal of a process, which the server uses too.
The crate holds the HTTP client, so that pds-oauth stays free of it. It
holds no HTTP server: the console serves with Topcoat and the MCP server
with axum, and each keeps the shape of its own responses.

## Modules

| Module | Holds |
|---|---|
| `client` | `DpopClient`, `Peer`, `Pending`, `Tokens`, `ClientKey`: the pushed authorization request, the token endpoint, revocation, and every request that a proof of the session's key signs. A sidecar holds no password; the person signs in on the PDS's pages. |
| `request` | `param`, `cookie`, `fields`, `field`, `same_origin`: the query, the cookies, the fields of a form and the origin of the browser. |
| `config` | `origin`: the checks on a configured URL. |
| `process` | `init_logging`, `shutdown`, and `health`, the check behind the `health` command of each sidecar and the `HEALTHCHECK` of its image. |

## Place in the workspace

Depends on pds-identity and pds-oauth. Used by pds-console, pds-mcp and
linkjar-pds.

## What it needs

Nothing of its own. The sidecar that uses it needs a reachable PDS.

## Tests

Inline tests, run by `cargo test -p pds-sidecar`. The sign-in flows are
tested end to end in `crates/linkjar-pds/tests/oauth.rs`.

## Specification

SPEC §22 and `docs/sidecars.md`.

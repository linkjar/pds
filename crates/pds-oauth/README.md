# pds-oauth

The OAuth rules without network I/O: DPoP proofs and nonces, PKCE, client
metadata validation, scopes, tokens (HS256 and ES256K JWTs, ephemeral
sign-in tokens, client assertions), the server-rendered pages with their
theme, and the client half (`agent`) that the sidecars use. The routes,
the database rows and the fetching are in `pds-xrpc/src/oauth`.

## Modules

| Module | Holds |
|---|---|
| `dpop` | `check_proof`, `Nonces`, `jwk_thumbprint` (RFC 9449). |
| `pkce` | `verify_s256`. |
| `client` | `validate`: client ids and metadata documents. |
| `scope` | `Permissions`: from scopes to what a token may do. |
| `token` | `mint_access`, `verify_access`, `mint_ephemeral`, `check_assertion`, and the lifetimes. |
| `pages` | The sign-in, sign-up, consent, account and error pages: `Brand`, `Flow`, `consent`, `account_home`. The functions are pure. |
| `theme` | The LinkJar journey: the design tokens, the stylesheet and the assets under `assets/` (`asset`). |
| `agent` | `dpop_proof` and the client assertions a sidecar sends. |

## Place in the workspace

Depends on pds-identity. Used by pds-xrpc, pds-sidecar, pds-console and
pds-mcp.

## What it needs

Nothing: secrets are passed in, and the font and the marks are compiled in
from `assets/` (see `assets/NOTICE.md`). There is no JWT crate; the tokens
are built by hand.

## Tests

`cargo test -p pds-oauth`: inline tests, among them
`pages_keep_the_contract_of_the_harness`, which holds the pages to what the
parity harness's page driver expects. The pages are shown in
`docs/pages/README.md`.

## Specification

SPEC §11 (§11.1 to §11.4), §5.5, §21.3 and §22.

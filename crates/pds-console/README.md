# pds-console

The operations console, a sidecar whose binary is `linkjar-pds-console`.
Its pages are rendered on the server with Topcoat. An operator signs in
through the PDS's own OAuth pages, with the console as a loopback client,
and every read and every action is a call to the PDS's ops listener. It
has no database and no secrets, and keeps its sessions in memory.

## Commands

`linkjar-pds-console` serves. `bundle-assets [DIR]` writes the asset
bundle and exits. `health` asks the console on this host for `/health` and
exits 0 only on 200: the health check of the image.

## Modules

| Module | Holds |
|---|---|
| `config` | The environment. |
| `client` | The OAuth and DPoP client of the PDS, and the client of the ops listener. |
| `session` | The sessions, in memory. |
| `pages` | The pages: the overview, the accounts, one account, the audit log, the takedown dialogs. |
| `live` | The live view, fed by the `internal.ops.subscribeState` stream. |
| `ui` | Components in the anatomy of Base UI, rendered with Topcoat. |
| `assets` | The asset bundle: `bundle-assets [DIR]` writes it (the image does this at build time), and a run without one builds it in the temporary directory. |

## Place in the workspace

Depends on pds-sidecar, pds-oauth and pds-identity. linkjar-pds has it as
a dev-dependency for the sign-in test.

## What it needs

- `CONSOLE_PUBLIC_URL` (a loopback `http://127.0.0.1:<port>` or
  `http://[::1]:<port>`), `CONSOLE_PDS_PUBLIC_URL` and `CONSOLE_OPS_URL`.
  `CONSOLE_ADDRESS` defaults to `127.0.0.1:2585`; `CONSOLE_PDS_INTERNAL_URL`
  is optional; `LOG_LEVEL` as for the server.
- The PDS must list the console's client id in `PDS_OPS_CLIENT_IDS` and
  serve its ops listener at `PDS_OPS_ADDRESS`.
- At build time, the Tailwind CLI: `build.rs` downloads the pinned release
  and checks its hash, or uses the executable that `TAILWIND_CLI` names.

`docs/sidecars.md` §1 is the design. The deployment beside the PDS is in
`linkjar/infra`.

## Tests

`cargo test -p pds-console` renders the pages against a stand-in PDS
router (`src/tests.rs`). The sign-in against a real server is in
`crates/linkjar-pds/tests/oauth.rs`. The screenshots are in
`docs/research/2026-10-16-console-topcoat/`.

## Specification

SPEC §22 (§22.1) and `docs/sidecars.md`.

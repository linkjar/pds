# linkjar-pds

The server binary. `serve` runs the PDS on a data directory in the
Reference server's layout; the other commands check, export, audit,
migrate and reconcile that directory. The library side exposes the same
parts, so that the tests and pds-sim run the server in process.

## Commands

| Command | Does |
|---|---|
| `serve` | Runs the server. |
| `verify [--json]` | Checks the data directory without writing; exit 1 on a difference. |
| `export-car <did>` | Writes one repository as a CAR to stdout. |
| `audit export [<from> [<to>]]` | Writes an audit bundle to stdout. |
| `audit seal` | Writes the first audit entry, which stands for the history before the log. |
| `audit verify <file>` | Checks a bundle offline; needs no configuration. |
| `migrate-storage --to postgres\|sqlite` | Copies the shared databases between the SQLite files and Postgres. |
| `reconcile [--remove-orphans]` | Compares an entryway's accounts with its member host's repositories. |
| `check-config` | Validates the environment. |
| `health` | Asks `127.0.0.1:$PDS_PORT/xrpc/_health`; exit 0 only on 200. The image's health check. |
| `version`, `help` | |

## Modules

| Module | Holds |
|---|---|
| `config` | `Config`, read from the environment: every variable of SPEC Appendix C with the Reference's defaults. An unknown `PDS_*` name gets a warning, and so does a variable that this version accepts and does not read (`UNREAD` in `config.rs` names them and why). |
| `serve` | The listeners, the lock on the data directory, the repair at start, the shutdown. |
| `verify` | The directory checker. |
| `reconcile` | The entryway and member-host comparison. |

## Place in the workspace

Depends on pds-types, pds-repo, pds-storage, pds-blob, pds-identity,
pds-sequencer, pds-xrpc, pds-ext, pds-ext-linkjar, pds-audit and
pds-sidecar; pds-lexicon and pds-oauth come through pds-xrpc. pds-console
and pds-mcp are dev-dependencies, for the sign-in tests. Used by pds-sim.

## What it needs

- `PDS_JWT_SECRET` and `PDS_ADMIN_PASSWORD`; everything else has a
  default. The self-hosting guide's
  [environment page](https://pds.linkjar.io/configure/environment/) and
  SPEC Appendix C list the variables. `PDS_PROFILE` picks `stock` or
  `linkjar`.
- A writable data directory (`PDS_DATA_DIRECTORY`) with the SQLite files,
  the actor stores and `linkjar-pds.lock`, and a blob directory or an S3
  bucket.
- The listener of `PDS_PORT` (2583; the image sets 3000) on every
  address. The metrics listener (`PDS_METRICS_ADDRESS`) and the ops
  listener (`PDS_OPS_ADDRESS`) are optional.
- Outbound HTTPS to the PLC directory, the AppView, the crawlers and
  SMTP. Postgres is optional.
- `LOG_LEVEL` is the tracing filter, `info` by default.

## Tests

`cargo test -p linkjar-pds`. `tests/` covers accounts, the command line
(it runs the built binary), crashes at the fail points, OAuth (the console
and the MCP server sign in against the real server), the proxy, sessions,
`verify` and writes; `tests/common/mod.rs` builds a data directory with
the Reference's schema. They need loopback ports and the temporary
directory. `PDS_TEST_POSTGRES_URL` turns the Postgres cases on.
`PDS_TEST_MCP_CLIENT=node` with `parity/node_modules` installed runs the
official MCP client. Building the tests builds pds-console, which needs
the Tailwind CLI.

## Specification

SPEC §14 (§14.1, §14.2 and Appendix C).

# pds-xrpc

The whole HTTP surface of the server. One fallback entry point,
`api::entry`, dispatches `/xrpc/<nsid>` and the plain routes. The crate
holds the request validation with the Reference's messages, every
credential check, the rate limits, the write path, the proxy to the
AppView and to other services, read-after-write, mail, metrics, the
audit-log writer, the ops listener and the HTTP side of OAuth. It does not
own the OAuth rules or the page HTML (pds-oauth), the schema (pds-storage),
the environment (linkjar-pds) or operator behaviour (pds-ext).

## Modules

| Module | Holds |
|---|---|
| `api` | The dispatcher and the handlers: `account`, `admin`, `email`, `entryway`, `identity`, `import`, `invites`, `lifecycle`, `linkjar`, `plc`, `prefs`, `proxy`, `read_after_write`, `repo`, `repo_write`, `root`, `server`, `session`, `subscribe`, `sync`. |
| `oauth` | The HTTP side of OAuth: `account`, `authorize`, `captcha`, `clients`, `endpoints`, `external` (sign-in through a provider of the profile), `metadata`, `notices`, `resource`. |
| `state` | `AppState`, `ServerConfig`, `WriteState`, `Branding`: everything the handlers share. |
| `write` | The write path: a commit from a batch of writes, its event, and `repair` after a crash. |
| `auth`, `service_auth`, `password` | Credentials: legacy sessions, app passwords, OAuth access tokens with DPoP, service tokens, the admin password. |
| `ratelimit`, `params`, `body`, `error` | The limits, query parsing, body limits, and `XrpcError` with the Reference's wording. |
| `audit`, `ops`, `metrics` | `AuditLog`; the ops listener (`ops::serve`) that the console calls; the Prometheus metrics. |
| `mail`, `stream`, `subscribers` | Mail over SMTP with the Reference's templates, or the profile's own (`Profile::mail_template`); streamed bodies; the firehose subscribers. |

## Place in the workspace

Depends on pds-types, pds-lexicon, pds-repo, pds-identity, pds-storage,
pds-blob, pds-sequencer, pds-oauth, pds-ext and pds-audit. Used by
linkjar-pds and pds-sim, and by the fuzz targets `xrpc_body` and
`xrpc_request`.

## What it needs

No environment of its own: linkjar-pds reads the `PDS_*` variables into
`ServerConfig`. A data directory through `DataLayout`, with the actor key
files. Over the network: the PLC directory, the AppView, SMTP, hCaptcha
when it is on, the identity providers of the `linkjar` profile, client
metadata documents, and the entryway or the member host in those modes.

## Tests

`cargo test -p pds-xrpc` runs the inline tests of each module and
`tests/head.rs`. The server-level tests, with a data directory and real
listeners, are in `crates/linkjar-pds/tests/`.

## Specification

SPEC §4 (§4.1 to §4.5), §5, §10, §11, §12, §13, §20, §21.3 and §22.

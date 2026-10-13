# Unit 8, first part: migration in, metrics, the lock and the runbook

Date: 2026-10-13 · [Plan](../plan.md) · [SPEC §18](../SPEC.md#18-cutover-and-rollback) · [SPEC §14.4](../SPEC.md#144-telemetry)

Unit 8 is the cutover. Most of it is work on staging, on production and in
`linkjar/infra`, which the owner does. This part is what the repository
can hold.

## 1. Result

| Part | State |
|---|---|
| An account that moves here | Built. `createAccount` with `did` and a service token of that DID. |
| Metrics | Built: `GET /metrics` on `PDS_METRICS_ADDRESS`, in the Prometheus text format. |
| One server for a data directory | Built: the lock file `linkjar-pds.lock`. |
| The cutover procedure | Written: [`docs/runbooks/cutover.md`](../runbooks/cutover.md). |

## 2. An account that moves here

The request carries the DID and a service token that the DID's signing
key signed, with the server as audience and
`com.atproto.server.createAccount` as method. The server then:

- creates the account deactivated, with an empty repository under a new
  signing key;
- sends no operation to the PLC directory, because the old server still
  holds the identity;
- appends no event until `activateAccount`;
- refuses a handle under a domain that is not the server's unless the
  handle names the DID.

The next steps are the ones the server had already: `importRepo`,
`uploadBlob`, `getRecommendedDidCredentials`, the PLC operation, and
`activateAccount`. With hCaptcha configured, the `linkjar` profile closes
the legacy `createAccount` to every request except this one.

## 3. Metrics

The listener is separate from the public one and has no authentication:
bind it to a private address.

| Metric | Type | Labels |
|---|---|---|
| `pds_build_info` | gauge | `version`, `profile` |
| `pds_http_requests_total` | counter | `route`, `status` |
| `pds_http_request_duration_seconds` | histogram | `route` |
| `pds_audit_entries` | gauge | |
| `pds_audit_unsigned_entries` | gauge | |
| `pds_audit_checkpoint_age_seconds` | gauge | |
| `pds_notice_outbox_depth` | gauge | |

A `route` is the NSID of a method the server handles, a path it knows,
`proxied`, or `other`. A client cannot create a label by asking for a
path. The server writes the text itself: no metrics crate is added.

## 4. The lock

`serve` takes an exclusive lock on `linkjar-pds.lock` in the data
directory before it opens a database. A second `serve` on the directory
prints the reason and exits 2. The lock is the operating system's, so it
ends with the process. `verify`, `audit export` and `audit verify` read
only and take no lock. The reference image does not know the file: the
runbook stops one server before it starts the other.

## 5. How it is checked

`crates/linkjar-pds/tests/accounts.rs`: a migration with its refusals
(no token, a token of another DID, a token for another method, a wrong
key), the state of the new account, and the metrics over HTTP.
`crates/pds-xrpc/src/metrics.rs`: the labels and the text.
`crates/linkjar-pds/tests/cli.rs`: the second server.

## 6. What is not built

1. Export over OTLP (SPEC §14.4).
2. Metrics of the commit path, the sequencer, the subscribers, the actor
   store cache, SQLite busy events and the background tasks.
3. A Nix package and a release workflow for the image.
4. The Harness has no scenario for a migration between two servers. The
   test above drives the Candidate alone.

## 7. What is the owner's

1. The restore drill on staging and `verify` on the copy.
2. The service definition, the alert rules and the recovery runbook in
   `linkjar/infra`.
3. The measurement against the targets of SPEC §16 on the production
   host.
4. The cutover and the 48 hours after it.

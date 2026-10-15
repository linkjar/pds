# Cutover from the Reference to the Candidate

[SPEC §18](../SPEC.md#18-cutover-and-rollback) · [Plan](../plan.md)

This runbook replaces the reference image with `linkjar-pds` on the same
host name and the same data directory. Each step names the command and
what to expect. The operator runs every step: none of them is automated.

## 1. Before the day

Do each of these on staging, on a restored copy of the production data
directory. Production is touched only at cutover.

1. **Restore a copy.** Follow the recovery runbook of `linkjar/infra`.
2. **Check the copy.** Set the `PDS_*` variables of production, with
   `PDS_DATA_DIRECTORY` and the blob store of the copy.

   ```bash
   linkjar-pds check-config
   ```

   ```bash
   linkjar-pds verify
   ```

   `verify` must print no difference and exit 0. It writes nothing. A
   finding is a fact about production data: resolve it before the day.

   Check that `PDS_BSKY_APP_VIEW_URL` and `PDS_BSKY_APP_VIEW_DID` are set.
   Without an AppView the server has no handler for `getPreferences`, as
   the Reference, and the Bluesky app cannot load its settings.
3. **Run the Harness against the copy** with the `linkjar` profile
   ([parity/README.md](../../parity/README.md)).
4. **Create the checkpoint key.** 32 random bytes as 64 hexadecimal
   digits, kept in the host's secret store as
   `PDS_AUDIT_KEY_ED25519_PRIVATE_KEY_HEX`.

   ```bash
   openssl rand -hex 32
   ```

5. **Name the operator's repository.** Set `PDS_AUDIT_CHECKPOINT_REPO` to
   the DID of the operator's account on the server. Without it the
   checkpoints stay in `audit.sqlite` and nobody outside sees them.
6. **Set the profile and the listeners.** `PDS_PROFILE=linkjar`, and
   `PDS_METRICS_ADDRESS` on a private address, such as `127.0.0.1:9464`.
7. **Leave the secrets as they are:** `PDS_JWT_SECRET`, `PDS_DPOP_SECRET`,
   the rotation key, the admin password, the SMTP and the provider
   credentials. Sessions from before the cutover depend on them.
8. **Choose the image.** Each push to `main` publishes the server as
   `ghcr.io/linkjar/pds-server`, for `amd64` and `arm64`, and the run of
   the "Server images" workflow lists the digest in its summary. Deploy
   `ghcr.io/linkjar/pds-server@sha256:…`, never a tag. The sidecars are
   `ghcr.io/linkjar/pds-console` and `ghcr.io/linkjar/pds-mcp`.

   A host without a login to `ghcr.io` takes the image by copy. Pull the
   host's architecture on a workstation, then load it on the host.

   ```bash
   docker pull --platform linux/arm64 ghcr.io/linkjar/pds-server@sha256:…
   ```

   ```bash
   docker save --platform linux/arm64 ghcr.io/linkjar/pds-server@sha256:… | ssh operator@host sudo docker load
   ```

   `docker load` prints the image ID. With the containerd image store it
   is the digest of the architecture's manifest in the published index.
   Pin that ID, so that the host never pulls.
9. **Update `linkjar/infra`:** the service definition for the new image,
   the alert rules for the metrics below, and the recovery runbook. The
   recovery service must also copy `audit.sqlite` and `ops.sqlite`, the
   Candidate's own files in the data directory.

## 2. The cutover

1. Announce maintenance on the status page.
2. Stop the reference container. Check that it is stopped: the two servers
   must never serve one data directory.
3. Check the data directory.

   ```bash
   linkjar-pds verify
   ```

   Stop here when it exits 1. Start the reference container again.
4. Seal the history. This writes the first entry of the audit log: the
   number of accounts and the SHA-256 of their DIDs and repository heads.

   ```bash
   linkjar-pds audit seal
   ```

   It prints `sealed <n> accounts as sha256:…` and `signed by ed25519:…`.
   Keep both lines with the record of the cutover.
5. Start the Candidate.

   ```bash
   linkjar-pds serve
   ```

6. Check what a client sees.

   ```bash
   linkjar-pds health
   ```

   ```bash
   curl -s https://pds.linkjar.social/xrpc/com.atproto.server.describeServer
   ```

   ```bash
   curl -s https://pds.linkjar.social/.well-known/oauth-authorization-server
   ```

   The relay's connection to `subscribeRepos` must resume at its last
   cursor. Look for it in the relay's own status.

   ```bash
   curl -s "https://bsky.network/xrpc/com.atproto.sync.getHostStatus?hostname=pds.linkjar.social"
   ```

   Check that a handle host answers with its DID.

   ```bash
   curl -s https://<handle>.linkjar.social/.well-known/atproto-did
   ```
7. Smoke test. From a workstation with the Harness, sign in to a test
   account; the `linkjar` profile takes no sign-up by `createAccount`. The
   test writes a post with an image and an app password, and removes both.

   ```bash
   read -s PDS_SMOKE_PASSWORD && export PDS_SMOKE_PASSWORD
   ```

   ```bash
   pnpm parity smoke https://pds.linkjar.social --account <test handle>
   ```

   Then sign in from the web app, from iOS and from the extension; request
   a password reset mail for a test account and receive it.

## 3. The watch

Watch for 48 hours.

| Metric | What to look for |
|---|---|
| `pds_http_requests_total{status=~"5.."}` | A rate above the Reference's baseline. |
| `pds_http_request_duration_seconds` | The 99th percentile by route against the targets of SPEC §16.2. |
| `pds_audit_unsigned_entries` | It returns to 0 within five minutes of a change. |
| `pds_audit_checkpoint_age_seconds` | It stays under 600 while `pds_audit_unsigned_entries` is above 0. |
| `pds_notice_outbox_depth` | It returns to 0. A value that grows means SMTP fails. |

After the first day, export the audit log and verify it away from the
host.

```bash
linkjar-pds audit export > audit-bundle.json
```

```bash
pds-audit-verify audit-bundle.json
```

## 4. Rollback

The rollback window is open until the operator applies the first
Candidate-only migration of a shared database. No such migration exists
yet: the migrations of the `linkjar` profile are the patch stack's own, and
the reference image knows them.

1. Stop the Candidate.
2. Start the reference image on the same data directory.

The Candidate's own files stay in place and the Reference ignores them:
`ops.sqlite`, `audit.sqlite`, `linkjar-pds.lock`. A session that an OAuth
client opened on the Candidate goes on at the Reference, because the rows
are the Reference's.

## 5. What this runbook does not cover

- The restore itself and the backup schedule, which the recovery runbook
  of `linkjar/infra` owns.
- A move to Postgres or to an entryway (units 11 and 12).
- Export over OTLP: the Candidate has the metrics listener only.

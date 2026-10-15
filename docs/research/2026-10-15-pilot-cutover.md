# The Candidate on the pilot host

Date: 2026-10-15 · [Plan](../plan.md) · [Cutover runbook](../runbooks/cutover.md) · [SPEC §18](../SPEC.md#18-cutover-and-rollback)

On 2026-10-15 at 08:40 UTC `linkjar-pds` replaced the reference image on
the host that serves `pds.linkjar.social`. The host runs the invite-only
public pilot that the operator approved on 2026-10-06, with five accounts.
The switch followed §2 of the cutover runbook on the host's own data
directory. The 48-hour watch, the restore drill and the measurements on
this hardware are still open.

## 1. Result

| Step | State |
|---|---|
| `check-config` with the host's environment and `PDS_PROFILE=linkjar` | Valid. |
| `verify` on the data directory, the reference stopped | 5 accounts, 5 repositories, 9 records, 1 blob reference, 41 events, 0 differences. |
| `audit seal` | 5 accounts sealed; not signed, as no checkpoint key is set. |
| The switch | NixOS rebuild with the image `ghcr.io/linkjar/pds-server@sha256:2545bf0c…` (`main` at `6013cca`). |
| Backups after the switch | Litestream replicates all ten databases; the host-material backup succeeds; the recovery monitor reports a matching inventory. |
| `pnpm parity smoke https://pds.linkjar.social` | 8 of 8. |
| `pnpm parity smoke … --account ljtester1.linkjar.social` | 20 of 20 after the AppView settings below; the post, image and app password it made are gone. |
| The five accounts | Each repository exports; each handle verifies; `plc.directory` points each at `pds.linkjar.social`. |
| The network | `bsky.network` lists the host as active with five accounts and follows its firehose; the Bluesky AppView shows all five handles. |

## 2. What the switch found

Each finding was fixed on the day.

- **The recovery tools refused the Candidate's own databases.** The
  host-material backup allowed the three shared databases and the actor
  stores only, and failed on `audit.sqlite` and `ops.sqlite`. The tools in
  `linkjar/infra` now accept and replicate both, with a test
  ([linkjar/infra#8](https://github.com/linkjar/infra/pull/8)). The cutover
  runbook named the two files; the recovery service did not know them.
- **The host named no AppView.** Without `PDS_BSKY_APP_VIEW_URL` and
  `PDS_BSKY_APP_VIEW_DID` the server has no handler for `getPreferences`
  and answers "No service configured", as the Reference does. The
  reference image ran with the same environment. Both variables are now
  set to Bluesky's AppView.
- **No handle resolved.** DNS had records for `pds` and one former handle
  only, and the tunnel sent only those names to the server, so the AppView
  showed every account as `handle.invalid`. A wildcard DNS record and a
  tunnel rule now send `*.linkjar.social` to the server, which answers each
  handle host with its DID. A no-op handle update per account sent an
  identity event, after which the AppView showed the handles.
- **`createAccount` is refused under the `linkjar` profile.** Sign-up goes
  through the OAuth page, as with the patched reference, so
  `smoke --write` cannot create its account there.
  [linkjar/pds#61](https://github.com/linkjar/pds/pull/61) added
  `smoke --account` for an account that exists.
- **The OAuth pages served a script.** Before the switch the Candidate
  served `/oauth/authorize` and the account pages without a browser's
  fetch metadata, which the Reference refuses.
  [linkjar/pds#60](https://github.com/linkjar/pds/pull/60) made them match.

## 3. How the image reached the host

The host has no login to `ghcr.io`, where the images are private. The arm64
image was pulled on a workstation and copied with `docker save` and
`docker load`. NixOS pins it by its local image ID,
`sha256:b5a220a0…`, which is the arm64 manifest of the published index;
an ID of that form makes the module run it with `--pull never`. A system
built from `fix/loosen-recovery-alerts` with the reference pin reproduced
the running system, so the switch changed the PDS container and nothing
else.

## 4. Rollback

The reference image `ghcr.io/linkjar/pds@sha256:12c1de78…` (revision 11)
stays on the host. The data directory before the switch is
`/var/lib/linkjar-pds-backup-20261015-pre-rust`, and the environment files
before each change are `pds.env.pre-rust-20261015` and
`pds.env.pre-appview-20261015`. §4 of the cutover runbook applies: no
Candidate-only migration has run, so the reference image can start on the
same data directory.

## 5. What is open

- The 48-hour watch, with the metrics of the cutover runbook. The host
  exposes no metrics listener of the Candidate yet.
- The restore drill, and the SPEC §16 measurements on this hardware.
- The Bluesky app and the `linkjar.io` browser gate by hand. The smoke
  test drives the SDK that the app uses, not the app.
- A checkpoint key and the operator's repository for the audit log.
- Merging [linkjar/infra#8](https://github.com/linkjar/infra/pull/8), which
  stands on the draft [linkjar/infra#5](https://github.com/linkjar/infra/pull/5).

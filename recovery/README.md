# PDS recovery preparation

Implements the local tooling portion of [linkjar.io #91](https://github.com/linkjar/linkjar.io/issues/91).
Nothing here provisions a VM, changes DNS, starts the public PDS or proves an R2
restore. The target from #89 is **RPO ≤60 seconds and RTO ≤30 minutes**, measured
at the actual launch dataset. The [provisioning runbook](../docs/runbooks/pds.md)
and its pending external inputs still apply.

## Data and encryption

Litestream 0.5.17 replicates `account.sqlite`, `sequencer.sqlite`,
`did_cache.sqlite` and recursively discovers new `actors/**/*.sqlite` with
`dir`, `pattern` and `watch`. Set `PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT=true`;
Litestream then owns WAL checkpointing. The reviewed layout is upstream's default
under the Compose host data directory. Custom database locations fail the
inventory gate and require an updated configuration.

SQLite replicas use the private R2 S3 endpoint, `region: auto`, one-second sync,
24-hour snapshots, seven-day snapshot retention and one-hour fine-grained L0
retention. Older retained points have coarser granularity. Each host incarnation
must use a **new replication prefix**, such as `staging/2026-09-12-host-a/databases`.
Never run a restored database against the old source replica while validating it.
Do not add bucket lifecycle deletion rules that can remove required LTX files.

Litestream 0.5 rejects client-side age encryption. The configuration requires an
independent 32-byte SSE-C key in a private file for R2; credentials alone cannot
read those object bodies without the key. SSE-C is server-side encryption and
Cloudflare processes plaintext database bytes. It is not a claim of client-side
or zero-knowledge database backup. Private Jar records remain client ciphertext.

Actor signing keys (`actors/<hash>/<DID>/key`), `actors/reserved_keys`, and the
online configuration directory are captured in a manifest and encrypted restic
repository. The timer runs 30 seconds after each completed backup. A slow or
failed capture raises the age alert; SQLite's one-second sync does not imply a
one-second recovery point for a new account's signing key. Decrypted temporary
archives use a private directory and mode 0600 and are removed after backup.

Keep the restic password and SSE-C key in the operator vault as well as the host.
Their loss makes recovery impossible. The offline higher-priority PLC recovery
private key remains offline and is **never** placed in the online configuration
directory. Back up online PLC material, JWT/DPoP secrets, TLS configuration and
image/host configuration under `/etc/linkjar-pds`; include `staging.env` there.
Only that explicit online directory is captured. Apple `.p8` custody remains in
the operator vault; only its signed runtime JWT belongs on the host.

R2 database replicas and encrypted host material use a dedicated private backup
bucket/repository, separate from the blob bucket and production/staging peers.
Use scoped credentials and retain provider snapshots as a second layer. Host
snapshots may contain live secrets and require restricted access. They do not
replace R2 blob durability or the independently stored vault/recovery material.

## Install without opening traffic

Transfer a clean checkout of the reviewed `linkjar/pds` commit to
`/opt/linkjar-pds` on the approved Ubuntu 24.04 host (Git must be installed).
Supply `/etc/linkjar-pds/{staging.env,image.env}` and private backup inputs based
on [`backup.env.example`](backup.env.example). The example is an input inventory,
not a ready-to-run configuration. Use a unique `PDS_BACKUP_PREFIX`, a dedicated
restic repository and an unused prefix when first initializing it. Initialize
that selected restic repository once with `restic init`; recovery must use the
existing repository, never silently initialize an empty replacement.

`install-tools.py --directory /opt/linkjar-pds-tools` downloads Litestream 0.5.17
and restic 0.19.1 and checks the official release archive SHA-256 values before
installing binaries. `tools.json` contains the reviewed Linux AMD64/ARM64 and
local Darwin ARM64 pins. No global developer tool is replaced by local tests.
The raw env wrapper accepts only backup-specific names and does not interpret
quotes, dollar signs or shell commands. Do not source the env file.

After restore and structural validation, run `bootstrap.sh` as root with
`PDS_RELEASE_COMMIT`, `DOCKER_APT_VERSION` and `COMPOSE_APT_VERSION` set to the
reviewed commit and available Ubuntu package versions recorded in the private
host inventory. It installs the selected Docker/Compose packages and pinned
backup binaries, verifies the checkout and existing data, pulls the configured
PDS image and installs service files. It does not overwrite a data directory,
start the PDS, or start replication. The PDS runtime UID/GID is 1000; the restore
operator must prepare restored data ownership for that image before starting it.

After the isolated acceptance below succeeds and the old host is fenced:

```sh
systemctl enable --now linkjar-pds-recovery-monitor.service linkjar-pds-litestream.service linkjar-pds-host-material.timer
```

Verify fresh backup progress against the **new** epoch before opening traffic.
The monitor and Litestream metrics bind only to loopback. Do not route them
through public PDS/handle hosts. These unit files target staging; review paths,
environment labels and bucket/prefix isolation before producing production units.

## Restore into an isolated directory

First fence the old instance: stop it or revoke its network/storage access, and
block public traffic to the replacement. There must never be two active PDS
writers for these identities. Preserve the damaged disk and source backups.

Use restic `snapshots --tag linkjar-pds-host-material --json` with the private
backup env to select an exact full snapshot ID. Choose a UTC database cutoff
after that host-material snapshot completed and within 60 seconds of its capture.
Use the original replication prefix explicitly for restore; the new prefix in
the runtime env is only for future replication.

```sh
python3 recovery/run-with-env.py /etc/linkjar-pds-backup/backup.env -- \
  python3 recovery/pds-recovery.py restore \
  --snapshot FULL_64_CHARACTER_SNAPSHOT_ID \
  --timestamp UTC_TIMESTAMP \
  --prefix staging/ORIGINAL-HOST-EPOCH/databases \
  --destination /var/lib/linkjar-pds/isolated-restore \
  --litestream /opt/linkjar-pds-tools/litestream \
  --restic /opt/linkjar-pds-tools/restic
```

The destination must not exist. A missing replica is an error, not permission to
create an empty database. The tool verifies archive paths and hashes, restores
every inventoried database with a full integrity check and compares actor
coverage and repository CID/revision against the shared account database. It
refuses missing keys, malformed key sizes, stale host material and mismatched
roots. A failed restore is marked `RESTORE-INCOMPLETE`; retry in a new directory.
An intentional older recovery point requires the matching older snapshot and a
recorded loss estimate. Do not raise `--max-host-material-age` merely to suppress
a failed freshness check.

A successful structural result still says `trafficApproved: false`. Independent
SQLite snapshots are not a transactionally consistent whole-PDS snapshot. Before
moving restored data to the reviewed runtime path, complete all of these checks:

1. Reconcile account and actor roots with the sequencer history. Account records,
   commits and emitted events must describe the same state. Investigate a mismatch
   and choose a consistent recovery point; do not edit CIDs or invent events just
   to pass the tool.
2. Import every restored key with the pinned PDS crypto implementation and verify
   its public key against that DID's current signing-key document. Verify the
   service endpoint and online/offline PLC custody. A 32-byte length check alone
   does not prove that a signing key is correct.
3. Verify referenced blobs through the private blob bucket and prove signed writes
   for designated staging fixtures. Restore secrets needed for OAuth continuity;
   prove web and iOS authorization and refresh with real provider credentials.
4. Keep the old host fenced and replacement public routing disabled while starting
   the PDS on its loopback port for checks. Confirm no startup migrations or
   restore errors. Preserve a pre-start snapshot for rollback.
5. Record incident/cutoff times, last recovered write, host-material capture,
   bytes/database count, end-to-end time-to-serve and observed data loss. Promote
   routing only after the operator accepts the evidence and new backups are
   healthy.

The tool reports local restore duration and host-material age, not a measured
end-to-end RTO/RPO. Do not substitute those values for the fresh-VM R2 drill.

## Retention and rotation

The initial technical policy is seven days of SQLite history, seven days of all
host-material captures, plus one host-material snapshot per day for 30 days.
Record this in the final hosting privacy/retention policy before public signup.
For the dedicated staging repository, an operator maintenance run can use:

```sh
python3 recovery/run-with-env.py /etc/linkjar-pds-backup/backup.env -- \
  /opt/linkjar-pds-tools/restic forget --host linkjar-pds-staging \
  --tag linkjar-pds-host-material --keep-within 7d --keep-daily 30 --prune
```

Run `restic check` before scheduled pruning and periodically verify a restored
snapshot. No pruning job is enabled by this change. Schedule maintenance away
from the short capture loop, observe lock failures and keep stale-backup alerts
active. Do not delete a source epoch until its required retention and incident
investigation are complete.

Rotate R2 credentials by adding and testing the replacement against both backup
and restore, installing it atomically in the raw env file, restarting the agent
and only then revoking the old credential. Keep blob and backup scopes separate.
For SSE-C rotation start a new epoch with a new key; retain the old key for every
old object until that epoch expires. Add/test a restic repository key before
retiring its predecessor. Rotate SMTP credentials through the existing host
inventory, test ordinary mail and an Apple relay recipient, then revoke the old
key. Keep private values out of logs and tickets.

For PLC rotation, verify the current PLC operation chain and vault custody first,
prepare and review the operation with the supported PLC/PDS tools, sign with an
authorized key, verify the resulting DID document and test writes. A lost-host
restore is not a reason to generate a new DID or casually rotate a repo key.
Takedown requests go to the monitored operator contact: verify identity, scope and
applicable policy; record a restricted audit; use the supported authenticated PDS
admin operation for the precise DID and verify its public status. Avoid deleting
backup history or resetting identities as a takedown shortcut. The final operator,
jurisdiction and legal policy are still pending #94.

## Monitoring and acceptance

Scrape `127.0.0.1:9093/metrics` with job `linkjar-pds-recovery` from the local
Grafana collector. The small monitor reports only aggregate values, not DIDs,
email addresses, tokens or paths. Litestream's one-minute health-gated heartbeat
updates it only when replication is healthy; restarting the monitor resets that
signal and cannot reuse a stale success. The heartbeat is a lag indicator with
minute granularity, not a certified per-database RPO measurement.

Import [`alerts.yml`](alerts.yml) into the existing Grafana/Prometheus ruler and
route severity labels to the monitored contact. Configure an external blackbox
health probe with job `linkjar-pds-public`. Rules cover missing/public-down probes,
missing telemetry, replication heartbeat, stale keys, changed database inventory,
disk above 70%, more than 25 new account rows in five minutes, and delayed
security mail. The initial signup threshold counts imported accounts too; tune it
from launch observations. Security-mail queue age does **not** measure provider
bounces. A Resend bounce-event feed and its live alert remain a separate missing
part of #91 and must be completed before claiming alert acceptance.

On staging, stop the PDS and backup agents independently, interrupt backup access,
create a new fixture account, simulate disk pressure on a disposable filesystem,
and verify alerts reach the operator and resolve after recovery. Do not fill a
production disk. Confirm missed scrapes cannot silently suppress the rules.
Provider host snapshots must be enabled, access restricted and a recovery point
verified through the selected host's console; no host purchase/snapshot action
has been performed here.

## Local evidence

```sh
python3 recovery/install-tools.py --directory .build/recovery-tools
python3 tests/recovery.py
```

Eight tests cover a local file-replica crash drill (Litestream killed without a
final sync), three synthetic actor databases including two discovered after
startup, encrypted restic key/config recovery, reserved keys, corruption,
missing/mismatched inventory, stale snapshots, unsafe archives, existing-target
refusal, raw secret-file semantics and aggregate metrics. The measured fixture
report is `.build/recovery-drill.json`. It cannot establish real PDS/R2 recovery.
Prometheus 3.14.0 is pinned by image digest in `prometheus-image.txt`; its rule
checker and healthy/failing fixtures in `alerts.test.json` pass locally.

## Sources checked 2026-09-12

- [Pinned Litestream release](https://github.com/benbjohnson/litestream/releases/tag/v0.5.17), [directory watcher](https://litestream.io/guides/directory-watcher/), [configuration and age limitation](https://litestream.io/reference/config/), [restore behavior](https://litestream.io/reference/restore/), [metrics](https://litestream.io/reference/metrics/) and [R2 configuration](https://litestream.io/guides/s3-compatible/).
- [R2 SSE-C](https://developers.cloudflare.com/r2/examples/ssec/) and [S3 API compatibility](https://developers.cloudflare.com/r2/api/s3/api/).
- [Restic 0.19.1](https://github.com/restic/restic/releases/tag/v0.19.1) and [repository credentials/encryption setup](https://restic.readthedocs.io/en/stable/030_preparing_a_new_repo.html).
- [Prometheus 3.14.0](https://github.com/prometheus/prometheus/releases/tag/v3.14.0).
- [Pinned PDS actor storage](https://github.com/bluesky-social/atproto/blob/7ca16cc6989f8247637615aca17c5abb911b8fb1/packages/pds/src/actor-store/actor-store.ts) and [production guidance](https://atproto.com/guides/going-to-production).

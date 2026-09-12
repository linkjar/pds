#!/usr/bin/env bash
# Run on the approved Ubuntu 24.04 host after the operator supplies private inputs.
set -euo pipefail
umask 077

if [[ ${EUID} != 0 ]]; then
  echo 'Run bootstrap as root on the approved replacement host.' >&2
  exit 1
fi
# shellcheck source=/dev/null
if [[ $(. /etc/os-release; echo "${ID}:${VERSION_ID}") != ubuntu:24.04 ]]; then
  echo 'This bootstrap targets Ubuntu 24.04 only.' >&2
  exit 1
fi
: "${DOCKER_APT_VERSION:?Set the reviewed Ubuntu docker.io package version}"
: "${COMPOSE_APT_VERSION:?Set the reviewed Ubuntu docker-compose-v2 package version}"
: "${PDS_RELEASE_COMMIT:?Set the reviewed 40-character linkjar/pds commit}"
[[ $PDS_RELEASE_COMMIT =~ ^[0-9a-f]{40}$ ]] || exit 1
# The release directory is transferred from a reviewed checkout, not fetched as main.
[[ $(git -C /opt/linkjar-pds rev-parse HEAD) == "$PDS_RELEASE_COMMIT" ]] || exit 1
[[ -z $(git -C /opt/linkjar-pds status --porcelain --untracked-files=normal) ]] || {
  echo 'The release checkout contains unreviewed changes.' >&2
  exit 1
}
for input in /etc/linkjar-pds/staging.env /etc/linkjar-pds/image.env /etc/linkjar-pds-backup/backup.env /etc/linkjar-pds-backup/sse.key /etc/linkjar-pds-backup/restic.password; do
  [[ -f $input && ! -L $input ]] || { echo 'Required private provisioning inputs are missing.' >&2; exit 1; }
done

apt-get update
apt-get install -y --no-install-recommends "docker.io=$DOCKER_APT_VERSION" "docker-compose-v2=$COMPOSE_APT_VERSION" python3 ca-certificates
python3 /opt/linkjar-pds/recovery/install-tools.py --directory /opt/linkjar-pds-tools
install -d -m 0750 -o root -g 1000 /etc/linkjar-pds-backup
chown 1000:1000 /etc/linkjar-pds-backup/backup.env /etc/linkjar-pds-backup/sse.key
chmod 0600 /etc/linkjar-pds-backup/backup.env /etc/linkjar-pds-backup/sse.key /etc/linkjar-pds-backup/restic.password
install -d -m 0700 -o 1000 -g 1000 /var/lib/linkjar-pds-backup /var/lib/linkjar-pds-backup/state
install -d -m 0700 /var/cache/linkjar-pds-backup
# Existing data is not overwritten or recursively chowned by bootstrap.
[[ -d /var/lib/linkjar-pds/staging ]] || {
  echo 'Restore into a fresh directory and complete the recovery checks before installing services.' >&2
  exit 1
}
python3 /opt/linkjar-pds/recovery/pds-recovery.py validate --data /var/lib/linkjar-pds/staging >/dev/null
python3 - <<'PY'
from pathlib import Path
import re
values = dict(line.split('=', 1) for line in Path('/etc/linkjar-pds/staging.env').read_text().splitlines() if line and not line.startswith('#') and '=' in line)
if values.get('PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT') != 'true':
    raise SystemExit('Enable PDS_SQLITE_DISABLE_WAL_AUTO_CHECKPOINT=true while Litestream owns checkpoints')
for key in ['PDS_ACCOUNT_DB_LOCATION', 'PDS_SEQUENCER_DB_LOCATION', 'PDS_DID_CACHE_DB_LOCATION', 'PDS_ACTOR_STORE_DIRECTORY']:
    if values.get(key):
        raise SystemExit('Custom database paths require an updated recovery configuration')
if values.get('PDS_DATA_DIRECTORY') != '/app/data':
    raise SystemExit('Expected the reviewed Compose data directory')
image = dict(line.split('=', 1) for line in Path('/etc/linkjar-pds/image.env').read_text().splitlines() if line and not line.startswith('#') and '=' in line)
if not re.fullmatch(r'ghcr.io/linkjar/pds@sha256:[0-9a-f]{64}', image.get('PDS_IMAGE', '')):
    raise SystemExit('Select an immutable verified LinkJar PDS image digest')
if image.get('PDS_DATA_PATH') != '/var/lib/linkjar-pds/staging' or image.get('PDS_ENV_FILE') != '/etc/linkjar-pds/staging.env':
    raise SystemExit('Image environment does not match the reviewed staging paths')
if any(path.stat().st_uid != 1000 for path in Path('/var/lib/linkjar-pds/staging').rglob('*')):
    raise SystemExit('Prepare restored data ownership for runtime UID 1000')
PY
# Configuration validation keeps raw secret values out of output.
docker compose --env-file /etc/linkjar-pds/image.env -f /opt/linkjar-pds/staging/compose.yml config --quiet
docker compose --env-file /etc/linkjar-pds/image.env -f /opt/linkjar-pds/staging/compose.yml pull
install -m 0644 /opt/linkjar-pds/recovery/systemd/* /etc/systemd/system/
systemctl daemon-reload
# Enabling/start is a separate explicit step after the restored host passes acceptance.
# Starting Litestream here could modify a source replica while the restore is being checked.
printf '%s\n' 'Backup services installed but stopped. After fenced restore acceptance, select a NEW replication epoch and enable the services as documented before opening traffic.'

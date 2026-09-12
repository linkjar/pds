#!/usr/bin/env python3
"""Load a private raw backup env file without shell or systemd interpolation."""
import os
from pathlib import Path
import sys

ALLOWED = {'AWS_ACCESS_KEY_ID', 'AWS_SECRET_ACCESS_KEY', 'AWS_DEFAULT_REGION',
           'PDS_BACKUP_PREFIX', 'PDS_BACKUP_R2_ENDPOINT', 'PDS_BACKUP_R2_BUCKET', 'PDS_BACKUP_SSE_KEY_FILE',
           'RESTIC_REPOSITORY', 'RESTIC_PASSWORD_FILE', 'RESTIC_CACHE_DIR'}


def read_env(path):
    if path.is_symlink() or path.stat().st_mode & 0o077:
        raise ValueError('Backup environment must be a private regular file')
    values = {}
    for line in path.read_text().splitlines():
        if not line or line.startswith('#'):
            continue
        key, separator, value = line.partition('=')
        if not separator or key not in ALLOWED or key in values or '\x00' in value:
            raise ValueError('Invalid or repeated backup environment name')
        values[key] = value
    return values


if __name__ == '__main__':
    if len(sys.argv) < 4 or sys.argv[2] != '--':
        raise SystemExit('Usage: run-with-env.py PRIVATE_ENV -- COMMAND [ARGS...]')
    values = read_env(Path(sys.argv[1]))
    os.execvpe(sys.argv[3], sys.argv[3:], {**os.environ, **values})

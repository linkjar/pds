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
    separator = 2
    credentials = None
    if len(sys.argv) > 3 and sys.argv[2] == '--credentials':
        credentials = Path(sys.argv[3])
        separator = 4
    if len(sys.argv) <= separator + 1 or sys.argv[separator] != '--':
        raise SystemExit('Usage: run-with-env.py PRIVATE_ENV [--credentials DIRECTORY] -- COMMAND [ARGS...]')
    values = read_env(Path(sys.argv[1]))
    if credentials is not None:
        for key, name in [('RESTIC_PASSWORD_FILE', 'restic.password'), ('PDS_BACKUP_SSE_KEY_FILE', 'sse.key')]:
            path = credentials / name
            if path.is_symlink() or not path.is_file() or path.stat().st_mode & 0o077:
                raise ValueError('Credentials must be private regular files')
            values[key] = str(path)
    command = sys.argv[separator + 1:]
    os.execvpe(command[0], command, {**os.environ, **values})

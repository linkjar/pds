#!/usr/bin/env python3
"""Exercise compiled account storage and browser/API bindings in an isolated image."""
import os
import pathlib
import subprocess
import sys
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
image = sys.argv[1] if len(sys.argv) > 1 else 'linkjar-pds:production'
mount = f'{ROOT / "tests"}:/linkjar-tests:ro'
subprocess.run(['docker', 'run', '--rm', '--entrypoint', 'node', '-v', mount,
                image, '/linkjar-tests/external-accounts.mjs'], check=True)
container = subprocess.check_output(['docker', 'run', '-d', '--rm', '--entrypoint', 'node',
    '-p', '127.0.0.1::3000', '-e', 'UPSTREAM_DIR=/app', '-e', 'UI_PORT=3000',
    '-e', 'UI_HOST=0.0.0.0', '-v', mount, image, '/linkjar-tests/external-ui-server.mjs'], text=True).strip()
try:
    port = subprocess.check_output(['docker', 'port', container, '3000/tcp'], text=True).strip().rsplit(':', 1)[1]
    base = f'http://127.0.0.1:{port}'
    for _ in range(100):
        try:
            urllib.request.urlopen(base + '/oauth/authorize', timeout=1).close()
            break
        except OSError:
            time.sleep(.1)
    else:
        subprocess.run(['docker', 'logs', container], check=False)
        raise RuntimeError('Compiled UI fixture did not start')
    subprocess.run(['node', 'tests/external-ui.mjs'], cwd=ROOT,
                   env={**os.environ, 'EXTERNAL_UI_BASE': base}, check=True)
finally:
    subprocess.run(['docker', 'rm', '-f', container], stdout=subprocess.DEVNULL, check=False)

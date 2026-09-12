#!/usr/bin/env python3
"""Local Caddy integration. Isolated network; no host ports or public accounts."""
import argparse
import pathlib
import subprocess
import tempfile
import uuid

ROOT = pathlib.Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument('source', nargs='?', type=pathlib.Path)
parser.add_argument('--pds-image', default='linkjar-pds:production')
parser.add_argument('--caddy-image', default='caddy:2.10.2-alpine@sha256:4c6e91c6ed0e2fa03efd5b44747b625fec79bc9cd06ac5235a779726618e530d')
args = parser.parse_args()
name = f'linkjar-handles-{uuid.uuid4().hex[:10]}'
backend, proxy = f'{name}-pds', f'{name}-proxy'
def run(*cmd):
    return subprocess.run(cmd, check=True, text=True, capture_output=True).stdout.strip()
try:
    run('docker', 'network', 'create', name)
    with tempfile.TemporaryDirectory() as tmp:
        config = pathlib.Path(tmp) / 'Caddyfile'
        config.write_text('''{\n admin off\n auto_https off\n}\nimport /etc/caddy/handles\n:8080 {\n import linkjar_handle_hosts\n}\n''')
        source_args = ['-v', f'{args.source.resolve()}:/patch-src:ro'] if args.source else ['-e', 'LINKJAR_USE_COMPILED=1']
        run('docker', 'run', '-d', '--name', backend, '--network', name, '--user', 'root', '--entrypoint', 'node', *source_args, '-v', f'{ROOT / "tests"}:/linkjar-tests:ro', args.pds_image, '/linkjar-tests/handle-proxy-backend.mjs')
        run('docker', 'run', '-d', '--name', proxy, '--network', name, '-e', f'PDS_UPSTREAM={backend}:3000', '-e', f'PDS_RECOVERY_UPSTREAM={backend}:3001', '-v', f'{config}:/etc/caddy/Caddyfile:ro', '-v', f'{ROOT / "staging/Caddyfile.handles"}:/etc/caddy/handles:ro', args.caddy_image)
        print(run('docker', 'exec', backend, 'node', '/linkjar-tests/handle-proxy-probes.mjs', f'http://{proxy}:8080'))
except subprocess.CalledProcessError as error:
    print(error.stderr)
    for container in (backend, proxy):
        subprocess.run(['docker', 'logs', container], check=False)
    raise
finally:
    for container in (backend, proxy):
        subprocess.run(['docker', 'rm', '-f', container], check=False, capture_output=True)
    subprocess.run(['docker', 'network', 'rm', name], check=False, capture_output=True)

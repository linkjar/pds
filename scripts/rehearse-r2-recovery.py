#!/usr/bin/env python3
"""Encrypted R2 crash/restore rehearsal using isolated synthetic accounts only."""
import argparse
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import time
from datetime import datetime, timezone
from types import SimpleNamespace
import uuid

ROOT = Path(__file__).resolve().parent.parent
def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--backup-env', type=Path, required=True)
parser.add_argument('--key-directory', type=Path, required=True)
parser.add_argument('--tools', type=Path, required=True)
parser.add_argument('--report', type=Path, required=True)
args = parser.parse_args()
os.umask(0o077)
raw = load('raw_env', ROOT / 'recovery/run-with-env.py')
fixture_module = load('fixture', ROOT / 'tests/recovery.py')
r = fixture_module.r
fixture = fixture_module.RecoveryTests()
fixture.setUp()
epoch = 'rehearsals/' + uuid.uuid4().hex
values = raw.read_env(args.backup_env)
values.update({
    'PDS_BACKUP_PREFIX': epoch + '/databases',
    'RESTIC_REPOSITORY': 's3:' + values['PDS_BACKUP_R2_ENDPOINT'].rstrip('/') + '/' + values['PDS_BACKUP_R2_BUCKET'] + '/' + epoch + '/host-material',
    'RESTIC_PASSWORD_FILE': str(args.key_directory.resolve() / 'restic.password'),
    'PDS_BACKUP_SSE_KEY_FILE': str(args.key_directory.resolve() / 'sse.key'),
    'RESTIC_CACHE_DIR': str(fixture.root / 'cache'),
})
os.environ.update(values)
litestream, restic = (str(args.tools.resolve() / name) for name in ('litestream', 'restic'))
process = None
started = time.monotonic()
try:
    config = r.config_for(fixture.data, values['PDS_BACKUP_PREFIX'])
    config['addr'] = '127.0.0.1:0'
    config.pop('heartbeat-url', None)
    config.pop('heartbeat-interval', None)
    cfg_path = fixture.root / 'litestream.json'
    r.private_write(cfg_path, json.dumps(config).encode())
    with (fixture.root / 'replicate.log').open('wb') as log:
        process = subprocess.Popen([litestream, 'replicate', '-config', str(cfg_path), '-no-expand-env'], stdout=log, stderr=log)
        time.sleep(1)
        fixture.create_actor('bbbbbbbbbbbbbbbbbbbbbbbb')
        fixture.create_actor('cccccccccccccccccccccccc')
        (fixture.data / 'actors/reserved_keys/reserved-fixture').write_bytes(os.urandom(32))
        databases, _ = r.inventory(fixture.data, fixture.config)
        probe_cfg = fixture.root / 'probe.json'
        r.private_write(probe_cfg, json.dumps(r.config_for(fixture.data, values['PDS_BACKUP_PREFIX'], databases=databases)).encode())
        deadline = time.monotonic() + 120
        print('Replicating synthetic shared and dynamically discovered actor databases to encrypted R2', flush=True)
        while True:
            probe = fixture.root / ('probe-' + uuid.uuid4().hex)
            probe.mkdir()
            try:
                for name in databases:
                    target = probe / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    if name.startswith('actors/'):
                        target.with_name('key').write_bytes((fixture.data / name).with_name('key').read_bytes())
                    r.checked([litestream, 'restore', '-config', str(probe_cfg), '-no-expand-env', '-integrity-check', 'full', '-o', str(target), str(fixture.data / name)], timeout=30)
                r.validate(probe)
                break
            except (RuntimeError, ValueError, OSError):
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('R2 replicas did not pass the bounded catch-up check') from None
                time.sleep(1)
        print('R2 replica inventory and repository roots validated; capturing encrypted host material', flush=True)
        r.checked([restic, 'init'])
        with contextlib.redirect_stdout(io.StringIO()) as captured:
            r.backup(SimpleNamespace(data=fixture.data, config=fixture.config, state=fixture.state, restic=restic, environment='linkjar-pds-staging'))
        status = json.loads(captured.getvalue())
        process.kill(); process.wait(timeout=10)
        stamp = datetime.now(timezone.utc).isoformat()
        print('Replicator killed without final sync; restoring exclusively from R2', flush=True)
        with contextlib.redirect_stdout(io.StringIO()) as captured:
            r.restore(SimpleNamespace(destination=fixture.root / 'restored', snapshot=status['snapshot'], timestamp=stamp, restic=restic, litestream=litestream, prefix=values['PDS_BACKUP_PREFIX'], local_replica=None, max_host_material_age=60))
        result = json.loads(captured.getvalue())
        result.update({'epoch': epoch, 'syntheticFixture': True, 'totalSeconds': round(time.monotonic() - started, 3), 'productionAcceptance': False})
        r.private_write(args.report, json.dumps(result, indent=2).encode())
        print(json.dumps({key: result[key] for key in ('databases', 'actors', 'integrity', 'restoreSeconds', 'hostMaterialAgeSeconds', 'productionAcceptance')}), flush=True)
finally:
    if process is not None and process.poll() is None:
        process.kill(); process.wait(timeout=10)
    fixture.tearDown()

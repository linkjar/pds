#!/usr/bin/env python3
"""Crash/local-replica drill and fail-closed restore boundary tests; no public accounts."""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import time
from datetime import datetime, timezone
from types import SimpleNamespace
import unittest
import unittest.mock

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('recovery', ROOT / 'recovery/pds-recovery.py')
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
TOOLS = Path(os.environ.get('RECOVERY_TOOLS', ROOT / '.build/recovery-tools')).resolve()


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='linkjar-recovery-91-')
        self.root = Path(self.tmp.name)
        self.data = self.root / 'data'
        self.config = self.root / 'config'
        self.state = self.root / 'state'
        for directory in [self.data / 'actors/reserved_keys', self.config, self.state]:
            directory.mkdir(parents=True)
        (self.config / 'staging.env').write_text('PDS_JWT_SECRET=synthetic-fixture-only\n')
        self.connections = []
        self.account = self.db(self.data / 'account.sqlite')
        self.account.execute('CREATE TABLE actor (did TEXT PRIMARY KEY)')
        self.account.execute('CREATE TABLE repo_root (did TEXT PRIMARY KEY, cid TEXT, rev TEXT)')
        self.account.commit()
        for name in ['sequencer.sqlite', 'did_cache.sqlite']:
            db = self.db(self.data / name)
            db.execute('CREATE TABLE fixture (value TEXT)')
            db.commit()
        self.create_actor('aaaaaaaaaaaaaaaaaaaaaaaa')
        self.replica = self.root / 'replica'
        self.env = {**os.environ, 'RESTIC_REPOSITORY': str(self.root / 'restic'),
                    'RESTIC_PASSWORD': 'local-synthetic-fixture-password', 'RESTIC_CACHE_DIR': str(self.root / 'cache')}

    def tearDown(self):
        for connection in self.connections:
            connection.close()
        self.tmp.cleanup()

    def db(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        db = sqlite3.connect(path)
        db.execute('PRAGMA journal_mode=WAL')
        db.execute('PRAGMA wal_autocheckpoint=0')
        self.connections.append(db)
        return db

    def create_actor(self, suffix):
        did = 'did:plc:' + suffix
        path = self.data / 'actors' / hashlib.sha256(did.encode()).hexdigest()[:2] / did
        db = self.db(path / 'store.sqlite')
        (path / 'key').write_bytes(os.urandom(32))
        db.execute('CREATE TABLE repo_root (did TEXT PRIMARY KEY, cid TEXT, rev TEXT)')
        db.execute('INSERT INTO repo_root VALUES (?, ?, ?)', (did, 'fixture-cid', 'fixture-rev'))
        db.execute('CREATE TABLE records (value TEXT)')
        db.execute("INSERT INTO records VALUES ('before-crash')")
        db.commit()
        self.account.execute('INSERT INTO actor (did) VALUES (?)', (did,))
        self.account.execute('INSERT INTO repo_root VALUES (?, ?, ?)', (did, 'fixture-cid', 'fixture-rev'))
        self.account.commit()
        return path

    def test_missing_key_and_mismatched_roots_are_rejected(self):
        key = next((self.data / 'actors').rglob('key'))
        original = key.read_bytes()
        key.unlink()
        with self.assertRaises((ValueError, FileNotFoundError)):
            r.inventory(self.data, self.config)
        key.write_bytes(original)
        self.account.execute("UPDATE repo_root SET cid='different-commit'")
        self.account.commit()
        with self.assertRaisesRegex(ValueError, 'roots disagree'):
            r.validate(self.data)

    def test_symlinks_and_incomplete_database_inventory_are_rejected(self):
        (self.config / 'external').symlink_to(self.root / 'elsewhere')
        with self.assertRaisesRegex(ValueError, 'Symlinks'):
            r.inventory(self.data, self.config)
        (self.config / 'external').unlink()
        (self.data / 'did_cache.sqlite').rename(self.data / 'missing.db')
        with self.assertRaisesRegex(ValueError, 'shared database'):
            r.inventory(self.data, self.config)

    def test_archive_traversal_is_rejected_without_writing_outside_destination(self):
        archive = self.root / 'host.tar'
        with tarfile.open(archive, 'w') as tar:
            info = tarfile.TarInfo('../escape')
            info.size = 1
            tar.addfile(info, io.BytesIO(b'x'))
        with self.assertRaisesRegex(ValueError, 'Unsafe archive'):
            r.unpack(archive, self.root / 'output')
        self.assertFalse((self.root / 'escape').exists())

    def test_restore_refuses_existing_destination_before_invoking_tools(self):
        with self.assertRaisesRegex(ValueError, 'must not exist'):
            r.restore(SimpleNamespace(destination=self.data))

    def test_corrupt_sqlite_fails_integrity_gate(self):
        for connection in self.connections:
            connection.close()
        self.connections.clear()
        (self.data / 'sequencer.sqlite').write_bytes(b'not sqlite')
        with self.assertRaises(sqlite3.DatabaseError):
            r.validate(self.data)

    def test_metrics_do_not_expose_identity_and_missing_status_stays_unhealthy(self):
        monitor_spec = importlib.util.spec_from_file_location('monitor', ROOT / 'recovery/monitor.py')
        monitor = importlib.util.module_from_spec(monitor_spec)
        monitor_spec.loader.exec_module(monitor)
        self.account.execute('ALTER TABLE actor ADD COLUMN createdAt TEXT')
        self.account.execute("UPDATE actor SET createdAt='2026-09-12T17:00:00.000Z'")
        self.account.execute('CREATE TABLE sign_in_notice (createdAt INTEGER)')
        self.account.commit()
        result = monitor.metrics(self.data, self.state, 0, now=1000)
        self.assertIn('linkjar_pds_key_capture_seconds 0', result)
        self.assertIn('linkjar_pds_backup_inventory_matches 0', result)
        self.assertNotIn('did:plc:', result)
        self.assertNotIn('synthetic-fixture-only', result)
        databases, _ = r.inventory(self.data, self.config)
        r.private_write(self.state / 'key-backup-status.json', json.dumps({'captureStartedAt': 990, 'lastSuccess': 995,
            'databases': len(databases), 'inventoryDigest': hashlib.sha256(json.dumps(databases).encode()).hexdigest()}).encode())
        self.assertIn('linkjar_pds_backup_inventory_matches 1', monitor.metrics(self.data, self.state, 999, now=1000))
        self.create_actor('dddddddddddddddddddddddd')
        self.assertIn('linkjar_pds_backup_inventory_matches 0', monitor.metrics(self.data, self.state, 999, now=1000))

    def test_raw_environment_preserves_values_and_rejects_system_variables(self):
        env_spec = importlib.util.spec_from_file_location('raw_env', ROOT / 'recovery/run-with-env.py')
        raw = importlib.util.module_from_spec(env_spec)
        env_spec.loader.exec_module(raw)
        path = self.root / 'backup.env'
        value = 'literal$\\value # \"quoted\"'
        r.private_write(path, ('AWS_SECRET_ACCESS_KEY=' + value + '\n').encode())
        self.assertEqual(raw.read_env(path)['AWS_SECRET_ACCESS_KEY'], value)
        r.private_write(path, b'PATH=/untrusted\n')
        with self.assertRaises(ValueError):
            raw.read_env(path)

    def test_real_encrypted_key_backup_and_dynamic_replication_survive_abrupt_exit(self):
        litestream, restic = str(TOOLS / 'litestream'), str(TOOLS / 'restic')
        config = r.config_for(self.data, 'staging/databases', local_replica=self.replica)
        config['addr'] = '127.0.0.1:0'
        cfg_path = self.root / 'litestream.json'
        r.private_write(cfg_path, json.dumps(config).encode())
        log = (self.root / 'replicate.log').open('wb')
        process = subprocess.Popen([litestream, 'replicate', '-config', str(cfg_path), '-no-expand-env'], stdout=log, stderr=log)
        try:
            time.sleep(1)
            self.create_actor('bbbbbbbbbbbbbbbbbbbbbbbb')
            self.create_actor('cccccccccccccccccccccccc')
            (self.data / 'actors/reserved_keys/reserved-fixture').write_bytes(os.urandom(32))
            databases, files = r.inventory(self.data, self.config)
            deadline = time.monotonic() + 25
            while not all(list((self.replica / name).rglob('*.ltx')) for name in databases):
                if process.poll() is not None or time.monotonic() > deadline:
                    self.fail('Litestream directory replication did not complete')
                time.sleep(.2)
            # A replica file existing does not prove the shared DB has caught up
            # with newly discovered actors. Confirm the uploaded account state
            # before the deliberate crash, without asking for a final sync.
            while True:
                probe = self.root / 'account-probe.sqlite'
                try:
                    r.checked([litestream, 'restore', '-o', str(probe), (self.replica / 'account.sqlite').as_uri()])
                    with sqlite3.connect(probe) as db:
                        if db.execute('SELECT count(*) FROM actor').fetchone()[0] == 3:
                            break
                except (RuntimeError, sqlite3.Error):
                    pass
                finally:
                    probe.unlink(missing_ok=True)
                if process.poll() is not None or time.monotonic() > deadline:
                    self.fail('Shared account replica did not catch up')
                time.sleep(.2)
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                with unittest.mock.patch.dict(os.environ, self.env, clear=True):
                    r.checked([restic, 'init'])
                    r.backup(SimpleNamespace(data=self.data, config=self.config, state=self.state,
                                            restic=restic, environment='linkjar-pds-staging'))
            status = json.loads(printed.getvalue())
            self.assertEqual(len(status['snapshot']), 64)
            self.assertEqual(status['files'], 5)
            # No graceful shutdown sync: recover only the replica already uploaded.
            process.kill()
            process.wait(timeout=10)
            stamp = datetime.now(timezone.utc).isoformat()
            destination = self.root / 'restored'
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                with unittest.mock.patch.dict(os.environ, self.env, clear=True):
                    r.restore(SimpleNamespace(destination=destination, snapshot=status['snapshot'], timestamp=stamp,
                                              restic=restic, litestream=litestream, prefix='staging/databases', local_replica=self.replica, max_host_material_age=60))
            # A stale key/config snapshot must not be silently paired with newer databases.
            stale = datetime.fromtimestamp(time.time() + 120, timezone.utc).isoformat()
            with unittest.mock.patch.dict(os.environ, self.env, clear=True):
                with self.assertRaisesRegex(ValueError, 'stale'):
                    r.restore(SimpleNamespace(destination=self.root / 'stale', snapshot=status['snapshot'], timestamp=stale,
                                              restic=restic, litestream=litestream, prefix='staging/databases', local_replica=self.replica, max_host_material_age=60))
            self.assertTrue((self.root / 'stale/RESTORE-INCOMPLETE').is_file())
            result = json.loads(printed.getvalue())
            self.assertEqual(result['actors'], 3)
            self.assertEqual(result['databases'], 6)
            self.assertFalse(result['trafficApproved'])
            for name, source in files:
                self.assertEqual((destination / name).read_bytes(), source.read_bytes())
            for path in (destination / 'data/actors').rglob('store.sqlite'):
                with sqlite3.connect(path) as db:
                    self.assertEqual(db.execute('SELECT value FROM records').fetchall(), [('before-crash',)])
            repository_bytes = b''.join(p.read_bytes() for p in (self.root / 'restic').rglob('*') if p.is_file())
            self.assertNotIn(b'synthetic-fixture-only', repository_bytes)
            report = {**result, 'scope': 'Synthetic PDS-shaped SQLite and keys; local file replicas and encrypted local restic, abrupt Litestream exit. Not R2, real PDS writes, public OAuth or VM recovery.'}
            (ROOT / '.build').mkdir(exist_ok=True)
            (ROOT / '.build/recovery-drill.json').write_text(json.dumps(report, indent=2) + '\n')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            log.close()


if __name__ == '__main__':
    import unittest.mock
    unittest.main()

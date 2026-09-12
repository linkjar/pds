#!/usr/bin/env python3
"""Loopback backup heartbeat and aggregate recovery metrics; no account labels."""
import argparse
import hashlib
import shutil
import importlib.util
import json
from pathlib import Path
import sqlite3
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

spec = importlib.util.spec_from_file_location('recovery', Path(__file__).with_name('pds-recovery.py'))
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)


def metrics(data, state, heartbeat, now=None):
    now = time.time() if now is None else now
    result = {'linkjar_pds_backup_heartbeat_seconds': heartbeat,
              'linkjar_pds_key_capture_seconds': 0,
              'linkjar_pds_key_backup_success_seconds': 0,
              'linkjar_pds_key_backup_databases': 0,
              'linkjar_pds_recovery_inventory_ok': 0,
              'linkjar_pds_backup_inventory_matches': 0}
    status = {}
    try:
        status = json.loads((state / 'key-backup-status.json').read_text())
        for name, key in [('key_capture_seconds', 'captureStartedAt'),
                          ('key_backup_success_seconds', 'lastSuccess'), ('key_backup_databases', 'databases')]:
            result['linkjar_pds_' + name] = float(status[key])
    except (OSError, ValueError, KeyError, TypeError):
        pass
    try:
        paths = [path for path in r.walk(data) if path.suffix == '.sqlite']
        names = sorted(path.relative_to(data).as_posix() for path in paths)
        result['linkjar_pds_databases'] = len(paths)
        keys_present = all(r.regular(path.with_name('key')).stat().st_size == 32 for path in paths if path.parent != data)
        digest = hashlib.sha256(json.dumps(names).encode()).hexdigest()
        result['linkjar_pds_backup_inventory_matches'] = int(keys_present and digest == status.get('inventoryDigest'))
        stat = shutil.disk_usage(data)
        result['linkjar_pds_disk_used_ratio'] = stat.used / stat.total
        cutoff = datetime.fromtimestamp(now - 300, timezone.utc).isoformat(timespec='milliseconds').replace('+00:00', 'Z')
        with r.sqlite_read(data / 'account.sqlite') as db:
            result['linkjar_pds_accounts_created_5m'] = db.execute('SELECT count(*) FROM actor WHERE createdAt >= ?', (cutoff,)).fetchone()[0]
            # Includes pending security notices from revision 7; email content stays private.
            row = db.execute('SELECT min(createdAt), count(*) FROM sign_in_notice').fetchone()
            result['linkjar_pds_security_mail_pending'] = row[1]
            result['linkjar_pds_security_mail_oldest_age_seconds'] = max(0, now - row[0] / 1000) if row[0] else 0
        result['linkjar_pds_recovery_inventory_ok'] = 1
    except (OSError, ValueError, sqlite3.Error):
        pass
    return ''.join(f'# TYPE {name} gauge\n{name} {value}\n' for name, value in sorted(result.items()))


def serve(data, state, port):
    last_heartbeat = 0
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            nonlocal last_heartbeat
            if self.path == '/heartbeat':
                with lock:
                    last_heartbeat = time.time()
                self.send_response(204)
                self.end_headers()
            elif self.path == '/metrics':
                with lock:
                    heartbeat = last_heartbeat
                body = metrics(data, state, heartbeat).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'text/plain; version=0.0.4')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self.send_response(404)
                self.end_headers()

    ThreadingHTTPServer(('127.0.0.1', port), Handler).serve_forever()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--state', type=Path, required=True)
    parser.add_argument('--port', type=int, default=9093)
    args = parser.parse_args()
    serve(args.data, args.state, args.port)

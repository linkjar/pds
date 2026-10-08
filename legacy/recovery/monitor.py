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


def metrics(data, state, heartbeat, now=None, mail=None):
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
    result['linkjar_pds_mail_webhook_configured'] = int(mail is not None)
    result['linkjar_pds_mail_telemetry_ok'] = 0
    if mail is not None:
        try:
            result.update(mail.metrics(now))
            result['linkjar_pds_mail_telemetry_ok'] = 1
        except (OSError, sqlite3.Error):
            pass
    return ''.join(f'# TYPE {name} gauge\n{name} {value}\n' for name, value in sorted(result.items()))


def make_server(data, state, port, mail=None):
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
                body = metrics(data, state, heartbeat, mail=mail).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'text/plain; version=0.0.4')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            status = 400
            try:
                if self.path != '/webhooks/resend':
                    status = 404
                elif mail is None:
                    status = 503
                elif self.headers.get('transfer-encoding') or len(self.headers.get_all('content-length', [])) != 1:
                    status = 411
                elif any(len(self.headers.get_all(name, [])) != 1 for name in ['svix-id', 'svix-timestamp', 'svix-signature']):
                    status = 400
                else:
                    size = int(self.headers.get('content-length', '0'))
                    if not 0 < size <= 64 * 1024:
                        status = 413
                    else:
                        body = self.rfile.read(size)
                        if len(body) != size:
                            raise ValueError('Incomplete request')
                        mail.receive(body, self.headers)
                        status = 204
            except (ValueError, TypeError, TimeoutError):
                status = 400
            except (OSError, sqlite3.Error):
                status = 503
            self.close_connection = True
            self.send_response(status)
            self.send_header('Content-Length', '0')
            self.end_headers()

    class BoundedServer(ThreadingHTTPServer):
        daemon_threads = True
        slots = threading.BoundedSemaphore(16)

        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(5)
            return connection, address

        def process_request(self, request, address):
            if not self.slots.acquire(blocking=False):
                try:
                    request.sendall(b'HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n')
                finally:
                    self.shutdown_request(request)
                return
            try:
                super().process_request(request, address)
            except BaseException:
                self.slots.release()
                raise

        def process_request_thread(self, request, address):
            try:
                super().process_request_thread(request, address)
            finally:
                self.slots.release()

        def handle_error(self, *_):
            # Do not log request payloads or signed headers.
            pass

    return BoundedServer(('127.0.0.1', port), Handler)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, required=True)
    parser.add_argument('--state', type=Path, required=True)
    parser.add_argument('--port', type=int, default=9093)
    parser.add_argument('--resend-secret-file', type=Path)
    parser.add_argument('--resend-sender-file', type=Path)
    args = parser.parse_args()
    mail = None
    if args.resend_secret_file or args.resend_sender_file:
        if not args.resend_secret_file or not args.resend_sender_file:
            parser.error('Set both Resend webhook key and sender files')
        mail_spec = importlib.util.spec_from_file_location('mail_events', Path(__file__).with_name('mail-events.py'))
        module = importlib.util.module_from_spec(mail_spec)
        mail_spec.loader.exec_module(module)
        mail = module.MailEvents(args.state / 'mail/mail-events.sqlite', args.resend_secret_file, args.resend_sender_file)
    make_server(args.data, args.state, args.port, mail).serve_forever()

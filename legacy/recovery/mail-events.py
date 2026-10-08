"""Verify Resend/Svix events and retain only deduplicated aggregate alert inputs."""
import base64
from contextlib import contextmanager
from datetime import datetime
from email.utils import parseaddr
import hashlib
import hmac
import json
import re
import sqlite3
import time

KINDS = {'email.sent', 'email.bounced', 'email.failed', 'email.complained'}
MAX_BODY = 64 * 1024
RETENTION = 7 * 86400


def verify(body, headers, secret, now):
    if not 0 < len(body) <= MAX_BODY:
        raise ValueError('Invalid webhook body')
    message_id = headers.get('svix-id', '')
    timestamp = headers.get('svix-timestamp', '')
    signatures = headers.get('svix-signature', '')
    if not re.fullmatch(r'[a-zA-Z0-9_-]{1,128}', message_id) or not re.fullmatch(r'[0-9]{1,12}', timestamp) or len(signatures) > 4096:
        raise ValueError('Invalid webhook headers')
    if abs(now - int(timestamp)) > 300:
        raise ValueError('Webhook timestamp outside tolerance')
    if not secret.startswith('whsec_'):
        raise ValueError('Invalid webhook key')
    key = base64.b64decode(secret[6:], validate=True)
    if not 16 <= len(key) <= 128:
        raise ValueError('Invalid webhook key')
    expected = base64.b64encode(hmac.digest(key, message_id.encode() + b'.' + timestamp.encode() + b'.' + body, 'sha256')).decode()
    valid = False
    for signature in signatures.split():
        version, separator, candidate = signature.partition(',')
        if version == 'v1' and separator:
            valid |= hmac.compare_digest(expected, candidate)
    if not valid:
        raise ValueError('Invalid webhook signature')
    return message_id, json.loads(body)


class MailEvents:
    def __init__(self, database, secret_file, sender_file):
        for path in [secret_file, sender_file]:
            if path.is_symlink() or not path.is_file():
                raise ValueError('Mail webhook configuration must use regular files')
        if secret_file.stat().st_mode & 0o077:
            raise ValueError('Mail webhook key must be private')
        self.secret = secret_file.read_text().strip()
        self.sender = parseaddr(sender_file.read_text().strip())[1].lower()
        if not self.sender or '@' not in self.sender:
            raise ValueError('Configure the exact PDS sending address')
        # Reject malformed configuration at startup, before accepting any events.
        key = base64.b64decode(self.secret.removeprefix('whsec_'), validate=True)
        if not self.secret.startswith('whsec_') or not 16 <= len(key) <= 128:
            raise ValueError('Invalid webhook key')
        database.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.database = database
        with self.connect() as db:
            db.execute('CREATE TABLE IF NOT EXISTS mail_event (id_hash TEXT PRIMARY KEY, kind TEXT NOT NULL, occurred REAL NOT NULL, received REAL NOT NULL)')
            db.execute('CREATE INDEX IF NOT EXISTS mail_event_time ON mail_event(occurred)')
            db.execute('CREATE TABLE IF NOT EXISTS mail_status (id INTEGER PRIMARY KEY CHECK(id=1), last_received REAL NOT NULL)')
        database.chmod(0o600)

    @contextmanager
    def connect(self):
        db = sqlite3.connect(self.database, timeout=5)
        try:
            with db:
                yield db
        finally:
            db.close()

    def receive(self, body, headers, now=None):
        now = time.time() if now is None else now
        message_id, event = verify(body, headers, self.secret, now)
        if not isinstance(event, dict):
            raise ValueError('Invalid event')
        kind = event.get('type')
        if kind not in KINDS:
            return
        data = event.get('data')
        if not isinstance(data, dict) or not isinstance(data.get('from'), str):
            raise ValueError('Missing event sender')
        if parseaddr(data['from'])[1].lower() != self.sender:
            return
        created = event.get('created_at')
        if not isinstance(created, str):
            raise ValueError('Missing event timestamp')
        occurred = datetime.fromisoformat(created.replace('Z', '+00:00'))
        if occurred.tzinfo is None or occurred.timestamp() > now + 300:
            raise ValueError('Invalid event timestamp')
        # Older replayed events are acknowledged without becoming fresh incidents.
        if occurred.timestamp() < now - RETENTION:
            return
        digest = hashlib.sha256(message_id.encode()).hexdigest()
        with self.connect() as db:
            db.execute('DELETE FROM mail_event WHERE occurred < ?', (now - RETENTION,))
            db.execute('INSERT OR IGNORE INTO mail_event VALUES (?, ?, ?, ?)', (digest, kind, occurred.timestamp(), now))
            db.execute('INSERT INTO mail_status VALUES (1, ?) ON CONFLICT(id) DO UPDATE SET last_received=excluded.last_received', (now,))

    def metrics(self, now=None):
        now = time.time() if now is None else now
        with self.connect() as db:
            db.execute('DELETE FROM mail_event WHERE occurred < ?', (now - RETENTION,))
            counts = dict(db.execute('SELECT kind, count(*) FROM mail_event WHERE occurred >= ? AND occurred <= ? GROUP BY kind', (now - 300, now)))
            status = db.execute('SELECT last_received FROM mail_status WHERE id=1').fetchone()
        result = {f'linkjar_pds_mail_{kind.split(".")[1]}_5m': counts.get(kind, 0) for kind in sorted(KINDS)}
        result['linkjar_pds_mail_last_event_seconds'] = status[0] if status else 0
        return result

#!/usr/bin/env python3
"""Signed mail-event ingestion through the real HTTP boundary, without sending mail."""
import base64
from datetime import datetime, timezone
import hashlib
import hmac
import http.client
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'recovery' / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


mail = load('mail_events', 'mail-events.py')
monitor = load('monitor', 'monitor.py')


class MailTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='linkjar-mail-events-')
        self.root = Path(self.tmp.name)
        self.key = os.urandom(24)
        self.secret = 'whsec_' + base64.b64encode(self.key).decode()
        self.key_file, self.sender_file = self.root / 'secret', self.root / 'sender'
        self.key_file.write_text(self.secret)
        self.key_file.chmod(0o600)
        self.sender_file.write_text('LinkJar <pds@fixture.invalid>')
        self.database = self.root / 'events.sqlite'
        self.store = mail.MailEvents(self.database, self.key_file, self.sender_file)

    def tearDown(self):
        self.tmp.cleanup()

    def event(self, kind='email.bounced', message_id='msg_fixture', timestamp=None, occurred=None, sender='LinkJar <pds@fixture.invalid>'):
        timestamp = int(time.time()) if timestamp is None else timestamp
        occurred = timestamp if occurred is None else occurred
        body = json.dumps({'type': kind, 'created_at': datetime.fromtimestamp(occurred, timezone.utc).isoformat(),
                           'data': {'from': sender, 'to': ['private-recipient@fixture.invalid'], 'subject': 'private subject'}}).encode()
        signature = base64.b64encode(hmac.digest(self.key, f'{message_id}.{timestamp}.'.encode() + body, 'sha256')).decode()
        return body, {'svix-id': message_id, 'svix-timestamp': str(timestamp), 'svix-signature': 'v1,' + signature}

    def test_official_svix_signature_vector(self):
        # Public interoperability fixture from Svix's manual verification guide.
        identity, event = mail.verify(b'{"event_type":"ping","data":{"success":true}}',
            {'svix-id': 'msg_loFOjxBNrRLzqYUf', 'svix-timestamp': '1731705121',
             'svix-signature': 'v1,rAvfW3dJ/X/qxhsaXPOyyCGmRKsaKWcsNccKXlIktD0='},
            'whsec_plJ3nmyCDGBKInavdOK15jsl', 1731705121)
        self.assertEqual(identity, 'msg_loFOjxBNrRLzqYUf')
        self.assertTrue(event['data']['success'])

    def test_tampering_expired_future_and_wrong_version_signatures_are_rejected(self):
        body, headers = self.event(timestamp=1000)
        for payload, candidate, now in [(body + b' ', headers, 1000), (body, headers, 1301), (body, headers, 699),
                                        (body, {**headers, 'svix-signature': headers['svix-signature'].replace('v1,', 'v2,')}, 1000)]:
            with self.assertRaises(ValueError):
                mail.verify(payload, candidate, self.secret, now)
        valid = {**headers, 'svix-signature': 'v1,wrong ' + headers['svix-signature']}
        mail.verify(body, valid, self.secret, 1000)

    def test_duplicates_survive_restart_without_persisting_recipients_or_body(self):
        body, headers = self.event(timestamp=1000)
        self.store.receive(body, headers, 1000)
        reopened = mail.MailEvents(self.database, self.key_file, self.sender_file)
        reopened.receive(body, headers, 1000)
        self.assertEqual(reopened.metrics(1001)['linkjar_pds_mail_bounced_5m'], 1)
        stored = self.database.read_bytes()
        self.assertNotIn(b'private-recipient', stored)
        self.assertNotIn(b'private subject', stored)
        self.assertNotIn(b'pds@fixture.invalid', stored)
        self.assertNotIn(b'msg_fixture', stored)

    def test_other_senders_and_old_event_replays_do_not_inflate_alerts(self):
        self.store.receive(*self.event(timestamp=1_000_000, sender='other@fixture.invalid'), now=1_000_000)
        self.store.receive(*self.event(timestamp=1_000_000, occurred=1), now=1_000_000)
        self.assertEqual(self.store.metrics(1_000_000)['linkjar_pds_mail_bounced_5m'], 0)
        self.store.receive(*self.event(timestamp=1_000_000), now=1_000_000)
        self.assertEqual(self.store.metrics(1_000_001)['linkjar_pds_mail_bounced_5m'], 1)
        self.store.metrics(1_000_000 + mail.RETENTION + 1)
        with self.store.connect() as db:
            self.assertEqual(db.execute('SELECT count(*) FROM mail_event').fetchone()[0], 0)

    def test_real_http_accepts_only_bounded_signed_events_and_exposes_aggregate_metrics(self):
        server = monitor.make_server(self.root / 'data', self.root, 0, self.store)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def request(method, path, body=None, headers=None):
            connection = http.client.HTTPConnection(*server.server_address, timeout=5)
            try:
                connection.request(method, path, body=body, headers=headers or {})
                response = connection.getresponse()
                return response.status, response.read().decode()
            finally:
                connection.close()
        try:
            body, headers = self.event()
            self.assertEqual(request('POST', '/webhooks/resend', body, headers)[0], 204)
            self.assertEqual(request('POST', '/webhooks/resend', body, headers)[0], 204)
            self.assertEqual(request('POST', '/webhooks/resend', body + b' ', headers)[0], 400)
            self.assertEqual(request('POST', '/webhooks/resend', b'x' * (mail.MAX_BODY + 1), headers)[0], 413)
            self.assertEqual(request('POST', '/webhooks/resend', body, {**headers, 'Transfer-Encoding': 'chunked'})[0], 411)
            self.assertEqual(request('POST', '/heartbeat', body, headers)[0], 404)
            self.assertEqual(request('GET', '/webhooks/resend')[0], 404)
            status, metrics = request('GET', '/metrics')
            self.assertEqual(status, 200)
            self.assertIn('linkjar_pds_mail_bounced_5m 1', metrics)
            self.assertIn('linkjar_pds_mail_webhook_configured 1', metrics)
            self.assertNotIn('private-recipient', metrics)
            self.assertNotIn('pds@fixture.invalid', metrics)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


if __name__ == '__main__':
    unittest.main()

#!/usr/bin/env python3
"""Isolated, disposable local PDS parity probes; never creates public identities."""
import argparse
import json
import pathlib
import secrets
import subprocess
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent

def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()

def request(base, path, body=None):
    req = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(),
                                 headers={'Content-Type': 'application/json'})
    try:
        response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        content = response.read().decode()
        try:
            content = json.loads(content)
        except ValueError:
            pass
        return {'status': response.status, 'body': content}

def start(image, suffix):
    name = 'linkjar-pds-smoke-' + secrets.token_hex(4) + '-' + suffix
    env = {
        'PDS_HOSTNAME': 'localhost', 'PDS_DEV_MODE': 'true',
        # The distribution bakes its separate 0.4.50xx release label into this
        # env var. Compare the same explicit config and verify package versions
        # separately below so this cannot hide a mismatched implementation.
        'PDS_VERSION': json.loads((ROOT / 'upstream.json').read_text())['tag'].split('@')[-1],
        'PDS_DATA_DIRECTORY': '/tmp/pds', 'PDS_BLOBSTORE_DISK_LOCATION': '/tmp/pds/blobs',
        'PDS_JWT_SECRET': secrets.token_hex(32), 'PDS_ADMIN_PASSWORD': secrets.token_hex(24),
        'PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX': '0' * 63 + '1',
        'PDS_DID_PLC_URL': 'http://127.0.0.1:9', 'PDS_CRAWLERS': '',
        'PDS_INVITE_REQUIRED': 'true', 'LOG_ENABLED': 'false',
    }
    args = ['run', '-d', '--name', name, '--label', 'io.linkjar.test=issue98',
            '--tmpfs', '/tmp/pds:uid=1000,gid=1000,mode=0700',
            '-p', '127.0.0.1::3000']
    for key, value in env.items():
        args += ['-e', key + '=' + value]
    docker(*args, image)
    try:
        port = docker('port', name, '3000/tcp').rsplit(':', 1)[1]
        base = 'http://127.0.0.1:' + port
        for _ in range(90):
            if docker('inspect', '--format', '{{.State.Running}}', name) != 'true':
                raise RuntimeError('PDS exited: ' + docker('logs', name))
            try:
                if request(base, '/xrpc/_health')['status'] == 200:
                    return name, base
            except OSError:
                pass
            time.sleep(1)
        raise RuntimeError('PDS did not become healthy: ' + docker('logs', name))
    except BaseException:
        docker('rm', '-fv', name)
        raise

def probe(base):
    return {
        'health': request(base, '/xrpc/_health'),
        'describeServer': request(base, '/xrpc/com.atproto.server.describeServer'),
        'oauthMetadata': request(base, '/.well-known/oauth-authorization-server'),
        'missingSession': request(base, '/xrpc/com.atproto.server.getSession'),
        'invalidSession': request(base, '/xrpc/com.atproto.server.createSession',
                                  {'identifier': 'missing.test', 'password': 'not-a-password'}),
        'invalidRecord': request(base, '/xrpc/com.atproto.repo.getRecord?repo=invalid'),
    }

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--unpatched', default='linkjar-pds:unpatched')
    parser.add_argument('--branded', default='linkjar-pds:branding-smoke')
    parser.add_argument('--production', default='linkjar-pds:production')
    parser.add_argument('--browser', action='store_true')
    args = parser.parse_args()
    pin = json.loads((ROOT / 'upstream.json').read_text())
    containers = []
    try:
        official, official_url = start(pin['officialImage'], 'official')
        containers.append(official)
        candidate, candidate_url = start(args.unpatched, 'unpatched')
        containers.append(candidate)
        version = pin['tag'].split('@')[-1]
        for container in (official, candidate):
            package_version = docker('exec', container, 'node', '-p',
                                     'require("@atproto/pds/package.json").version')
            assert package_version == version, (container, package_version, version)
        expected, actual = probe(official_url), probe(candidate_url)
        assert expected == actual, json.dumps({'official': expected, 'candidate': actual}, indent=2)
        assert actual['health']['status'] == 200
        assert actual['describeServer']['status'] == 200
        assert actual['oauthMetadata']['status'] == 200
        assert actual['health']['body']['version'] == pin['tag'].split('@')[-1]
        for name in ('missingSession', 'invalidSession', 'invalidRecord'):
            assert actual[name]['status'] in (400, 401)
        branded, branded_url = start(args.branded, 'branded')
        containers.append(branded)
        production, production_url = start(args.production, 'production')
        containers.append(production)
        production_health = request(production_url, '/xrpc/_health')
        assert production_health == actual['health']
        assert request(production_url, '/.well-known/oauth-authorization-server')['status'] == 200
        if args.browser:
            subprocess.run(['node', str(ROOT / 'scripts/browser-smoke.mjs'),
                            candidate_url, branded_url], check=True)
        result = {'upstream': pin, 'parity': actual, 'productionHealth': production_health,
                  'browserVerified': args.browser,
                  'limits': 'Local HTTP smoke only; no public signup, federation, real IdPs, TLS or staging deployment.'}
        (ROOT / '.build').mkdir(exist_ok=True)
        (ROOT / '.build/smoke-result.json').write_text(json.dumps(result, indent=2) + '\n')
        print('Official/unpatched API parity passed; browser verification:', args.browser)
    finally:
        for name in containers:
            docker('rm', '-fv', name)

if __name__ == '__main__':
    main()

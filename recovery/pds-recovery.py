#!/usr/bin/env python3
"""PDS backup inventory, encrypted host material, and isolated restore checks."""
import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import sqlite3
import subprocess
import tarfile
import tempfile
import time
from datetime import datetime, timezone
from urllib.parse import urlparse

SHARED = {'account.sqlite', 'sequencer.sqlite', 'did_cache.sqlite'}
TAG = 'linkjar-pds-host-material'


def regular(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f'Expected a regular file: {path.name}')
    return path


def walk(root):
    if root.is_symlink() or not root.is_dir():
        raise ValueError('Expected a real directory')
    for parent, directories, files in os.walk(root, followlinks=False):
        for name in directories + files:
            path = Path(parent) / name
            if path.is_symlink():
                raise ValueError('Symlinks are not allowed in the backup inventory')
        for name in files:
            yield regular(Path(parent) / name)


def private_write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as output:
        temp = Path(output.name)
        output.write(data)
    os.replace(temp, path)


def sqlite_read(path):
    regular(path)
    return sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)


def inventory(data, config):
    databases = []
    files = []
    for path in walk(data):
        rel = path.relative_to(data).as_posix()
        if path.suffix == '.sqlite':
            if rel not in SHARED and not re.fullmatch(r'actors/[0-9a-f]{2}/did:[^/]+/store\.sqlite', rel):
                raise ValueError('Unexpected database location; extend the reviewed replication configuration')
            databases.append(rel)
        elif (path.name == 'key' and rel.startswith('actors/')) or rel.startswith('actors/reserved_keys/'):
            if path.stat().st_size != 32:
                raise ValueError('Unexpected actor signing key length')
            files.append(('data/' + rel, path))
    if not SHARED.issubset(databases):
        raise ValueError('A required shared database is missing')
    key_names = {name for name, _ in files}
    for name in databases:
        if name.startswith('actors/') and 'data/' + str(PurePosixPath(name).with_name('key')) not in key_names:
            raise ValueError('An actor database has no signing key')
    files += [('config/' + p.relative_to(config).as_posix(), p) for p in walk(config)]
    if not any(name == 'config/staging.env' for name, _ in files):
        raise ValueError('The online configuration must contain staging.env')
    return sorted(databases), sorted(files)


def config_for(data, prefix, *, local_replica=None, databases=None):
    def replica(suffix):
        if local_replica:
            return {'type': 'file', 'path': str(local_replica / suffix)}
        endpoint = os.environ.get('PDS_BACKUP_R2_ENDPOINT', '')
        parsed = urlparse(endpoint)
        if parsed.scheme != 'https' or not parsed.hostname or not parsed.hostname.endswith('.r2.cloudflarestorage.com') or parsed.path not in ('', '/') or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError('Set a valid HTTPS R2 S3 endpoint')
        bucket = os.environ.get('PDS_BACKUP_R2_BUCKET', '')
        if not re.fullmatch(r'[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]', bucket):
            raise ValueError('Set the private backup bucket name')
        key_path = Path(os.environ.get('PDS_BACKUP_SSE_KEY_FILE', '/nonexistent'))
        regular(key_path)
        if key_path.stat().st_mode & 0o077 or len(base64.b64decode(key_path.read_text().strip(), validate=True)) != 32:
            raise ValueError('SSE-C key must be a private file containing a base64 32-byte key')
        if not os.environ.get('AWS_ACCESS_KEY_ID') or not os.environ.get('AWS_SECRET_ACCESS_KEY'):
            raise ValueError('Set bucket-scoped backup credentials')
        return {'type': 's3', 'bucket': bucket, 'path': prefix + '/' + suffix,
                'endpoint': endpoint, 'region': 'auto', 'force-path-style': True,
                'sign-payload': True, 'concurrency': 2,
                'sse-customer-key-path': str(key_path.resolve())}
    if not isinstance(prefix, str) or not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9/_-]{0,127}', prefix) or '..' in prefix:
        raise ValueError('Invalid backup prefix')
    if databases is None:
        dbs = [{'path': str(data / name), 'replica': replica(name)} for name in sorted(SHARED)]
        dbs.append({'dir': str(data / 'actors'), 'pattern': '*.sqlite', 'recursive': True,
                    'watch': True, 'replica': replica('actors')})
    else:
        dbs = [{'path': str(data / name), 'replica': replica(name)} for name in databases]
    return {'addr': '127.0.0.1:9090', 'sync-interval': '1s',
            'heartbeat-url': 'http://127.0.0.1:9093/heartbeat', 'heartbeat-interval': '1m',
            'snapshot': {'interval': '24h', 'retention': '168h'},
            'l0-retention': '1h', 'shutdown-sync-timeout': '30s',
            'logging': {'level': 'warn', 'type': 'json'}, 'dbs': dbs}


def checked(command, **kwargs):
    # Child output can include account paths or environment-derived diagnostics.
    # Keep it out of CI/metrics and expose only a bounded action/exit status.
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=kwargs.pop('timeout', 600), **kwargs)
    if result.returncode:
        raise RuntimeError(f'{Path(command[0]).name} {command[1]} failed (exit {result.returncode})')
    return result.stdout


def backup(args):
    started = time.time()
    if any(args.state.resolve().is_relative_to(root.resolve()) for root in [args.data, args.config]):
        raise ValueError('Backup state must be outside data and online configuration')
    databases, files = inventory(args.data, args.config)
    manifest = {'version': 1, 'startedAt': started, 'databases': databases, 'files': {}}
    with tempfile.TemporaryDirectory(prefix='linkjar-key-backup-', dir=args.state) as tmp:
        archive = Path(tmp) / 'host-material.tar'
        with tarfile.open(archive, 'w') as tar:
            for name, source in files:
                regular(source)
                if source.stat().st_size > 64 * 1024 * 1024:
                    raise ValueError('Unexpectedly large host material file')
                with source.open('rb') as stream:
                    before = os.fstat(stream.fileno())
                    content = stream.read()
                    after = os.fstat(stream.fileno())
                if (before.st_mtime_ns, before.st_size) != (after.st_mtime_ns, after.st_size):
                    raise ValueError('Host material changed during capture; retry')
                manifest['files'][name] = hashlib.sha256(content).hexdigest()
                info = tarfile.TarInfo(name)
                info.size, info.mode, info.mtime = len(content), 0o600, int(before.st_mtime)
                tar.addfile(info, io.BytesIO(content))
            manifest['completedAt'] = time.time()
            body = json.dumps(manifest, sort_keys=True).encode()
            info = tarfile.TarInfo('manifest.json')
            info.size, info.mode = len(body), 0o600
            tar.addfile(info, io.BytesIO(body))
        archive.chmod(0o600)
        with archive.open('rb') as stream:
            output = checked([args.restic, 'backup', '--stdin', '--stdin-filename', 'host-material.tar',
                              '--json', '--host', args.environment, '--tag', TAG], stdin=stream)
        rows = [json.loads(line) for line in output.splitlines()]
        summary = next(row for row in rows if row.get('message_type') == 'summary')
        status = {'lastSuccess': time.time(), 'captureStartedAt': started,
                  'snapshot': summary['snapshot_id'], 'databases': len(databases), 'files': len(files),
                  'inventoryDigest': hashlib.sha256(json.dumps(databases).encode()).hexdigest()}
        status_path = args.state / 'key-backup-status.json'
        private_write(status_path, json.dumps(status).encode())
        if os.geteuid() == 0:
            owner = args.data.stat()
            os.chown(status_path, owner.st_uid, owner.st_gid)
        print(json.dumps(status))


def safe_member(name):
    value = PurePosixPath(name)
    if value.is_absolute() or '..' in value.parts or str(value) != name or '\\' in name:
        raise ValueError('Unsafe archive path')
    return value


def unpack(archive, target):
    with tarfile.open(archive) as tar:
        members = tar.getmembers()
        names = [m.name for m in members]
        if len(names) != len(set(names)):
            raise ValueError('Duplicate archive paths')
        for member in members:
            safe_member(member.name)
            if not member.isfile() or member.size > 64 * 1024 * 1024:
                raise ValueError('Unexpected archive member')
        record = tar.extractfile('manifest.json')
        manifest = json.load(record)
        if manifest.get('version') != 1 or set(names) != set(manifest['files']) | {'manifest.json'}:
            raise ValueError('Invalid backup manifest')
        for member in members:
            content = tar.extractfile(member).read()
            if member.name != 'manifest.json' and hashlib.sha256(content).hexdigest() != manifest['files'][member.name]:
                raise ValueError('Host material checksum mismatch')
            if member.name != 'manifest.json' and not member.name.startswith(('data/actors/', 'config/')):
                raise ValueError('Unexpected host material path')
            private_write(target / member.name, content)
        for name in manifest['databases']:
            safe_member(name)
            if name not in SHARED and not re.fullmatch(r'actors/[0-9a-f]{2}/did:[^/]+/store\.sqlite', name):
                raise ValueError('Invalid database inventory')
        return manifest


def validate(data):
    paths = [p for p in walk(data) if p.suffix == '.sqlite']
    if not SHARED.issubset({p.name for p in paths if p.parent == data}):
        raise ValueError('Shared database missing')
    for path in paths:
        with sqlite_read(path) as db:
            if db.execute('PRAGMA integrity_check').fetchall() != [('ok',)]:
                raise ValueError('SQLite integrity check failed')
    with sqlite_read(data / 'account.sqlite') as db:
        actors = {row[0] for row in db.execute('SELECT did FROM actor')}
        roots = {row[0]: row[1:] for row in db.execute('SELECT did, cid, rev FROM repo_root')}
    recovered = set()
    for path in paths:
        if path.parent == data:
            continue
        did = path.parent.name
        expected = data / 'actors' / hashlib.sha256(did.encode()).hexdigest()[:2] / did / 'store.sqlite'
        if path != expected or did not in actors:
            raise ValueError('Actor inventory and account database disagree')
        if regular(path.with_name('key')).stat().st_size != 32:
            raise ValueError('Missing or malformed actor signing key')
        with sqlite_read(path) as db:
            rows = db.execute('SELECT did, cid, rev FROM repo_root').fetchall()
        if len(rows) != 1 or rows[0][0] != did or roots.get(did) != rows[0][1:]:
            raise ValueError('Shared and actor repository roots disagree; reconcile before serving')
        recovered.add(did)
    if recovered != actors or set(roots) != actors:
        raise ValueError('Missing actor database or shared repository root')
    return {'databases': len(paths), 'actors': len(actors), 'integrity': 'passed',
            'trafficApproved': False, 'remaining': 'Verify sequencer continuity, DID signing keys, blob reads, signed writes and OAuth on the fenced staging host.'}


def restore(args):
    if args.destination.exists():
        raise ValueError('Restore destination must not exist; never overwrite an existing data directory')
    if not re.fullmatch(r'[0-9a-f]{64}', args.snapshot):
        raise ValueError('Choose an exact 64-character restic snapshot ID')
    stamp = datetime.fromisoformat(args.timestamp.replace('Z', '+00:00'))
    if stamp.tzinfo is None or stamp.utcoffset().total_seconds() != 0:
        raise ValueError('Restore timestamp must include UTC timezone')
    args.destination.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix='linkjar-restore-') as tmp:
            archive = Path(tmp) / 'host-material.tar'
            with archive.open('wb') as output:
                process = subprocess.run([args.restic, 'dump', args.snapshot, '/host-material.tar'],
                                         stdout=output, stderr=subprocess.PIPE, timeout=600)
            archive.chmod(0o600)
            if process.returncode:
                raise RuntimeError('Restic restore failed')
            manifest = unpack(archive, args.destination)
            if stamp.timestamp() < manifest['completedAt']:
                raise ValueError('Choose host material captured before the database restore timestamp')
            if stamp.timestamp() - manifest['startedAt'] > args.max_host_material_age:
                raise ValueError('Host material is stale for this restore timestamp')
            data = args.destination / 'data'
            cfg = config_for(data, args.prefix, local_replica=args.local_replica, databases=manifest['databases'])
            cfg_path = Path(tmp) / 'restore.json'
            private_write(cfg_path, json.dumps(cfg).encode())
            for name in manifest['databases']:
                output = data / name
                output.parent.mkdir(parents=True, exist_ok=True)
                checked([args.litestream, 'restore', '-config', str(cfg_path), '-no-expand-env',
                         '-timestamp', args.timestamp, '-parallelism', '2', '-integrity-check', 'full',
                         '-o', str(output), str(output)])
            result = validate(data)
            result.update({'restoreSeconds': round(time.monotonic() - started, 3),
                           'snapshot': args.snapshot, 'requestedTimestamp': args.timestamp,
                           'hostMaterialAgeSeconds': max(0, stamp.timestamp() - manifest['startedAt'])})
            private_write(args.destination / 'validation.json', json.dumps(result, indent=2).encode())
            print(json.dumps(result))
    except BaseException:
        private_write(args.destination / 'RESTORE-INCOMPLETE', b'Do not start this restore. Inspect and retry in a new destination.\n')
        raise


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    config = sub.add_parser('config')
    config.add_argument('--data', type=Path, required=True)
    config.add_argument('--output', type=Path, required=True)
    backup_parser = sub.add_parser('backup-host-material')
    backup_parser.add_argument('--data', type=Path, required=True)
    backup_parser.add_argument('--config', type=Path, required=True)
    backup_parser.add_argument('--state', type=Path, required=True)
    backup_parser.add_argument('--restic', default='restic')
    backup_parser.add_argument('--environment', required=True, choices=['linkjar-pds-staging', 'linkjar-pds-production'])
    restore_parser = sub.add_parser('restore')
    restore_parser.add_argument('--destination', type=Path, required=True)
    restore_parser.add_argument('--snapshot', required=True)
    restore_parser.add_argument('--timestamp', required=True)
    restore_parser.add_argument('--max-host-material-age', type=int, default=60, help='Maximum seconds between host material capture and database restore timestamp')
    restore_parser.add_argument('--restic', default='restic')
    restore_parser.add_argument('--litestream', default='litestream')
    check = sub.add_parser('validate')
    check.add_argument('--data', type=Path, required=True)
    for item in [config, restore_parser]:
        item.add_argument('--prefix', default=os.environ.get('PDS_BACKUP_PREFIX'), help='Unique replication epoch prefix; defaults to PDS_BACKUP_PREFIX')
        item.add_argument('--local-replica', type=Path, help='Disposable local tests only; production uses R2 with SSE-C')
    args = parser.parse_args()
    for field in ['data', 'config', 'state', 'destination', 'local_replica']:
        if getattr(args, field, None) is not None:
            value = getattr(args, field)
            if value.is_symlink():
                raise ValueError('Do not use symlink roots')
            setattr(args, field, value.absolute())
    if args.command == 'config':
        private_write(args.output, json.dumps(config_for(args.data, args.prefix, local_replica=args.local_replica), indent=2).encode())
    elif args.command == 'backup-host-material':
        args.state.mkdir(parents=True, exist_ok=True, mode=0o700)
        backup(args)
    elif args.command == 'restore':
        if args.max_host_material_age < 1:
            raise ValueError('Host material age limit must be positive')
        restore(args)
    else:
        print(json.dumps(validate(args.data)))


if __name__ == '__main__':
    main()

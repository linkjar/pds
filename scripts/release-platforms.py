#!/usr/bin/env python3
"""Reject incomplete or unexpected release indexes before publishing a stable tag."""
import json
import re
import sys


def validate(index):
    if index.get('schemaVersion') != 2 or index.get('mediaType') not in {
        'application/vnd.oci.image.index.v1+json',
        'application/vnd.docker.distribution.manifest.list.v2+json',
    }:
        raise ValueError('Release must be an OCI index or Docker manifest list')
    entries = index.get('manifests')
    if not isinstance(entries, list) or not entries:
        raise ValueError('Release index has no manifests')
    platforms = set()
    runtime_digests = set()
    attestations = []
    for entry in entries:
        digest = entry.get('digest', '')
        if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
            raise ValueError('Invalid child digest')
        platform = entry.get('platform', {})
        pair = (platform.get('os'), platform.get('architecture'))
        if pair == ('unknown', 'unknown'):
            annotations = entry.get('annotations', {})
            if annotations.get('vnd.docker.reference.type') != 'attestation-manifest':
                raise ValueError('Unknown platform is not a build attestation')
            attestations.append(annotations.get('vnd.docker.reference.digest'))
            continue
        if pair not in {('linux', 'amd64'), ('linux', 'arm64')} or pair in platforms:
            raise ValueError('Unexpected or duplicate runtime platform')
        if platform.get('variant') not in ({None, ''} if pair[1] == 'amd64' else {None, '', 'v8'}):
            raise ValueError('Unexpected CPU variant')
        platforms.add(pair)
        runtime_digests.add(digest)
    if platforms != {('linux', 'amd64'), ('linux', 'arm64')}:
        raise ValueError('Both native runtime platforms are required')
    if any(digest not in runtime_digests for digest in attestations):
        raise ValueError('Attestation references a missing runtime manifest')


if __name__ == '__main__':
    try:
        validate(json.load(open(sys.argv[1])))
    except (ValueError, TypeError, AttributeError, KeyError) as error:
        raise SystemExit(f'Release platform validation failed: {error}')
    print('Release contains exactly linux/amd64 and linux/arm64 plus valid build attestations')

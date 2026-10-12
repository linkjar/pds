#!/usr/bin/env python3
"""Update the release pin from annotated or lightweight stable PDS tags."""
import argparse
import importlib.util
import json
import pathlib
import re
import subprocess
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
REPO = 'https://github.com/bluesky-social/atproto.git'

def releases(refs):
    tags = {}
    peeled = {}
    for line in refs.splitlines():
        sha, ref = line.split()
        match = re.fullmatch(r'refs/tags/(@atproto/pds@(\d+)\.(\d+)\.(\d+))(\^\{\})?', ref)
        if match:
            (peeled if match[5] else tags)[match[1]] = (tuple(map(int, match.group(2, 3, 4))), sha)
    return {tag: peeled.get(tag, entry) for tag, entry in tags.items()}

def report(pin, tags):
    """List the stable releases after the pin, oldest first, as Markdown."""
    pinned = tags[pin['tag']][0]
    newer = sorted((entry, tag) for tag, entry in tags.items() if entry[0] > pinned)
    lines = [f"The pin is `{pin['tag']}` at `{pin['commit']}`. Upstream has {len(newer)} newer stable release(s):", '']
    for (version, sha), tag in newer:
        name = '.'.join(map(str, version))
        lines.append(f"- [`{tag}`](https://github.com/bluesky-social/atproto/releases/tag/%40atproto%2Fpds%40{name}) at `{sha}`")
    return '\n'.join(lines) + '\n'

def main():
    args = argparse.ArgumentParser()
    args.add_argument('--write', action='store_true')
    args.add_argument('--report', type=pathlib.Path, help='write the newer releases to this file as Markdown')
    options = args.parse_args()
    pin = json.loads((ROOT.parent / 'upstream.json').read_text())
    refs = subprocess.check_output(['git', 'ls-remote', '--tags', REPO, '@atproto/pds@*'], text=True)
    tags = releases(refs)
    latest = max(tags, key=lambda tag: tags[tag][0])
    if latest == pin['tag']:
        if tags[latest][1] != pin['commit']:
            raise SystemExit('Pinned upstream tag moved; manual investigation required')
        print('PDS pin is current:', latest)
        return
    print(pin['tag'], '->', latest, tags[latest][1])
    if options.report:
        options.report.write_text(report(pin, tags))
    if options.write:
        pin.update(tag=latest, commit=tags[latest][1], revision=1)
        # Keep the official comparator digest fixed. If that release is not yet
        # distributed officially, parity CI intentionally blocks this update.
        url = f"https://raw.githubusercontent.com/bluesky-social/atproto/{pin['commit']}/services/pds/Dockerfile"
        with urllib.request.urlopen(url, timeout=30) as response:
            dockerfile = response.read().decode()
        spec = importlib.util.spec_from_file_location('generate', ROOT / 'scripts/generate-dockerfile.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        generated = module.generate(pin, dockerfile)
        (ROOT.parent / 'upstream.json').write_text(json.dumps(pin, indent=2) + '\n')
        (ROOT.parent / 'docs/legacy/upstream.Dockerfile').write_text(dockerfile)
        (ROOT / 'Dockerfile').write_text(generated)

if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Report the stable PDS releases newer than the pin in upstream.json.

The pin is the compatibility baseline of the server (docs/SPEC.md section
2.1). A newer upstream release changes nothing here by itself; the weekly
report keeps the owner aware of what upstream shipped after the pin.
"""
import argparse
import json
import pathlib
import re
import subprocess

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
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument('--report', type=pathlib.Path, help='write the newer releases to this file as Markdown')
    options = args.parse_args()
    pin = json.loads((ROOT / 'upstream.json').read_text())
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

if __name__ == '__main__':
    main()

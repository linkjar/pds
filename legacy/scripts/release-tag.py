#!/usr/bin/env python3
"""Print the release tag that upstream.json names, as a workflow output line."""
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent

def release_tag(pin):
    # '@atproto/pds@0.5.34' at revision 11 is 'atproto-pds-0.5.34-11'.
    version = pin['tag'].split('@')[-1]
    return f"atproto-pds-{version}-{pin['revision']}"

if __name__ == '__main__':
    print('tag=' + release_tag(json.loads((ROOT.parent / 'upstream.json').read_text())))

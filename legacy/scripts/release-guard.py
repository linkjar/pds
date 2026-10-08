#!/usr/bin/env python3
"""Check absence only after this job has published a unique staging tag."""
import re
import subprocess
import sys

def confirmed_absent(returncode, stderr):
    # GHCR returns DENIED for a package that has never been created. That is
    # deliberately not an absence result: it is indistinguishable from bad auth.
    return returncode != 0 and bool(re.search(r'(^|\n)(manifest unknown|no such manifest:)', stderr))

def main():
    result = subprocess.run(['docker', 'manifest', 'inspect', sys.argv[1]], text=True, capture_output=True)
    if result.returncode == 0:
        raise SystemExit('Release tag exists. Increment upstream.json revision before publishing.')
    if not confirmed_absent(result.returncode, result.stderr):
        raise SystemExit('Cannot establish that release tag is unused; refusing to publish.\n' + result.stderr)
    print('Release tag is confirmed absent')

if __name__ == '__main__':
    main()

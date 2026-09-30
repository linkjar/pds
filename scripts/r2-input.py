#!/usr/bin/env python3
"""Split an R2 endpoint or dashboard bucket URL without contacting storage."""
import re
import sys
from urllib.parse import urlsplit


def parse(value):
    url = urlsplit(value.strip())
    if (url.scheme != 'https' or url.username or url.password or url.port
            or url.query or url.fragment or not url.hostname
            or not re.fullmatch(r'[a-f0-9]{32}(?:\.(?:eu|fedramp))?\.r2\.cloudflarestorage\.com', url.hostname)):
        raise ValueError('Paste the HTTPS R2 S3 endpoint or bucket URL, without credentials or query parameters.')
    bucket = url.path.strip('/')
    if bucket and not re.fullmatch(r'[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]', bucket):
        raise ValueError('The URL must contain an endpoint or one bucket, not an object path.')
    if bucket == 'linkjar-tfstate':
        raise ValueError('linkjar-tfstate stores infrastructure state. Choose a separate private PDS blob or backup bucket.')
    return 'https://' + url.hostname, bucket


if __name__ == '__main__':
    try:
        print('\n'.join(parse(sys.argv[1])))
    except (ValueError, IndexError):
        print('Invalid R2 input. Use a separate PDS bucket and the account S3 endpoint; never use linkjar-tfstate.', file=sys.stderr)
        sys.exit(1)

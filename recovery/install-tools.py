#!/usr/bin/env python3
"""Install reviewed release binaries into an explicit directory; verify archives."""
import argparse
import bz2
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import tarfile
import tempfile
import urllib.request


def install(destination):
    pins = json.loads(Path(__file__).with_name('tools.json').read_text())
    system = platform.system().lower()
    machine = {'aarch64': 'arm64', 'AMD64': 'x86_64'}.get(platform.machine(), platform.machine())
    destination.mkdir(parents=True, exist_ok=True)
    for name, pin in pins.items():
        target = f'{system}-{machine}' if name == 'litestream' else f'{system}_{"amd64" if machine == "x86_64" else machine}'
        expected = pin['sha256'].get(target)
        if not expected:
            raise ValueError(f'No reviewed {name} binary for {target}')
        with urllib.request.urlopen(pin['url'].format(platform=target), timeout=60) as response:
            archive = response.read(150_000_001)
        if len(archive) > 150_000_000 or hashlib.sha256(archive).hexdigest() != expected:
            raise ValueError(f'{name} release checksum mismatch')
        if name == 'restic':
            binary = bz2.decompress(archive)
        else:
            with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as tar:
                members = [m for m in tar.getmembers() if Path(m.name).name == name and m.isfile()]
                if len(members) != 1:
                    raise ValueError('Unexpected release archive layout')
                binary = tar.extractfile(members[0]).read()
        with tempfile.NamedTemporaryFile(dir=destination, delete=False) as output:
            output.write(binary)
            temp = Path(output.name)
        temp.chmod(0o755)
        os.replace(temp, destination / name)
        print(f'Installed verified {name} {pin["version"]}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, required=True)
    install(parser.parse_args().directory.resolve())

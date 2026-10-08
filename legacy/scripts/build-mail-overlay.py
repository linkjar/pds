#!/usr/bin/env python3
"""Build the pinned mail overlay from the canonical source patch, without PDS secrets."""
from pathlib import Path
import subprocess,tempfile
ROOT=Path(__file__).resolve().parents[1]
BASE='ghcr.io/linkjar/pds@sha256:65439dafd5c431e86400a877cc33d91a932ec931aef52223dc44972535080d07'
def run(*args,**kwargs):return subprocess.check_output(args,text=True,**kwargs).strip()
with tempfile.TemporaryDirectory(prefix='linkjar-mail-build-') as tmp:
 root=Path(tmp);target=root/'packages/pds/src';target.mkdir(parents=True)
 container=run('docker','create',BASE)
 try:run('docker','cp',container+':/app/packages/pds/src/mailer',str(target))
 finally:run('docker','rm',container)
 run('git','apply','--check',str(ROOT/'patches/101-mail-templates.patch'),cwd=root)
 run('git','apply',str(ROOT/'patches/101-mail-templates.patch'),cwd=root)
 (root/'Dockerfile').write_bytes((ROOT/'Dockerfile.mail').read_bytes())
 subprocess.run(['docker','build','-t','linkjar-pds:mail-overlay',str(root)],check=True)
 print(run('docker','image','inspect','linkjar-pds:mail-overlay','--format','{{.Id}}'))

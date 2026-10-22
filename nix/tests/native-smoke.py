"""Run a packaged executable on disposable local data without Docker."""

import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time


def main():
    binary = str(Path(sys.argv[1]).resolve())
    version = subprocess.check_output([binary, "version"], text=True).strip()
    assert version.startswith("linkjar-pds "), version

    with tempfile.TemporaryDirectory(prefix="pds-native-") as directory:
        dualstack = socket.has_dualstack_ipv6()
        family = socket.AF_INET6 if dualstack else socket.AF_INET
        with socket.socket(family) as listener:
            if dualstack:
                listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
            listener.bind(("::" if dualstack else "0.0.0.0", 0))
            listener.listen()
            port = listener.getsockname()[1]
        environment = {
            key: os.environ[key]
            for key in ("PATH", "HOME", "TMPDIR", "SSL_CERT_FILE", "NIX_SSL_CERT_FILE")
            if key in os.environ
        }
        environment.update(
            PDS_HOSTNAME="localhost",
            PDS_PORT=str(port),
            PDS_VERSION=version.split(" ", 1)[1],
            PDS_DATA_DIRECTORY=directory,
            PDS_BLOBSTORE_DISK_LOCATION=str(Path(directory) / "blocks"),
            PDS_JWT_SECRET="native-smoke-test-only",
            PDS_ADMIN_PASSWORD="native-smoke-test-only",
            PDS_DID_PLC_URL="http://127.0.0.1:9",
        )
        subprocess.run([binary, "check-config"], env=environment, check=True)
        with tempfile.TemporaryFile(mode="w+") as logs:
            process = subprocess.Popen(
                [binary, "serve"], env=environment, stdout=logs, stderr=logs
            )
            try:
                deadline = time.monotonic() + 30
                while True:
                    if process.poll() is not None:
                        raise AssertionError("The native server exited before becoming healthy")
                    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                    try:
                        connection.request("GET", "/xrpc/_health")
                        response = connection.getresponse()
                        body = response.read()
                        if response.status == 200:
                            assert json.loads(body)["version"] == environment["PDS_VERSION"]
                            break
                    except (OSError, http.client.HTTPException):
                        pass
                    finally:
                        connection.close()
                    if time.monotonic() >= deadline:
                        raise AssertionError("The native server did not become healthy")
                    time.sleep(0.1)
            except BaseException:
                logs.seek(0)
                print(logs.read(), file=sys.stderr)
                raise
            finally:
                process.terminate()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            assert process.returncode == 0, process.returncode
        report = json.loads(
            subprocess.check_output([binary, "verify", "--json"], env=environment, text=True)
        )
        assert report["differences"] == 0, report
        assert (Path(directory) / "account.sqlite").is_file()
    print(f"{version}: native startup, health, graceful shutdown, and data verification passed")


if __name__ == "__main__":
    main()

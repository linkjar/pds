#!/usr/bin/env python3
"""Prepare an encrypted LinkJar infrastructure plan; never apply it."""
import ipaddress
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

os.umask(0o077)
root = Path(__file__).resolve().parents[1]
state_dir = Path.home() / ".local/share/linkjar-infra"
state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
state_dir.chmod(0o700)
tofu = shutil.which("tofu")
if not tofu:
    sys.exit("Run inside a shell with OpenTofu available.")

def credential(account):
    try:
        result = subprocess.run(["security", "find-generic-password", "-s", "linkjar", "-a", account, "-w"], capture_output=True, text=True, timeout=180)
    except subprocess.TimeoutExpired:
        sys.exit(f"Keychain access timed out for linkjar/{account}; allow the macOS prompt and retry.")
    if result.returncode or not result.stdout.strip():
        sys.exit(f"Missing Keychain item linkjar/{account}")
    return result.stdout.strip()

env = os.environ.copy()
env["TF_VAR_hcloud_token"] = credential("hcloud_token")
env["TF_VAR_state_passphrase"] = credential("tfstate_passphrase")
public_key = (Path.home() / ".ssh/linkjar_ed25519.pub").read_text().strip()
if public_key not in (root / "nixos/host.nix").read_text():
    sys.exit("The dedicated SSH public key must match nixos/host.nix before planning.")
env["TF_VAR_ssh_public_key"] = public_key
trace = subprocess.run(["curl", "-4", "--fail", "--silent", "--show-error", "--max-time", "20", "https://www.cloudflare.com/cdn-cgi/trace"], capture_output=True, text=True, check=True).stdout
address = next((line[3:] for line in trace.splitlines() if line.startswith("ip=")), "")
if not isinstance(ipaddress.ip_address(address), ipaddress.IPv4Address):
    sys.exit("An IPv4 address is required for the operator SSH allowlist.")
env["TF_VAR_admin_ipv4_cidr"] = address + "/32"
env["TF_VAR_enable_public_https"] = "false"
base = [tofu, f"-chdir={root / 'infra/hetzner'}"]
plan = state_dir / "review.tfplan"
with (state_dir / "plan.log").open("w") as log:
    init = subprocess.run(base + ["init", "-input=false", "-reconfigure", f"-backend-config=path={state_dir / 'terraform.tfstate'}"], env=env, stdout=log, stderr=subprocess.STDOUT)
    if init.returncode:
        sys.exit(f"Backend initialization failed. Private log: {state_dir / 'plan.log'}")
    result = subprocess.run(base + ["plan", "-input=false", "-no-color", "-detailed-exitcode", f"-out={plan}"], env=env, stdout=log, stderr=subprocess.STDOUT)
    if result.returncode not in (0, 2):
        sys.exit(f"Planning failed. Private log: {state_dir / 'plan.log'}")
# show -json includes credentials in its variables; keep it only in memory and
# print/write this allowlisted projection, never the full plan JSON.
shown = subprocess.run(base + ["show", "-json", str(plan)], env=env, capture_output=True, text=True, check=True)
data = json.loads(shown.stdout)
summary = []
for resource in data.get("resource_changes", []):
    change = resource["change"]
    after = change.get("after") or {}
    summary.append({"address": resource["address"], "actions": change["actions"], "configuration": {key: after[key] for key in ("name", "server_type", "location", "image", "public_net") if key in after}})
report = {"applied": False, "plan": str(plan), "changes": summary}
(state_dir / "review-summary.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))

# Dedicated LinkJar PDS on NixOS — deployment candidate

This adapts the OpenTofu → nixos-anywhere/disko approach inspected in
`/Users/kadza/sync/cc-remote`. It does not use that project's state, host, host
keys, Cloudflare Tunnel, deployment timer or disk installer. No infrastructure
has been purchased, installed, reimaged or exposed by adding these files.

The candidate is an ARM64 CAX21 in Nuremberg (`nbg1`), with a public IPv4.
This matches cc-remote’s ARM architecture; the PDS still has a separate host
identity, key, disk installation target and state. PDS image revision 8 is pinned by its verified multi-platform
digest. The host keeps port 3000 on loopback and uses the existing reviewed Caddy
handle routing. Service startup and public HTTPS are disabled initially.

## Before purchasing

1. Confirm CAX21 stock and the account's final quote. The signed-in Hetzner Console
   showed €10.49/month plus €0.50 for IPv4: €10.99/month excluding VAT on
   2026-09-13. R2 and mail are additional. Obtain approval for this recurring quote.
2. Create a separate Hetzner project/token for LinkJar if isolation is desired.
   Store the token and a new state-encryption passphrase in the operator vault.
3. Create a dedicated operator SSH key. Put its **public** half in
   `nixos/host.nix` and `TF_VAR_ssh_public_key`; keep the private half local.
4. Set `TF_VAR_admin_ipv4_cidr` to the operator's IPv4 with `/32`.

Run OpenTofu only from `infra/hetzner`, with `TF_VAR_hcloud_token` and
`TF_VAR_state_passphrase` set without printing them. `tofu init`, then
`tofu plan -out=pds.tfplan`. Review the exact plan and quote before applying.
State and saved plans are encrypted and ignored by Git; back up the state and
passphrase through the approved secret store. Do not point this configuration
at cc-remote's R2 backend or existing resources.

## Install only on the newly created PDS server

Check the new server ID/IP against the reviewed plan. Review the disk layout
and the nonempty `adminSshKeys` before installation. nixos-anywhere repartitions
`/dev/sda`; never invoke it against cc-remote or a host containing retained data.
Use this flake's pinned nixos-anywhere input, the `#linkjar-pds` configuration,
and the aarch64 installer/default detected architecture. A Mac must use remote
building or an aarch64-linux builder. Keep SSH reachable from the approved /32.
After installation, SSH as `operator` with the dedicated key.

## Runtime and launch gates

The existing [provisioning wizard](../scripts/provision-wizard.sh) and
[PDS runbook](../docs/runbooks/pds.md) still supply the runtime values and checks.
Place the raw PDS environment file at `/etc/linkjar-pds/pds.env`, root-owned mode
0600, through the approved secret channel. Never import plaintext secrets into
Nix: they would become readable store contents. This candidate uses a runtime
file; adapting cc-remote's sops-nix delivery with separate recipients remains a
follow-up before calling secret delivery declarative.

Install a Cloudflare origin certificate for `pds.linkjar.social` and
`*.linkjar.social` at `/var/lib/caddy/linkjar-origin.pem` and its private key at
`/var/lib/caddy/linkjar-origin-key.pem`, readable only by the Caddy service. Set
Cloudflare to Full (strict). Complete runtime configuration, encrypted key and
SQLite backups, restore rehearsal, SMTP/hCaptcha/provider checks and operator
policy inputs before opening signup. The backup runner, monitoring and pull
deployment from cc-remote are not silently installed by this candidate.

Enable `host.enablePds` only for a configured test deployment. Review activation
before setting `enable_public_https` and DNS. Do not expose incomplete public
registration. Keep the live acceptance items in issues #90–#94 open until they
are exercised on the actual new host.

Published pricing reference:
https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/

## Validation recorded locally

Both disabled and enabled-service NixOS configurations evaluated to complete
system derivations. The enabled evaluation used a synthetic SSH public key and
restored the disabled host file afterward. OpenTofu initialization without a
backend, formatting and validation passed without provider credentials. These
checks do not prove a boot, installation, live TLS, restore or account signup.
The PR workflow repeats both evaluations and offline OpenTofu validation.

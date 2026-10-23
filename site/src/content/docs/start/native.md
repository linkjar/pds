---
title: Install with Nix or a binary
description: Configure a native LinkJar PDS, run it with systemd or NixOS, and put HTTPS in front of it without Docker.
sidebar:
  order: 5
---

The server runs as one Rust executable. First [download or build it](/start/builds/).
For Linux without Nix, use the portable static executable. On NixOS, use the
module below. A foreground example is included for local checks.

You need a domain, a writable data directory, a blob store, CA certificates,
and a TLS proxy for public HTTPS. The HTTP listener binds all interfaces.
Keep port `2583` behind the host's firewall and allow public traffic through
your TLS proxy. Never enable `PDS_DEV_MODE` on a public host.

## Configure the server

For a new installation, generate a JWT secret and admin password separately:

```bash
openssl rand --hex 32
openssl rand --hex 32
```

Generate a secp256k1 PLC rotation key on Linux:

```bash
openssl ecparam --name secp256k1 --genkey --noout --outform DER | tail --bytes=+8 | head --bytes=32 | xxd --plain --cols 32
```

Keep these values in a secret manager or a restricted file. Reuse the original
secrets when moving an existing server. The rotation key controls the accounts'
DIDs; replacing it is not an installation reset.

Create `/etc/linkjar-pds/pds.env`, owned by root with mode `0600`, under a
root-owned directory. Create the directory before editing the file:

```bash
sudo install -d -m 0700 -o root -g root /etc/linkjar-pds
sudoedit /etc/linkjar-pds/pds.env
sudo chmod 0600 /etc/linkjar-pds/pds.env
```

Put one literal `NAME=value` assignment on each line.
Replace the three secret placeholders:

```dotenv title="/etc/linkjar-pds/pds.env"
PDS_HOSTNAME=pds.example.com
PDS_JWT_SECRET=REPLACE_WITH_JWT_SECRET
PDS_ADMIN_PASSWORD=REPLACE_WITH_ADMIN_PASSWORD
PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX=REPLACE_WITH_ROTATION_KEY
PDS_BLOBSTORE_DISK_LOCATION=/var/lib/linkjar-pds/blocks
PDS_INVITE_REQUIRED=true
PDS_BSKY_APP_VIEW_URL=https://api.bsky.app
PDS_BSKY_APP_VIEW_DID=did:web:api.bsky.app
PDS_REPORT_SERVICE_URL=https://mod.bsky.app
PDS_REPORT_SERVICE_DID=did:plc:ar7c4by46qjdydhdevvrndac
PDS_CRAWLERS=https://bsky.network
```

Systemd reads this file; the binary does not load `.env` files itself. Keep all
settings needed by operator commands in this runtime file. The examples below
set `PDS_DATA_DIRECTORY` and `PDS_PORT` in the service definition. Leave them
out of the file so they cannot override those settings.

For [S3 storage](/configure/blob-storage/), remove
`PDS_BLOBSTORE_DISK_LOCATION` and add the S3 settings and credentials.
Set up [email](/configure/email/) before inviting other people.
[Environment variables](/configure/environment/) lists the remaining options.

## NixOS service

Add a pinned input to your host's flake. Replace `<reviewed-commit>` with the
full commit you intend to run:

```nix
inputs.linkjar-pds.url = "github:linkjar/pds/<reviewed-commit>";
```

Add `inputs.linkjar-pds.nixosModules.default` to your host's `modules` list.
In a host module, enable the service and put the same package on the operator's
command path:

```nix
{ inputs, pkgs, ... }:
{
  services.linkjar-pds = {
    enable = true;
    environmentFile = "/etc/linkjar-pds/pds.env";
  };

  environment.systemPackages = [
    inputs.linkjar-pds.packages.${pkgs.stdenv.hostPlatform.system}.linkjar-pds
  ];
}
```

This example assumes your flake passes `inputs` through `specialArgs` to the
host modules. You can instead reference the input in the outer flake expression.

Provision the runtime file before activating the service. Your existing secret
manager can supply it at another runtime path, such as `/run/secrets/linkjar-pds.env`.
Use a quoted string for `environmentFile`; never put secrets in a Nix expression,
`builtins.toFile`, or a Nix path copied into the store.

The module creates the `linkjar-pds` user and group and mode `0700` state at
`/var/lib/linkjar-pds`. It checks configuration before startup, restarts after
failure, and restricts filesystem access. It supplies CA certificates and keeps
the HTTP port closed in the firewall by default. Set `services.linkjar-pds.port`
if you need a port other than `2583`. Configure your TLS proxy separately.

Activate the reviewed host configuration with your usual `nixos-rebuild`
procedure. Then check the service:

```bash
sudo systemctl status linkjar-pds
sudo journalctl -u linkjar-pds -n 50 --no-pager
curl --fail http://127.0.0.1:2583/xrpc/_health
```

The service protects `/home` and `/root`. Keep additional key files in an
accessible runtime secret directory. Use `services.linkjar-pds.environment`
only for public settings; [operator commands](/operate/cli/#native-services)
need those settings too.

## Linux binary with systemd

This example uses the executable installed at `/usr/local/bin/linkjar-pds`
by the [download guide](/start/builds/#download-a-portable-linux-build).
Keep the host's CA certificate package installed.

Create a dedicated account and data directory. On Debian or Ubuntu:

```bash
sudo useradd --system --user-group --home-dir /var/lib/linkjar-pds --shell /usr/sbin/nologin linkjar-pds
sudo install -d -m 0700 -o linkjar-pds -g linkjar-pds /var/lib/linkjar-pds
```

Create the account only once. If you use another distribution, use its system
account command. Save this unit as `/etc/systemd/system/linkjar-pds.service`:

```ini title="/etc/systemd/system/linkjar-pds.service"
[Unit]
Description=LinkJar AT Protocol Personal Data Server
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User=linkjar-pds
Group=linkjar-pds
WorkingDirectory=/var/lib/linkjar-pds
EnvironmentFile=/etc/linkjar-pds/pds.env
Environment=PDS_DATA_DIRECTORY=/var/lib/linkjar-pds
Environment=PDS_PORT=2583
ExecStartPre=/usr/local/bin/linkjar-pds check-config
ExecStart=/usr/local/bin/linkjar-pds serve
Restart=on-failure
RestartSec=5
TimeoutStopSec=30
StateDirectory=linkjar-pds
StateDirectoryMode=0700
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
CapabilityBoundingSet=

[Install]
WantedBy=multi-user.target
```

After saving the runtime environment file and firewall rules, start it:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now linkjar-pds
sudo systemctl status linkjar-pds
sudo journalctl -u linkjar-pds -n 50 --no-pager
curl --fail http://127.0.0.1:2583/xrpc/_health
```

## Public HTTPS

Point the TLS proxy at `127.0.0.1:2583`. It must handle the server hostname,
account handle names, and long-lived WebSocket connections for the firehose.
For a native Caddy installation, adapt the repository's Caddyfile:

```caddyfile title="Caddyfile"
{
    email operator@example.com
    on_demand_tls {
        ask http://127.0.0.1:2583/tls-check
    }
}

pds.example.com, *.pds.example.com {
    tls {
        on_demand
    }
    reverse_proxy 127.0.0.1:2583
}
```

Run Caddy as its own service with persistent certificate storage. The DNS and
ports in [the DNS guide](/start/dns/) apply to this setup too. The `/tls-check`
guard limits certificate requests to the server and active account handles.
For a different proxy, configure equivalent hostname, TLS, and WebSocket support.

## Foreground checks

On a private development host, run the binary as your own user with disposable
state. Set the required settings in that shell first:

```bash
export PDS_HOSTNAME=pds.example.com
export PDS_DATA_DIRECTORY="$PWD/native-data"
export PDS_BLOBSTORE_DISK_LOCATION="$PWD/native-data/blocks"
export PDS_JWT_SECRET=REPLACE_WITH_JWT_SECRET
export PDS_ADMIN_PASSWORD=REPLACE_WITH_ADMIN_PASSWORD
linkjar-pds check-config
linkjar-pds serve
```

Use the executable path from your build if it is not installed on `PATH`.
Stop it with Ctrl-C. This example does not configure public TLS or a rotation
key for creating accounts. Never point it at a live production data directory.

## Move from Docker

Take a tested [backup](/operate/backup/), stop the old server, and preserve the
complete data directory and every secret. Review the
[authentication limits on rollback](/operate/update/#roll-back).

Copy the stopped data to `/var/lib/linkjar-pds`, then assign it to
`linkjar-pds:linkjar-pds`. Change container paths such as `/app/data` to host
paths in the environment. Preserve external database and blob-store settings.
Run [native verification](/operate/cli/#native-services) before starting.
Only one server may use the data, including copies on different hosts.

Next: [verify the public setup](/start/verify/), then
[create an account](/start/first-account/). Use the operator guides for
[updates](/operate/update/), [backups](/operate/backup/), and
[monitoring](/operate/monitoring/).

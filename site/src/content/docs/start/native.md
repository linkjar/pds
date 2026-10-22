---
title: Install with Nix or a binary
description: Build and run LinkJar PDS directly with Nix, use its NixOS service, or build a portable Linux executable without Docker.
sidebar:
  order: 4
---

The server is one Rust executable, `linkjar-pds`. Docker is optional.
The same executable provides `serve`, `check-config`, `health`, `verify`,
and the operator commands in the [command-line guide](/operate/cli/).

You still need a domain, a writable data directory, a blob store, CA
certificates, and a TLS proxy or tunnel for public HTTPS. The server's
own HTTP listener binds all interfaces. Keep its port behind the host's
firewall and expose public traffic through your TLS proxy.

## Build and run with Nix

Check out the reviewed revision of [the repository](https://github.com/linkjar/pds).
Run these commands from its root:

```bash
nix build .#linkjar-pds
```

The executable is `result/bin/linkjar-pds`. Keep its Nix store closure;
this package uses the libraries managed by Nix.

```bash
nix run . -- version
```

Configure the `PDS_*` environment variables, then run:

```bash
result/bin/linkjar-pds check-config
```

```bash
result/bin/linkjar-pds serve
```

Use a service manager for a persistent installation. The NixOS module below
sets one up. On other hosts, configure your existing service manager to run
`linkjar-pds serve` as a dedicated user.

The flake supports Linux on `x86_64` and `aarch64`, and macOS on Apple
Silicon. `flake.lock` pins Nixpkgs and the Rust overlay. `Cargo.lock` pins
the Rust dependencies. Update these locks deliberately.

## NixOS service

Add a pinned input to your host's flake. Replace `<reviewed-commit>` with
the commit you intend to run:

```nix
inputs.linkjar-pds.url = "github:linkjar/pds/<reviewed-commit>";
```

Add `inputs.linkjar-pds.nixosModules.default` to the `modules` list of
your host's `nixpkgs.lib.nixosSystem`. Configure the service in a host module:

```nix
{
  services.linkjar-pds = {
    enable = true;
    environmentFile = "/run/secrets/linkjar-pds.env";
    environment = {
      PDS_HOSTNAME = "pds.example.com";
      PDS_BLOBSTORE_DISK_LOCATION = "/var/lib/linkjar-pds/blocks";
    };
  };
}
```

The module runs as the `linkjar-pds` system user, stores data in
`/var/lib/linkjar-pds`, and uses port `2583`. It checks configuration before
starting, restarts after a failure, and restricts writes to its service
state and private temporary directory. It does not open a firewall port
by default. Point your TLS proxy or tunnel at `127.0.0.1:2583`.

Store secrets in the runtime environment file using your existing secret
manager, such as sops-nix. Systemd reads this file before dropping privileges;
it can be owned by root with mode `0600`. Put these variables there:

```dotenv
PDS_JWT_SECRET=replace-with-a-random-secret
PDS_ADMIN_PASSWORD=replace-with-a-random-password
PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX=replace-with-a-valid-rotation-key
```

Use the key-generation steps in the [installation guide](/start/install/).
Keep passwords and keys out of Nix expressions and the Nix store.
`environmentFile` must be an absolute runtime path outside the store.
Leave `PDS_PORT` and `PDS_DATA_DIRECTORY` out of that file: the module
manages them. Set `services.linkjar-pds.port` to change the listener port.

For [S3 storage](/configure/blob-storage/), omit
`PDS_BLOBSTORE_DISK_LOCATION` and put the S3 settings and credentials in the
runtime file. The service must be able to read every additional key or
configuration file you name. Its home-directory protection blocks `/home`
and `/root`; use a runtime secret path such as `/run/secrets`.

## A portable Linux executable

On Linux, the flake can build a static musl executable:

```bash
nix build .#linkjar-pds-static
```

Copy `result/bin/linkjar-pds` to a Linux host with the same CPU architecture.
This executable needs neither Nix nor Docker, and has no shared-library
or dynamic-loader dependency. The host must still supply CA certificates
for outbound HTTPS. Set `SSL_CERT_FILE` if the certificate bundle is in a
nonstandard location.

The **Native packages** GitHub Actions workflow builds `x86_64-linux` and
`aarch64-linux` archives with SHA-256 checksums. Download the artifact for
your architecture from a successful run at the revision you reviewed.
The workflow checks the static ELF format and runs the copied executable
on disposable data before publishing its artifact.

You can also compile directly with Cargo and a C compiler:

```bash
cargo build --release --locked --package linkjar-pds --bin linkjar-pds
```

This produces `target/release/linkjar-pds` for the build host. A normal
Cargo build can depend on that host's shared libraries; use the static
Linux package when you need the portable executable described above.

## Development

```bash
nix develop
```

The shell provides Rust, the C build tools, libclang for the simulation,
Node, pnpm, and the Rust policy tools. The existing `devenv` workflow also
remains available.

## Moving an existing server

Plan the cutover before replacing a running Docker deployment. Take a
tested [backup](/operate/backup/), stop the old process, and preserve the
complete data directory and all secrets. That includes the authentication
state described in the [update and rollback guide](/operate/update/).

The NixOS service uses a different user and data path from the container.
Check file ownership and every absolute path in the environment before
starting it. Verify the copied data with the selected executable. Run one
server against a data directory at a time.

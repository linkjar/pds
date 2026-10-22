---
title: Update
description: Update a pinned native binary, NixOS package, or container image, and understand rollback limits.
sidebar:
  order: 1
---

An update replaces the executable and restarts the service on the existing data.
Record the current build, keep its configuration and secrets, and take a tested
[backup](/operate/backup/) first. Build or download the replacement before
stopping the server. Review changes between the two source commits.

This release adds a credential recovery table to the account database and an
index to actor databases on writable open. The first index build takes a SQLite
write lock and extra disk space. Allow for that work in your maintenance window.

## Native binary

[Download and verify](/start/builds/#download-a-portable-linux-build) a new archive,
or build the selected source revision. Keep the old executable in its versioned
`/opt/linkjar-pds/<commit>` directory. Install the new executable under a new
commit directory before changing the symlink.

Stop the service, then select the new executable. Set `PDS_REV` to the full
reviewed commit whose binary you installed:

```bash
sudo systemctl stop linkjar-pds
sudo ln -sfn "/opt/linkjar-pds/$PDS_REV/linkjar-pds" /usr/local/bin/linkjar-pds
```

Use the [native command helper](/operate/cli/#native-services), with
`PDS_BIN=/usr/local/bin/linkjar-pds`, to check configuration and data before startup:

```bash
pds check-config
pds verify
sudo systemctl start linkjar-pds
```

A failed check leaves the server stopped. Fix the problem before reopening it.
A Cargo build needs its compatible host libraries. A normal Nix package needs
its store closure and a retained garbage-collection root. The static Linux
binary needs the host's CA bundle. Keep the build type consistent unless you
have tested a change, including throughput on your workload.

## NixOS

Change the `linkjar-pds` input URL in your host flake to the full reviewed commit.
Update only that input in the host's lockfile:

```bash
nix flake update linkjar-pds
```

Review the host flake and lockfile diff, then build the host configuration
with your usual `nixos-rebuild build --flake ...` command. Take the backup and
activate the reviewed configuration with `nixos-rebuild switch --flake ...`.
Use your actual host flake target in place of `...`.

The module validates configuration before starting the service. Keep the
runtime environment file available through activation. NixOS generations
record the package; they do not snapshot the PDS data or runtime secrets.
See the [Nix input update reference](https://nix.dev/manual/nix/2.28/command-ref/new-cli/nix3-flake-update.html)
for updating a single input.

## Docker Compose

Resolve the image tag to its immutable digest:

```bash
docker buildx imagetools inspect ghcr.io/linkjar/pds-server:main --format '{{.Manifest.Digest}}'
```

Or [build it yourself](/start/install/#choose-the-image).
Set `PDS_IMAGE` in `.env` to the new digest, then:

```bash
docker compose pull
docker compose up -d
```

Compose replaces the server container and leaves Caddy running. Pin a digest;
`:main` moves with every merge and cannot identify an exact rollback build.

## Verify the update

For NixOS or a native systemd service:

```bash
sudo systemctl status linkjar-pds
sudo journalctl -u linkjar-pds --since '2 minutes ago' --no-pager
```

For Docker:

```bash
docker compose ps
docker compose logs --since 2m pds
```

From outside the host, check HTTPS and the server metadata as in
[setup verification](/start/verify/). Also test sign-in and a representative read:

```bash
curl --fail https://pds.example.com/xrpc/_health
```

On `SIGTERM`, the server stops taking connections, finishes writes in flight,
closes the firehose so the relay can resume at its cursor, and exits.
The supplied native services allow 30 seconds; Compose allows 60 seconds.

## Roll back

For a native binary, stop the service, point `/usr/local/bin/linkjar-pds` back to
the retained executable, check it, and start. For NixOS, activate the previous
reviewed host configuration or generation. For Docker, restore the old digest
in `.env` and run `docker compose up -d`.

Check schema compatibility for the selected older build before serving data.
An executable rollback does not restore the databases. The additive recovery
table and actor index remain readable by older servers, but authentication
protection can differ.

The reference server and older Candidate builds do not enforce credential
epochs or the member host's live session check. They can accept revoked,
unexpired legacy tokens after a password recovery. If recovery must remain
effective, rotate the token-signing secrets and keys and require sign-in again
before serving with such a build. Keep the account database and its recovery
table in every [backup](/operate/backup/).

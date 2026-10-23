---
title: Builds and downloads
description: Choose a native package, download a checked Linux binary, or build LinkJar PDS with Nix or Cargo.
sidebar:
  order: 4
---

`linkjar-pds` is the Rust server executable. Choose a build for your host,
then follow [native installation](/start/native/) for configuration and startup.

| Build | Platforms | Needs on the destination host |
|---|---|---|
| Nix package | `x86_64-linux`, `aarch64-linux`, `aarch64-darwin` | Nix and the package's store closure |
| Portable static executable | `x86_64-linux`, `aarch64-linux` | Linux and a CA certificate bundle; no Nix or Docker |
| Cargo release build | The build host's supported Rust target | Its runtime libraries and CA certificates |
| [Container image](/start/install/) | Linux AMD64 and ARM64 | Docker Engine and Compose |

The Linux packages are built on their own architectures. Apple Silicon supports
the normal Nix package for local use. There is no Intel macOS package or portable
macOS archive. The packages contain the PDS server; console and MCP sidecars
are separate builds. See [binaries and ports](/reference/binaries/) for their
build commands, runtime files, and network connections.

## Download a portable Linux build

Open the [Native packages workflow](https://github.com/linkjar/pds/actions/workflows/nix.yml).
Choose a successful run and check its source commit before downloading.
Use a reviewed `main` build for an installation. Pull request artifacts are
review builds and can contain unmerged changes.

The artifacts are `linkjar-pds-x86_64-linux` and `linkjar-pds-aarch64-linux`.
Each contains a `.tar.gz` archive and its `.tar.gz.sha256` checksum file.
GitHub keeps them for 14 days. They require GitHub access to download and are
not permanent GitHub Release assets. Build from a pinned revision if an artifact
has expired.

With the GitHub CLI, list successful main builds:

```bash
gh run list --repo linkjar/pds --workflow nix.yml --branch main --status success
```

Set the run ID you reviewed, then inspect its commit and result:

```bash
PDS_RUN_ID=REPLACE_WITH_REVIEWED_RUN_ID
gh run view "$PDS_RUN_ID" --repo linkjar/pds --json headSha,event,status,conclusion,url
```

Select the host architecture:

```bash
case "$(uname -m)" in
  x86_64) PDS_SYSTEM=x86_64-linux ;;
  aarch64|arm64) PDS_SYSTEM=aarch64-linux ;;
  *) echo 'No portable build for this architecture' >&2; exit 1 ;;
esac
```

On Linux, download into a new, empty directory:

```bash
gh run download "$PDS_RUN_ID" --repo linkjar/pds \
  --name "linkjar-pds-$PDS_SYSTEM" --dir native-download
```

Verify the archive before extracting it:

```bash
(cd native-download && sha256sum --check "linkjar-pds-$PDS_SYSTEM.tar.gz.sha256") &&
mkdir native-unpack &&
tar -xzf "native-download/linkjar-pds-$PDS_SYSTEM.tar.gz" -C native-unpack &&
./native-unpack/linkjar-pds version
```

A checksum detects damaged or changed downloads. Also check the run's repository
and source commit; the checksum alone does not establish who built it. The CLI
version is the Cargo package version and can be shared by several commits.
Keep the full commit and archive checksum in your deployment record.

Install the reviewed executable in a directory named after its commit:

```bash
PDS_REV=$(gh run view "$PDS_RUN_ID" --repo linkjar/pds --json headSha --jq '.headSha') &&
sudo install -d -m 0755 "/opt/linkjar-pds/$PDS_REV" &&
sudo install -m 0755 native-unpack/linkjar-pds "/opt/linkjar-pds/$PDS_REV/linkjar-pds" &&
sudo install -m 0644 native-unpack/LICENSE "/opt/linkjar-pds/$PDS_REV/LICENSE" &&
sudo ln -sfn "/opt/linkjar-pds/$PDS_REV/linkjar-pds" /usr/local/bin/linkjar-pds
```

This symlink is for a new installation. For an existing server, follow
[the update procedure](/operate/update/#native-binary) before changing it.
Keep CA certificates installed for outbound HTTPS. Set `SSL_CERT_FILE` in the
service environment if the bundle is in a nonstandard location.

## Build with Nix

Use Nix with the `nix-command` and `flakes` experimental features enabled.
Check out the reviewed source commit and run these commands from the repository
root. Keep `flake.lock` and `Cargo.lock` unchanged for that build.

```bash
nix build .#linkjar-pds
result/bin/linkjar-pds version
nix run . -- version
```

The normal package uses libraries in the Nix store. Keep the `result` symlink
as a garbage-collection root, or install the package through your host's Nix
configuration. Copying just this executable to another host is insufficient.
Use the NixOS module for a persistent NixOS installation.

On a Linux builder, create a portable static musl executable instead:

```bash
nix build .#linkjar-pds-static --out-link result-static
result-static/bin/linkjar-pds version
```

Copy `result-static/bin/linkjar-pds` to a Linux host with the same CPU architecture.
The static executable has no dynamic loader or shared-library dependency. Its
throughput has not been benchmarked against the normal package; compare your
workload before changing a production build.

`flake.lock` pins Nixpkgs and rust-overlay. The overlay resolves the toolchain
in `rust-toolchain.toml` against those locked inputs. `Cargo.lock` pins the Rust
dependencies. A normal build does not update either lockfile.

## Build with Cargo

Install the toolchain selected by `rust-toolchain.toml`, a C compiler, CMake,
and pkg-config. Build only the server:

```bash
cargo build --release --locked --package linkjar-pds --bin linkjar-pds
target/release/linkjar-pds version
```

The output is `target/release/linkjar-pds`. It targets the build host and can
depend on its shared libraries. Node, pnpm, the console assets, and the simulation
are not required to compile this server target.

For a development shell with the workspace tools:

```bash
nix develop
```

## What CI checks

The **Native packages** workflow evaluates all supported platforms, then builds
the normal and static packages on both Linux architectures. It checks the copied
static executable for dynamic dependencies and tests configuration, startup,
HTTP health, graceful shutdown, and database verification on temporary data.
An x86-64 NixOS VM test checks the service user, state permissions, persistence,
restart, and firewall isolation.

The separate **Rust** workflow runs the workspace tests and Rust checks.
The **Site** workflow validates documentation and saves the static `pds-docs`
artifact. These workflows do not deploy the server or documentation. The docs
are hosted on Cloudflare Workers at [pds.linkjar.io](https://pds.linkjar.io/);
GitHub Pages is not used.

Next: [configure and run the native server](/start/native/).

# Native packages

The server is already a Rust executable. These packages build it without
Docker. The source and Cargo dependencies are separate: the source includes
the workspace manifests, crates, lexicons, and xtask; the dependency fetches
use the checksums in `Cargo.lock`.

```sh
nix build .#linkjar-pds
nix run . -- version
nix develop
```

The default package supports `x86_64-linux`, `aarch64-linux`, and
`aarch64-darwin`. `flake.lock` pins Nixpkgs and rust-overlay. The overlay
resolves `rust-toolchain.toml` against that pinned revision, so the `stable`
channel does not change between lockfile updates.

The normal package depends on its Nix store closure. On Linux, build the
portable musl executable with:

```sh
nix build .#linkjar-pds-static
```

Copy `result/bin/linkjar-pds` to a host with the same architecture. Keep
the host's CA certificate package installed. This binary has no dynamic
loader or shared-library dependency; the CI checks both ELF properties,
copies it out of the store, and runs it on temporary data.

## Service

Import `nixosModules.default` into a NixOS host, then enable
`services.linkjar-pds`. The module uses a dedicated user, mode `0700`
state at `/var/lib/linkjar-pds`, configuration validation before startup,
and systemd filesystem restrictions. The HTTP port is `2583`; it is not
opened in the firewall by default. Supply a TLS proxy or tunnel separately.

Put secrets in the `environmentFile` runtime path, outside the Nix store.
Use `environment` only for public settings. Choose a disk blob location
under the service state directory or configure S3. The module deliberately
does not choose between these two storage backends. Keep `PDS_PORT` and
`PDS_DATA_DIRECTORY` out of the runtime file; the module manages them.

The [installation guide](../site/src/content/docs/start/native.md) has
the input, module, and secret-file examples. An existing Docker deployment
needs a separate cutover: preserve all data and secrets, check paths and
ownership, and run one server against a directory at a time.

## Checks and artifacts

```sh
nix flake check --all-systems --no-build
nix build .#linkjar-pds
python3 nix/tests/native-smoke.py result/bin/linkjar-pds
nix build .#checks.x86_64-linux.nixos-service --no-link --print-build-logs
```

The installed-package checks run `version`, `help`, and `check-config`.
The native smoke test starts the executable with disposable data, checks
HTTP health, stops it gracefully, and verifies its databases. The NixOS VM
test checks service startup, state-directory permissions and persistence,
restart, and denial of connections from another machine by the firewall.
The VM test needs an x86-64 Linux builder with KVM.

The package does not repeat `cargo test`. That suite builds the console,
requires Tailwind, and uses loopback sockets and a Node client. The existing
Rust workflow runs it independently. A production package builds only the
server executable and does not compile the console or simulation.

The **Native packages** workflow builds both Linux architectures, runs
these packaging checks, and uploads static `.tar.gz` archives with SHA-256
checksums for 14 days. It publishes no release and deploys no service.
The x86-64 job also runs the NixOS VM test.

Format Nix sources with `nix fmt`. Update inputs with `nix flake update`,
then rerun the checks and review the lockfile changes.

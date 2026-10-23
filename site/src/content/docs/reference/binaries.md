---
title: Binaries and ports
description: The PDS, operations console, and MCP server, their build targets, and the network connections between them.
---

Start with `linkjar-pds`. Add the console for operators or the MCP server for
AI clients when you need them. Each is a separate Rust executable.

## What runs where

| Executable | Purpose | Distribution |
|---|---|---|
| `linkjar-pds` | Hosts accounts, repositories, blobs, OAuth, and the optional private operations API | Nix package, portable Linux archive, Cargo, or Docker |
| `linkjar-pds-console` | Gives operators a browser interface to the private operations API | Cargo with its asset bundle, or `Dockerfile.console` |
| `linkjar-pds-mcp` | Connects an AI client to an account through MCP and OAuth | Cargo, or `Dockerfile.mcp` |

The Nix package and portable archives currently include **only `linkjar-pds`**.
There are no published native sidecar archives or sidecar NixOS modules in this
repository. The `pds-sidecar` crate is a shared library, not another executable.

The console and MCP server use authenticated APIs. They do not open the PDS
database or need its admin password or PLC rotation key. The MCP server has
its own OAuth client key.

## Network map

| Service | Native address or port | Who connects |
|---|---|---|
| TLS proxy | Public `443`; usually `80` for HTTPS redirects and certificate setup | Apps, browsers, relays, and remote AI clients |
| PDS HTTP | `2583`, on all interfaces; `PDS_PORT` changes it | The proxy and trusted sidecars; block public access with the host firewall |
| PDS operations API | Disabled unless `PDS_OPS_ADDRESS` is set; use `127.0.0.1:2584` on one host | The console and authorized operator tools |
| Operations console | `127.0.0.1:2585`; set with `CONSOLE_ADDRESS` | Your browser through an SSH tunnel |
| MCP HTTP | `127.0.0.1:2586`; set with `MCP_ADDRESS` | A TLS proxy for the MCP hostname |

Port `2584` is a convention in the examples, not an automatically enabled
listener. Keep it private. The console's browser URL must be an HTTP loopback
address; a public console hostname is not supported.

The PDS image sets its internal HTTP port to `3000`. Compose sidecars use
`http://pds:3000`; sidecars beside a native installation use
`http://127.0.0.1:2583`. Inside a container, `127.0.0.1` is that container,
not the host or a sibling service. Use the [Compose examples](/scale/sidecars/)
for that topology.

## Platforms and runtime files

| Build | Supported package targets | Files to retain |
|---|---|---|
| Normal PDS Nix package | Linux AMD64, Linux ARM64, Apple Silicon macOS | The Nix store closure |
| Portable PDS archive | Linux AMD64 and Linux ARM64 | The executable, license, deployment commit, and checksum |
| Sidecars built with Cargo | Build for the intended host; no portable sidecar release matrix yet | Executable and required runtime libraries; the console also needs its `assets/` directory |

Native services need CA certificates for outbound HTTPS. The MCP and console
keep sessions in memory, so restarting either requires its users to sign in
again. The PDS data directory must persist independently of executable updates.

## Build the sidecars

Use a reviewed checkout and the toolchain in `rust-toolchain.toml`. The
[build prerequisites](/start/builds/#build-with-cargo) apply here too.

```bash title="Build the MCP server"
cargo build --release --locked --package pds-mcp --bin linkjar-pds-mcp
```

```bash title="Build and bundle the operations console"
cargo build --release --locked --package pds-console --bin linkjar-pds-console &&
target/release/linkjar-pds-console bundle-assets target/release/assets
```

Bundle the console assets **on the build machine**, before copying the
executable elsewhere. Copy `target/release/assets/` alongside the executable
as `assets/`. Without that bundle, the console attempts to build its assets
at runtime using files from the source tree.

The console build downloads a pinned, checksum-verified Tailwind CLI. To use
an existing compatible executable, set `TAILWIND_CLI` before building.

## Choose the next guide

- [Install the native PDS](/start/native/) or [use Docker Compose](/start/install/).
- [Connect an AI client with MCP](/connect/mcp/).
- [Set up the operations console](/connect/console/).
- [Use the PDS operator commands](/operate/cli/).

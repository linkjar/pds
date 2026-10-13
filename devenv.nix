{ pkgs, config, ... }:

{
  # LinkJar PDS development environment. See docs/SPEC.md section 14.5.
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  # The parity harness (parity/) is a TypeScript project driven by the
  # official SDK; Node matches the monorepo's pin.
  languages.javascript = {
    enable = true;
    package = pkgs.nodejs_24;
    pnpm.enable = true;
  };

  packages = with pkgs; [
    cargo-deny
    cargo-vet
    cargo-audit
    cargo-fuzz
    sqlite
    jq
    python3
  ];

  # Postgres is only needed for the shared-tier backend tests (unit 11).
  services.postgres = {
    enable = false;
    initialDatabases = [{ name = "pds_test"; }];
  };

  # The hooks are defined here and installed nowhere. Linked worktrees share
  # one .git/hooks, so an installed hook points at the worktree that entered
  # the shell last, runs for all of them, and breaks when that worktree is
  # removed. With the stage "manual" devenv installs no hook; run them with
  # `pds:hooks`. CI holds the same gates.
  git-hooks.default_stages = [ "manual" ];
  git-hooks.hooks = {
    rustfmt.enable = true;
    clippy = {
      enable = true;
      settings.allFeatures = true;
      settings.denyWarnings = true;
    };
  };

  scripts = {
    "pds:check".exec = ''
      set -euo pipefail
      cargo fmt --all -- --check
      cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
      cargo test --workspace --locked
      cargo deny --all-features --locked check
      cargo vet --locked
      cargo xtask codegen --check
    '';
    "pds:codegen".exec = "cargo xtask codegen";
    # The git hooks, run by hand over every file.
    "pds:hooks".exec = ''prek run --all-files --hook-stage manual --config "$DEVENV_ROOT/.pre-commit-config.yaml" "$@"'';
    # The parity harness (SPEC section 17). `parity:run reference` needs Docker.
    "parity:check".exec = ''
      set -euo pipefail
      cd parity
      pnpm install --frozen-lockfile
      pnpm typecheck
      pnpm test
    '';
    "parity:run".exec = ''cd parity && pnpm install --frozen-lockfile --silent && pnpm parity run "$@"'';
    "parity:compare".exec = ''cd parity && pnpm parity compare "$@"'';
    "docs:diagrams".exec = "cd docs/architecture && pnpm install --silent && pnpm render";
    # Upstream sources for reading, at the commits this repository pins.
    "sources:vendor".exec = ''scripts/vendor-agent-sources.sh "$@"'';
  };

  # `sqlite-plugin`, which the simulation uses, generates its bindings
  # with bindgen at build time.
  env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

  enterShell = ''
    echo "LinkJar PDS: $(rustc --version)"
  '';
}

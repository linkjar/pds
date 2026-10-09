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
    cargo-nextest
    bacon
    cargo-shear
    sqlite
    jq
    python3
  ];

  # Postgres is only needed for the shared-tier backend tests (unit 11).
  services.postgres = {
    enable = false;
    initialDatabases = [{ name = "pds_test"; }];
  };

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
      cargo deny --locked check
      cargo xtask codegen --check
    '';
    "pds:codegen".exec = "cargo xtask codegen";
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
  };

  enterShell = ''
    echo "LinkJar PDS: $(rustc --version)"
  '';
}

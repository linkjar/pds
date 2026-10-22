{
  description = "LinkJar PDS: a native AT Protocol Personal Data Server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
      toolchainFor = pkgs: pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          toolchain = toolchainFor pkgs;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
          server = pkgs.callPackage ./nix/package.nix { inherit rustPlatform; };
          staticToolchain = toolchain.override {
            targets = [ pkgs.pkgsStatic.stdenv.hostPlatform.rust.rustcTarget ];
          };
        in
        {
          default = server;
          linkjar-pds = server;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          linkjar-pds-static = pkgs.pkgsStatic.callPackage ./nix/package.nix {
            rustPlatform = pkgs.pkgsStatic.makeRustPlatform {
              cargo = staticToolchain;
              rustc = staticToolchain;
            };
            inherit (pkgs) cmake pkg-config;
          };
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.linkjar-pds}/bin/linkjar-pds";
          meta.description = "Run the LinkJar PDS command line";
        };
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              (toolchainFor pkgs)
              clang
              cmake
              pkg-config
              libclang
              nodejs_24
              pnpm
              sqlite
              jq
              cargo-deny
              cargo-vet
            ];
            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };
        }
      );

      nixosModules.default = import ./nix/module.nix self;
      nixosModules.linkjar-pds = self.nixosModules.default;

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          package = self.packages.${system}.linkjar-pds;
        }
        // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          nixos-service = pkgs.callPackage ./nix/tests/service.nix {
            module = self.nixosModules.default;
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);
    };
}

{
  lib,
  rustPlatform,
  cmake,
  pkg-config,
}:

rustPlatform.buildRustPackage {
  pname = "linkjar-pds";
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../lexicons
      ../xtask
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "--package"
    "linkjar-pds"
    "--bin"
    "linkjar-pds"
  ];
  nativeBuildInputs = [
    cmake
    pkg-config
  ];

  # The full suite builds the console and runs socket/Node integration tests.
  # CI runs it separately; this derivation checks the installed executable.
  doCheck = false;
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    "$out/bin/linkjar-pds" version
    "$out/bin/linkjar-pds" help
    env -i PDS_JWT_SECRET=nix-build-test PDS_ADMIN_PASSWORD=nix-build-test \
      "$out/bin/linkjar-pds" check-config
    runHook postInstallCheck
  '';

  meta = {
    description = "An AT Protocol Personal Data Server written in Rust";
    homepage = "https://pds.linkjar.io";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "linkjar-pds";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}

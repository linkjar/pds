{
  description = "LinkJar infrastructure host; PDS is the first service";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    disko = { url = "github:nix-community/disko"; inputs.nixpkgs.follows = "nixpkgs"; };
    nixos-anywhere = {
      url = "github:nix-community/nixos-anywhere";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.disko.follows = "disko";
    };
  };
  outputs = { nixpkgs, disko, ... }: {
    nixosConfigurations.linkjar = nixpkgs.lib.nixosSystem {
      system = "aarch64-linux";
      modules = [ disko.nixosModules.disko ./nixos/configuration.nix ];
    };
  };
}

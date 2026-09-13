# Public configuration only. No credentials or recovery private keys in Nix.
{
  adminSshKeys = [ "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKSVHoHlpj+Zs82ktXYDXYCv5vV0tvEgrLRRLbD084Fu linkjar infrastructure operator" ];
  enablePds = false; # Enable only after the launch prerequisites in nixos/README.md.
}

# Public configuration only. No credentials or recovery private keys in Nix.
{
  adminSshKeys = [ ]; # Add the dedicated operator public key before installation.
  enablePds = false; # Enable only after the launch prerequisites in nixos/README.md.
}

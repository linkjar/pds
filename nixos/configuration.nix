{ lib, pkgs, modulesPath, ... }:
let
  host = import ./host.nix;
  image = "ghcr.io/linkjar/pds@sha256:65439dafd5c431e86400a877cc33d91a932ec931aef52223dc44972535080d07";
  handles = builtins.replaceStrings [ "{$PDS_UPSTREAM:pds:3000}" ] [ "127.0.0.1:3000" ]
    (builtins.readFile ../staging/Caddyfile.handles);
in {
  imports = [ (modulesPath + "/profiles/qemu-guest.nix") ./disko.nix ];
  networking.hostName = "linkjar-pds";
  networking.useDHCP = lib.mkDefault true;
  networking.firewall.allowedTCPPorts = [ 22 ] ++ lib.optionals host.enablePds [ 443 ];
  system.stateVersion = "26.05";
  time.timeZone = "UTC";
  boot.initrd.kernelModules = [ "virtio_gpu" ];
  boot.kernelParams = [ "console=tty" "console=ttyAMA0,115200" ];
  boot.loader.grub = { enable = true; efiSupport = true; efiInstallAsRemovable = true; device = "nodev"; configurationLimit = 5; };
  boot.loader.efi = { canTouchEfiVariables = false; efiSysMountPoint = "/boot/efi"; };
  zramSwap = { enable = true; memoryPercent = 50; };
  services.qemuGuest.enable = true;
  services.openssh = {
    enable = true;
    settings = { PasswordAuthentication = false; KbdInteractiveAuthentication = false; PermitRootLogin = "no"; };
  };
  users.users.operator = { isNormalUser = true; extraGroups = [ "wheel" ]; openssh.authorizedKeys.keys = host.adminSshKeys; };
  security.sudo.wheelNeedsPassword = false;
  assertions = [{ assertion = !host.enablePds || host.adminSshKeys != [ ]; message = "Set dedicated PDS operator public keys before enabling the service."; }];
  environment.systemPackages = with pkgs; [ curl jq git ];
  virtualisation.docker.enable = true;
  virtualisation.oci-containers = {
    backend = "docker";
    containers = lib.optionalAttrs host.enablePds {
      linkjar-pds = {
        inherit image;
        environmentFiles = [ "/etc/linkjar-pds/pds.env" ];
        volumes = [ "/var/lib/linkjar-pds:/app/data" ];
        ports = [ "127.0.0.1:3000:3000" ];
        extraOptions = [ "--stop-timeout=60" ];
      };
    };
  };
  systemd.tmpfiles.rules = [
    "d /etc/linkjar-pds 0700 root root -"
    "d /var/lib/linkjar-pds 0700 1000 1000 -"
  ];
  services.caddy = lib.mkIf host.enablePds {
    enable = true;
    extraConfig = handles + ''
      https://pds.linkjar.social, https://*.linkjar.social {
        tls /var/lib/caddy/linkjar-origin.pem /var/lib/caddy/linkjar-origin-key.pem
        import linkjar_handle_hosts
      }
    '';
  };
  # Use Cloudflare Full (strict) with an origin certificate covering both names.
  # The existing recovery scripts/runbook remain required before public signup.
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.gc = { automatic = true; dates = "weekly"; options = "--delete-older-than 30d"; };
}

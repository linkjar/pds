flake:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.linkjar-pds;
in
{
  options.services.linkjar-pds = {
    enable = lib.mkEnableOption "the native LinkJar PDS service";
    package = lib.mkOption {
      type = lib.types.package;
      default = flake.packages.${pkgs.stdenv.hostPlatform.system}.linkjar-pds;
      defaultText = lib.literalExpression "inputs.linkjar-pds.packages.\${system}.linkjar-pds";
      description = "The LinkJar PDS package to run.";
    };
    environmentFile = lib.mkOption {
      type = lib.types.str;
      example = "/run/secrets/linkjar-pds.env";
      description = ''
        Absolute path to a runtime environment file containing the secrets and
        PDS settings. Keep this file outside the Nix store and restrict its
        permissions. Do not use builtins.toFile or a Nix path literal for it.
      '';
    };
    environment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        PDS_HOSTNAME = "pds.example.com";
      };
      description = "Non-secret settings. Put passwords, tokens, and keys in environmentFile.";
    };
    port = lib.mkOption {
      type = lib.types.port;
      default = 2583;
      description = "The HTTP listener port. Use a TLS proxy or tunnel for public traffic.";
    };
    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether to allow incoming connections to the HTTP listener.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion =
          lib.hasPrefix "/" cfg.environmentFile && !(lib.hasPrefix builtins.storeDir cfg.environmentFile);
        message = "services.linkjar-pds.environmentFile must be an absolute runtime path outside the Nix store.";
      }
    ];

    users.groups.linkjar-pds = { };
    users.users.linkjar-pds = {
      isSystemUser = true;
      group = "linkjar-pds";
      home = "/var/lib/linkjar-pds";
    };

    systemd.services.linkjar-pds = {
      description = "LinkJar AT Protocol Personal Data Server";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      environment = cfg.environment // {
        PDS_DATA_DIRECTORY = "/var/lib/linkjar-pds";
        PDS_PORT = toString cfg.port;
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      };
      serviceConfig = {
        Type = "simple";
        User = "linkjar-pds";
        Group = "linkjar-pds";
        EnvironmentFile = cfg.environmentFile;
        ExecStartPre = "${lib.getExe cfg.package} check-config";
        ExecStart = "${lib.getExe cfg.package} serve";
        Restart = "on-failure";
        RestartSec = 5;
        TimeoutStopSec = 30;
        StateDirectory = "linkjar-pds";
        StateDirectoryMode = "0700";
        WorkingDirectory = "/var/lib/linkjar-pds";
        UMask = "0077";
        NoNewPrivileges = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictSUIDSGID = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        CapabilityBoundingSet = "";
      };
    };

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
  };
}

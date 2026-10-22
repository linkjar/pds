{ testers, module }:

testers.runNixOSTest {
  name = "linkjar-pds-native-service";
  nodes = {
    server = { pkgs, ... }: {
      imports = [ module ];
      services.linkjar-pds = {
        enable = true;
        environmentFile = "/etc/pds-test.env";
      };
      # Disposable test credentials; never copy real secrets into a derivation.
      environment.etc."pds-test.env" = {
        mode = "0600";
        text = ''
          PDS_HOSTNAME=localhost
          PDS_JWT_SECRET=nixos-vm-test-only
          PDS_ADMIN_PASSWORD=nixos-vm-test-only
          PDS_BLOBSTORE_DISK_LOCATION=/var/lib/linkjar-pds/blocks
        '';
      };
      environment.systemPackages = [ pkgs.curl ];
    };
    client = { pkgs, ... }: {
      environment.systemPackages = [ pkgs.curl ];
    };
  };
  testScript = ''
    start_all()
    server.wait_for_unit("linkjar-pds.service")
    server.wait_for_open_port(2583)
    server.succeed("curl --fail http://127.0.0.1:2583/xrpc/_health")
    client.fail("curl --fail --connect-timeout 3 http://server:2583/xrpc/_health")
    server.succeed("test $(stat -c %U /var/lib/linkjar-pds) = linkjar-pds")
    server.succeed("test $(stat -c %a /var/lib/linkjar-pds) = 700")
    server.succeed("runuser -u linkjar-pds -- touch /var/lib/linkjar-pds/retained")
    server.succeed("systemctl restart linkjar-pds")
    server.wait_for_unit("linkjar-pds.service")
    server.wait_for_open_port(2583)
    server.succeed("curl --fail http://127.0.0.1:2583/xrpc/_health")
    server.succeed("test -f /var/lib/linkjar-pds/retained")
    server.succeed("test -f /var/lib/linkjar-pds/account.sqlite")
  '';
}

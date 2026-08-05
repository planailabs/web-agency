{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.web-agency-server;
  settingsFormat = pkgs.formats.toml { };
  configFile = settingsFormat.generate "config.toml" cfg.settings;
  envFiles = lib.filter (f: f != null) [ cfg.environmentFile cfg.otlpHeadersFile ];
in
{
  imports = [
    ../proxy/module.nix
  ];
  options.services.web-agency-server = {
    enable = lib.mkEnableOption "web-agency server";

    package = lib.mkPackageOption pkgs "web-agency-server" { };

    settings = lib.mkOption {
      type = settingsFormat.type;
      default = { };
      description = ''
        Configuration for web-agency-server in Nix attribute set form.
        Rendered to config.toml.
      '';
      example = lib.literalExpression ''
        {
          database.url = "postgres://localhost/web_agency";
          web.port = 7380;
          auth = {
            client_id = "xxx.apps.googleusercontent.com";
            client_secret = "GOCSPX-xxx";
            redirect_uri = "https://agency.example.com/auth/callback";
            allowed_domains = [ "example.com" ];
            cookie_secret = "generate-with-openssl-rand-hex-32";
          };
          secrets.encryption_key = "base64-encoded-32-byte-key";
        }
      '';
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        File containing environment variables (e.g. secrets) for the service.
        Lines should be KEY=VALUE. Useful for passing secrets without
        putting them in the Nix store.
      '';
    };

    otlpEndpoint = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "http://localhost:4318";
      description = ''
        OTLP/HTTP collector base URL for traces. Unset means no trace export;
        metrics are collected either way and scraped from /api/metrics.
        Other OTEL_* variables (sampling, resource attributes) can be passed
        through {option}`environmentFile`.
      '';
    };

    otlpHeaders = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        "X-Scope-OrgID" = "web-agency";
      };
      description = ''
        Headers sent with every OTLP export, as OTEL_EXPORTER_OTLP_HEADERS.

        ::: {.warning}
        These values land in the world-readable Nix store. Put API keys and
        other credentials in {option}`otlpHeadersFile` instead.
        :::
      '';
    };

    otlpHeadersFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = "/run/secrets/otlp-headers.env";
      description = ''
        File holding the OTLP export headers, kept out of the Nix store.
        A single line of `OTEL_EXPORTER_OTLP_HEADERS=key=value,key2=value2`
        (comma-separated, as the OpenTelemetry spec defines it) — typically
        the collector's auth header. Overrides {option}`otlpHeaders`.
      '';
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether to open the web port in the firewall.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.web-agency-server = {
      description = "web-agency server";
      after = [ "network.target" "postgresql.service" ];
      wants = [ "network.target" ];
      wantedBy = [ "multi-user.target" ];

      environment = {
        CONFIG_PATH = configFile;
      } // lib.optionalAttrs (cfg.otlpEndpoint != null) {
        OTEL_EXPORTER_OTLP_ENDPOINT = cfg.otlpEndpoint;
      } // lib.optionalAttrs (cfg.otlpHeaders != { }) {
        OTEL_EXPORTER_OTLP_HEADERS = lib.concatStringsSep "," (
          lib.mapAttrsToList (name: value: "${name}=${value}") cfg.otlpHeaders
        );
      };

      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        Restart = "on-failure";
        RestartSec = 5;

        DynamicUser = true;
        # Pin the primary group to the static shared group so:
        #  - the unit name never collides with the "web-agency" group
        #    that DynamicUser would otherwise auto-allocate, and
        #  - StateDirectory (/var/lib/web-agency-server, mode 0750) is
        #    group-readable by the proxy, which joins the same group via
        #    SupplementaryGroups.
        Group = "web-agency";
        StateDirectory = "web-agency-server";
        StateDirectoryMode = "0750";

        # Hardening
        CapabilityBoundingSet = "";
        LockPersonality = true;
        # V8 (Node.js / wrangler) needs JIT — cannot deny W^X.
        MemoryDenyWriteExecute = false;
        NoNewPrivileges = true;
        PrivateDevices = true;
        PrivateTmp = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        SystemCallArchitectures = "native";
      } // lib.optionalAttrs (envFiles != [ ]) {
        # Read after Environment=, so a secret here wins over the store-visible
        # otlpHeaders.
        EnvironmentFile = envFiles;
      };
    };

    # Shared group so the proxy can read the server's internal token file.
    users.groups.web-agency = { };

    services.web-agency-server.settings = {
      # Peer auth: the server connects as its OS user (the dynamic user
      # for the systemd.services.web-agency-server unit, named
      # "web-agency-server"), so the postgres role must match.
      database.url = "postgres:///web-agency?host=/run/postgresql";
      proxy.internal_token_path = lib.mkDefault "/var/lib/web-agency-server/internal.token";
    };

    # Pre-configure proxy from the server's settings so values aren't duplicated.
    services.web-agency-proxy.settings.proxy = lib.mkIf (cfg.settings ? proxy) (
      let
        p = cfg.settings.proxy;
        port = cfg.settings.web.port or 7380;
      in
      {
        agency_domain = lib.mkIf (p ? agency_domain) (lib.mkDefault p.agency_domain);
        agency_upstream = lib.mkDefault "127.0.0.1:${toString port}";
        server_url = lib.mkDefault "http://127.0.0.1:${toString port}";
        acme_email = lib.mkIf (p ? acme_email) (lib.mkDefault p.acme_email);
        http_addr = lib.mkIf (p ? http_addr) (lib.mkDefault p.http_addr);
        https_addr = lib.mkIf (p ? https_addr) (lib.mkDefault p.https_addr);
      }
    );

    # Where the server writes the shared token; the proxy picks it up via
    # LoadCredential (its own settings.proxy.internal_token_path points at
    # the credentials directory, not this file).
    services.web-agency-proxy.internalTokenFile = lib.mkIf (cfg.settings ? proxy) (
      lib.mkDefault (cfg.settings.proxy.internal_token_path or "/var/lib/web-agency-server/internal.token")
    );

    systemd.services.web-agency-proxy = lib.mkIf (cfg.settings ? proxy) {
      after = [ "web-agency-server.service" ];
      requires = [ "web-agency-server.service" ];
    };

    services.postgresql = {
      enable = true;

      ensureUsers = [{
        name = "web-agency-server";
        ensureClauses.superuser = true;
      }];

      ensureDatabases = [ "web-agency" ];
    };

    networking.firewall = lib.mkIf cfg.openFirewall {
      allowedTCPPorts = [ (cfg.settings.web.port or 7380) ];
    };
  };
}

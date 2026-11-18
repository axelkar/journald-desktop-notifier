{
  description = "journald-desktop-notifier devel and build";

  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.zst";

  # shell.nix compatibility
  inputs.flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.tar.gz";

  outputs =
    { self, nixpkgs, ... }:
    let
      sharedModuleCode =
        {
          cfg,
          lib,
          pkgs,
        }:
        let
          settingsFormat = pkgs.formats.json { };
        in
        {
          configFile = settingsFormat.generate "journald-desktop-notifier.json" cfg.settings;

          options.services.journald-desktop-notifier = {
            enable = lib.mkEnableOption "a system journal error notifier";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalMD "`journald-desktop-notifier` from the flake defining this module";
              description = ''
                Package to use.
              '';
            };
            systemdTarget = lib.mkOption {
              type = lib.types.str;
              default = "graphical-session.target";
              example = "sway-session.target";
              description = ''
                Systemd target to bind to.
              '';
            };
            settings = lib.mkOption {
              type = lib.types.submodule {
                freeformType = settingsFormat.type;
              };
              example = lib.literalExpression ''
                {
                  match = [{
                    PRIORITY = "^0|1|2|3$";
                    ignore = [{
                      SYSLOG_IDENTIFIER = "^systemd-coredump$";
                      MESSAGE = "user 30001"; # nixbld1
                    }];
                  }];
                }
              '';
              description = ''
                Configuration for journald-desktop-notifier.
              '';
            };
          };
        };
    in
    {
      packages = builtins.mapAttrs (system: pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "journald-desktop-notifier";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = with pkgs; [
            # TODO: remove override when https://github.com/NixOS/nixpkgs/pull/571072 reaches a stable Nixpkgs version like 26.11
            (systemdLibs.override { withCompression = true; })
          ];

          meta = with nixpkgs.lib; {
            description = "System journal error notifier";
            homepage = "https://github.com/axelkar/journald-desktop-notifier";
            license = with licenses; [
              mit
              asl20
            ];
          };
        };
      }) nixpkgs.legacyPackages;
      checks = builtins.mapAttrs (system: pkgs: {
        vm-test = pkgs.testers.runNixOSTest (
          import ./nix/vm-test.nix self.nixosModules.default
        );
      }) (
        nixpkgs.lib.filterAttrs (system: _: nixpkgs.lib.hasSuffix "-linux" system) nixpkgs.legacyPackages
      );
      devShells = builtins.mapAttrs (system: pkgs: {
        default = pkgs.mkShell {
          strictDeps = true;
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          RUSTFLAGS = "-C link-arg=-fuse-ld=lld";
          nativeBuildInputs = with pkgs; [
            cargo
            rustc
            llvmPackages.bintools # LLD

            rustfmt
            clippy
            rust-analyzer
          ];

          inputsFrom = [ self.packages.${system}.default ];
        };
      }) nixpkgs.legacyPackages;
      nixosModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.services.journald-desktop-notifier;
          inherit (sharedModuleCode { inherit cfg lib pkgs; }) options configFile;
        in
        {
          inherit options;

          config = lib.mkIf cfg.enable {
            systemd.user.services.journald-desktop-notifier = {
              description = "System journal error notifier";
              partOf = [ cfg.systemdTarget ];
              after = [ cfg.systemdTarget ]; # Make sure a notification daemon is running

              serviceConfig = {
                Restart = "on-failure";
                ExecStart = "${cfg.package}/bin/journald-desktop-notifier ${configFile}";
              };

              wantedBy = [ cfg.systemdTarget ];
            };
          };
        };
      homeModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.services.journald-desktop-notifier;
          inherit (sharedModuleCode { inherit cfg lib pkgs; }) options configFile;
        in
        {
          inherit options;

          config = lib.mkIf cfg.enable {
            systemd.user.services.journald-desktop-notifier = {
              Unit = {
                Description = "System journal error notifier";
                PartOf = [ cfg.systemdTarget ];
                After = [ cfg.systemdTarget ]; # Make sure a notification daemon is running
              };

              Service = {
                Restart = "on-failure";
                ExecStart = "${cfg.package}/bin/journald-desktop-notifier ${configFile}";
              };

              Install.WantedBy = [ cfg.systemdTarget ];
            };
          };
        };
    };
}

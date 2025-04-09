{
  description = "journald-desktop-notifier devel and build";

  # Unstable required until Rust 1.85 (2024 edition) is on stable
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # shell.nix compatibility
  inputs.flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.tar.gz";

  outputs = { self, nixpkgs, ... }:
    let
      # System types to support.
      targetSystems = [ "x86_64-linux" "aarch64-linux" ];

      # Helper function to generate an attrset '{ x86_64-linux = f "x86_64-linux"; ... }'.
      forAllSystems = nixpkgs.lib.genAttrs targetSystems;

      sharedModuleCode = { cfg, lib, pkgs }: let
        settingsFormat = pkgs.formats.json {};
      in {
        configFile = settingsFormat.generate "journald-desktop-notifier.json" cfg.settings;

        options.services.journald-desktop-notifier = {
          enable = lib.mkEnableOption "a system journal error notifier";
          package = lib.mkOption {
            type = lib.types.package;
            default = self.packages.${pkgs.system}.default;
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
                  __allow = [{
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
    in {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage rec {
            pname = "journald-desktop-notifier";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

            src = ./.;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgs; [
              pkg-config
            ];

            buildInputs = with pkgs; [
              systemd
            ];

            meta = with nixpkgs.lib; {
              description = "System journal error notifier";
              homepage = "https://github.com/axelkar/journald-desktop-notifier";
            };
          };
        }
      );
      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            strictDeps = true;
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            RUSTFLAGS = "-C link-arg=-fuse-ld=lld";
            nativeBuildInputs = with pkgs; [
              cargo
              rustc
              llvmPackages.bintools # LLD
              pkg-config

              rustfmt
              clippy
              rust-analyzer
            ];

            inherit (self.packages.${system}.default) buildInputs;
          };
        }
      );
      nixosModules.default = { config, lib, pkgs, ... }:
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
      homeModules.default = { config, lib, pkgs, ... }:
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

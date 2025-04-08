{
  description = "journald-desktop-notifier devel and build";

  # Unstable required until Rust 1.85 (2024 edition) is on stable
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  # shell.nix compatibility
  inputs.flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.tar.gz";

  outputs = { self, nixpkgs, ... }:
    let
      # System types to support.
      targetSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];

      # Helper function to generate an attrset '{ x86_64-linux = f "x86_64-linux"; ... }'.
      forAllSystems = nixpkgs.lib.genAttrs targetSystems;
    in {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage rec {
            pname = "journald-desktop-notifier";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

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

              cargo-release
            ];

            inherit (self.packages.${system}.default) buildInputs;
          };
        }
      );
    };
}

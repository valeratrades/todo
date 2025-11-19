{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    pre-commit-hooks.url = "github:cachix/git-hooks.nix";
    v-utils.url = "github:valeratrades/.github";
  };
  outputs = { self, nixpkgs, rust-overlay, flake-utils, pre-commit-hooks, v-utils }:
    flake-utils.lib.eachDefaultSystem
      (
        system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
            allowUnfree = true;
          };
          #NB: can't load rust-bin from nightly.latest, as there are week guarantees of which components will be available on each day.
          rust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
            extensions = [ "rust-src" "rust-analyzer" "rust-docs" "rustc-codegen-cranelift-preview" ];
          });
          pre-commit-check = pre-commit-hooks.lib.${system}.run (v-utils.files.preCommit { inherit pkgs; });
          manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
          pname = manifest.name;
          stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;

          workflowContents = v-utils.ci {
            inherit pkgs;
            lastSupportedVersion = "nightly-2025-08-01";
            jobsErrors = [ "rust-tests" "rust-miri" ];
            jobsWarnings = [ "rust-doc" "rust-clippy" "rust-machete" "rust-sorted" "rust-sorted-derives" "tokei" ];
            jobsOther = [ "loc-badge" ];
          };
          readme = v-utils.readme-fw {
            inherit pkgs pname;
            lastSupportedVersion = "nightly-1.90";
            rootDir = ./.;
            licenses = [{ name = "Blue Oak 1.0.0"; outPath = "LICENSE"; }];
            badges = [ "msrv" "crates_io" "docs_rs" "loc" "ci" ];
          };
        in
        {
          packages =
            let
              rustc = rust;
              cargo = rust;
              rustPlatform = pkgs.makeRustPlatform {
                inherit rustc cargo stdenv;
              };
            in
            {
              default = rustPlatform.buildRustPackage rec {
                inherit pname;
                version = manifest.version;

                buildInputs = with pkgs; [
                  openssl.dev
                ];
                nativeBuildInputs = with pkgs; [ pkg-config ];

                cargoLock.lockFile = ./Cargo.lock;
                src = pkgs.lib.cleanSource ./.;
              };
            };

          devShells.default =
            with pkgs;
            mkShell {
              inherit stdenv;
              shellHook =
                pre-commit-check.shellHook +
                workflowContents.shellHook +
                ''
                  cp -f ${v-utils.files.licenses.blue_oak} ./LICENSE

                  cargo -Zscript -q ${v-utils.hooks.appendCustom} ./.git/hooks/pre-commit
                  cp -f ${(v-utils.hooks.treefmt) { inherit pkgs; }} ./.treefmt.toml
                  cp -f ${(v-utils.hooks.preCommit) { inherit pkgs pname; }} ./.git/hooks/custom.sh

                  mkdir -p ./.cargo
                  #cp -f ${(v-utils.files.rust.config { inherit pkgs; })} ./.cargo/config.toml #TODO: procedurally add aliases here
                  cp -f ${(v-utils.files.rust.clippy { inherit pkgs; })} ./.cargo/.clippy.toml
                  #cp -f ${ (v-utils.files.rust.toolchain { inherit pkgs; }) } ./.cargo/rust-toolchain.toml
                  cp -f ${(v-utils.files.rust.rustfmt { inherit pkgs; })} ./.rustfmt.toml
                  cp -f ${(v-utils.files.rust.deny { inherit pkgs; })} ./deny.toml
                  cp -f ${ (v-utils.files.gitignore { inherit pkgs; langs = [ "rs" ]; }) } ./.gitignore

                  cp -f ${readme} ./README.md

                  alias qr="./target/debug/${pname}"
                '';

              packages = [
                mold-wrapped
                openssl
                pkg-config
                rust
              ] ++ pre-commit-check.enabledPackages;

              env.RUST_BACKTRACE = 1;
              env.RUST_LIB_BACKTRACE = 0;
            };
        }
      )
    // {
      homeManagerModules."watch-monitors" = { config, lib, pkgs, ... }:
        let
          inherit (lib) mkEnableOption mkOption mkIf;
          inherit (lib.types) package;
          cfg = config.services.todo-watch-monitors;
          manifest = (lib.importTOML ./Cargo.toml).package;
          pname = manifest.name;
        in
        {
          options.services.todo-watch-monitors = {
            enable = mkEnableOption "todo watch-monitors daemon";

            package = mkOption {
              type = package;
              default = self.packages.${pkgs.system}.default;
              description = "The todo package to use.";
            };
          };

          config = mkIf cfg.enable {
            systemd.user.services.todo-watch-monitors = {
              Unit = {
                Description = "todo watch-monitors daemon - periodic screenshot capture";
                After = [ "graphical-session.target" ];
              };

              Install = {
                WantedBy = [ "graphical-session.target" ];
              };

              Service = {
                Type = "simple";
                ExecStart = "${cfg.package}/bin/${pname} watch-monitors";
                Restart = "on-failure";
                RestartSec = "10s";
              };
            };

            home.packages = [ cfg.package ];
          };
        };
    };
}

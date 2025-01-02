{
  inputs = {
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
		pre-commit-hooks.url = "github:cachix/git-hooks.nix";
  };

  outputs = { nixpkgs, rust-overlay, flake-utils, pre-commit-hooks, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = builtins.trace "flake.nix sourced" [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
      in
      {
				packages =
					let
						manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
					in
						{
						default = pkgs.rustPlatform.buildRustPackage rec {
							pname = manifest.name;
							version = manifest.version;

							buildInputs = with pkgs; [
								openssl
								openssl.dev
							];
							nativeBuildInputs = with pkgs; [ pkg-config ];
							env.PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

							cargoLock.lockFile = ./Cargo.lock;
							src = pkgs.lib.cleanSource ./.;
						};
					};

						checks = {
					pre-commit-check = pre-commit-hooks.lib.${system}.run {
						src = ./.;
						hooks = {
							nixpkgs-fmt.enable = true;
						};
					};
				};

        devShells.default = with pkgs; mkShell {
					stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;
          packages = [
						mold-wrapped
            openssl
            pkg-config
            (rust-bin.fromRustupToolchainFile ./.cargo/rust-toolchain.toml)
          ];
					 shellHook = ''
            echo "Run pre-commit-check with: nix run .#pre-commit-check"
          '';
        };
      }
    );
}


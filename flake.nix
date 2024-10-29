{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    fenix.url = "github:nix-community/fenix";
  };

  outputs = inputs@{ nixpkgs, flake-parts, ... }:
    flake-parts.lib.mkFlake {inherit inputs;} {
      systems = nixpkgs.lib.systems.flakeExposed;
      perSystem = {
        lib,
        pkgs,
        system,
        config,
        ...
      }: 
      {
        _module.args.pkgs = import nixpkgs {
          inherit system;
          overlays = [
            (inputs.fenix.overlays.default)
          ];
        };

        devShells.default = with pkgs; let 
        toolchain = pkgs.fenix.complete.withComponents [
          "rustc"
          "cargo"
          "clippy"
        ];
        in mkShell
        {
          packages = with pkgs; [
						hello
            openssl
            rust-analyzer-nightly
            toolchain
						pkg-config
          ];
        };
      };
    };
}

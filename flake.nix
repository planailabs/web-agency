{
  inputs = {
    # Include git submodules (common, design) in the flake source.
    self.submodules = true;

    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    {
      overlays.default = import ./overlay.nix { gitSha = self.rev or self.dirtyRev or "unknown"; };
      nixosModules.default = import ./server/module.nix;
      nixosModules.web-agency = import ./server/module.nix;
      nixosModules.web-agency-proxy = import ./proxy/module.nix;
    } //
    flake-utils.lib.eachDefaultSystem (system:
      let
        gitSha = self.rev or self.dirtyRev or "unknown";
        overlays = [
          (import rust-overlay)
          (import ./overlay.nix { inherit gitSha; })
        ];
        pkgs = import nixpkgs { inherit system overlays; };
        toolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
          targets = [ "wasm32-unknown-unknown" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            toolchain
            cargo-edit
            cargo-watch

            # Dioxus CLI (patched with --skip-platform-features)
            dioxus-cli-patched

            # Build dependencies
            pkg-config
            openssl
            nodejs
            tailwindcss_3

            # Ephemeral test databases (pgtemp spawns initdb/postgres)
            postgresql

            # cargo-progenitor installed via: cargo install cargo-progenitor
            wrangler

            # proxy (BoringSSL build via boring-sys)
            cmake
            clang
            libclang.lib
            perl

            # For WASM
            wasm-bindgen-cli_0_2_121
            binaryen  # wasm-opt
            lld
          ];

          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        };

        packages = {
          default = pkgs.web-agency-server;
          web-agency = pkgs.web-agency-server;
          web-agency-proxy = pkgs.web-agency-proxy;
          web-agency-mocks = pkgs.web-agency-mocks;
          dioxus-cli-patched = pkgs.dioxus-cli-patched;
        };

        # nixpkgs.lib (not pkgs.lib): evaluating pkgs for unsupported systems
        # (x86_64-darwin) throws, and `nix flake show` walks every system.
        checks = nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          # Full-workload NixOS VM test: server + proxy + postgres + provider
          # mocks + pebble ACME. Run with:
          #   nix build .#checks.x86_64-linux.full -L
          full = pkgs.callPackage ./tests/full.nix { };
        };
      });
}

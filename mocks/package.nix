{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "web-agency-mocks";
  version = "0.1.0";
  src = ../.;
  cargoLock.lockFile = ../Cargo.lock;
  cargoLock.outputHashes = import ../extra-hashes.nix;

  cargoBuildFlags = [
    "-p" "mock-spaceship"
    "-p" "mock-cloudflare"
    "-p" "mock-changedetection"
  ];

  doCheck = false;

  meta = {
    description = "Web agency provider API mocks – Spaceship, Cloudflare and ChangeDetection.io";
    license = lib.licenses.asl20;
  };
}

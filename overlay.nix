{ gitSha ? "unknown" }:

final: prev:
let
  # Patched dioxus-cli with:
  # - --skip-platform-features: prevents dx from auto-adding "web"/"desktop"/etc.
  # - --embed: embeds public assets into the server binary via rust-embed
  # - optional codesign: falls back to rcodesign for cross-compilation
  dioxus-cli-patched = prev.dioxus-cli.overrideAttrs (old: {
    patches = (old.patches or []) ++ [
      ./patches/dioxus-cli-all.patch
    ];
  });
in
{
  inherit dioxus-cli-patched;

  web-agency-server = prev.callPackage ./server/package.nix { inherit gitSha; };
  web-agency-proxy = prev.callPackage ./proxy/package.nix { };
  web-agency-mocks = prev.callPackage ./mocks/package.nix { };
}

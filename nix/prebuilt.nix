# Pinned prebuilt Skia the desktop build consumes.
#
# The Nix sandbox has no network, so `freya-skia-bindings`'s build.rs
# gets its prebuilt archive through `SKIA_BINARIES_URL=file://…`. The
# key must match what the build script computes (crate version + repo
# hash + target + enabled features); a mismatch makes it fall back to a
# from-source build, which fails loudly offline. Bump the tag/hash
# together with `freya-skia-bindings` in Cargo.lock. The key the local
# build used is logged by `cargo build -vv` as
# "TRYING TO DOWNLOAD AND INSTALL SKIA BINARIES: <tag>/<key>".
{
  fetchurl,
  runCommand,
  system,
}:
let
  skiaTag = "0.104.0";
  skiaHash = "746613b43b596b9d2114";

  linuxFeatures = "egl-ganesh-gl-jpegd-jpege-svg-textlayout-vulkan-wayland-webpd-webpe-x11";
  darwinFeatures = "ganesh-jpegd-jpege-metal-svg-textlayout-webpd-webpe";

  perSystem = {
    x86_64-linux = {
      triple = "x86_64-unknown-linux-gnu";
      features = linuxFeatures;
      hash = "sha256-3BJEhkPhqH6QOloL8UL3cueOS3g/56q8Xnn9sFD3wDw=";
    };
    aarch64-linux = {
      triple = "aarch64-unknown-linux-gnu";
      features = linuxFeatures;
      hash = "sha256-rRQJzPQAXfhdnfo2X9u+tjsNNbA5/11yZ1WohvkSEeo=";
    };
    aarch64-darwin = {
      triple = "aarch64-apple-darwin";
      features = darwinFeatures;
      hash = "sha256-bzANEpQzqtfTll+fHMVJS0vnZECptrnZTteIp4BNMQY=";
    };
  };

  cfg = perSystem.${system} or (throw "sismo: no prebuilt skia pinned for ${system}");
  skiaKey = "${skiaHash}-${cfg.triple}-${cfg.features}";
in
{
  # Directory holding `skia-binaries-<key>.tar.gz`, ready for the
  # `file://<dir>/skia-binaries-{key}.tar.gz` URL template.
  skia = runCommand "skia-binaries-${skiaTag}-${cfg.triple}" { } ''
    mkdir -p $out
    ln -s ${
      fetchurl {
        url = "https://github.com/marc2332/rust-skia/releases/download/${skiaTag}/skia-binaries-${skiaKey}.tar.gz";
        inherit (cfg) hash;
      }
    } $out/skia-binaries-${skiaKey}.tar.gz
  '';
}

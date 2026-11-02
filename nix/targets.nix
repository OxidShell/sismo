# Release matrix — single source of truth for what gets built where.
#
# `nix run .#matrix` serialises this into the JSON the release workflow
# fans out over, so adding a platform/format here is the only change
# needed to ship it.
#
# kind = "nix"    → built and bundled natively on `runner` by Nix.
# kind = "cross"  → cross-built and bundled on `runner` (x86_64-linux) by Nix.
{
  desktop = {
    x86_64-linux = {
      kind = "nix";
      runner = "ubuntu-latest";
      formats = [
        "appimage"
        "deb"
        "rpm"
        "tar.gz"
        "flatpak"
      ];
    };
    aarch64-linux = {
      kind = "nix";
      runner = "ubuntu-24.04-arm";
      formats = [
        "appimage"
        "deb"
        "rpm"
        "tar.gz"
      ];
    };
    aarch64-darwin = {
      kind = "nix";
      runner = "macos-latest";
      formats = [
        "app"
        "dmg"
        "pkg"
        "tar.gz"
      ];
    };
    # MSVC target linked with LLVM against the xwin SDK (nix/windows.nix);
    # no Windows runner involved.
    x86_64-windows = {
      kind = "cross";
      runner = "ubuntu-latest";
      formats = [
        "nsis"
        "msi"
        "zip"
      ];
    };
  };
}

{
  nixConfig = {
    extra-substituters = [ "https://cache.sergioribera.rs/main" ];
    extra-trusted-public-keys = [ "main:vFI3N1JP9edRFwBwdk9ebUKTIPWK9R1ECbkdA7Q593M=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-bundle-app = {
      url = "github:SergioRibera/nix-bundle-app";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      crane,
      rust-overlay,
      nix-bundle-app,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            (import rust-overlay)
            nix-bundle-app.overlays.default
          ];
          config.allowUnfree = true;
        };
        lib = pkgs.lib;
      in
      {
        # Devshell — bring `just`, wayland/x11 dev libs for the desktop shell,
        # and the utilities the mobile builds shell out to.
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            just
            claude-code
            pkg-config
            libGL
            wayland
            libxkbcommon

            libX11
            libXcursor
            libXrandr
            libXi

            # freya-skia links against system freetype/fontconfig.
            freetype
            fontconfig
          ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
            with pkgs;
            [
              libGL
              wayland
              libxkbcommon
              libX11
              libXcursor
              libXrandr
              libXi
              freetype
              fontconfig
            ]
          );
        };

        formatter = pkgs.nixfmt-rfc-style;
      }
    );
}

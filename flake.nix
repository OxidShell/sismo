{
  description = "sismo — remote and recorded performance + log analysis for Rust apps";

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

        craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.stable.latest.minimal);
        sismo = import ./nix/package.nix {
          inherit
            pkgs
            lib
            craneLib
            system
            ;
        };
        inherit (sismo) pname version;

        targets = import ./nix/targets.nix;
        bundler = pkgs.nixBundleApp;

        info = {
          name = pname;
          inherit version;
          license = "MIT OR Apache-2.0";
          maintainer = "Sergio Ribera <contact@sergioribera.rs>";
          description = "Performance and log viewer for Rust apps";
          summary = "Timeline, flamegraph, functions and logs for sismo recordings and live apps";
          homepage = "https://github.com/SergioRibera/sismo";
          bundleId = "rs.sergioribera.sismo";
          minimumSystemVersion = "11.0";
          # Pinned so every future MSI upgrades the previous install.
          msiUpgradeCode = "F0C2CAD2-BF1F-43D0-8429-08734D9A45AD";
          downloadUrl = "https://github.com/SergioRibera/sismo/releases/download/v${version}";
          desktopEntries = [
            {
              name = "sismo";
              genericName = "Profiler";
              comment = info.summary;
              exec = "/opt/${pname}/bin/${pname} open %f";
              categories = [
                "Development"
                "Profiling"
              ];
              keywords = [
                "profiler"
                "tracing"
                "logs"
                "flamegraph"
              ];
            }
          ];
          flatpak = {
            runtimeVersion = "24.08";
            finishArgs = [
              "--share=ipc"
              # LAN discovery and live streams (zudp).
              "--share=network"
              "--socket=wayland"
              "--socket=fallback-x11"
              "--device=dri"
              # Open and save recordings anywhere.
              "--filesystem=home"
            ];
          };
        };

        # `tar.gz` → `tar_gz`: keeps attr names CLI-friendly (`.#bundle-tar_gz`).
        attrOf = format: lib.replaceStrings [ "." ] [ "_" ] format;

        # flatpak is a source kit finished by flatpak-builder in its own job,
        # so it's exposed as `flatpak-kit` instead of joining `bundles`.
        mkBundles =
          {
            drv,
            formats,
            info,
            target ? null,
          }:
          let
            built = lib.genAttrs (lib.remove "flatpak" formats) (
              format:
              bundler.bundle {
                inherit
                  drv
                  format
                  info
                  target
                  ;
              }
            );
          in
          lib.mapAttrs' (format: value: lib.nameValuePair "bundle-${attrOf format}" value) built
          // {
            bundles = pkgs.linkFarm "${pname}-${info.version}-${system}-bundles" (
              lib.mapAttrsToList (name: path: { inherit name path; }) built
            );
          }
          // lib.optionalAttrs (builtins.elem "flatpak" formats) {
            flatpak-kit = bundler.bundle {
              inherit drv info target;
              format = "flatpak";
            };
          };

        nativeTarget = targets.desktop.${system} or null;
        nativeBundles = lib.optionalAttrs (nativeTarget != null && nativeTarget.kind == "nix") (mkBundles {
          drv = sismo.unwrapped;
          inherit info;
          inherit (nativeTarget) formats;
        });

        # Windows: `cargo build` runs on a Windows runner (MSVC + prebuilt
        # Skia), the job uploads `sismo.exe`, and this wraps that directory
        # into a derivation for nix-bundle-app. Impure by necessity:
        # `SISMO_WINDOWS_DIST=<dir> nix build --impure .#windows-bundles`.
        # The attributes only exist while the variable is set, so a pure
        # `nix flake check` never trips over them.
        windowsDist = builtins.getEnv "SISMO_WINDOWS_DIST";
        windowsBundles =
          let
            drv = pkgs.runCommand "${pname}-${version}-x86_64-windows" { } ''
              mkdir -p $out/bin
              cp -r ${
                builtins.path {
                  path = windowsDist;
                  name = "${pname}-windows-dist";
                }
              }/. $out/bin/
            '';
          in
          lib.mapAttrs' (name: value: lib.nameValuePair "windows-${name}" value) (mkBundles {
            inherit drv info;
            inherit (targets.desktop.x86_64-windows) formats;
            target = {
              arch = "x86_64";
              os = "windows";
            };
          });

        matrix =
          let
            entries = lib.mapAttrsToList (system: t: t // { inherit system; }) targets.desktop;
          in
          builtins.toJSON {
            nix = map (t: { inherit (t) system runner; }) (builtins.filter (t: t.kind == "nix") entries);
            prebuilt = map (t: {
              inherit (t) system runner bundleRunner;
            }) (builtins.filter (t: t.kind == "prebuilt") entries);
          };
      in
      {
        packages = {
          default = sismo.wrapped;
          ${pname} = sismo.wrapped;
          "${pname}-unwrapped" = sismo.unwrapped;
          inherit (sismo) cargoArtifacts;
        }
        // nativeBundles
        // lib.optionalAttrs (system == "x86_64-linux" && windowsDist != "") windowsBundles;

        apps.matrix = {
          type = "app";
          program = toString (pkgs.writeShellScript "sismo-matrix" "echo '${matrix}'");
        };

        # Devshell — `just`, wayland/x11 dev libs for the viewer and the
        # release tooling.
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            just
            claude-code
            cargo-release
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
          LD_LIBRARY_PATH = lib.makeLibraryPath (
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

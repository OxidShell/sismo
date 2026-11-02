# Cross build of the viewer for `x86_64-pc-windows-msvc` from Linux.
#
# rust-skia only publishes prebuilt Skia for the MSVC ABI and Skia's own
# GN files hardwire the MSVC toolchain on Windows, so a mingw build is
# off the table. Instead the MSVC target is linked here with LLVM:
# clang-cl + lld-link against the Windows SDK and CRT that `xwin`
# downloads from Microsoft (fixed-output, versions pinned below).
{
  pkgs,
  lib,
  crane,
  src,
  pname,
  version,
}:
let
  target = "x86_64-pc-windows-msvc";
  targetEnv = "X86_64_PC_WINDOWS_MSVC";

  toolchain = pkgs.rust-bin.stable.latest.minimal.override { targets = [ target ]; };
  craneLib = (crane.mkLib pkgs).overrideToolchain (_: toolchain);
  llvm = pkgs.llvmPackages;

  # Windows SDK + Visual C++ CRT headers and import libraries. Pinned so the
  # output hash stays put; bump both together after re-running
  # `xwin --accept-license list` and refreshing `outputHash`.
  xwinSdk = pkgs.stdenvNoCC.mkDerivation {
    pname = "xwin-sdk";
    version = "10.0.26100-14.44.17.14";
    dontUnpack = true;
    nativeBuildInputs = [
      pkgs.xwin
      pkgs.cacert
    ];
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    buildCommand = ''
      export HOME=$TMPDIR
      xwin --accept-license --arch x86_64 --cache-dir $TMPDIR/xwin \
        --manifest-version 17 --sdk-version 10.0.26100 --crt-version 14.44.17.14 \
        splat --output $TMPDIR/splat
      # xwin links from its cache into the output; keep both on one filesystem.
      cp -r $TMPDIR/splat $out
    '';
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = "sha256-UFQjsFVBwcF/9e9tVFoG0Z1JySxyTnFqoaRwr/tUWzA=";
  };

  # `freya-skia-bindings` downloads its prebuilt archive; the sandbox has
  # none, so it is pinned here. Same hash/features scheme as prebuilt.nix.
  # `-static`: built against the static CRT, matching `+crt-static` below so
  # the exe needs no VC++ redistributable.
  skiaKey = "746613b43b596b9d2114-${target}-ganesh-gl-jpegd-jpege-svg-textlayout-vulkan-webpd-webpe-static";
  skia = pkgs.runCommand "skia-binaries-0.104.0-${target}" { } ''
    mkdir -p $out
    ln -s ${
      pkgs.fetchurl {
        url = "https://github.com/marc2332/rust-skia/releases/download/0.104.0/skia-binaries-${skiaKey}.tar.gz";
        hash = "sha256-FDNepZ/1GyQ62bYJp/p/+yLC/nKYQYwav1ULoEJIvMY=";
      }
    } $out/skia-binaries-${skiaKey}.tar.gz
  '';

  # clang-cl with the MSVC include tree, for the odd build script that
  # compiles C for the target.
  clangCl = pkgs.writeShellScriptBin "clang-cl" ''
    exec ${llvm.clang-unwrapped}/bin/clang-cl --target=${target} \
      /imsvc ${xwinSdk}/crt/include \
      /imsvc ${xwinSdk}/sdk/include/ucrt \
      /imsvc ${xwinSdk}/sdk/include/um \
      /imsvc ${xwinSdk}/sdk/include/shared \
      "$@"
  '';

  libDirs = [
    "${xwinSdk}/crt/lib/x86_64"
    "${xwinSdk}/sdk/lib/um/x86_64"
    "${xwinSdk}/sdk/lib/ucrt/x86_64"
  ];

  commonArgs = {
    inherit src pname version;
    strictDeps = true;
    cargoLock = ../Cargo.lock;
    cargoToml = ../crates/sismo-viewer/Cargo.toml;
    cargoExtraArgs = "--locked -p sismo-viewer --bin ${pname}";
    doCheck = false;

    CARGO_BUILD_TARGET = target;
    "CARGO_TARGET_${targetEnv}_LINKER" = "${llvm.lld}/bin/lld-link";
    "CARGO_TARGET_${targetEnv}_RUSTFLAGS" = lib.concatStringsSep " " (
      [ "-Ctarget-feature=+crt-static" ] ++ map (d: "-Lnative=${d}") libDirs
    );
    "CC_${target}" = "${clangCl}/bin/clang-cl";
    "CXX_${target}" = "${clangCl}/bin/clang-cl";
    "AR_${target}" = "${llvm.llvm}/bin/llvm-lib";

    SKIA_BINARIES_URL = "file://${skia}/skia-binaries-{key}.tar.gz";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
{
  inherit xwinSdk cargoArtifacts;
  package = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      meta = {
        description = "Desktop viewer and headless CLI for sismo sessions and recordings";
        homepage = "https://github.com/SergioRibera/sismo";
        license = with lib.licenses; [
          mit
          asl20
        ];
      };
    }
  );
}

# Desktop build of the `sismo` viewer binary (crane).
{
  pkgs,
  lib,
  craneLib,
  system,
}:
let
  workspace = lib.importTOML ../Cargo.toml;
  inherit (workspace.workspace.package) version;
  # The binary is `sismo`, built from the `sismo-viewer` crate.
  pname = "sismo";
  crate = "sismo-viewer";

  prebuilt = pkgs.callPackage ./prebuilt.nix { inherit system; };

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
    ];
  };

  # Mostly dlopen'd by winit / glutin, so the Nix wrapper puts them on
  # LD_LIBRARY_PATH. Packaged bundles leave them to the host instead
  # (nix-bundle-app `info.hostLibs` defaults), where the GPU driver
  # stack must win.
  dlopenLibs = with pkgs; [
    libGL
    vulkan-loader
    wayland
    libxkbcommon
    libX11
    libXcursor
    libXrandr
    libXi
  ];

  commonArgs = {
    inherit src pname version;
    strictDeps = true;
    cargoLock = ../Cargo.lock;
    cargoToml = ../crates/sismo-viewer/Cargo.toml;
    cargoExtraArgs = "--locked -p ${crate} --bin ${pname}";
    doCheck = false;

    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs =
      lib.optionals pkgs.stdenv.hostPlatform.isLinux (
        with pkgs;
        [
          fontconfig
          freetype
          libxcb
        ]
        ++ dlopenLibs
      )
      ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];

    SKIA_BINARIES_URL = "file://${prebuilt.skia}/skia-binaries-{key}.tar.gz";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  unwrapped = craneLib.buildPackage (
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
        mainProgram = pname;
      };
    }
  );

  wrapped =
    if pkgs.stdenv.hostPlatform.isLinux then
      pkgs.runCommand "${pname}-${version}"
        {
          nativeBuildInputs = [ pkgs.makeWrapper ];
          inherit (unwrapped) meta;
          passthru = { inherit unwrapped; };
        }
        ''
          mkdir -p $out/bin
          makeWrapper ${unwrapped}/bin/${pname} $out/bin/${pname} \
            --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath dlopenLibs}
        ''
    else
      unwrapped;
in
{
  inherit
    pname
    version
    cargoArtifacts
    unwrapped
    wrapped
    dlopenLibs
    prebuilt
    src
    ;
}

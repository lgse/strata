{
  lib,
  rustPlatform,
  pkg-config,
  wrapGAppsHook4,
  glib,
  gtk4,
  gtksourceview5,
  poppler,
  fontconfig,
  gst_all_1,
  gdk-pixbuf,
  librsvg,
  bubblewrap,
  ffmpeg,
  ffmpegthumbnailer,
  imagemagick,
  libraw,
  squashfsTools,
  util-linux,
  coreutils,
  xdg-utils,
  adwaita-icon-theme,
  rarSupport ? false,
  buildCommit ? null,
}:
let
  manifest = builtins.fromTOML (builtins.readFile ./Cargo.toml);
  helpers = [
    ffmpeg
    ffmpegthumbnailer
    imagemagick
    libraw
    squashfsTools
    util-linux
    coreutils
    fontconfig
  ];
in
rustPlatform.buildRustPackage {
  pname = "strata";
  version = manifest.package.version;
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./build.rs
      ./src
      ./data
      ./integrations
      ./LICENSE
      ./THIRD_PARTY_LICENSES.md
    ];
  };
  cargoLock.lockFile = ./Cargo.lock;
  buildNoDefaultFeatures = !rarSupport;

  nativeBuildInputs = [
    pkg-config
    wrapGAppsHook4
    glib
  ];
  buildInputs = [
    gtk4
    gtksourceview5
    poppler
    fontconfig
    gdk-pixbuf
    librsvg
    adwaita-icon-theme
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-libav
  ];

  STRATA_BUILD_COMMIT = buildCommit;
  STRATA_SANDBOX_ROOT = "/nix/store";
  STRATA_UPDATER = "disabled";
  STRATA_TRUSTED_COMMAND_PATH = lib.makeBinPath (
    helpers
    ++ [
      bubblewrap
      xdg-utils
    ]
  );
  STRATA_SANDBOX_PATH = lib.makeBinPath helpers;
  STRATA_SANDBOX_PRLIMIT = "${lib.getBin util-linux}/bin/prlimit";
  STRATA_SANDBOX_GDK_PIXBUF_MODULE_FILE = "${librsvg}/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache";

  postInstall = ''
    install -Dm644 data/io.github.lgse.Strata.desktop \
      "$out/share/applications/io.github.lgse.Strata.desktop"
    substituteInPlace "$out/share/applications/io.github.lgse.Strata.desktop" \
      --replace-fail 'Exec=strata' "Exec=$out/bin/strata"
    install -Dm644 data/icons/scalable/apps/io.github.lgse.Strata.svg \
      "$out/share/icons/hicolor/scalable/apps/io.github.lgse.Strata.svg"
    install -Dm644 data/io.github.lgse.Strata.FileManager1.service \
      "$out/share/strata/io.github.lgse.Strata.FileManager1.service"
    substituteInPlace "$out/share/strata/io.github.lgse.Strata.FileManager1.service" \
      --replace-fail /usr/bin/strata "$out/bin/strata"
    install -Dm644 ${./nix/install-source.toml} "$out/share/strata/install-source.toml"
    install -Dm644 LICENSE "$out/share/licenses/strata/LICENSE"
    install -Dm644 THIRD_PARTY_LICENSES.md "$out/share/licenses/strata/THIRD_PARTY_LICENSES.md"
  '';

  # GTK tests require an isolated display and D-Bus; run separately from packaging.
  doCheck = false;

  meta = {
    inherit (manifest.package) description;
    homepage = "https://github.com/lgse/strata";
    license = [ lib.licenses.mit ] ++ lib.optional rarSupport lib.licenses.unfreeRedistributable;
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
    mainProgram = "strata";
  };
}

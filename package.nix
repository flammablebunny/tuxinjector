{
  lib,
  stdenv,
  craneLib,
  clang,
  dbus,
  libclang,
  libx11,
  libxcb,
  libxcursor,
  libxext,
  libxfixes,
  libxi,
  libxinerama,
  libxkbcommon,
  libxrandr,
  libxrender,
  libxt,
  libxtst,
  pipewire,
  pkg-config,
  libiconv,
}:
let
  # X11 libs that companion apps (nbb) need at runtime,
  # but the game doesn't load. The wrapper sets TUXINJECTOR_X11_LIBS so
  # the .so can pass them to companion apps using LD_LIBRARY_PATH.
  x11Libs = [
    libx11
    libxcb
    libxcursor
    libxext
    libxfixes
    libxi
    libxinerama
    libxkbcommon
    libxrandr
    libxrender
    libxt
    libxtst
  ];
in
craneLib.buildPackage {
  pname = "tuxinjector";
  version = "1.0.0";

  src = ./.;

  nativeBuildInputs = [
    clang
    pkg-config
  ];

  buildInputs = [
    libclang.lib
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [
    dbus
    pipewire
  ]
  ++ lib.optionals stdenv.hostPlatform.isDarwin [
    libiconv
  ];

  env.LIBCLANG_PATH = "${libclang.lib}/lib";

  postInstall = ''
    mkdir -p $out/bin

    cat << EOF >> $out/bin/tuxinjector-wrapper
    #!/usr/bin/env bash
  ''
  + lib.optionalString stdenv.hostPlatform.isLinux ''
    export LD_PRELOAD="$out/lib/libtuxinjector.so"
    export TUXINJECTOR_X11_LIBS="${lib.makeLibraryPath x11Libs}"
  ''
  + lib.optionalString stdenv.hostPlatform.isDarwin ''
    export DYLD_INSERT_LIBRARIES="$out/lib/libtuxinjector.dylib"
  ''
  + ''
    exec "\$@"
    EOF

    chmod 755 $out/bin/tuxinjector-wrapper
  '';

  meta = {
    description = "Minecraft speedrunning overlay for Linux & MacOS";
    license = lib.licenses.gpl3;
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}

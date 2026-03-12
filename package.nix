{
  lib,
  stdenv,
  fetchurl,
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
}:
let
  version = "1.0.0";

  # Pre-built binaries from GitHub releases, verified by SHA-512.
  # Building from source changes the hash, which is illegal per speedrun.com injection rules.
  binaryInfo = {
    x86_64-linux = {
      filename = "tuxinjector_x64.so";
      hash = "sha512-Z8tqlpkn1Ck+XIEqcDZnfhJ7AgwlDhrsNJRX6C/j+yd7IDBuEHprVY+j4ZOuIXbNcaBv7AHZrhB6XDeA6bUvkA==";
    };
    i686-linux = {
      filename = "tuxinjector_x86.so";
      hash = "sha512-7TvZ1GeVZWqWmHfaQDxGw9V3xaTZY2l33O3sQXwSYZDode2tXoeFFA5nNpzyq/GAZ1zMknrP95ptUb/abDVK8A==";
    };
    aarch64-linux = {
      filename = "tuxinjector_aarch64.so";
      hash = "sha512-1EHVUuRWbdTvjitsQDyguqXxpxSHgjFj2ISyxJ1C4620Pq+2U59+4U443gEOG5paTfqAowPiB3w207odNe5ZKA==";
    };
    armv7l-linux = {
      filename = "tuxinjector_aarch32.so";
      hash = "sha512-ZnHZukKxUyFlPUzr/heMQMFQyUYOhdxE3zu7e0GritfV5eTHJ1p9aI2q1MxPZ2/GzZPVUm386G2T0OlePp/N2w==";
    };
  };

  info = binaryInfo.${stdenv.hostPlatform.system}
    or (throw "tuxinjector: unsupported platform ${stdenv.hostPlatform.system}");

  binary = fetchurl {
    url = "https://github.com/flammablebunny/tuxinjector/releases/download/v${version}/${info.filename}";
    inherit (info) hash;
  };

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
stdenv.mkDerivation {
  pname = "tuxinjector";
  inherit version;

  dontUnpack = true;

  installPhase = ''
    runHook preInstall

    mkdir -p $out/lib $out/bin
    cp ${binary} $out/lib/libtuxinjector.so

    cat > $out/bin/tuxinjector-wrapper << 'WRAPPER'
    #!/usr/bin/env bash
    export LD_PRELOAD="PLACEHOLDER_LIB"
    export TUXINJECTOR_X11_LIBS="PLACEHOLDER_X11"
    exec "$@"
    WRAPPER

    substituteInPlace $out/bin/tuxinjector-wrapper \
      --replace-warn "PLACEHOLDER_LIB" "$out/lib/libtuxinjector.so" \
      --replace-warn "PLACEHOLDER_X11" "${lib.makeLibraryPath x11Libs}"

    chmod 755 $out/bin/tuxinjector-wrapper

    runHook postInstall
  '';

  meta = {
    description = "Minecraft speedrunning overlay for Linux & macOS";
    license = lib.licenses.gpl3;
    platforms = [ "x86_64-linux" "i686-linux" "aarch64-linux" "armv7l-linux" ];
  };
}

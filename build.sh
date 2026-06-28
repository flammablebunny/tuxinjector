#!/usr/bin/env bash
set -euo pipefail

# Builds tuxinjector and produces a shareable distributable binary for both linux and macos.
# On macos: rewrites Nix store rpaths to system paths, renames to tuxinjector.dylib
# On linux: renames to tuxinjector.so

PROFILE="${1:-release}"
CARGO_FLAGS=()
if [ "$PROFILE" = "release" ]; then
    CARGO_FLAGS+=(--release)
fi

TARGET_DIR="target/$PROFILE"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$ARCH" in
    x86_64)  ARCH_SUFFIX="x64" ;;
    i686|i386) ARCH_SUFFIX="x86" ;;
    aarch64|arm64) ARCH_SUFFIX="aarch64" ;;
    armv7l|armhf) ARCH_SUFFIX="aarch32" ;;
    *) echo "error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

echo ":: Building tuxinjector ($PROFILE)..."

# build browser helper first so it gets embedded via include_bytes
if pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    echo ":: Building tuxinjector-browser helper..."
    if cargo build --manifest-path crates/tuxinjector-browser/Cargo.toml "${CARGO_FLAGS[@]}" 2>/dev/null; then
        cp "crates/tuxinjector-browser/target/$PROFILE/tuxinjector-browser" "assets/tuxinjector-browser_${ARCH_SUFFIX}"
        echo ":: tuxinjector-browser embedded into assets/"
    else
        echo ":: tuxinjector-browser build failed (non-fatal)"
    fi
else
    echo ":: Skipping tuxinjector-browser (webkit2gtk not found)"
fi

cargo build "${CARGO_FLAGS[@]}"

if [ "$OS" = "Darwin" ]; then
    SRC="$TARGET_DIR/libtuxinjector.dylib"
    DST="$TARGET_DIR/tuxinjector_${ARCH_SUFFIX}.dylib"

    if [ ! -f "$SRC" ]; then
        echo "error: $SRC not found" >&2
        exit 1
    fi

    # rewrite any /nix/store paths to the system equivalents
    # note: macos 11+ moved system dylibs into a shared cache, so they don't
    # exist as files on disk, but the runtime linker still resolves /usr/lib/ paths
    nix_deps=$(otool -L "$SRC" | grep '/nix/store' | awk '{print $1}' || true)
    if [ -n "$nix_deps" ]; then
        echo ":: Rewriting Nix store paths..."
        for dep in $nix_deps; do
            lib_name=$(basename "$dep")
            system_path="/usr/lib/$lib_name"
            echo "   $dep -> $system_path"
            install_name_tool -change "$dep" "$system_path" "$SRC"
        done
    fi

    mv "$SRC" "$DST"
    echo ":: Built: $DST"

elif [ "$OS" = "Linux" ]; then
    SRC="$TARGET_DIR/libtuxinjector.so"
    DST="$TARGET_DIR/tuxinjector_${ARCH_SUFFIX}.so"

    if [ ! -f "$SRC" ]; then
        echo "error: $SRC not found" >&2
        exit 1
    fi

    mv "$SRC" "$DST"

    # Portability: the nix toolchain bakes a /nix/store glibc path into the .so's
    # RUNPATH. When tux is LD_PRELOADed on another distro that happens to have nix
    # installed (the store path exists), the loader uses that nix glibc -- which is
    # often OLDER than the host's -- and it then shadows the host glibc for the
    # whole process. The game's Mesa (built against the host glibc) then can't find
    # its required GLIBC_x.yz symbols and libGLX_mesa fails to load ("Failed to
    # find a suitable GLXFBConfig"). Strip the nix glibc entry so tux resolves
    # libc/libm from the host. On NixOS the loader still finds nix glibc via its
    # default search, so the dev's own runs are unaffected.
    if command -v patchelf >/dev/null 2>&1; then
        rpath="$(patchelf --print-rpath "$DST" 2>/dev/null || true)"
        if printf '%s' "$rpath" | grep -qi glibc; then
            new="$(printf '%s' "$rpath" | tr ':' '\n' | grep -vi glibc | paste -sd: -)"
            patchelf --set-rpath "$new" "$DST" \
                && echo ":: stripped nix glibc from RUNPATH (portable build)"
        fi
    fi

    echo ":: Built: $DST"

else
    echo "error: unsupported OS: $OS" >&2
    exit 1
fi

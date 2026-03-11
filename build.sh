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

echo ":: Building tuxinjector ($PROFILE)..."
cargo build "${CARGO_FLAGS[@]}"

TARGET_DIR="target/$PROFILE"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$ARCH" in
    x86_64)  ARCH_SUFFIX="x64" ;;
    i686|i386) ARCH_SUFFIX="x86" ;;
    aarch64) ARCH_SUFFIX="aarch64" ;;
    armv7l|armhf) ARCH_SUFFIX="aarch32" ;;
    *) echo "error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

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
    # cargo produces libtuxinjector.so on Linux
    SRC="$TARGET_DIR/libtuxinjector.so"
    DST="$TARGET_DIR/tuxinjector_${ARCH_SUFFIX}.so"

    if [ ! -f "$SRC" ]; then
        echo "error: $SRC not found" >&2
        exit 1
    fi

    mv "$SRC" "$DST"
    echo ":: Built: $DST"

else
    echo "error: unsupported OS: $OS" >&2
    exit 1
fi

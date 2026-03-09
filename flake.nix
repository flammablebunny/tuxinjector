{
  description = "Tuxinjector - Minecraft speedrunning overlay for Linux & macOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachSystem [
      "x86_64-linux" "aarch64-linux"
      "x86_64-darwin" "aarch64-darwin"
    ] (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        isLinux = pkgs.stdenv.isLinux;
        isDarwin = pkgs.stdenv.isDarwin;

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

        # libs needed at build time (platform-specific)
        buildInputs =
          pkgs.lib.optionals isLinux (with pkgs; [
            libGL
            libGLU
            mesa
            libxkbcommon
            libx11
            libxrandr
            libxinerama
            libxcursor
            libxi
            libxext
            pipewire
            dbus
          ])
          ++ pkgs.lib.optionals isDarwin (with pkgs; [
            # Frameworks (OpenGL, Cocoa, etc.) are provided by the default Darwin SDK in stdenv.
            # Only libiconv is still needed explicitly for some Rust crates.
            libiconv
          ]);

        # minimal nativeBuildInputs for the package build
        pkgNativeBuildInputs = with pkgs; [
          pkg-config
          cmake
          clang
          llvmPackages.libclang
        ];

        # X11 libs that companion apps (nbb) need at runtime (Linux only)
        x11Libs = pkgs.lib.optionalString isLinux (
          pkgs.lib.makeLibraryPath (with pkgs; [
            libxtst
            libxi
            libxt
            libxinerama
            libxkbcommon
            libx11
            libxcb
            libxext
            libxrender
            libxfixes
            libxrandr
            libxcursor
          ])
        );

        tuxinjector = pkgs.rustPlatform.buildRustPackage {
          pname = "tuxinjector";
          version = "1.0.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          inherit buildInputs;
          nativeBuildInputs = pkgNativeBuildInputs;

          env.LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          meta = with pkgs.lib; {
            description = "Minecraft speedrunning overlay for Linux & macOS";
            license = licenses.gpl3;
            platforms = platforms.linux ++ platforms.darwin;
          };
        };

        # Linux wrapper: sets LD_PRELOAD and TUXINJECTOR_X11_LIBS
        linuxWrapper = pkgs.writeShellScriptBin "tuxinjector" ''
          export LD_PRELOAD="${tuxinjector}/lib/libtuxinjector.so"
          export TUXINJECTOR_X11_LIBS="${x11Libs}"
          exec "$@"
        '';

        # macOS wrapper: sets DYLD_INSERT_LIBRARIES
        darwinWrapper = pkgs.writeShellScriptBin "tuxinjector" ''
          export DYLD_INSERT_LIBRARIES="${tuxinjector}/lib/libtuxinjector.dylib"
          exec "$@"
        '';
      in
      {
        devShells.default = pkgs.mkShell {
          inherit buildInputs;
          nativeBuildInputs = pkgNativeBuildInputs ++ (with pkgs; [
            rustToolchain
            python3Packages.mkdocs
            python3Packages.mkdocs-material
            python3Packages.pymdown-extensions
          ]);

          shellHook = ''
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
            ${pkgs.lib.optionalString isLinux ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath buildInputs}:$LD_LIBRARY_PATH"
              export TUXINJECTOR_X11_LIBS="${x11Libs}"
            ''}
            echo "tuxinjector dev shell ready (${system})"
            echo "  cargo build --release    # build the .${if isLinux then "so" else "dylib"}"
            echo "  cargo test               # run tests"
            echo "  cargo clippy             # lint"
            echo "  mkdocs serve             # preview docs"
          '';
        };

        packages.default = if isLinux then linuxWrapper else darwinWrapper;
        packages.lib = tuxinjector;
      }
    );
}

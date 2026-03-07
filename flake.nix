{
  description = "Tuxinjector - Minecraft speedrunning overlay for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

        # libs needed at build time
        buildInputs = with pkgs; [
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
        ];

        # minimal nativeBuildInputs for the package build
        pkgNativeBuildInputs = with pkgs; [
          pkg-config
          cmake
          clang
          llvmPackages.libclang
        ];

        # X11 libs that companion apps (nbb) need at runtime,
        # but the game doesn't load. The wrapper sets TUXINJECTOR_X11_LIBS so
        # the .so can pass them to companion apps using LD_LIBRARY_PATH.
        x11Libs = pkgs.lib.makeLibraryPath (with pkgs; [
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
        ]);

        tuxinjector = pkgs.rustPlatform.buildRustPackage {
          pname = "tuxinjector";
          version = "1.0.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          inherit buildInputs;
          nativeBuildInputs = pkgNativeBuildInputs;

          env.LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          meta = with pkgs.lib; {
            description = "Minecraft speedrunning overlay for Linux";
            license = licenses.gpl3;
            platforms = platforms.linux;
          };
        };

        # Wrapper script for Prism/MCSR Launcher. Sets LD_PRELOAD and TUXINJECTOR_X11_LIBS,
        # then execs the launcher. Use as the launchers wrapper command.
        wrapper = pkgs.writeShellScriptBin "tuxinjector" ''
          export LD_PRELOAD="${tuxinjector}/lib/libtuxinjector.so"
          export TUXINJECTOR_X11_LIBS="${x11Libs}"
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
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath buildInputs}:$LD_LIBRARY_PATH"
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
            export TUXINJECTOR_X11_LIBS="${x11Libs}"
            echo "tuxinjector dev shell ready"
            echo "  cargo build --release    # build the .so"
            echo "  cargo test               # run tests"
            echo "  cargo clippy             # lint"
            echo "  mkdocs serve             # preview docs"
          '';
        };

        packages.default = wrapper;
        packages.lib = tuxinjector;
      }
    );
}

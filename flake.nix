{
  description = "Tuxinjector - Minecraft speedrunning overlay for Linux & macOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    crane.url = "github:ipetkov/crane";

    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
  };

  outputs =
    {
      crane,
      flake-parts,
      ...
    }@inputs:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem =
        {
          pkgs,
          lib,
          self',
          ...
        }:
        {
          packages = {
            default = self'.packages.tuxinjector;
            tuxinjector = pkgs.callPackage ./package.nix {};
          };

          devShells.default =
            let
              craneLib = crane.mkLib pkgs;
            in
            pkgs.mkShell {
              nativeBuildInputs = with pkgs; [
                clang
                pkg-config
                cargo
                rustc
              ];

              buildInputs = [
                pkgs.libclang.lib
              ]
              ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                pkgs.dbus
                pkgs.pipewire
                pkgs.webkitgtk_4_1
                pkgs.gtk3
              ]
              ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
                pkgs.libiconv
              ];

              packages = with pkgs; [
                clippy
                rust-analyzer
                rustfmt
                python3Packages.mkdocs
                python3Packages.mkdocs-material
                python3Packages.pymdown-extensions
              ];

              env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

              shellHook = ''
                echo "tuxinjector dev shell ready"
                echo "  ./build.sh               # build the .so (building and using a local binary is not MCSR Ranked / Speedrun.com legal)"
                echo "  cargo clippy             # lint"
                echo "  mkdocs serve             # preview docs"
              '';
            };

          formatter = pkgs.nixfmt-tree;
        };
    };
}

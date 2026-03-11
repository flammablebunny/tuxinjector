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
          self',
          ...
        }:
        {
          packages = {
            default = self'.packages.tuxinjector;
            tuxinjector = pkgs.callPackage ./package.nix { craneLib = crane.mkLib pkgs; };
          };

          devShells.default = pkgs.mkShell {
            inputsFrom = [ self'.packages.default ];

            packages = with pkgs; [
              clippy
              rust-analyzer
              rustfmt
              python3Packages.mkdocs
              python3Packages.mkdocs-material
              python3Packages.pymdown-extensions
            ];

            # needed for clippy
            env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

            shellHook = ''
              echo "tuxinjector dev shell ready"
              echo "  nix build                # build the .so & run tests"
              echo "  cargo clippy             # lint"
              echo "  mkdocs serve             # preview docs"
            '';
          };

          formatter = pkgs.nixfmt-tree;
        };
    };
}

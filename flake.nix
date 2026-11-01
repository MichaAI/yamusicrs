{
  description = "yamusicrs — TUI-клиент Яндекс Музыки";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      crane,
    }:
    let
      mkYamusicrs = pkgs: pkgs.callPackage ./nix/package.nix { craneLib = crane.mkLib pkgs; };
    in
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        yamusicrs = mkYamusicrs pkgs;
      in
      {
        packages = {
          default = yamusicrs;
          inherit yamusicrs;
        };

        apps.default = flake-utils.lib.mkApp { drv = yamusicrs; };

        # Для разработки: `nix develop -c cargo build --release` — инкрементально в ./target.
        devShells.default = pkgs.mkShell {
          inputsFrom = [ yamusicrs ];
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          # Как в пакете: без rpath wgpu не найдёт libvulkan.so.1 и блоб уйдёт на CPU.
          RUSTFLAGS = "-C link-arg=-Wl,-rpath,${yamusicrs.gpuLibs}";
        };

        checks = {
          package = yamusicrs;
          inherit (yamusicrs) tests clippy fmt;
        };

        formatter = pkgs.nixfmt;
      }
    )
    // {
      overlays.default = final: _prev: { yamusicrs = mkYamusicrs final; };
      homeManagerModules.default = import ./nix/hm-module.nix self;
      nixosModules.default = import ./nix/nixos-module.nix self;
    };
}

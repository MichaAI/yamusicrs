{
  description = "yamusicrs — TUI-клиент Яндекс Музыки";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        nativeBuildInputs = with pkgs; [ pkg-config rustPlatform.bindgenHook ];
        buildInputs = with pkgs; [ pipewire ];
        # wgpu подгружает Vulkan/GL на лету — прописываем пути прямо в бинарь (rpath).
        gpuLibs = pkgs.lib.makeLibraryPath (with pkgs; [ vulkan-loader libGL ]);
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "yamusicrs";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          inherit nativeBuildInputs buildInputs;
          RUSTFLAGS = "-C link-arg=-Wl,-rpath,${gpuLibs}";
        };

        devShells.default = pkgs.mkShell {
          inherit buildInputs;
          RUSTFLAGS = "-C link-arg=-Wl,-rpath,${gpuLibs}";
          nativeBuildInputs = nativeBuildInputs ++ (with pkgs; [ cargo rustc rustfmt clippy rust-analyzer ]);
        };
      });
}

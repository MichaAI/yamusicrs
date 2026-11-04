{
  description = "yamusicrs — TUI-клиент Яндекс Музыки";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
    # Rust со стандартной библиотекой под Android — только для devShell android.
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      crane,
      fenix,
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
        devShells = {
          default = pkgs.mkShell {
            inputsFrom = [ yamusicrs ];
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
            ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            # Как в пакете: линковка mold'ом, rpath на Vulkan (без него wgpu не найдёт
            # libvulkan.so.1 и блоб уйдёт на CPU).
            RUSTFLAGS = "${yamusicrs.commonArgs.RUSTFLAGS} -C link-arg=-Wl,-rpath,${yamusicrs.gpuLibs}";
          };
        }
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          # Android-клиент: `nix develop .#android -c ./gradlew -p android assembleDebug`.
          android =
            let
              apkgs = import nixpkgs {
                inherit system;
                config = {
                  allowUnfree = true;
                  android_sdk.accept_license = true;
                };
              };
              ndkVersion = "29.0.14206865";
              buildTools = "36.1.0";
              android = apkgs.androidenv.composeAndroidPackages {
                platformVersions = [ "37.2" ];
                buildToolsVersions = [ buildTools ];
                includeNDK = true;
                ndkVersions = [ ndkVersion ];
                includeEmulator = false;
                includeSystemImages = false;
                includeSources = false;
              };
              sdk = "${android.androidsdk}/libexec/android-sdk";
              rust =
                with fenix.packages.${system};
                combine [
                  stable.cargo
                  stable.rustc
                  targets.aarch64-linux-android.stable.rust-std
                  targets.x86_64-linux-android.stable.rust-std
                ];
            in
            apkgs.mkShell {
              packages = [
                rust
                apkgs.cargo-ndk
                apkgs.jdk21
                apkgs.cmake
                android.platform-tools
              ];
              ANDROID_HOME = sdk;
              ANDROID_SDK_ROOT = sdk;
              ANDROID_NDK_HOME = "${sdk}/ndk/${ndkVersion}";
              JAVA_HOME = apkgs.jdk21.home;
              # Свой target: другой rustc, чем в основной оболочке, — не перемешивать артефакты.
              CARGO_TARGET_DIR = "target/android";
              # aapt2 из Maven динамически слинкован и на NixOS не запускается — берём из SDK.
              GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${sdk}/build-tools/${buildTools}/aapt2";
            };
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

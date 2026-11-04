{
  lib,
  craneLib,
  pkg-config,
  patchelf,
  mold,
  rustPlatform,
  pipewire,
  vulkan-loader,
  libGL,
  # Команда терминала для ярлыка, например "kitty" или "kitty --class yamusicrs".
  # null — ярлык с Terminal=true, терминал выберет окружение рабочего стола.
  desktopTerminal ? null,
}:

let
  # wgpu открывает Vulkan/GL через dlopen — их нет в NEEDED, поэтому путь прописываем в rpath сами.
  gpuLibs = lib.makeLibraryPath [
    vulkan-loader
    libGL
  ];

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../data
    ];
  };

  commonArgs = {
    inherit src;
    # Корень — workspace без [package]: имя и версию задаём сами; ставится бинарь крейта tui.
    pname = "yamusicrs";
    version = "0.1.0";
    cargoExtraArgs = "--locked -p yamusicrs";
    strictDeps = true;
    # Профиль с LTO из Cargo.toml; `release` оставлен быстрым для разработки.
    CARGO_PROFILE = "dist";
    # mold — линковка в разы быстрее обычного ld.
    RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
    nativeBuildInputs = [
      pkg-config
      mold
      rustPlatform.bindgenHook
    ];
    buildInputs = [ pipewire ];
  };

  # Зависимости собираются отдельной derivation и берутся из store, пока не менялся Cargo.lock:
  # правка исходников пересобирает только сам yamusicrs.
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  yamusicrs = craneLib.buildPackage (
    commonArgs
    // {
      inherit cargoArtifacts;
      # Тесты — отдельной проверкой (checks.test), чтобы пакет не компилировался дважды.
      doCheck = false;
      nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ patchelf ];

      postInstall = ''
        install -Dm644 data/yamusicrs.svg $out/share/icons/hicolor/scalable/apps/yamusicrs.svg
        install -Dm644 data/yamusicrs.desktop $out/share/applications/yamusicrs.desktop
      ''
      + lib.optionalString (desktopTerminal != null) ''
        substituteInPlace $out/share/applications/yamusicrs.desktop \
          --replace-fail "Exec=yamusicrs" "Exec=${desktopTerminal} -e $out/bin/yamusicrs" \
          --replace-fail "Terminal=true" "Terminal=false"
      '';

      # patchelf --shrink-rpath в fixup выкидывает «неиспользуемые» пути, поэтому добавляем после него.
      postFixup = ''
        patchelf --add-rpath ${gpuLibs} $out/bin/yamusicrs
      '';

      passthru = {
        inherit gpuLibs cargoArtifacts commonArgs;
        tests = craneLib.cargoTest (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "--locked --workspace";
          }
        );
        clippy = craneLib.cargoClippy (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "--locked --workspace";
            cargoClippyExtraArgs = "--all-targets";
          }
        );
        fmt = craneLib.cargoFmt { inherit src; };
      };

      meta = {
        description = "TUI-клиент Яндекс Музыки: lossless, PipeWire, kitty-обложки";
        license = lib.licenses.asl20;
        mainProgram = "yamusicrs";
        platforms = lib.platforms.linux;
      };
    }
  );
in
yamusicrs

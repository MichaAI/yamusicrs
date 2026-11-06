# nix

Упаковка и модули. Корневой `flake.nix` собирает всё из этой папки.

| Файл | Что даёт |
|---|---|
| `package.nix` | пакет на [crane](https://github.com/ipetkov/crane): зависимости отдельной derivation, профиль `dist` (thin LTO), линковка mold, rpath на Vulkan/GL, ярлык и иконка |
| `hm-module.nix` | `programs.yamusicrs` для home-manager |
| `nixos-module.nix` | `programs.yamusicrs` для NixOS |

## Выходы флейка

```sh
nix build                      # ./result/bin/yamusicrs
nix run . -- --help
nix develop                    # оболочка разработки (cargo, clippy, rust-analyzer)
nix develop .#android          # оболочка для Android-сборки (только x86_64-linux)
nix flake check                # package, tests, clippy, fmt
nix fmt                        # nixfmt
```

Также: `overlays.default` (добавляет `pkgs.yamusicrs`), `nixosModules.default`,
`homeManagerModules.default`. Системы: `x86_64-linux`, `aarch64-linux`.

## home-manager

```nix
{
  inputs.yamusicrs.url = "path:/home/me/projects/yamusicrs"; # или git-URL
  # …
  imports = [ inputs.yamusicrs.homeManagerModules.default ];
  programs.yamusicrs = {
    enable = true;
    desktopTerminal = "kitty";   # ярлык запустит `kitty -e yamusicrs`
    settings = {                 # → ~/.config/yamusicrs/config.toml; пусто — файл не трогаем
      quality = "lossless";
      library_max_gb = 50;
    };
  };
}
```

Параметры: `enable`, `package`, `desktopTerminal` (null — `Terminal=true`, терминал выберет
окружение), `settings` (TOML-атрибуты, ключи как в `config.toml`).

## NixOS

```nix
imports = [ inputs.yamusicrs.nixosModules.default ];
programs.yamusicrs.enable = true;
```

Ставит пакет и (при `hiResRates = true`, по умолчанию) разрешает PipeWire частоты
44.1–192 кГц, чтобы lossless не ресемплился в 48 кГц. Отключить: `programs.yamusicrs.hiResRates = false;`.

## Оверлей без флейков в системе

```nix
nixpkgs.overlays = [ inputs.yamusicrs.overlays.default ];
environment.systemPackages = [ pkgs.yamusicrs ];
```

## Заметки

- `CARGO_PROFILE = "dist"` только в пакете; в разработке `cargo build --release` инкрементальный и быстрый.
- Меняете `Cargo.lock` — пересоберутся зависимости (долго), меняете код — только сам крейт.
- В исходники пакета входят только `Cargo.*`, `crates/` и `data/`; правки в `docs/`, `android/` пакет не пересобирают.

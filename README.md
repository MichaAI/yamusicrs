# yamusicrs

Неофициальный клиент Яндекс Музыки на Rust. Два интерфейса поверх одного ядра:

- **TUI для Linux** — lossless через нативный PipeWire, Моя волна, синхронные тексты,
  обложки в kitty, локальная библиотека из обычных файлов с тегами, MPRIS.
- **Android-приложение** — Jetpack Compose (Material 3 Expressive), то же ядро через UniFFI,
  звук через AAudio, офлайн-скачивание.

Оба клиента умеют Ynison: видны на других устройствах в «Слушать на…» и управляются с них.

## Содержание

- [Состав репозитория](#состав-репозитория)
- [Установка и сборка TUI](#установка-и-сборка-tui)
- [Сборка Android-приложения](#сборка-android-приложения)
- [Запуск и команды](#запуск-и-команды)
- [Вход](#вход) · [Звук](#звук) · [Качество и библиотека](#качество-и-библиотека)
- [Клавиши](#клавиши) · [Настройки](#настройки) · [Файлы](#файлы)
- [Разработка](#разработка)

## Состав репозитория

| Путь | Что там |
|---|---|
| [`crates/core`](crates/README.md) | ядро без UI: API, аудиодвижок, библиотека, офлайн-волна, Ynison |
| [`crates/tui`](crates/README.md) | десктопный TUI (бинарь `yamusicrs`), MPRIS, GPU-блоб |
| [`crates/android`](crates/README.md) | UniFFI-мост ядра для Android, вывод через AAudio |
| [`crates/uniffi-bindgen`](crates/README.md) | генератор Kotlin-привязок |
| [`android/`](android/README.md) | Gradle-проект приложения (Kotlin, Compose) |
| [`nix/`](nix/README.md) | пакет (crane), модули NixOS и home-manager |
| [`docs/`](docs/README.md) | заметки о протоколах: Ynison, эндпоинты официальных клиентов |
| [`tools/`](tools/README.md) | вспомогательные скрипты |
| `data/` | ярлык `.desktop` и иконка |

## Установка и сборка TUI

Нужен Linux с PipeWire (≥ 0.3.49) и Vulkan (или GL) для анимированного блоба;
без GPU блоб рисуется на CPU.

### Nix (рекомендуется)

```sh
nix run .                         # из клона репозитория, без установки
nix profile install .             # в профиль: бинарь, ярлык в меню, иконка
```

Флейк отдаёт `packages.default`, `apps.default`, `overlays.default`,
`nixosModules.default` и `homeManagerModules.default` — подробности и примеры
конфигурации в [nix/README.md](nix/README.md).

### Без Nix

Зависимости: Rust (edition 2024, свежий stable), `pkg-config`, `clang`/libclang
(bindgen для PipeWire), заголовки `libpipewire-0.3`, рантайм `libvulkan`.

```sh
# Debian/Ubuntu: libpipewire-0.3-dev libclang-dev pkg-config libvulkan1
# Arch:          pipewire clang pkgconf vulkan-icd-loader
cargo build --release             # → target/release/yamusicrs
cargo install --path crates/tui   # в ~/.cargo/bin
```

Ярлык и иконка: `data/yamusicrs.desktop` → `~/.local/share/applications/`,
`data/yamusicrs.svg` → `~/.local/share/icons/hicolor/scalable/apps/`.

Профили: `release` — быстрая инкрементальная сборка для разработки,
`dist` (`--profile dist`) — с thin LTO, меньше и быстрее; им собирается Nix-пакет.

## Сборка Android-приложения

Коротко (всё окружение — JDK 21, Android SDK/NDK, Rust-таргеты — даёт dev-оболочка):

```sh
nix develop .#android -c ./android/gradlew -p android assembleRelease
# APK: android/app/build/outputs/apk/release/app-release.apk
adb install -r android/app/build/outputs/apk/release/app-release.apk
```

Подробно, в том числе без Nix, — в [android/README.md](android/README.md).

## Запуск и команды

`yamusicrs --help` — подробно.

```
yamusicrs                      плеер
yamusicrs --offline            сразу без сети: снимок библиотеки и офлайн-волна из скачанного
yamusicrs login [--token Т]    войти заново: по коду с QR или готовым токеном/ссылкой
yamusicrs logout               забыть токен
yamusicrs paths                где конфиг, библиотека, состояние и лог
yamusicrs debug play ФАЙЛ…     проиграть файлы без интерфейса (--crossfade СЕК)
yamusicrs debug tinyml П З     прогнать программу офлайн-волны
```

## Вход

При первом запуске показывается код и QR: открой `ya.ru/device`, введи код —
готово. Либо вставь OAuth-токен (или целиком ссылку с `#access_token=…`) прямо
в окно входа. Токен лежит в `~/.local/share/yamusicrs/token.json` (права `0600`)
и обновляется через refresh token, если он был выдан.

## Звук

- Декодирование — symphonia, вывод — собственный поток PipeWire в F32 с частотой
  исходника (`node.rate`), без cpal/ALSA-прослоек.
- Gapless: следующий трек открывается заранее и декодируется встык. При смене
  частоты поток пересоздаётся (~50 мс).
- Громкость 100% — множитель ровно 1.0, т.е. bit-perfect до PipeWire.
- Нормализация по R128 (`normalize = true`) выключена по умолчанию.

Чтобы PipeWire не ресемплил в 48 кГц, разреши частоты графа. NixOS:

```nix
services.pipewire.extraConfig.pipewire."92-hires" = {
  "context.properties"."default.clock.allowed-rates" = [ 44100 48000 88200 96000 176400 192000 ];
};
```

Для Bluetooth (LDAC и др.) частоту всё равно задаёт кодек — тут важно только,
что на вход PipeWire идёт lossless.

## Качество и библиотека

`Q` переключает качество стрима: **Lossless → Высокое → Экономия** (запоминается).

Каждый прослушанный трек сохраняется в `~/Music/YandexMusic`:

```
Artist/Album (Year)/07. Title.flac
```

- FLAC из MP4-контейнера Яндекса ремуксится в нативный `.flac` побитово
  (STREAMINFO с MD5 + SEEKTABLE), AAC — в `.m4a`, MP3 — `.mp3`.
- Теги: название, исполнители, альбом, дата, номер трека/диска, жанр, ReplayGain
  (из R128 Яндекса), текст песни, обложка 1000×1000, ссылка на трек.
- Если в библиотеке трек хуже выбранного качества — он играет сразу, а лучшая
  версия докачивается в фоне и заменяет файл. Если лучше нет — перепроверка раз в 30 дней.
- Лимит размера: `library_max_gb` в конфиге (0 — без лимита), вытесняются давно не игранные.

## Клавиши

| | |
|---|---|
| `1`–`7`, `Tab` | Волна · Коллекция · Плейлисты · Поиск · Очередь · Новинки · Настройки |
| `↑↓`/`jk`, `PgUp/PgDn`, `g/G` | навигация |
| `Enter` / `Esc` | играть/открыть / назад |
| `Space`, `n`, `p` | пауза, следующий, предыдущий |
| `←→` (`Shift`) | перемотка 5 с (30 с) |
| `+` `-` | громкость |
| `l` / `L` | ♥ играющий / выделенный |
| `d` | «не рекомендовать» (волна) |
| `a` / `A` | в конец очереди / следующим |
| `o` / `i` | альбом / исполнитель |
| `w` | Моя волна (настройки — на вкладке Волна) |
| `t` | текст на весь экран |
| `r` / `s` | повтор трека / перемешивание |
| `Q` | качество |
| `?` | справка |

MPRIS поддерживается: медиаклавиши и `playerctl` работают. Имя на шине —
`org.mpris.MediaPlayer2.yamusicrs`, переопределяется `YAMUSICRS_MPRIS_NAME`
(например, для второго экземпляра: иначе он заберёт имя у первого).

## Настройки

Большинство параметров переключается во вкладке `7` (Настройки); то же самое
можно править в `~/.config/yamusicrs/config.toml` (файл создаётся при первом запуске,
нераспознанный файл заменяется значениями по умолчанию, в логе будет предупреждение).

| Параметр | По умолчанию | Смысл |
|---|---|---|
| `quality` | `lossless` | `lossless` / `hq` / `nq` |
| `library_dir` | `~/Music/YandexMusic` | корень локальной библиотеки |
| `library_max_gb` | `0` | лимит библиотеки, ГиБ (0 — без лимита) |
| `volume` | `0.8` | громкость при старте |
| `normalize`, `normalize_target_lufs` | `true`, `-14` | нормализация R128 (ломает bit-perfect) |
| `crossfade_secs` | `3` | кроссфейд, с; 0 — встык |
| `kitty_images` | авто | обложки через kitty graphics protocol |
| `gpu`, `blob_dither`, `fps` | `true`, `1`, `30` | отрисовка блоба и частота анимаций |
| `report_plays` | `true` | отправлять историю прослушиваний в Яндекс |
| `ynison` | `true` | видимость и управление с других устройств |
| `fast_start`, `warm_ahead` | `true`, `5` | быстрый старт в AAC с подменой на lossless; прогрев очереди |
| `wave_after_list` | `true` | после списка продолжать волной |
| `offline_mode` | `false` | всегда без сети (как `--offline`) |
| `smart_download`, `smart_quality`, `smart_limit_gb` | `false`, `hq`, `2` | умное скачивание для офлайн-волны |
| `smart_from_hour`, `smart_to_hour`, `smart_wifi_only` | `0`, `0`, `true` | окно и условия докачки |

Полный список с комментариями — в [`crates/core/src/config.rs`](crates/core/src/config.rs).

## Файлы

`yamusicrs paths` печатает актуальные пути.

- `~/.config/yamusicrs/config.toml` — настройки
- `~/.local/share/yamusicrs/` — токен, индекс библиотеки, снимок для офлайна
- `~/.local/state/yamusicrs/` — лог (`YAMUSICRS_LOG=yamusicrs=debug` для подробного), состояние
- `~/.cache/yamusicrs/` — кеш картинок
- `~/Music/YandexMusic/` — библиотека

Переменные окружения: `YAMUSICRS_LOG` (фильтр tracing), `YAMUSICRS_MPRIS_NAME`
(второй экземпляр; такой экземпляр не включает Ynison, если не задать `YAMUSICRS_YNISON=1`).

## Разработка

```sh
nix develop                        # или direnv: .envrc уже есть
cargo build --release              # инкрементально в ./target
cargo test --workspace             # тесты (часть помечена #[ignore] и требует файлов)
nix flake check                    # пакет, тесты, clippy, rustfmt
nix fmt                            # nixfmt для .nix
cargo fmt                          # rustfmt.toml в корне
```

Пакет собран на [crane](https://github.com/ipetkov/crane): зависимости — отдельная
derivation, `nix build` после правки кода пересобирает только сам yamusicrs.
В devshell и пакете путь к `libvulkan` вшит в rpath, иначе wgpu его не найдёт и
блоб уйдёт на CPU. Запуская тестовый экземпляр рядом с рабочим, задавайте
`YAMUSICRS_MPRIS_NAME`.

API неофициальный; описание взято из [MarshalX/yandex-music-api](https://github.com/MarshalX/yandex-music-api)
и разбора официальных клиентов ([docs](docs/README.md)). Проект не связан с Яндексом.

## Лицензия

Apache-2.0, см. [LICENSE](LICENSE). Шрифт Nunito в Android-приложении — под SIL OFL
(`android/app/src/main/assets/licenses/`).

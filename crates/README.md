# crates

Cargo workspace (`Cargo.toml` в корне; по умолчанию собирается только `tui`).

```
yamusic-core ◄── yamusicrs (tui)
     ▲
     └────────── yamusic-android ◄── uniffi-bindgen (генерация Kotlin)
```

## core — `yamusic-core`

Ядро без UI. Всё, что не зависит от платформы.

| Модуль | Назначение |
|---|---|
| `api/` | клиент Яндекс Музыки, авторизация (код/QR, токен, refresh), модели |
| `audio/` | декодирование (symphonia), ресемплер, источник данных, вывод через PipeWire |
| `engine/` | плеер: очередь, gapless/кроссфейд, волна, быстрый старт, Ynison-режим |
| `library.rs` | локальная библиотека: ремукс в `.flac`/`.m4a`/`.mp3`, теги, лимит размера |
| `offline.rs`, `offline_wave.rs`, `tinyml.rs` | офлайн-режим и офлайн-волна (программа TinyMl от сервера) |
| `lyrics.rs` | синхронные тексты |
| `ynison.rs`, `ynison_pb.rs` | синхронизация устройств (WebSocket + protobuf) |
| `config.rs` | `config.toml`, пути |

Фича `pipewire` включает вывод звука в PipeWire (нужна десктопу; Android её не берёт).

```sh
cargo test -p yamusic-core
# проба Ynison (пассивно регистрируется устройством на 20 с):
cargo run --release -p yamusic-core --example ynison_probe -- ~/.local/share/yamusicrs/token.json
```

## tui — `yamusicrs`

Бинарь. ratatui + crossterm, обложки по kitty graphics protocol, анимированный блоб
на wgpu (`render/blob.wgsl`, запасной путь — CPU), MPRIS на zbus.

```sh
cargo run --release                 # = cargo run --release -p yamusicrs
cargo run --release -- debug play a.flac b.flac --crossfade 3
```

Системные зависимости: `libpipewire-0.3`, libclang (bindgen), `pkg-config`; Vulkan/GL —
в рантайме. Подробности — в корневом [README](../README.md).

## android — `yamusic-android`

`cdylib`, мост для приложения: объект `Core` (UniFFI) принимает команды и шлёт состояние
колбэками; вывод звука — AAudio (`aaudio.rs`), лог — в logcat. Без таргета Android
крейт собирается как обычная библиотека (для проверок), но используется через Gradle:
см. [android/README.md](../android/README.md).

## uniffi-bindgen

Тонкая обёртка над `uniffi` CLI в library-mode. Вручную обычно не вызывается — это делает Gradle:

```sh
cargo run --release -p uniffi-bindgen -- generate --library path/to/libyamusic_android.so \
  --language kotlin --out-dir out --no-format
```

# Ynison — синхронизация устройств (заметки к реализации)

Восстановлено 2026-11-04 по десктопному клиенту 5.63.1 (`yandex-music.asar` из nix store,
бандл Next.js, поиск по строкам `ynison`, `update_full_state`, `GetRedirectToYnison`).
Android 2026.09.5 использует тот же протокол.

Что даёт: yamusicrs виден на телефоне как устройство, с телефона — пауза/перемотка/треки/громкость;
перехват очереди (включая волну) между устройствами с той же секунды.

## Подключение

Оба шага — WebSocket. Заголовки браузер передать не может, поэтому они идут в
`Sec-WebSocket-Protocol`: три протокола `Bearer`, `v2`, `encodeURIComponent(JSON.stringify(headers))`.

### 1. Redirector

`wss://ynison.music.yandex.ru/redirector.YnisonRedirectService/GetRedirectToYnison`

```json
{
  "Ynison-Device-Id": "<стабильный id устройства>",
  "Ynison-Device-Info": "{\"app_name\":\"…\",\"app_version\":\"…\",\"type\":1}",
  "authorization": "OAuth <token>"
}
```

`type` в Device-Info — число: десктоп шлёт 1 (WEB), WEB_TV → 5.
(Опционально `X-Yandex-Music-Multi-Auth-User-Id`.)

Ответ — одно сообщение и закрытие: `{host, redirect_ticket, session_id}` или `{error: …}`.

### 2. Hub

`wss://<host>/ynison_state.YnisonStateService/PutYnisonState`, заголовки как выше плюс
`Ynison-Redirect-Ticket` и `Ynison-Session-Id`. Переподключение: таймаут 2 с, 2 попытки,
потом снова через redirector.

## Сообщения (protobuf в JSON-маппинге, snake_case)

Каждый запрос клиента — объект с одним из полей-действий плюс:
- `rid` — uuid запроса;
- `player_action_timestamp_ms` — `Date.now()` (для `update_full_state` — 0);
- `activity_interception_type` — `"DO_NOT_INTERCEPT_BY_DEFAULT"`.

Действия: `update_full_state`, `update_player_state`, `update_playing_status`,
`update_active_device`, `update_volume_info`, `update_session_params`, `sync_state_from_eov`.

Версия (у очереди, статуса, громкости):
`{device_id, version: random u63, timestamp_ms}`; в `update_full_state` timestamp = 0.

### update_full_state (первым после подключения)

```js
{
  device: {
    info: {device_id, title, type: "WEB", app_name, app_version},
    capabilities: {can_be_player: true, can_be_remote_controller: false, volume_granularity: 16},
    volume_info: {volume: 1, version: null},
    is_shadow: false
  },
  is_currently_active: false,
  sync_state_from_eov_optional: null,
  player_state: {
    status: {duration_ms, progress_ms, paused: true, playback_speed: 1, version},
    player_queue: {
      entity_id, entity_type,           // см. перечисления
      current_playable_index,           // -1 — пусто
      playable_list: [{playable_id, playable_type: "TRACK", album_id_optional, from, title, cover_url_optional, track_info?}],
      shuffle_optional: null | {playable_indices: [...]},
      options: {repeat_mode: "NONE"|"ONE"|"ALL"},
      entity_context: "BASED_ON_ENTITY_BY_DEFAULT",
      from_optional,
      initial_entity_optional: null | {entity_id, entity_type},
      adding_options_optional: null | {radio_options: {session_id: <rotor radioSessionId>}},
      queue: null,
      version
    },
    player_queue_inject_optional: null
  }
}
```

Для волны (`GENERATIVE`/радио) `progress_ms` десктоп шлёт 0.

### Ответ сервера

Полное состояние: `player_state` (очередь+статус), `devices[]`, `active_device_id_optional`,
либо `{error}`. Десктоп применяет чужой diff только когда сам не играет
(ENDED/IDLE/PAUSED/STOPPED), иначе игнорирует. Из статуса берёт `progress_ms` → seek
(или стартовая позиция, если не играет), `playback_speed`, `repeat_mode`, shuffle.

## Перечисления

- device type: `UNSPECIFIED WEB ANDROID IOS SMART_SPEAKER WEB_TV ANDROID_TV APPLE_TV`
- repeat_mode: `UNSPECIFIED NONE ONE ALL`
- playable_type: `UNSPECIFIED TRACK LOCAL_TRACK INFINITE VIDEO_CLIP`
- entity_type: `ARTIST PLAYLIST ALBUM RADIO VARIOUS GENERATIVE FM_RADIO VIDEO_WAVE LOCAL_TRACKS`
- entity_context: `BASED_ON_ENTITY_BY_DEFAULT`
- activity_interception_type: `DO_NOT_INTERCEPT_BY_DEFAULT`

## Что выяснилось при реализации (2026-11-04)

- Десктоп 5.63.1 и веб-клиент подключаются **только «тенью»**: `variables: {shadow: true}`,
  `is_shadow: true`, `can_be_remote_controller: false`, `app_name: "Browser"`,
  `title: "Music Desktop App"`, `type: WEB`, `device_id` — новый uuid на каждый запуск.
  Тень публикует свою очередь (`update_player_state`, статус всегда `paused: true`;
  пауза/позиция — `update_playing_status`) и применяет чужой diff, только когда сама не играет.
  Управлять тенью с телефона нельзя.
- Полноценное управляемое устройство есть только в Android, и там бинарный gRPC
  (`ynison.music.yandex.net:443`, `ynison_redirect.YnisonRedirectService/GetRedirectToYnison`),
  а не JSON-WebSocket.
- Пользователь выбрал **полноценный плеер** (`is_shadow: false`) поверх JSON-WebSocket.
- Первая проба (тень, как десктоп) 2026-11-04: redirector ответил нормально
  (`host`, `redirect_ticket`, `session_id`, `keep_alive_params`), хаб — сразу
  `{"error": {"grpc_code": 14, "http_code": 503, "message": "Ynison service is temporary unavailable.",
  "details": {"ynison-error-code": "500140000", "ynison-go-away-for-seconds": "3600"}}}`
  при app_version и 5.63.1, и 5.95.0. Сбой это или отказ нашему клиенту — не выяснено;
  go-away соблюдать.
- Для тестов можно использовать Waydroid с официальным Android-приложением (вместо телефона).
- Пробный скрипт: подключение тенью и дамп сообщений хаба — `tools/ynison_probe.py`
  (python + websockets, заголовки в subprotocols, Origin `https://music.yandex.ru`).

## Осторожно

- При подключении не забирать активность у телефона (`is_currently_active: false`,
  `paused: true`), иначе перебьём воспроизведение там.
- app_name/app_version — как у официального клиента (антифрод), см. `DESKTOP_CLIENT` в `crates/core/src/api/mod.rs`.
- Конфликты решаются версиями; свою версию генерировать заново на каждое изменение.

## Где взять исходники снова

- Десктоп: `nix run nixpkgs#asar -- extract /nix/store/*-yandex-music-*/share/nodejs/yandex-music.asar out`,
  дальше искать по строкам в `out/app/_next/static/chunks`.
- Android: см. память о реверсе APK (adb pull + `jadx --no-res`).

## Реализация в yamusicrs (`crates/core/src/ynison.rs`, `crates/core/src/engine/yn.rs`)

- Транспорт как у Music Assistant (их провайдер проверен с официальным Android вживую):
  OAuth — HTTP-заголовком `Authorization`, `Origin: https://music.yandex.ru`, subprotocol
  `Bearer, v2, {JSON}` без URL-кодирования; все числа версий/времени — строками.
- Устройство: постоянный `device_id` (`<data>/ynison-device-id`), `title` «yamusicrs · хост»,
  `type: WEB`, `can_be_player: true`, без `is_shadow`. При подключении — пустая очередь с
  `timestamp_ms: "0"` и `is_currently_active: false` (ничего не перехватывает), затем
  `update_session_params {mute_events_if_passive: true}`.
- Команды других устройств — только с автором версии не мы, не пустой и не `server-…`,
  и только новые (повторы той же версии игнорируются).
- Активность ушла (или `server-paused-on-active-device-disconnecting`) — пауза здесь.
- Включили что-то здесь, пока активен другой, — `update_active_device` себе + очередь.
- RADIO с `wave_queue…session_id` продолжается той же сессией ротора.
- Тестовые экземпляры (`YAMUSICRS_MPRIS_NAME`) к Ynison не подключаются без `YAMUSICRS_YNISON=1`;
  `YAMUSICRS_YNISON=0` выключает всегда; в настройках — «Ynison (Слушать на…)».
- Live-проверка ещё не сделана: хаб отвечал 503 + go-away.

Исследование MA: github.com/trudenboy/ma-provider-yandex-ynison,
`docs/research/2026-09-01-live-ynison-official-client.md`.

## gRPC, как у Android (2026-11-05)

С середины октября 2026 хаб не отдаёт JSON-WebSocket сторонним клиентам: сразу 503 +
`ynison-error-code 500140000` + `go-away 3600` (у Music Assistant так же на любых аккаунтах, IP
и device id; веб music.yandex.ru тоже не подключается). Работают только мобильные приложения,
а они ходят по gRPC. Транспорт yamusicrs переведён на gRPC (`crates/core/src/ynison.rs`,
схема и кодек protobuf ↔ JSON — `crates/core/src/ynison_pb.rs`).

- Redirector: unary `https://ynison.music.yandex.net/ynison_redirect.YnisonRedirectService/GetRedirectToYnison`,
  запрос — пустое сообщение, ответ `{host=1, redirect_ticket=2, session_id=3 int64, keep_alive_params=4}`.
- Хаб: двунаправленный стрим `https://<host>/ynison_state.YnisonStateService/PutYnisonState`.
- Метаданные (HTTP/2-заголовки): `authorization: OAuth …`, `Ynison-Device-Id`,
  `Ynison-Device-Info` — **base64 без паддинга от protobuf** `{type=1 enum, app_name=2, app_version=3}`,
  `Ynison-Redirect-Ticket`, `Ynison-Session-Id`. Ответные: `Ynison-Go-Away-For-Seconds`,
  `Ynison-Backoff-Millis`, `Ynison-Error-Code` (в заголовках или трейлерах вместе с `grpc-status`).
- User-Agent официального клиента: `grpc-java-okhttp/1.61.1`.
- Официальный клиент представляется `type ANDROID, app_name "Yandex Music",
  app_version "2026.09.5 #167.1gpr"` (класс `oby.c()`).

Схема восстановлена скриптом по строкам `newMessageInfo` protobuf-lite (имена полей в APK
не обфусцированы): `PutYnisonStateRequest` (класс `uco`) — oneof действий 1 update_full_state,
2 update_active_device, 3 update_playing_status, 4 update_player_state, 5 update_volume,
6 update_player_queue_inject, 9 update_session_params, 11 update_volume_info,
12 sync_state_from_eov; плюс 7 player_action_timestamp_ms, 8 rid, 10 activity_interception_type.
Ответ (`wco`): 1 player_state, 2 devices, 3 active_device_id_optional (обёртка `{value}`),
4 timestamp_ms, 5 rid. Полностью — в `ynison_pb.rs`.

Числа перечислений (из APK): activity_interception_type DO_NOT_INTERCEPT_BY_DEFAULT=**0**,
INTERCEPT_IF_NO_ONE_ACTIVE=1, INTERCEPT_EAGER=2; entity_type UNSPECIFIED=0 ARTIST PLAYLIST ALBUM
RADIO VARIOUS GENERATIVE FM_RADIO VIDEO_WAVE LOCAL_TRACKS=9; entity_context
BASED_ON_ENTITY_BY_DEFAULT=0 USER_TRACKS DOWNLOADED_TRACKS SEARCH MUSIC_HISTORY
MUSIC_HISTORY_SEARCH ARTIST_MY_COLLECTION ARTIST_FAMILIAR_FROM_WAVE=7; repeat UNSPECIFIED=0
NONE ONE ALL; playable_type UNSPECIFIED=0 TRACK LOCAL_TRACK INFINITE VIDEO_CLIP; device type
(имён в APK нет, порядок из десктопа) UNSPECIFIED=0 WEB=1 ANDROID=2 IOS SMART_SPEAKER WEB_TV
ANDROID_TV APPLE_TV, всего значений 10.

Проверка 2026-11-05 10:16 MSK (gRPC, `app_name "yamusicrs"`, type WEB): транспорт рабочий —
redirector ответил настоящим gRPC-статусом 14 с `ynison-error-code 500140000`, но go-away уже
**43200 с** (утренняя JSON-проба получила 3600): каждая попытка, похоже, продлевает отказ.
go-away теперь запоминается в `<data>/ynison-go-away` и соблюдается после перезапуска.
Открытый вопрос: пускает ли хаб по `app_name`/типу (официальный клиент — "Yandex Music", ANDROID).

Пульт: устройство подключается с `can_be_remote_controller: true`. Управление активным —
`update_playing_status` (пауза/позиция) и `update_player_state` (другой индекс очереди) со своей
версией, перевод — `update_active_device` на id устройства.

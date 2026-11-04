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

## Осторожно

- При подключении не забирать активность у телефона (`is_currently_active: false`,
  `paused: true`), иначе перебьём воспроизведение там.
- app_name/app_version — как у официального клиента (антифрод), см. `DESKTOP_CLIENT` в `src/api/mod.rs`.
- Конфликты решаются версиями; свою версию генерировать заново на каждое изменение.

## Где взять исходники снова

- Десктоп: `nix run nixpkgs#asar -- extract /nix/store/*-yandex-music-*/share/nodejs/yandex-music.asar out`,
  дальше искать по строкам в `out/app/_next/static/chunks`.
- Android: см. память о реверсе APK (adb pull + `jadx --no-res`).

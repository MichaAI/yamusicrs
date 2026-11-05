//! Protobuf Ynison ↔ JSON-представление (как в JSON-маппинге веба).
//!
//! Схема восстановлена по классам protobuf-lite официального Android-клиента 2026.09.5
//! (имена полей в них не обфусцированы, номера — из строк `newMessageInfo`), см. `docs/ynison.md`.
//! Логика Ynison работает с `serde_json::Value` в веб-именах, а сюда приходит только на краях:
//! кодирование запроса и разбор ответа.

use serde_json::{Map, Value, json};

#[derive(Clone, Copy)]
enum K {
    Str,
    I64,
    I32,
    U32,
    Bool,
    F64,
    Enum(&'static [(&'static str, i32)]),
    Msg(&'static [F]),
    /// Обёртка `{value: string}` (`*_optional`): в JSON — просто строка.
    Wrap,
}

#[derive(Clone, Copy)]
struct F {
    name: &'static str,
    num: u32,
    kind: K,
    rep: bool,
}

const fn f(name: &'static str, num: u32, kind: K) -> F {
    F { name, num, kind, rep: false }
}

const fn r(name: &'static str, num: u32, kind: K) -> F {
    F { name, num, kind, rep: true }
}

// ── перечисления ─────────────────────────────────────────────────────────────

const INTERCEPT: &[(&str, i32)] = &[
    ("DO_NOT_INTERCEPT_BY_DEFAULT", 0),
    ("INTERCEPT_IF_NO_ONE_ACTIVE", 1),
    ("INTERCEPT_EAGER", 2),
];
const ENTITY_TYPE: &[(&str, i32)] = &[
    ("UNSPECIFIED", 0),
    ("ARTIST", 1),
    ("PLAYLIST", 2),
    ("ALBUM", 3),
    ("RADIO", 4),
    ("VARIOUS", 5),
    ("GENERATIVE", 6),
    ("FM_RADIO", 7),
    ("VIDEO_WAVE", 8),
    ("LOCAL_TRACKS", 9),
];
const ENTITY_CONTEXT: &[(&str, i32)] = &[
    ("BASED_ON_ENTITY_BY_DEFAULT", 0),
    ("USER_TRACKS", 1),
    ("DOWNLOADED_TRACKS", 2),
    ("SEARCH", 3),
    ("MUSIC_HISTORY", 4),
    ("MUSIC_HISTORY_SEARCH", 5),
    ("ARTIST_MY_COLLECTION", 6),
    ("ARTIST_FAMILIAR_FROM_WAVE", 7),
];
const REPEAT: &[(&str, i32)] = &[("UNSPECIFIED", 0), ("NONE", 1), ("ONE", 2), ("ALL", 3)];
const PLAYABLE_TYPE: &[(&str, i32)] = &[
    ("UNSPECIFIED", 0),
    ("TRACK", 1),
    ("LOCAL_TRACK", 2),
    ("INFINITE", 3),
    ("VIDEO_CLIP", 4),
];
/// Порядок — как в десктопном protobuf (WEB = 1, его же шлёт веб в Device-Info).
pub const DEVICE_TYPE: &[(&str, i32)] = &[
    ("UNSPECIFIED", 0),
    ("WEB", 1),
    ("ANDROID", 2),
    ("IOS", 3),
    ("SMART_SPEAKER", 4),
    ("WEB_TV", 5),
    ("ANDROID_TV", 6),
    ("APPLE_TV", 7),
];

// ── сообщения ────────────────────────────────────────────────────────────────

const VERSION: &[F] = &[f("device_id", 1, K::Str), f("version", 2, K::I64), f("timestamp_ms", 3, K::I64)];
const STATUS: &[F] = &[
    f("progress_ms", 1, K::I64),
    f("duration_ms", 2, K::I64),
    f("paused", 3, K::Bool),
    f("playback_speed", 4, K::F64),
    f("version", 5, K::Msg(VERSION)),
];
const PLAYABLE: &[F] = &[
    f("playable_id", 1, K::Str),
    f("album_id_optional", 2, K::Wrap),
    f("playable_type", 3, K::Enum(PLAYABLE_TYPE)),
    f("from", 4, K::Str),
    f("title", 5, K::Str),
    f("cover_url_optional", 6, K::Wrap),
    f("navigation_id_optional", 9, K::Wrap),
    f("playback_action_id_optional", 10, K::Wrap),
];
const WAVE_ENTITY: &[F] = &[f("session_id", 1, K::Str), r("seeds", 2, K::Str)];
const ENTITY_OPTIONS: &[F] = &[f("wave_entity_optional", 1, K::Msg(WAVE_ENTITY))];
const WAVE_QUEUE: &[F] = &[
    r("recommended_playable_list", 1, K::Msg(PLAYABLE)),
    f("live_playable_index", 2, K::I32),
    f("entity_options", 3, K::Msg(ENTITY_OPTIONS)),
    f("navigation_id_optional", 4, K::Wrap),
    f("playback_action_id_optional", 5, K::Wrap),
];
const QUEUE_KIND: &[F] = &[f("wave_queue", 1, K::Msg(WAVE_QUEUE))];
const OPTIONS: &[F] = &[f("repeat_mode", 1, K::Enum(REPEAT))];
const SHUFFLE: &[F] = &[r("playable_indices", 1, K::U32)];
const INITIAL_ENTITY: &[F] = &[f("entity_id", 1, K::Str), f("entity_type", 2, K::Enum(ENTITY_TYPE))];
const RADIO_OPTIONS: &[F] = &[f("session_id", 1, K::Str)];
const ADDING_OPTIONS: &[F] = &[f("radio_options", 1, K::Msg(RADIO_OPTIONS))];
const QUEUE: &[F] = &[
    f("entity_id", 1, K::Str),
    f("entity_type", 2, K::Enum(ENTITY_TYPE)),
    f("current_playable_index", 3, K::I32),
    r("playable_list", 4, K::Msg(PLAYABLE)),
    f("options", 5, K::Msg(OPTIONS)),
    f("version", 6, K::Msg(VERSION)),
    f("shuffle_optional", 7, K::Msg(SHUFFLE)),
    f("entity_context", 8, K::Enum(ENTITY_CONTEXT)),
    f("from_optional", 9, K::Wrap),
    f("initial_entity_optional", 10, K::Msg(INITIAL_ENTITY)),
    f("adding_options_optional", 11, K::Msg(ADDING_OPTIONS)),
    f("queue", 12, K::Msg(QUEUE_KIND)),
    f("navigation_id_optional", 13, K::Wrap),
    f("filter_optional", 14, K::Wrap),
    f("playback_action_id_optional", 15, K::Wrap),
];
const INJECT_PLAYABLE: &[F] = &[
    f("playable_id", 1, K::Str),
    f("playable_type", 2, K::Enum(PLAYABLE_TYPE)),
    f("title", 3, K::Str),
    f("cover_url", 4, K::Wrap),
];
const QUEUE_INJECT: &[F] = &[
    f("playing_status", 1, K::Msg(STATUS)),
    f("playable", 2, K::Msg(INJECT_PLAYABLE)),
    f("version", 3, K::Msg(VERSION)),
];
const PLAYER_STATE: &[F] = &[
    f("status", 1, K::Msg(STATUS)),
    f("player_queue", 2, K::Msg(QUEUE)),
    f("player_queue_inject_optional", 3, K::Msg(QUEUE_INJECT)),
];
const INFO: &[F] = &[
    f("device_id", 1, K::Str),
    f("title", 2, K::Str),
    f("type", 3, K::Enum(DEVICE_TYPE)),
    f("app_name", 4, K::Str),
    f("app_version", 5, K::Str),
];
const CAPS: &[F] = &[
    f("can_be_player", 1, K::Bool),
    f("can_be_remote_controller", 2, K::Bool),
    f("volume_granularity", 3, K::U32),
];
const VOLUME_INFO: &[F] = &[f("volume", 1, K::F64), f("version", 2, K::Msg(VERSION))];
const SESSION: &[F] = &[f("id", 1, K::I64)];
const DEVICE: &[F] = &[
    f("info", 1, K::Msg(INFO)),
    f("volume", 2, K::F64),
    f("capabilities", 3, K::Msg(CAPS)),
    f("session", 4, K::Msg(SESSION)),
    f("is_offline", 5, K::Bool),
    f("volume_info", 6, K::Msg(VOLUME_INFO)),
];
const DEVICE_IN: &[F] = &[
    f("info", 1, K::Msg(INFO)),
    f("volume", 2, K::F64),
    f("capabilities", 3, K::Msg(CAPS)),
    f("volume_info", 4, K::Msg(VOLUME_INFO)),
];
const EOV: &[F] = &[f("actual_queue_id", 1, K::Str)];
const FULL_STATE: &[F] = &[
    f("player_state", 1, K::Msg(PLAYER_STATE)),
    f("is_currently_active", 2, K::Bool),
    f("device", 3, K::Msg(DEVICE_IN)),
    f("sync_state_from_eov_optional", 4, K::Msg(EOV)),
];

/// Запрос `PutYnisonState`: одно действие (oneof) и конверт.
const REQUEST: &[F] = &[
    f("update_full_state", 1, K::Msg(FULL_STATE)),
    f("update_active_device", 2, K::Msg(&[f("device_id_optional", 1, K::Wrap)])),
    f("update_playing_status", 3, K::Msg(&[f("playing_status", 1, K::Msg(STATUS))])),
    f("update_player_state", 4, K::Msg(&[f("player_state", 1, K::Msg(PLAYER_STATE))])),
    f("update_volume", 5, K::Msg(&[f("volume", 1, K::F64), f("device_id", 2, K::Str)])),
    f("update_player_queue_inject", 6, K::Msg(&[f("player_queue_inject", 1, K::Msg(QUEUE_INJECT))])),
    f("player_action_timestamp_ms", 7, K::I64),
    f("rid", 8, K::Str),
    f("update_session_params", 9, K::Msg(&[f("mute_events_if_passive", 1, K::Bool)])),
    f("activity_interception_type", 10, K::Enum(INTERCEPT)),
    f("update_volume_info", 11, K::Msg(&[f("device_id", 1, K::Str), f("volume_info", 2, K::Msg(VOLUME_INFO))])),
    f("sync_state_from_eov", 12, K::Msg(EOV)),
];
/// Ответ хаба: полное состояние.
const RESPONSE: &[F] = &[
    f("player_state", 1, K::Msg(PLAYER_STATE)),
    r("devices", 2, K::Msg(DEVICE)),
    f("active_device_id_optional", 3, K::Wrap),
    f("timestamp_ms", 4, K::I64),
    f("rid", 5, K::Str),
];
const KEEPALIVE: &[F] = &[
    f("keep_alive_time_seconds", 1, K::I32),
    f("keep_alive_timeout_seconds", 2, K::I32),
];
/// Ответ redirector.
const REDIRECT: &[F] = &[
    f("host", 1, K::Str),
    f("redirect_ticket", 2, K::Str),
    f("session_id", 3, K::I64),
    f("keep_alive_params", 4, K::Msg(KEEPALIVE)),
];
/// Заголовок `Ynison-Device-Info` (base64 без паддинга).
const DEVICE_INFO_HEADER: &[F] = &[
    f("type", 1, K::Enum(DEVICE_TYPE)),
    f("app_name", 2, K::Str),
    f("app_version", 3, K::Str),
];

pub fn encode_request(v: &Value) -> Vec<u8> {
    encode(REQUEST, v)
}

pub fn decode_response(b: &[u8]) -> Value {
    decode(RESPONSE, b)
}

pub fn decode_redirect(b: &[u8]) -> Value {
    decode(REDIRECT, b)
}

pub fn device_info_header(kind: &str, app_name: &str, app_version: &str) -> String {
    let b = encode(DEVICE_INFO_HEADER, &json!({"type": kind, "app_name": app_name, "app_version": app_version}));
    base64_nopad(&b)
}

// ── кодирование ──────────────────────────────────────────────────────────────

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn key(out: &mut Vec<u8>, num: u32, wire: u8) {
    varint(out, ((num as u64) << 3) | wire as u64);
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|x| x as i64)),
        Value::String(s) => s.parse().ok(),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}

fn encode(fields: &[F], v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(obj) = v.as_object() else { return out };
    for fd in fields {
        let Some(x) = obj.get(fd.name) else { continue };
        if x.is_null() {
            continue;
        }
        if fd.rep {
            for e in x.as_array().into_iter().flatten() {
                encode_one(&mut out, fd, e);
            }
        } else {
            encode_one(&mut out, fd, x);
        }
    }
    out
}

fn encode_one(out: &mut Vec<u8>, fd: &F, x: &Value) {
    match fd.kind {
        K::Str => {
            let s = match x {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            key(out, fd.num, 2);
            varint(out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        K::I64 | K::I32 | K::U32 => {
            if let Some(n) = int(x) {
                key(out, fd.num, 0);
                varint(out, n as u64);
            }
        }
        K::Bool => {
            key(out, fd.num, 0);
            varint(out, x.as_bool().unwrap_or(false) as u64);
        }
        K::F64 => {
            let n = x.as_f64().or_else(|| x.as_str()?.parse().ok()).unwrap_or(0.0);
            key(out, fd.num, 1);
            out.extend_from_slice(&n.to_le_bytes());
        }
        K::Enum(t) => {
            let n = match x {
                Value::String(s) => t.iter().find(|(name, _)| name == s).map(|(_, n)| *n as i64),
                v => int(v),
            };
            if let Some(n) = n {
                key(out, fd.num, 0);
                varint(out, n as u64);
            }
        }
        K::Msg(sub) => {
            let b = encode(sub, x);
            key(out, fd.num, 2);
            varint(out, b.len() as u64);
            out.extend_from_slice(&b);
        }
        K::Wrap => {
            let s = match x {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            let mut inner = Vec::new();
            key(&mut inner, 1, 2);
            varint(&mut inner, s.len() as u64);
            inner.extend_from_slice(s.as_bytes());
            key(out, fd.num, 2);
            varint(out, inner.len() as u64);
            out.extend_from_slice(&inner);
        }
    }
}

// ── разбор ───────────────────────────────────────────────────────────────────

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl Rd<'_> {
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.b.get(self.p)?;
            self.p += 1;
            v |= ((byte & 0x7F) as u64) << shift;
            if byte < 0x80 {
                return Some(v);
            }
        }
        None
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
}

fn decode(fields: &[F], b: &[u8]) -> Value {
    let mut obj = Map::new();
    let mut rd = Rd { b, p: 0 };
    while rd.p < b.len() {
        let Some(k) = rd.varint() else { break };
        let (num, wire) = ((k >> 3) as u32, (k & 7) as u8);
        let fd = fields.iter().find(|f| f.num == num);
        // Сырое значение поля.
        let raw: Raw = match wire {
            0 => match rd.varint() {
                Some(v) => Raw::Int(v),
                None => break,
            },
            1 => match rd.take(8) {
                Some(s) => Raw::Fixed64(u64::from_le_bytes(s.try_into().unwrap_or_default())),
                None => break,
            },
            2 => {
                let Some(n) = rd.varint() else { break };
                match rd.take(n as usize) {
                    Some(s) => Raw::Bytes(s),
                    None => break,
                }
            }
            5 => match rd.take(4) {
                Some(_) => continue,
                None => break,
            },
            _ => break,
        };
        let Some(fd) = fd else { continue };
        let vals: Vec<Value> = match (fd.kind, raw) {
            // Упакованные числа.
            (K::I64 | K::I32 | K::U32 | K::Enum(_) | K::Bool, Raw::Bytes(s)) if fd.rep => {
                let mut r2 = Rd { b: s, p: 0 };
                let mut v = Vec::new();
                while r2.p < s.len() {
                    match r2.varint() {
                        Some(x) => v.push(scalar(fd.kind, x)),
                        None => break,
                    }
                }
                v
            }
            (K::F64, Raw::Fixed64(x)) => vec![json!(f64::from_bits(x))],
            (K::Str, Raw::Bytes(s)) => vec![Value::String(String::from_utf8_lossy(s).into_owned())],
            (K::Msg(sub), Raw::Bytes(s)) => vec![decode(sub, s)],
            (K::Wrap, Raw::Bytes(s)) => {
                let w = decode(&[f("value", 1, K::Str)], s);
                vec![w.get("value").cloned().unwrap_or(Value::String(String::new()))]
            }
            (k, Raw::Int(x)) => vec![scalar(k, x)],
            _ => continue,
        };
        if fd.rep {
            let e = obj.entry(fd.name).or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(a) = e {
                a.extend(vals);
            }
        } else if let Some(v) = vals.into_iter().next() {
            obj.insert(fd.name.into(), v);
        }
    }
    // proto3 не передаёт значения по умолчанию — восстанавливаем их для скаляров,
    // иначе «paused: false» выглядело бы как отсутствующее поле.
    for fd in fields {
        if obj.contains_key(fd.name) {
            continue;
        }
        let d = match (fd.rep, fd.kind) {
            // Отсутствующий список — «не прислали», а не «пусто» (ответы хаба бывают частичными).
            (true, _) => continue,
            (_, K::Str) => json!(""),
            (_, K::I64) => json!("0"),
            (_, K::I32 | K::U32) => json!(0),
            (_, K::Bool) => json!(false),
            (_, K::F64) => json!(0.0),
            (_, K::Enum(t)) => json!(t.first().map_or("UNSPECIFIED", |e| e.0)),
            _ => continue,
        };
        obj.insert(fd.name.into(), d);
    }
    Value::Object(obj)
}

enum Raw<'a> {
    Int(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
}

fn scalar(k: K, x: u64) -> Value {
    match k {
        // int64 в JSON-маппинге — строкой (как отдавал веб).
        K::I64 => json!((x as i64).to_string()),
        K::I32 => json!(x as i64 as i32),
        K::U32 => json!(x as u32),
        K::Bool => json!(x != 0),
        K::Enum(t) => match t.iter().find(|(_, n)| *n as u64 == x) {
            Some((name, _)) => json!(name),
            None => json!(x as i64),
        },
        _ => json!(x),
    }
}

fn base64_nopad(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_full_state() {
        let req = json!({
            "update_player_state": {"player_state": {
                "status": {"paused": false, "progress_ms": "1500", "duration_ms": "200000", "playback_speed": 1.0,
                    "version": {"device_id": "me", "version": "42", "timestamp_ms": "7"}},
                "player_queue": {"entity_id": "user:onyourwave", "entity_type": "RADIO", "current_playable_index": 1,
                    "playable_list": [{"playable_id": "1", "playable_type": "TRACK", "from": "x", "title": "a"},
                                      {"playable_id": "2", "album_id_optional": "9", "playable_type": "TRACK", "from": "x", "title": "b"}],
                    "options": {"repeat_mode": "NONE"}, "from_optional": "x",
                    "queue": {"wave_queue": {"entity_options": {"wave_entity_optional": {"session_id": "s1"}}}}}}},
            "rid": "r", "player_action_timestamp_ms": "5", "activity_interception_type": "INTERCEPT_IF_NO_ONE_ACTIVE"
        });
        let b = encode_request(&req);
        let back = decode(REQUEST, &b);
        let ps = &back["update_player_state"]["player_state"];
        assert_eq!(ps["status"]["paused"], json!(false));
        assert_eq!(ps["status"]["progress_ms"], json!("1500"));
        assert_eq!(ps["player_queue"]["entity_type"], json!("RADIO"));
        assert_eq!(ps["player_queue"]["playable_list"][1]["album_id_optional"], json!("9"));
        assert_eq!(ps["player_queue"]["queue"]["wave_queue"]["entity_options"]["wave_entity_optional"]["session_id"], json!("s1"));
        assert_eq!(back["activity_interception_type"], json!("INTERCEPT_IF_NO_ONE_ACTIVE"));
    }

    #[test]
    fn base64() {
        assert_eq!(base64_nopad(b"Ma"), "TWE");
        assert_eq!(base64_nopad(b"Man"), "TWFu");
    }
}

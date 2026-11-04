//! Ynison — синхронизация воспроизведения между устройствами («Слушать на…» в приложениях).
//!
//! Транспорт: JSON поверх WebSocket, как у веба (redirector → hub), но устройство полноценное:
//! телефон видит yamusicrs в списке, переключает воспроизведение сюда и управляет им.
//! Подробности протокола — `docs/ynison.md`.
//!
//! Задача держит соединение, сливает частичные ответы хаба в полное состояние и отдаёт
//! приложению [`Event`]; приложение шлёт сюда [`Out`]. Решения «что делать со звуком»
//! принимает приложение.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Request};
use tokio_tungstenite::tungstenite::{self, Message};

const REDIRECT: &str = "wss://ynison.music.yandex.ru/redirector.YnisonRedirectService/GetRedirectToYnison";
const STATE_PATH: &str = "/ynison_state.YnisonStateService/PutYnisonState";
const ORIGIN: &str = "https://music.yandex.ru";
const APP_NAME: &str = "yamusicrs";
/// Автор статуса, когда сервер сам ставит паузу: активное устройство отключилось или его сменили.
pub const SERVER_DISCONNECT: &str = "server-paused-on-active-device-disconnecting";
/// Повторы подключения после обычных ошибок (секунды); go-away сервера соблюдается точно.
const LADDER: [u64; 3] = [1, 5, 30];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playable {
    pub id: String,
    pub album_id: Option<String>,
    pub title: String,
    pub cover: Option<String>,
    pub from: String,
}

#[derive(Clone, Debug)]
pub struct QueueState {
    /// `RADIO` — волна (entity_id — сиды через запятую), `ALBUM`, `PLAYLIST` (`uid:kind`), `ARTIST`, `VARIOUS`.
    pub entity_type: String,
    pub entity_id: String,
    pub from: String,
    /// Индекс текущего в `items`; -1 — пусто.
    pub index: i64,
    pub items: Vec<Playable>,
    /// `NONE` / `ONE` / `ALL`.
    pub repeat: String,
    /// Сессия ротора, если очередь — волна.
    pub wave_session: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    pub paused: bool,
    pub progress_ms: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub title: String,
}

/// Полное известное состояние после очередного ответа хаба.
#[derive(Clone, Debug, Default)]
pub struct Remote {
    pub active: Option<String>,
    pub devices: Vec<Device>,
    pub queue: Option<QueueState>,
    pub status: Option<Status>,
    /// Новая очередь от другого устройства пришла в этом сообщении.
    pub queue_cmd: bool,
    /// Новый статус от другого устройства пришёл в этом сообщении.
    pub status_cmd: bool,
    /// Сервер сам поставил паузу (активное устройство сменилось или отвалилось).
    pub server_paused: bool,
    /// Кто автор последнего статуса (для подписи «играет на …»).
    pub status_by: String,
}

impl Remote {
    pub fn device_title(&self, id: &str) -> Option<&str> {
        self.devices.iter().find(|d| d.id == id).map(|d| d.title.as_str())
    }
}

pub enum Event {
    Connected,
    /// Соединение потеряно; через сколько секунд следующая попытка.
    Lost { error: String, retry_in: u64 },
    State(Remote),
}

pub enum Out {
    /// Пауза/позиция (мы активны).
    Status(Status),
    /// Очередь вместе со статусом (сменился трек или очередь).
    Player(QueueState, Status),
    /// Забрать воспроизведение себе: включили что-то здесь.
    Activate,
}

pub struct Handle {
    tx: UnboundedSender<Out>,
    pub device_id: String,
}

impl Handle {
    pub fn send(&self, o: Out) {
        let _ = self.tx.send(o);
    }
}

pub fn spawn<F>(token: String, device_id: String, title: String, emit: F) -> Handle
where
    F: Fn(Event) + Send + Sync + 'static,
{
    let (tx, rx) = unbounded_channel();
    let me = Session {
        token,
        device_id: device_id.clone(),
        title,
    };
    tokio::spawn(run(me, rx, emit));
    Handle { tx, device_id }
}

struct Session {
    token: String,
    device_id: String,
    title: String,
}

enum End {
    /// Приложение закрыло канал — выходим.
    Closed,
    /// Сервер просит не возвращаться столько секунд.
    GoAway(u64, String),
    /// Обычная ошибка; `healthy` — сессия успела поработать (лестница с начала).
    Error(String, bool),
}

async fn run<F: Fn(Event)>(me: Session, mut rx: UnboundedReceiver<Out>, emit: F) {
    let mut step = 0usize;
    loop {
        let wait = match session(&me, &mut rx, &emit).await {
            End::Closed => return,
            End::GoAway(secs, e) => {
                step = 0;
                tracing::warn!("ynison: go-away {secs} с: {e}");
                (secs, e)
            }
            End::Error(e, healthy) => {
                if healthy {
                    step = 0;
                }
                let s = LADDER[step.min(LADDER.len() - 1)];
                step += 1;
                tracing::info!("ynison: {e}; повтор через {s} с");
                (s, e)
            }
        };
        emit(Event::Lost {
            error: wait.1,
            retry_in: wait.0,
        });
        // Пока ждём, команды приложения устаревают: после подключения состояние уйдёт заново.
        let sleep = tokio::time::sleep(Duration::from_secs(wait.0));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                m = rx.recv() => if m.is_none() { return },
            }
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn version(me: &str, ts_ms: u64) -> Value {
    let ns = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    json!({"device_id": me, "version": ns.to_string(), "timestamp_ms": ts_ms.to_string()})
}

/// Конверт запроса: `rid`, время действия, тип перехвата (числа — строками, иначе сервер рвёт соединение).
fn envelope(mut body: Value, intercept: &str) -> String {
    body["rid"] = json!(uuid());
    body["player_action_timestamp_ms"] = json!(now_ms().to_string());
    body["activity_interception_type"] = json!(intercept);
    body.to_string()
}

fn uuid() -> String {
    let b: [u8; 16] = rand::random();
    let h = hex::encode(b);
    format!("{}-{}-4{}-a{}-{}", &h[..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

fn request(me: &Session, url: &str, ticket: Option<(&str, &str)>) -> Result<Request<()>, tungstenite::Error> {
    let mut req = url.into_client_request()?;
    let info = json!({"app_name": APP_NAME, "app_version": env!("CARGO_PKG_VERSION"), "type": 1}).to_string();
    let mut proto = json!({"Ynison-Device-Id": me.device_id, "Ynison-Device-Info": info});
    if let Some((t, s)) = ticket {
        proto["Ynison-Redirect-Ticket"] = json!(t);
        proto["Ynison-Session-Id"] = json!(s);
    }
    let h = req.headers_mut();
    let val = |s: String| HeaderValue::from_str(&s).map_err(|e| tungstenite::Error::HttpFormat(e.into()));
    h.insert("Sec-WebSocket-Protocol", val(format!("Bearer, v2, {proto}"))?);
    h.insert("Authorization", val(format!("OAuth {}", me.token))?);
    h.insert("Origin", HeaderValue::from_static(ORIGIN));
    Ok(req)
}

/// Ошибка из ответа сервера: go-away или обычная.
fn server_error(v: &Value) -> End {
    let e = &v["error"];
    let msg = format!(
        "{} (grpc {}, http {})",
        e["message"].as_str().unwrap_or("ошибка Ynison"),
        e["grpc_code"],
        e["http_code"]
    );
    let away = e["details"]["ynison-go-away-for-seconds"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| e["details"]["ynison-go-away-for-seconds"].as_u64());
    match away {
        Some(s) if s > 0 => End::GoAway(s, msg),
        _ => End::Error(msg, false),
    }
}

async fn session<F: Fn(Event)>(me: &Session, rx: &mut UnboundedReceiver<Out>, emit: &F) -> End {
    // 1. Redirector: куда подключаться и с каким билетом.
    let req = match request(me, REDIRECT, None) {
        Ok(r) => r,
        Err(e) => return End::Error(format!("redirector: {e}"), false),
    };
    let redir: Value = match tokio_tungstenite::connect_async(req).await {
        Ok((mut ws, _)) => {
            let first = tokio::time::timeout(Duration::from_secs(10), ws.next()).await;
            let _ = ws.close(None).await;
            match first {
                Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str(&t) {
                    Ok(v) => v,
                    Err(e) => return End::Error(format!("redirector: {e}"), false),
                },
                Ok(other) => return End::Error(format!("redirector: неожиданный ответ {other:?}"), false),
                Err(_) => return End::Error("redirector: нет ответа".into(), false),
            }
        }
        Err(e) => return End::Error(format!("redirector: {e}"), false),
    };
    if redir.get("error").is_some() {
        return server_error(&redir);
    }
    let (Some(host), Some(ticket)) = (redir["host"].as_str(), redir["redirect_ticket"].as_str()) else {
        return End::Error("redirector не дал адрес (токен?)".into(), false);
    };
    let session_id = match &redir["session_id"] {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    let keepalive = redir["keep_alive_params"]["keep_alive_time_seconds"]
        .as_u64()
        .or_else(|| redir["keep_alive_params"]["keep_alive_time_seconds"].as_str()?.parse().ok())
        .unwrap_or(30)
        .clamp(10, 120);

    // 2. Хаб состояния.
    let url = format!("wss://{host}{STATE_PATH}");
    let req = match request(me, &url, Some((ticket, &session_id))) {
        Ok(r) => r,
        Err(e) => return End::Error(format!("hub: {e}"), false),
    };
    let mut ws = match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => ws,
        Err(e) => return End::Error(format!("hub: {e}"), false),
    };
    // Пустая очередь с нулевым временем версий: ничего не перехватываем, только появляемся в списке.
    let full = json!({
        "update_full_state": {
            "player_state": {
                "status": {
                    "paused": true, "duration_ms": "0", "progress_ms": "0", "playback_speed": 1,
                    "version": version(&me.device_id, 0),
                },
                "player_queue": {
                    "current_playable_index": -1, "entity_id": "", "entity_type": "VARIOUS",
                    "playable_list": [], "options": {"repeat_mode": "NONE"},
                    "entity_context": "BASED_ON_ENTITY_BY_DEFAULT",
                    "version": version(&me.device_id, 0), "from_optional": "",
                },
            },
            "device": {
                "info": {
                    "device_id": me.device_id, "title": me.title, "type": "WEB",
                    "app_name": APP_NAME, "app_version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {"can_be_player": true, "can_be_remote_controller": false, "volume_granularity": 16},
            },
            "is_currently_active": false,
        }
    });
    let params = json!({"update_session_params": {"mute_events_if_passive": true}});
    for m in [envelope(full, "DO_NOT_INTERCEPT_BY_DEFAULT"), envelope(params, "DO_NOT_INTERCEPT_BY_DEFAULT")] {
        if let Err(e) = ws.send(Message::text(m)).await {
            return End::Error(format!("hub: {e}"), false);
        }
    }
    emit(Event::Connected);

    let mut state = Merged::default();
    let mut healthy = false;
    let mut ping = tokio::time::interval(Duration::from_secs(keepalive));
    ping.tick().await;
    loop {
        tokio::select! {
            m = ws.next() => match m {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = match serde_json::from_str(&t) {
                        Ok(v) => v,
                        Err(e) => { tracing::warn!("ynison: не JSON: {e}"); continue }
                    };
                    if v.get("error").is_some() {
                        return match server_error(&v) {
                            End::Error(e, _) => End::Error(e, healthy),
                            other => other,
                        };
                    }
                    healthy = true;
                    emit(Event::State(state.apply(&v, &me.device_id)));
                }
                Some(Ok(Message::Close(f))) => return End::Error(format!("hub закрыл соединение: {f:?}"), healthy),
                Some(Ok(_)) => {}
                Some(Err(e)) => return End::Error(format!("hub: {e}"), healthy),
                None => return End::Error("hub: соединение закрыто".into(), healthy),
            },
            o = rx.recv() => {
                let Some(o) = o else {
                    let _ = ws.close(None).await;
                    return End::Closed;
                };
                let msg = outgoing(&me.device_id, o);
                if let Err(e) = ws.send(Message::text(msg)).await {
                    return End::Error(format!("hub: {e}"), healthy);
                }
            }
            _ = ping.tick() => {
                if let Err(e) = ws.send(Message::Ping(Vec::new().into())).await {
                    return End::Error(format!("hub: {e}"), healthy);
                }
            }
        }
    }
}

fn status_json(me: &str, s: Status) -> Value {
    json!({
        "progress_ms": s.progress_ms.to_string(),
        "duration_ms": s.duration_ms.to_string(),
        "paused": s.paused,
        "playback_speed": 1.0,
        "version": version(me, now_ms()),
    })
}

fn outgoing(me: &str, o: Out) -> String {
    match o {
        Out::Status(s) => {
            // Играем — можно перехватить, если никто не активен (как в официальных клиентах).
            let intercept = if s.paused {
                "DO_NOT_INTERCEPT_BY_DEFAULT"
            } else {
                "INTERCEPT_IF_NO_ONE_ACTIVE"
            };
            envelope(json!({"update_playing_status": {"playing_status": status_json(me, s)}}), intercept)
        }
        Out::Player(q, s) => {
            let items: Vec<Value> = q
                .items
                .iter()
                .map(|p| {
                    json!({
                        "playable_id": p.id,
                        "album_id_optional": p.album_id,
                        "playable_type": "TRACK",
                        "from": p.from,
                        "title": p.title,
                        "cover_url_optional": p.cover,
                    })
                })
                .collect();
            let mut queue = json!({
                "entity_id": q.entity_id,
                "entity_type": q.entity_type,
                "current_playable_index": q.index,
                "playable_list": items,
                "options": {"repeat_mode": q.repeat},
                "entity_context": "BASED_ON_ENTITY_BY_DEFAULT",
                "from_optional": q.from,
                "version": version(me, now_ms()),
            });
            if let Some(sid) = &q.wave_session {
                queue["queue"] = json!({"wave_queue": {"entity_options": {"wave_entity_optional": {"session_id": sid}}}});
            }
            envelope(
                json!({"update_player_state": {"player_state": {"player_queue": queue, "status": status_json(me, s)}}}),
                "DO_NOT_INTERCEPT_BY_DEFAULT",
            )
        }
        Out::Activate => envelope(
            json!({"update_active_device": {"device_id_optional": me}}),
            "DO_NOT_INTERCEPT_BY_DEFAULT",
        ),
    }
}

/// Слияние частичных ответов хаба: каждое пришедшее верхнеуровневое поле заменяет прежнее.
#[derive(Default)]
struct Merged {
    remote: Remote,
    /// Последние применённые версии чужих команд — повторы (ретрансляции) не считаются новыми.
    queue_ver: String,
    status_ver: String,
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn num(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_str()?.parse().ok()).unwrap_or(0)
}

impl Merged {
    fn apply(&mut self, v: &Value, me: &str) -> Remote {
        let r = &mut self.remote;
        r.queue_cmd = false;
        r.status_cmd = false;
        r.server_paused = false;
        if let Some(ds) = v["devices"].as_array() {
            r.devices = ds
                .iter()
                .map(|d| Device {
                    id: s(&d["info"]["device_id"]),
                    title: s(&d["info"]["title"]),
                })
                .collect();
        }
        let ps = &v["player_state"];
        let status_author = s(&ps["status"]["version"]["device_id"]);
        if let Some(a) = v.get("active_device_id_optional") {
            let a = s(a);
            r.active = (!a.is_empty()).then_some(a);
        } else if status_author == SERVER_DISCONNECT {
            // Сервер отдал паузу без активного устройства — владелец неизвестен.
            r.active = None;
        }
        if ps["player_queue"].is_object() {
            let q = &ps["player_queue"];
            let author = s(&q["version"]["device_id"]);
            let ver = format!("{author}/{}", s(&q["version"]["version"]));
            r.queue = Some(QueueState {
                entity_type: s(&q["entity_type"]),
                entity_id: s(&q["entity_id"]),
                from: s(&q["from_optional"]),
                index: q["current_playable_index"]
                    .as_i64()
                    .or_else(|| q["current_playable_index"].as_str()?.parse().ok())
                    .unwrap_or(-1),
                items: q["playable_list"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|p| matches!(p["playable_type"].as_str(), None | Some("TRACK")))
                    .map(|p| Playable {
                        id: s(&p["playable_id"]),
                        album_id: Some(s(&p["album_id_optional"])).filter(|a| !a.is_empty()),
                        title: s(&p["title"]),
                        cover: Some(s(&p["cover_url_optional"])).filter(|a| !a.is_empty()),
                        from: s(&p["from"]),
                    })
                    .collect(),
                repeat: Some(s(&q["options"]["repeat_mode"])).filter(|r| !r.is_empty()).unwrap_or("NONE".into()),
                wave_session: Some(s(&q["queue"]["wave_queue"]["entity_options"]["wave_entity_optional"]["session_id"]))
                    .filter(|x| !x.is_empty()),
            });
            if is_peer(&author, me) && ver != self.queue_ver {
                self.queue_ver = ver;
                r.queue_cmd = true;
            }
        }
        if ps["status"].is_object() {
            let st = &ps["status"];
            r.status = Some(Status {
                paused: st["paused"].as_bool().unwrap_or(true),
                progress_ms: num(&st["progress_ms"]),
                duration_ms: num(&st["duration_ms"]),
            });
            let ver = format!("{status_author}/{}", s(&st["version"]["version"]));
            if status_author == SERVER_DISCONNECT {
                r.server_paused = true;
            } else if is_peer(&status_author, me) && ver != self.status_ver {
                self.status_ver = ver;
                r.status_cmd = true;
            }
            if !status_author.is_empty() {
                r.status_by = status_author;
            }
        }
        r.clone()
    }
}

/// Автор — другое устройство (не мы, не сервер и не пустая версия эха).
fn is_peer(author: &str, me: &str) -> bool {
    !author.is_empty() && author != me && author != SERVER_DISCONNECT && !author.starts_with("server-") && author != "set-by-host-for-backward-compatibility"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_partial_and_detects_peer_commands() {
        let mut m = Merged::default();
        let full = json!({
            "active_device_id_optional": "phone",
            "devices": [{"info": {"device_id": "phone", "title": "Телефон", "type": "ANDROID"}}],
            "player_state": {
                "player_queue": {"entity_type": "RADIO", "entity_id": "user:onyourwave", "current_playable_index": 1,
                    "playable_list": [{"playable_id": "1", "playable_type": "TRACK"}, {"playable_id": "2", "playable_type": "TRACK", "album_id_optional": "9"}],
                    "version": {"device_id": "phone", "version": "5"}},
                "status": {"paused": false, "progress_ms": "1500", "duration_ms": "200000", "version": {"device_id": "phone", "version": "7"}}
            }
        });
        let r = m.apply(&full, "me");
        assert_eq!(r.active.as_deref(), Some("phone"));
        assert!(r.queue_cmd && r.status_cmd);
        assert_eq!(r.queue.as_ref().unwrap().items[1].album_id.as_deref(), Some("9"));
        assert_eq!(r.status.unwrap().progress_ms, 1500);
        // Ретрансляция того же — не команда; эхо с пустым автором — тоже.
        let r = m.apply(&full, "me");
        assert!(!r.queue_cmd && !r.status_cmd);
        let echo = json!({"player_state": {"status": {"paused": false, "progress_ms": "3000", "duration_ms": "200000",
            "version": {"device_id": "", "version": "0"}}}});
        let r = m.apply(&echo, "me");
        assert!(!r.status_cmd && r.queue.is_some() && r.active.as_deref() == Some("phone"));
        // Серверная пауза без активного устройства сбрасывает владельца.
        let off = json!({"player_state": {"status": {"paused": true, "progress_ms": "0", "duration_ms": "0",
            "version": {"device_id": SERVER_DISCONNECT, "version": "1"}}}});
        let r = m.apply(&off, "me");
        assert!(r.server_paused && r.active.is_none());
    }
}

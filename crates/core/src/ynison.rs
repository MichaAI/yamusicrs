//! Ynison — синхронизация воспроизведения между устройствами («Слушать на…» в приложениях).
//!
//! Транспорт — gRPC, как у официального Android-клиента: unary `GetRedirectToYnison` даёт хаб
//! и билет, дальше двунаправленный стрим `PutYnisonState` (protobuf, схема — `ynison_pb`).
//! JSON-WebSocket веба с середины октября 2026 хаб сторонним клиентам не отдаёт (503 + go-away).
//! Подробности — `docs/ynison.md`.
//!
//! Задача держит соединение, сливает ответы хаба в полное состояние и отдаёт приложению
//! [`Event`]; приложение шлёт сюда [`Out`]. Решения «что делать со звуком» принимает приложение.
//! Внутри всё в JSON-именах веба: в protobuf переводится только на краях.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::BodyExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::ynison_pb as pb;

const REDIRECT: &str = "https://ynison.music.yandex.net/ynison_redirect.YnisonRedirectService/GetRedirectToYnison";
const STATE_PATH: &str = "/ynison_state.YnisonStateService/PutYnisonState";
/// Как у официального клиента (grpc-okhttp из APK 2026.09.5).
const USER_AGENT: &str = "grpc-java-okhttp/1.61.1";
/// Представляемся официальным Android-приложением (см. `oby.c()` в APK 2026.09.5):
/// с другими именами хаб отвечает 500140000.
const APP_NAME: &str = "Yandex Music";
const APP_VERSION: &str = "2026.09.5 #167.1gpr";
/// Автор статуса, когда сервер сам ставит паузу: активное устройство отключилось или его сменили.
pub const SERVER_DISCONNECT: &str = "server-paused-on-active-device-disconnecting";
/// Повторы подключения после обычных ошибок (секунды); go-away и backoff сервера соблюдаются точно.
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
    /// `ANDROID`, `IOS`, `WEB`, `SMART_SPEAKER`…
    pub kind: String,
    /// Можно включить музыку на нём («Слушать на…»).
    pub can_play: bool,
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
    /// Пульт: перевести воспроизведение на устройство.
    Transfer(String),
    /// Пульт: пауза/игра и позиция активного устройства.
    RemoteStatus(Status),
    /// Пульт: другой трек очереди активного устройства.
    RemotePlayer(QueueState, Status),
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

/// `kind` — тип устройства в списке «Слушать на…»: `WEB` (компьютер) или `ANDROID`;
/// `delay` — сколько ждать до первой попытки (непрошедший go-away с прошлого запуска).
pub fn spawn<F>(token: String, device_id: String, title: String, kind: &'static str, delay: u64, emit: F) -> Handle
where
    F: Fn(Event) + Send + Sync + 'static,
{
    let (tx, rx) = unbounded_channel();
    let me = Session {
        token,
        device_id: device_id.clone(),
        title,
        kind,
    };
    tokio::spawn(async move {
        if delay > 0 {
            tracing::info!("ynison: сервер просил подождать, подключусь через {delay} с");
            emit(Event::Lost { error: "сервер просил подождать".into(), retry_in: delay });
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
        run(me, rx, emit).await
    });
    Handle { tx, device_id }
}

struct Session {
    token: String,
    device_id: String,
    title: String,
    kind: &'static str,
}

enum End {
    /// Приложение закрыло канал — выходим.
    Closed,
    /// Сервер просит не возвращаться столько секунд (go-away или backoff).
    GoAway(u64, String),
    /// Обычная ошибка; `healthy` — сессия успела поработать (лестница с начала).
    Error(String, bool),
}

async fn run<F: Fn(Event)>(me: Session, mut rx: UnboundedReceiver<Out>, emit: F) {
    let mut step = 0usize;
    loop {
        let wait = match session(&me, &mut rx, &emit).await {
            End::Closed => return,
            // go-away не соблюдаем (решение пользователя): обычная лестница повторов.
            End::GoAway(secs, e) => {
                let s = LADDER[step.min(LADDER.len() - 1)];
                step += 1;
                tracing::warn!("ynison: сервер просит подождать {secs} с ({e}); повтор через {s} с");
                (s, e)
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
    // Версия — случайное положительное int64, как у официальных клиентов.
    let v = rand::random::<u64>() >> 1;
    json!({"device_id": me, "version": v.to_string(), "timestamp_ms": ts_ms.to_string()})
}

/// Конверт запроса: `rid`, время действия, тип перехвата.
fn envelope(mut body: Value, intercept: &str) -> Value {
    body["rid"] = json!(uuid());
    body["player_action_timestamp_ms"] = json!(now_ms().to_string());
    body["activity_interception_type"] = json!(intercept);
    body
}

fn uuid() -> String {
    let b: [u8; 16] = rand::random();
    let h = hex::encode(b);
    format!("{}-{}-4{}-a{}-{}", &h[..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

// ── gRPC ─────────────────────────────────────────────────────────────────────

fn client(keepalive: Duration) -> reqwest::Result<reqwest::Client> {
    crate::api::with_roots(
        reqwest::Client::builder()
            .http2_prior_knowledge()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .http2_keep_alive_interval(keepalive)
            .http2_keep_alive_timeout(Duration::from_secs(20))
            .http2_keep_alive_while_idle(true),
    )
    .build()
}

fn headers(me: &Session, ticket: Option<(&str, &str)>) -> HeaderMap {
    let mut h = HeaderMap::new();
    let mut put = |k: &'static str, v: &str| {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(HeaderName::from_static(k), v);
        }
    };
    put("content-type", "application/grpc");
    put("te", "trailers");
    put("authorization", &format!("OAuth {}", me.token));
    put("ynison-device-id", &me.device_id);
    put("ynison-device-info", &pb::device_info_header(me.kind, APP_NAME, APP_VERSION));
    if let Some((t, s)) = ticket {
        put("ynison-redirect-ticket", t);
        put("ynison-session-id", s);
    }
    h
}

/// Сообщение gRPC: флаг сжатия, длина и тело.
fn frame(msg: &[u8]) -> Bytes {
    let mut b = Vec::with_capacity(msg.len() + 5);
    b.push(0);
    b.extend_from_slice(&(msg.len() as u32).to_be_bytes());
    b.extend_from_slice(msg);
    b.into()
}

/// Статус gRPC из заголовков или трейлеров: `None` — его там нет.
fn grpc_end(h: &HeaderMap, healthy: bool) -> Option<End> {
    let code: u32 = h.get("grpc-status")?.to_str().ok()?.parse().ok()?;
    let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let msg = get("grpc-message").map(|m| percent_decode(&m)).unwrap_or_default();
    let code_y = get("ynison-error-code").unwrap_or_default();
    let text = format!("gRPC {code}{}{}", if msg.is_empty() { String::new() } else { format!(": {msg}") },
        if code_y.is_empty() { String::new() } else { format!(" (ynison {code_y})") });
    if let Some(s) = get("ynison-go-away-for-seconds").and_then(|s| s.parse::<u64>().ok()).filter(|s| *s > 0) {
        return Some(End::GoAway(s, text));
    }
    if let Some(ms) = get("ynison-backoff-millis").and_then(|s| s.parse::<u64>().ok()).filter(|m| *m > 0) {
        return Some(End::GoAway(ms.div_ceil(1000), text));
    }
    Some(match code {
        0 => End::Error("хаб закрыл поток".into(), healthy),
        16 => End::Error(format!("токен не принят ({text})"), false),
        _ => End::Error(text, healthy),
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(x) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(x);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Входящий поток сообщений gRPC.
struct Incoming {
    body: reqwest::Body,
    buf: Vec<u8>,
}

impl Incoming {
    /// Следующее сообщение; `Err` — поток кончился (со статусом из трейлеров).
    async fn next(&mut self, healthy: bool) -> Result<Vec<u8>, End> {
        loop {
            if self.buf.len() >= 5 {
                let n = u32::from_be_bytes(self.buf[1..5].try_into().unwrap_or_default()) as usize;
                if self.buf.len() >= 5 + n {
                    if self.buf[0] != 0 {
                        return Err(End::Error("сжатое сообщение gRPC".into(), healthy));
                    }
                    let msg = self.buf[5..5 + n].to_vec();
                    self.buf.drain(..5 + n);
                    return Ok(msg);
                }
            }
            match self.body.frame().await {
                Some(Ok(f)) => match f.into_data() {
                    Ok(d) => self.buf.extend_from_slice(&d),
                    Err(f) => {
                        if let Ok(t) = f.into_trailers() {
                            return Err(grpc_end(&t, healthy).unwrap_or(End::Error("поток без статуса".into(), healthy)));
                        }
                    }
                },
                Some(Err(e)) => return Err(End::Error(format!("{e}"), healthy)),
                None => return Err(End::Error("соединение закрыто".into(), healthy)),
            }
        }
    }
}

async fn session<F: Fn(Event)>(me: &Session, rx: &mut UnboundedReceiver<Out>, emit: &F) -> End {
    // 1. Redirector: куда подключаться и с каким билетом (unary).
    let c = match client(Duration::from_secs(30)) {
        Ok(c) => c,
        Err(e) => return End::Error(format!("клиент: {e}"), false),
    };
    let resp = match c.post(REDIRECT).headers(headers(me, None)).body(frame(&[])).send().await {
        Ok(r) => r,
        Err(e) => return End::Error(format!("redirector: {e}"), false),
    };
    if let Some(e) = grpc_end(resp.headers(), false) {
        return e;
    }
    let mut inc = Incoming {
        body: resp.into(),
        buf: Vec::new(),
    };
    let redir = match inc.next(false).await {
        Ok(m) => pb::decode_redirect(&m),
        Err(e) => return e,
    };
    let (Some(host), Some(ticket)) = (
        redir["host"].as_str().filter(|h| !h.is_empty()),
        redir["redirect_ticket"].as_str().filter(|t| !t.is_empty()),
    ) else {
        return End::Error("redirector не дал адрес (токен?)".into(), false);
    };
    let session_id = s(&redir["session_id"]);
    let keepalive = redir["keep_alive_params"]["keep_alive_time_seconds"]
        .as_u64()
        .filter(|k| *k > 0)
        .unwrap_or(30)
        .clamp(10, 120);
    tracing::debug!("ynison: хаб {host}, сессия {session_id}");

    // 2. Хаб: двунаправленный стрим. Исходящие сообщения идут через канал в тело запроса.
    let c = match client(Duration::from_secs(keepalive)) {
        Ok(c) => c,
        Err(e) => return End::Error(format!("клиент: {e}"), false),
    };
    let (btx, brx) = unbounded_channel::<Bytes>();
    let stream = futures_util::stream::unfold(brx, |mut rx| async move {
        rx.recv().await.map(|b| (Ok::<_, std::io::Error>(b), rx))
    });
    // Пустая очередь с нулевым временем версий: ничего не перехватываем, только появляемся в списке.
    let full = json!({
        "update_full_state": {
            "player_state": {
                "status": {
                    "paused": true, "duration_ms": "0", "progress_ms": "0", "playback_speed": 1.0,
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
                    "device_id": me.device_id, "title": me.title, "type": me.kind,
                    "app_name": APP_NAME, "app_version": APP_VERSION,
                },
                "capabilities": {"can_be_player": true, "can_be_remote_controller": true, "volume_granularity": 16},
                "volume_info": {"volume": 1.0},
            },
            "is_currently_active": false,
        }
    });
    let params = json!({"update_session_params": {"mute_events_if_passive": true}});
    for m in [full, params] {
        let _ = btx.send(frame(&pb::encode_request(&envelope(m, "DO_NOT_INTERCEPT_BY_DEFAULT"))));
    }
    let url = format!("https://{host}{STATE_PATH}");
    let resp = match c
        .post(&url)
        .headers(headers(me, Some((ticket, &session_id))))
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return End::Error(format!("хаб: {e}"), false),
    };
    if let Some(e) = grpc_end(resp.headers(), false) {
        return e;
    }
    if !resp.status().is_success() {
        return End::Error(format!("хаб: HTTP {}", resp.status()), false);
    }
    let mut inc = Incoming {
        body: resp.into(),
        buf: Vec::new(),
    };
    emit(Event::Connected);

    let mut state = Merged::default();
    let mut healthy = false;
    loop {
        tokio::select! {
            m = inc.next(healthy) => match m {
                Ok(m) => {
                    healthy = true;
                    let v = pb::decode_response(&m);
                    emit(Event::State(state.apply(&v, &me.device_id)));
                }
                Err(e) => return e,
            },
            o = rx.recv() => {
                let Some(o) = o else { return End::Closed };
                let msg = outgoing(&me.device_id, o);
                if btx.send(frame(&pb::encode_request(&msg))).is_err() {
                    return End::Error("хаб: поток запроса закрыт".into(), healthy);
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

fn queue_json(me: &str, q: &QueueState) -> Value {
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
    queue
}

fn outgoing(me: &str, o: Out) -> Value {
    let player = |q: &QueueState, s: Status| json!({"update_player_state": {"player_state": {"player_queue": queue_json(me, q), "status": status_json(me, s)}}});
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
        Out::Player(q, s) => envelope(player(&q, s), "DO_NOT_INTERCEPT_BY_DEFAULT"),
        Out::Activate => envelope(
            json!({"update_active_device": {"device_id_optional": me}}),
            "DO_NOT_INTERCEPT_BY_DEFAULT",
        ),
        Out::Transfer(id) => envelope(
            json!({"update_active_device": {"device_id_optional": id}}),
            "DO_NOT_INTERCEPT_BY_DEFAULT",
        ),
        Out::RemoteStatus(s) => envelope(
            json!({"update_playing_status": {"playing_status": status_json(me, s)}}),
            "DO_NOT_INTERCEPT_BY_DEFAULT",
        ),
        Out::RemotePlayer(q, s) => envelope(player(&q, s), "DO_NOT_INTERCEPT_BY_DEFAULT"),
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
                    kind: s(&d["info"]["type"]),
                    can_play: d["capabilities"]["can_be_player"].as_bool().unwrap_or(true),
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

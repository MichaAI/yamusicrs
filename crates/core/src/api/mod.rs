//! Клиент неофициального API Яндекс Музыки (`api.music.yandex.net`).
//!
//! Эндпоинты и схемы подписей взяты из MarshalX/yandex-music-api.

pub mod auth;
pub mod models;

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::Sha256;

pub use models::*;

const BASE: &str = "https://api.music.yandex.net";
/// Новый API веб-клиента: на нём живёт «колесо» волны.
const WEB_BASE: &str = "https://api.music.yandex.ru";
const ANDROID_CLIENT: &str = "YandexMusicAndroid/24023621";
/// `get-file-info` подписывается ключом десктопного клиента и проверяется вместе с этим заголовком.
const DESKTOP_CLIENT: &str = "YandexMusicDesktopAppWindows/5.95.0";
const DESKTOP_SIGN_KEY: &[u8] = b"kzqU4XhfCaY6B6JTHODeq5";
const DEFAULT_SIGN_KEY: &[u8] = b"p93jhgh689SBReK6ghtw62";
/// Размер страницы лайкнутых альбомов/исполнителей.
const LIKES_PAGE: usize = 100;
/// Потолок на один запрос к API (не к загрузкам файлов).
const API_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const FILE_CODECS: &[&str] = &["flac", "flac-mp4", "aac-mp4", "he-aac-mp4", "aac", "he-aac", "mp3"];

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("сеть: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API {status}: {name}: {message}")]
    Api { status: u16, name: String, message: String },
    #[error("неожиданный ответ: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("нужна авторизация")]
    Unauthorized,
}

pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    token: String,
    pub uid: String,
}

/// Качество, которое просим у `get-file-info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Lossless,
    Hq,
    Nq,
    /// Как «Авто» мобильного клиента: качество выбирает сервер по скорости и типу сети
    /// из [`NET`].
    Auto,
}

impl Quality {
    /// Строка для `get-file-info` и `currentQuality` (как ebe.v0 официального клиента).
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Lossless => "lossless",
            Quality::Hq => "hq",
            Quality::Nq => "nq",
            Quality::Auto => "auto",
        }
    }
}

/// Что известно о сети — для качества «Авто». Мобильный клиент шлёт то же самое:
/// скорость, замеренную плеером на загрузках, оценку канала от системы и тип сети.
pub struct NetHints {
    /// Замер по нашим загрузкам, кбит/с; −1 — ещё не мерили.
    pub measured_kbps: std::sync::atomic::AtomicI32,
    /// Оценка системы (Android: linkDownstreamBandwidthKbps); −1 — нет.
    pub estimated_kbps: std::sync::atomic::AtomicI32,
    /// 0 — прочая (как Wi-Fi), 1 — Wi-Fi, 2 — мобильная, 3 — нет сети.
    pub network: std::sync::atomic::AtomicU8,
}

pub static NET: NetHints = NetHints {
    measured_kbps: std::sync::atomic::AtomicI32::new(-1),
    estimated_kbps: std::sync::atomic::AtomicI32::new(-1),
    network: std::sync::atomic::AtomicU8::new(0),
};

impl NetHints {
    /// Новый замер скорости загрузки: сглаживаем, чтобы один медленный ответ не ронял качество.
    pub fn measured(&self, kbps: i32) {
        use std::sync::atomic::Ordering::Relaxed;
        let old = self.measured_kbps.load(Relaxed);
        let v = if old < 0 { kbps } else { (old as i64 * 7 / 10 + kbps as i64 * 3 / 10) as i32 };
        self.measured_kbps.store(v, Relaxed);
    }

    /// Параметры «Авто» для `get-file-info` (пустые значения не шлются, как у официального клиента).
    fn query(&self) -> Vec<(&'static str, String)> {
        use std::sync::atomic::Ordering::Relaxed;
        let mut q = Vec::new();
        let m = self.measured_kbps.load(Relaxed);
        if m >= 0 {
            q.push(("measuredNetworkSpeedKbps", m.to_string()));
        }
        let e = self.estimated_kbps.load(Relaxed);
        if e >= 0 {
            q.push(("estimatedNetworkSpeedKbps", e.to_string()));
        }
        match self.network.load(Relaxed) {
            0 | 1 => q.push(("network", "wifi".into())),
            2 => q.push(("network", "mobile".into())),
            _ => {}
        }
        q
    }
}

pub fn http_client() -> reqwest::Client {
    let b = reqwest::Client::builder()
        .user_agent(concat!("yamusicrs/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        // Зависшая загрузка (сеть пропала посреди трека) не висит вечно: дальше её докачает повтор.
        .read_timeout(std::time::Duration::from_secs(20))
        // Соединение живёт между редкими действиями (пауза в пару минут — и снова без рукопожатия).
        .pool_idle_timeout(std::time::Duration::from_secs(300))
        .tcp_keepalive(std::time::Duration::from_secs(45));
    with_roots(b).build().expect("reqwest client")
}

/// Корни TLS: платформенному верификатору на Android нужна инициализация через JNI и свой
/// AAR — там проверяем по корням Mozilla.
pub fn with_roots(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    #[cfg(target_os = "android")]
    let b = b.tls_certs_only(
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|c| reqwest::Certificate::from_der(c).ok()),
    );
    b
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Сегмент пути: всё, кроме букв/цифр ASCII и `-_.~`, — в %XX (теги бывают кириллицей, с пробелами и запятыми).
fn path_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn hmac_b64(key: &[u8], msg: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    mac.update(msg.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

impl Client {
    pub fn new(token: String) -> Self {
        Self {
            http: http_client(),
            token,
            uid: String::new(),
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.req_as(method, path, ANDROID_CLIENT)
    }

    /// `.header()` в reqwest добавляет, а не заменяет, поэтому клиент задаётся ровно один раз.
    fn req_as(&self, method: reqwest::Method, path: &str, client: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{BASE}{path}"))
            .header("Authorization", format!("OAuth {}", self.token))
            .header("X-Yandex-Music-Client", client)
            .header("Accept-Language", "ru")
            .timeout(API_TIMEOUT)
    }

    /// Запрос с повторами: сбой соединения — всегда (запрос не ушёл), таймаут и 502/503/504 —
    /// только для GET (повторять POST с побочным эффектом нельзя). До двух повторов, 0,3 и 0,9 с.
    async fn send_raw(&self, mut rb: reqwest::RequestBuilder) -> Result<(reqwest::StatusCode, bytes::Bytes)> {
        let mut delay = std::time::Duration::from_millis(300);
        for attempt in 0.. {
            let again = rb.try_clone().filter(|_| attempt < 2);
            let (req_client, req) = rb.build_split();
            let req = req?;
            let get = req.method() == reqwest::Method::GET;
            let url = req.url().path().to_string();
            let r = async {
                let resp = req_client.execute(req).await?;
                let status = resp.status();
                Ok::<_, reqwest::Error>((status, resp.bytes().await?))
            }
            .await;
            let retry = match &r {
                Ok((st, _)) => get && matches!(st.as_u16(), 502..=504),
                Err(e) => e.is_connect() || (get && (e.is_timeout() || e.is_request() || e.is_body())),
            };
            match again {
                Some(next) if retry => {
                    tracing::debug!("{url}: попытка {} не удалась, повтор через {delay:?}", attempt + 1);
                    tokio::time::sleep(delay).await;
                    delay *= 3;
                    rb = next;
                }
                _ => return Ok(r?),
            }
        }
        unreachable!()
    }

    async fn send<T: DeserializeOwned>(&self, rb: reqwest::RequestBuilder) -> Result<T> {
        let (status, body) = self.send_raw(rb).await?;
        let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if !status.is_success() || v.get("error").is_some_and(|e| !e.is_null()) {
            let err = v.get("error").cloned().unwrap_or(Value::Null);
            let (name, message) = match &err {
                Value::Object(o) => (
                    o.get("name").and_then(Value::as_str).unwrap_or("error").to_string(),
                    o.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
                ),
                Value::String(s) => (s.clone(), String::new()),
                _ => (
                    "http".into(),
                    String::from_utf8_lossy(&body).chars().take(200).collect(),
                ),
            };
            return Err(ApiError::Api {
                status: status.as_u16(),
                name,
                message,
            });
        }
        let result = v.get("result").cloned().unwrap_or(v);
        Ok(serde_json::from_value(result)?)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        self.send(self.req(reqwest::Method::GET, path).query(query)).await
    }

    async fn post_form<T: DeserializeOwned>(&self, path: &str, form: &[(&str, String)]) -> Result<T> {
        self.send(self.req(reqwest::Method::POST, path).form(form)).await
    }

    async fn post_json<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        self.send(self.req(reqwest::Method::POST, path).json(body)).await
    }

    pub async fn account_status(&mut self) -> Result<AccountStatus> {
        let st: AccountStatus = self.get("/account/status", &[]).await?;
        match &st.account.uid {
            Some(uid) => self.uid = uid.clone(),
            None => return Err(ApiError::Unauthorized),
        }
        Ok(st)
    }

    // ── треки ────────────────────────────────────────────────────────────────

    pub async fn tracks(&self, ids: &[String]) -> Result<Vec<Track>> {
        // Пачки по 250 — параллельно, порядок сохраняется.
        let batches = futures_util::future::try_join_all(ids.chunks(250).map(|chunk| async move {
            let form = [("track-ids", chunk.join(",")), ("with-positions", "true".into())];
            self.post_form::<Vec<serde_json::Value>>("/tracks", &form).await
        }))
        .await?;
        let mut out = Vec::with_capacity(ids.len());
        for batch in batches {
            out.extend(batch.into_iter().filter_map(|v| {
                serde_json::from_value::<Track>(v)
                    .map_err(|e| tracing::warn!("пропускаю трек: {e}"))
                    .ok()
            }));
        }
        Ok(out)
    }

    pub async fn file_info(&self, track_id: &str, quality: Quality) -> Result<DownloadInfo> {
        let ts = now_ts();
        let codecs = FILE_CODECS.join("");
        let transport = "encraw";
        let msg = format!("{ts}{track_id}{}{codecs}{transport}", quality.as_str());
        let mut sign = hmac_b64(DESKTOP_SIGN_KEY, &msg);
        sign.pop();
        let rb = self
            .req_as(reqwest::Method::GET, "/get-file-info", DESKTOP_CLIENT)
            .query(&[
                ("ts", ts.to_string()),
                ("trackId", track_id.to_string()),
                ("quality", quality.as_str().to_string()),
                ("codecs", FILE_CODECS.join(",")),
                ("transports", transport.to_string()),
                ("sign", sign),
            ]);
        let rb = if quality == Quality::Auto { rb.query(&NET.query()) } else { rb };
        let fi: FileInfo = self.send(rb).await?;
        Ok(fi.download_info)
    }

    /// Текст песни. `synced` → LRC с таймкодами, иначе обычный текст.
    pub async fn lyrics(&self, track_id: &str, synced: bool) -> Result<String> {
        let numeric = track_id.split(':').next().unwrap_or(track_id);
        let ts = now_ts();
        let sign = hmac_b64(DEFAULT_SIGN_KEY, &format!("{numeric}{ts}"));
        let dl: LyricsDownload = self
            .get(
                &format!("/tracks/{numeric}/lyrics"),
                &[
                    ("format", if synced { "LRC" } else { "TEXT" }.into()),
                    ("timeStamp", ts.to_string()),
                    ("sign", sign),
                ],
            )
            .await?;
        Ok(self
            .http
            .get(dl.download_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?)
    }

    pub async fn play_audio(&self, track: &Track, played: f64, from: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let _: Value = self
            .post_form(
                "/play-audio",
                &[
                    ("track-id", track.id.clone()),
                    ("from-cache", "false".into()),
                    ("from", from.into()),
                    ("play-id", String::new()),
                    ("uid", self.uid.clone()),
                    ("timestamp", now.clone()),
                    ("track-length-seconds", format!("{:.0}", track.duration_secs())),
                    ("total-played-seconds", format!("{played:.0}")),
                    ("end-position-seconds", format!("{played:.0}")),
                    ("album-id", track.album().map(|a| a.id.clone()).unwrap_or_default()),
                    ("playlist-id", String::new()),
                    ("client-now", now),
                ],
            )
            .await?;
        Ok(())
    }

    // ── библиотека ───────────────────────────────────────────────────────────

    pub async fn liked_track_ids(&self) -> Result<Vec<String>> {
        let lib: LikesLibrary = self
            .get(
                &format!("/users/{}/likes/tracks", self.uid),
                &[("if-modified-since-revision", "0".into())],
            )
            .await?;
        Ok(lib.library.tracks.iter().map(TrackShort::full_id).collect())
    }

    pub async fn set_like(&self, track_id: &str, like: bool) -> Result<()> {
        let action = if like { "add-multiple" } else { "remove" };
        let _: Value = self
            .post_form(
                &format!("/users/{}/likes/tracks/{action}", self.uid),
                &[("track-ids", track_id.to_string())],
            )
            .await?;
        Ok(())
    }

    pub async fn dislike(&self, track_id: &str) -> Result<()> {
        let _: Value = self
            .post_form(
                &format!("/users/{}/dislikes/tracks/add-multiple", self.uid),
                &[("track-ids", track_id.to_string())],
            )
            .await?;
        Ok(())
    }

    // ── лайки альбомов и исполнителей ─────────────────────────────────────────

    /// id лайкнутых (1) и скрытых (-1) альбомов и исполнителей — одним запросом, как в десктопе.
    pub async fn library_ids(&self) -> Result<LibraryIds> {
        self.send(self.req_as(reqwest::Method::GET, "/library/all-ids", DESKTOP_CLIENT)).await
    }

    /// Лайк альбома (пути и поля — как в мобильном клиенте).
    pub async fn set_album_like(&self, album_id: &str, like: bool) -> Result<()> {
        let action = if like { "add-multiple" } else { "remove" };
        let _: Value = self
            .post_form(
                &format!("/users/{}/likes/albums/{action}", self.uid),
                &[("albumIds", album_id.to_string())],
            )
            .await?;
        Ok(())
    }

    pub async fn set_artist_like(&self, artist_id: &str, like: bool) -> Result<()> {
        let action = if like { "add-multiple" } else { "remove" };
        let _: Value = self
            .post_form(
                &format!("/users/{}/likes/artists/{action}", self.uid),
                &[("artistIds", artist_id.to_string())],
            )
            .await?;
        Ok(())
    }

    /// «Не рекомендовать исполнителя». Снять — только десктопным путём (в мобильном его нет).
    pub async fn set_artist_dislike(&self, artist_id: &str, dislike: bool) -> Result<()> {
        let _: Value = if dislike {
            self.post_form(
                &format!("/users/{}/dislikes/artists/add-multiple", self.uid),
                &[("artist-ids", artist_id.to_string())],
            )
            .await?
        } else {
            let path = format!("/users/{}/dislikes/artists/{artist_id}/remove", self.uid);
            self.send(self.req_as(reqwest::Method::POST, &path, DESKTOP_CLIENT)).await?
        };
        Ok(())
    }

    /// Все лайкнутые альбомы, свежие сверху (постранично, как десктоп).
    pub async fn liked_albums(&self) -> Result<Vec<Album>> {
        #[derive(serde::Deserialize)]
        struct Page {
            #[serde(default)]
            albums: Vec<Liked>,
        }
        #[derive(serde::Deserialize)]
        struct Liked {
            album: Option<Value>,
        }
        let mut out = Vec::new();
        for page in 0.. {
            let path = format!("/users/{}/likes/albums/page", self.uid);
            let q = [
                ("rich", "true".to_string()),
                ("page", page.to_string()),
                ("pageSize", LIKES_PAGE.to_string()),
            ];
            let p: Page = self.send(self.req_as(reqwest::Method::GET, &path, DESKTOP_CLIENT).query(&q)).await?;
            let n = p.albums.len();
            out.extend(
                p.albums
                    .into_iter()
                    .filter_map(|l| serde_json::from_value::<Album>(l.album?).ok()),
            );
            if n < LIKES_PAGE {
                break;
            }
        }
        Ok(out)
    }

    pub async fn liked_artists(&self) -> Result<Vec<Artist>> {
        #[derive(serde::Deserialize)]
        struct Page {
            #[serde(default)]
            artists: Vec<Value>,
        }
        let mut out = Vec::new();
        for page in 0.. {
            let path = format!("/users/{}/likes/artists/page", self.uid);
            let q = [("page", page.to_string()), ("pageSize", LIKES_PAGE.to_string())];
            let p: Page = self.send(self.req_as(reqwest::Method::GET, &path, DESKTOP_CLIENT).query(&q)).await?;
            let n = p.artists.len();
            out.extend(p.artists.into_iter().filter_map(|a| serde_json::from_value::<Artist>(a).ok()));
            if n < LIKES_PAGE {
                break;
            }
        }
        Ok(out)
    }

    // ── правка плейлистов (пути и параметры — как в мобильном клиенте) ───────

    pub async fn playlist_create(&self, title: &str) -> Result<Playlist> {
        let q = [("title", title.to_string()), ("visibility", "private".into())];
        self.send(
            self.req(reqwest::Method::POST, &format!("/users/{}/playlists/create", self.uid))
                .query(&q),
        )
        .await
    }

    /// Описание своего плейлиста (как в мобильном клиенте: `value` в query).
    pub async fn playlist_describe(&self, kind: &str, text: &str) -> Result<()> {
        let path = format!("/users/{}/playlists/{kind}/description", self.uid);
        let _: Value = self
            .send(self.req(reqwest::Method::POST, &path).query(&[("value", text)]))
            .await?;
        Ok(())
    }

    pub async fn playlist_visibility(&self, kind: &str, public: bool) -> Result<Playlist> {
        let path = format!("/users/{}/playlists/{kind}/visibility", self.uid);
        let v = if public { "public" } else { "private" };
        self.send(self.req(reqwest::Method::POST, &path).query(&[("value", v)])).await
    }

    pub async fn playlist_rename(&self, kind: &str, title: &str) -> Result<Playlist> {
        self.send(
            self.req(
                reqwest::Method::POST,
                &format!("/users/{}/playlists/{kind}/name", self.uid),
            )
            .query(&[("value", title)]),
        )
        .await
    }

    pub async fn playlist_delete(&self, kind: &str) -> Result<()> {
        let _: Value = self
            .send(self.req(
                reqwest::Method::POST,
                &format!("/users/{}/playlists/{kind}/delete", self.uid),
            ))
            .await?;
        Ok(())
    }

    /// Правка состава: `diff` — массив операций insert/delete над позициями.
    async fn playlist_change(&self, kind: &str, revision: u32, diff: Value) -> Result<Playlist> {
        self.send(
            self.req(
                reqwest::Method::POST,
                &format!("/users/{}/playlists/{kind}/change-relative", self.uid),
            )
            .query(&[("revision", revision.to_string())])
            .form(&[("diff", diff.to_string())]),
        )
        .await
    }

    pub async fn playlist_insert(&self, kind: &str, revision: u32, at: usize, track_id: &str) -> Result<Playlist> {
        let diff = json!([{ "op": "insert", "at": at, "tracks": [diff_track(track_id)] }]);
        self.playlist_change(kind, revision, diff).await
    }

    pub async fn playlist_remove(&self, kind: &str, revision: u32, at: usize, track_id: &str) -> Result<Playlist> {
        let diff = json!([{ "op": "delete", "from": at, "to": at + 1, "tracks": [diff_track(track_id)] }]);
        self.playlist_change(kind, revision, diff).await
    }

    /// Перестановка — удаление и вставка одной правкой.
    pub async fn playlist_move(
        &self,
        kind: &str,
        revision: u32,
        from: usize,
        to: usize,
        track_id: &str,
    ) -> Result<Playlist> {
        let t = diff_track(track_id);
        let diff = json!([
            { "op": "delete", "from": from, "to": from + 1, "tracks": [t.clone()] },
            { "op": "insert", "at": to, "tracks": [t] },
        ]);
        self.playlist_change(kind, revision, diff).await
    }

    /// Заголовок плейлиста без объектов треков: ревизия и порядок id — для правки.
    pub async fn playlist_header(&self, kind: &str) -> Result<Playlist> {
        self.get(
            &format!("/users/{}/playlists/{kind}", self.uid),
            &[("rich-tracks", "false".into())],
        )
        .await
    }

    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        self.get(&format!("/users/{}/playlists/list", self.uid), &[]).await
    }

    pub async fn playlist(&self, uid: &str, kind: &str) -> Result<Playlist> {
        self.get(
            &format!("/users/{uid}/playlists/{kind}"),
            &[("rich-tracks", "true".into())],
        )
        .await
    }

    /// Полный список треков плейлиста: если сервер не вложил объекты треков, догружаем их пачкой.
    pub async fn playlist_tracks(&self, pl: &Playlist) -> Result<Vec<Track>> {
        let missing: Vec<String> = pl
            .tracks
            .iter()
            .filter(|t| t.track.is_none())
            .map(TrackShort::full_id)
            .collect();
        let mut fetched = if missing.is_empty() {
            Vec::new()
        } else {
            self.tracks(&missing).await?
        }
        .into_iter();
        Ok(pl
            .tracks
            .iter()
            .filter_map(|t| t.track.clone().or_else(|| fetched.next()))
            .collect())
    }

    /// «Вы недавно слушали»: треки всех контекстов подряд, свежие первыми, без повторов.
    /// Полные модели сервер даёт только для первых; остальные догружаются пачкой.
    pub async fn history(&self) -> Result<Vec<Track>> {
        let v: Value = self.get("/music-history", &[("fullModelsCount", "50".into())]).await?;
        let mut seen = std::collections::HashSet::new();
        let mut slots: Vec<(String, Option<Track>)> = Vec::new();
        let tabs = v["historyTabs"].as_array().cloned().unwrap_or_default();
        for item in tabs.iter().flat_map(|t| t["items"].as_array().cloned().unwrap_or_default()) {
            for t in item["tracks"].as_array().cloned().unwrap_or_default() {
                let d = &t["data"];
                let Some(id) = d["itemId"]["trackId"].as_str() else { continue };
                if !seen.insert(id.to_string()) {
                    continue;
                }
                let full_id = match d["itemId"]["albumId"].as_str() {
                    Some(a) => format!("{id}:{a}"),
                    None => id.to_string(),
                };
                let track = serde_json::from_value::<Track>(d["fullModel"].clone()).ok();
                slots.push((full_id, track));
            }
        }
        let missing: Vec<String> = slots.iter().filter(|s| s.1.is_none()).map(|s| s.0.clone()).collect();
        let mut fetched: std::collections::HashMap<String, Track> = std::collections::HashMap::new();
        for chunk in missing.chunks(200) {
            for t in self.tracks(chunk).await? {
                fetched.insert(t.id.clone(), t);
            }
        }
        Ok(slots
            .into_iter()
            .filter_map(|(id, t)| t.or_else(|| fetched.remove(id.split(':').next().unwrap_or(&id))))
            .collect())
    }

    /// Чарт «Яндекс Музыки» — обычный плейлист внутри ответа лендинга.
    pub async fn chart(&self) -> Result<Playlist> {
        let v: Value = self.get("/landing3/chart", &[]).await?;
        Ok(serde_json::from_value(v["chart"].clone())?)
    }

    /// Блок главной «Настроили для вас» (как в десктопе).
    pub async fn personal_playlists(&self) -> Result<Vec<PersonalPlaylist>> {
        let v: Value = self
            .send(self.req_as(reqwest::Method::GET, "/landing-blocks/personal-playlists", DESKTOP_CLIENT))
            .await?;
        Ok(v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| serde_json::from_value::<PersonalPlaylist>(i["data"].clone()).ok())
            .collect())
    }

    /// Авторы, лейбл, источник фонограммы: пары «роль — имена».
    pub async fn credits(&self, track_id: &str) -> Result<Vec<(String, String)>> {
        let numeric = track_id.split(':').next().unwrap_or(track_id);
        let v: Value = self.get(&format!("/tracks/{numeric}/credits"), &[]).await?;
        Ok(v["credits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c["title"].as_str()?.to_string(), c["value"].as_str()?.to_string())))
            .collect())
    }

    /// Дерево тегов: настроения, занятия, жанры, эпохи.
    pub async fn metatags(&self) -> Result<Vec<MetaTree>> {
        let v: Value = self
            .send(self.req_as(reqwest::Method::GET, "/landing3/metatags", DESKTOP_CLIENT))
            .await?;
        Ok(serde_json::from_value(v["trees"].clone())?)
    }

    /// Страница тега (параметры — как у десктопа).
    pub async fn metatag(&self, tag: &str) -> Result<Metatag> {
        let path = format!("/metatags/{}", path_escape(tag));
        let q = [
            ("playlistsCount", "30".to_string()),
            ("albumsCount", "30".into()),
            ("artistsCount", "30".into()),
            ("tracksCount", "0".into()),
        ];
        self.send(self.req_as(reqwest::Method::GET, &path, DESKTOP_CLIENT).query(&q)).await
    }

    /// Топ исполнителей за месяц: исполнитель, сколько слушал (с), движение в топе (`up`/`down`/`same`/`new`).
    pub async fn top_artists(&self) -> Result<Vec<(Artist, u64, String)>> {
        let v: Value = self
            .send(self.req_as(reqwest::Method::GET, "/personal/top/artists/month", DESKTOP_CLIENT))
            .await?;
        Ok(v["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let a = serde_json::from_value::<Artist>(e["artist"].clone()).ok()?;
                let secs = e["listenTimeSeconds"].as_u64().unwrap_or(0);
                let progress = e["top"]["progress"].as_str().unwrap_or("").to_string();
                Some((a, secs, progress))
            })
            .collect())
    }

    /// Серверная история поиска: найденное и открытое на любом устройстве.
    pub async fn search_history(&self) -> Result<Vec<Found>> {
        let path = format!("/users/{}/search-history", self.uid);
        let v: Vec<Value> = self.send(self.req_as(reqwest::Method::GET, &path, DESKTOP_CLIENT)).await?;
        Ok(v.into_iter()
            .filter_map(|e| {
                let kind = e["type"].as_str()?.to_string();
                let o = e[kind.as_str()].clone();
                Some(match kind.as_str() {
                    "track" => Found::Track(serde_json::from_value(o).ok()?),
                    "album" => Found::Album(serde_json::from_value(o).ok()?),
                    "artist" => Found::Artist(serde_json::from_value(o).ok()?),
                    "playlist" => Found::Playlist(serde_json::from_value(o).ok()?),
                    _ => return None,
                })
            })
            .collect())
    }

    /// «Похожие треки» (как в мобильном клиенте).
    pub async fn similar_tracks(&self, track_id: &str) -> Result<Vec<Track>> {
        let numeric = track_id.split(':').next().unwrap_or(track_id);
        let v: Value = self.get(&format!("/tracks/{numeric}/similar"), &[]).await?;
        Ok(v["similarTracks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| serde_json::from_value::<Track>(t.clone()).ok())
            .collect())
    }

    pub async fn album(&self, id: &str) -> Result<Album> {
        self.get(&format!("/albums/{id}/with-tracks"), &[]).await
    }

    /// «Новые релизы» с главной: id альбомов, затем сами альбомы одной пачкой.
    pub async fn new_releases(&self) -> Result<Vec<Album>> {
        let nr: NewReleases = self.get("/landing3/new-releases", &[]).await?;
        let ids: Vec<String> = nr.new_releases.iter().map(u64::to_string).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let raw: Vec<Value> = self.post_form("/albums", &[("album-ids", ids.join(","))]).await?;
        Ok(raw
            .into_iter()
            .filter_map(|v| {
                serde_json::from_value::<Album>(v)
                    .map_err(|e| tracing::warn!("пропускаю альбом: {e}"))
                    .ok()
            })
            .collect())
    }

    /// Карточка исполнителя; дискография целиком догружается вторым запросом
    /// (в `brief-info` только десяток альбомов). Её ошибка карточку не роняет.
    pub async fn artist(&self, id: &str) -> Result<ArtistBrief> {
        let (bp, ap) = (format!("/artists/{id}/brief-info"), format!("/artists/{id}/direct-albums"));
        let q = [
            ("page", "0".to_string()),
            ("page-size", "200".into()),
            ("sort-by", "year".into()),
        ];
        let (fp, cp) = (format!("/artists/{id}/familiar-you"), format!("/artists/{id}/concerts"));
        let fq = [
            ("waveTracksLimit", "100".to_string()),
            ("collectionTracksLimit", "100".into()),
        ];
        let (brief, albums, familiar, concerts, all) = tokio::join!(
            self.get::<ArtistBrief>(&bp, &[]),
            self.get::<ArtistAlbums>(&ap, &q),
            self.send::<Value>(self.req_as(reqwest::Method::GET, &fp, DESKTOP_CLIENT).query(&fq)),
            self.send::<Value>(self.req_as(reqwest::Method::GET, &cp, DESKTOP_CLIENT)),
            self.artist_tracks(id),
        );
        let mut brief = brief?;
        // В `brief-info` — лишь несколько популярных; полный список — постранично.
        if !all.is_empty() {
            brief.popular_tracks = all;
        }
        match albums {
            Ok(a) => brief.discography = a.albums,
            Err(e) => tracing::warn!("дискография {id}: {e}"),
        }
        // Сперва то, что в коллекции, потом услышанное в волне; без повторов.
        match familiar {
            Ok(v) => {
                let mut seen = std::collections::HashSet::new();
                for part in ["collection", "wave"] {
                    for t in v[part]["tracks"].as_array().into_iter().flatten() {
                        if let Ok(t) = serde_json::from_value::<Track>(t.clone()) {
                            if part == "collection" {
                                brief.collection.push(t.clone());
                            } else {
                                brief.wave.push(t.clone());
                            }
                            if seen.insert(t.id.clone()) {
                                brief.familiar.push(t);
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("familiar-you {id}: {e}"),
        }
        match concerts {
            Ok(v) => {
                brief.concerts = v["concerts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| serde_json::from_value::<Concert>(c.clone()).ok())
                    .collect();
            }
            Err(e) => tracing::warn!("концерты {id}: {e}"),
        }
        Ok(brief)
    }

    /// Все треки исполнителя по порядку выдачи (популярные первыми), до 500; ошибка — пусто.
    async fn artist_tracks(&self, id: &str) -> Vec<Track> {
        let mut out: Vec<Track> = Vec::new();
        for page in 0..5 {
            let q = [("page", page.to_string()), ("page-size", "100".into())];
            let v: Value = match self.get(&format!("/artists/{id}/tracks"), &q).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("треки {id}: {e}");
                    break;
                }
            };
            let part: Vec<Track> = v["tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| serde_json::from_value(t.clone()).ok())
                .collect();
            let n = part.len();
            out.extend(part);
            let total = v["pager"]["total"].as_u64().unwrap_or(0) as usize;
            if n == 0 || out.len() >= total {
                break;
            }
        }
        out
    }

    /// Лёгкая карточка исполнителя для экрана волны: фото и концерты, без дискографии.
    /// Шапка исполнителя, как у экрана волны официального клиента (`/artists/{id}/info`):
    /// его фото (`artist.cover`), а нет — первое из `covers`. Ответ без обёртки `result`.
    pub async fn artist_photo(&self, id: &str) -> Result<Option<String>> {
        let v: Value = self.get(&format!("/artists/{id}/info"), &[]).await?;
        let uri = |c: &Value| c["uri"].as_str().filter(|u| !u.is_empty()).map(str::to_string);
        Ok(uri(&v["artist"]["cover"]).or_else(|| v["covers"].as_array()?.iter().find_map(uri)))
    }

    pub async fn search(&self, text: &str) -> Result<SearchResult> {
        let q = [
            ("text", text.to_string()),
            ("type", "all".into()),
            ("page", "0".into()),
            ("nocorrect", "false".into()),
        ];
        // Волны по запросу («грустное», «рок», «для сна») отдаёт только новый поиск — им же
        // пользуются официальные клиенты. Его сбой обычную выдачу не роняет.
        let (found, waves) = tokio::join!(self.get::<SearchResult>("/search", &q), self.search_waves(text));
        let mut found = found?;
        found.waves = waves.unwrap_or_else(|e| {
            tracing::debug!("волны по запросу: {e}");
            Vec::new()
        });
        Ok(found)
    }

    /// Волны по настроению, жанру, занятию, подходящие под запрос (`search/instant/mixed`).
    pub async fn search_waves(&self, text: &str) -> Result<Vec<SearchWave>> {
        let q = [
            ("text", text.to_string()),
            ("type", "wave".into()),
            ("page", "0".into()),
            ("pageSize", "12".into()),
            ("withLikesCount", "false".into()),
            ("withBestResults", "false".into()),
        ];
        let v: Value = self
            .send(self.req_as(reqwest::Method::GET, "/search/instant/mixed", DESKTOP_CLIENT).query(&q))
            .await?;
        Ok(v["results"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["type"] == "wave")
            .filter_map(|r| serde_json::from_value::<SearchWave>(r["wave"].clone()).ok())
            .filter(|w| !w.seeds.is_empty())
            .collect())
    }

    /// Адрес для загрузки своего трека в свой плейлист (как «Загрузить трек» веб-версии):
    /// сервер отвечает, куда слать файл.
    pub async fn upload_target(&self, kind: &str, file_name: &str) -> Result<String> {
        let q = [
            ("uid", self.uid.clone()),
            ("playlist-id", format!("{}:{kind}", self.uid)),
            ("visibility", "private".into()),
            ("path", file_name.to_string()),
        ];
        // Только POST (GET сервер не принимает); ответ без обёртки `result`.
        let v: Value = self.send(self.req(reqwest::Method::POST, "/loader/upload-url").query(&q)).await?;
        v["post-target"]
            .as_str()
            .map(str::to_string)
            .ok_or(ApiError::Api { status: 0, name: "upload".into(), message: "сервер не дал адрес загрузки".into() })
    }

    /// Загружает свой аудиофайл в плейлист `kind`: трек появится в нём после обработки сервером.
    pub async fn upload_track(&self, kind: &str, file_name: &str, data: Vec<u8>) -> Result<()> {
        let target = self.upload_target(kind, file_name).await?;
        let part = reqwest::multipart::Part::bytes(data).file_name(file_name.to_string());
        let form = reqwest::multipart::Form::new().part("file", part);
        let resp = self
            .http
            .post(target)
            .multipart(form)
            // Файл большой: потолок — на чтение (см. клиент), а не на запрос целиком.
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ApiError::Api {
                status: status.as_u16(),
                name: "upload".into(),
                message: body.chars().take(200).collect(),
            });
        }
        Ok(())
    }

    // ── Моя волна ────────────────────────────────────────────────────────────

    /// Персональные пресеты Моей волны — то, что веб показывает дугой.
    pub async fn wheel(&self) -> Result<Vec<WheelItem>> {
        let rb = self
            .http
            .post(format!("{WEB_BASE}/wheel/new"))
            .header("Authorization", format!("OAuth {}", self.token))
            .header("X-Yandex-Music-Client", ANDROID_CLIENT)
            .header("Accept-Language", "ru")
            .timeout(API_TIMEOUT)
            .json(&json!({}));
        let w: Wheel = self.send(rb).await?;
        Ok(w.items
            .into_iter()
            .filter(|i| i.data.wave.as_ref().is_some_and(|w| !w.seeds.is_empty()))
            .collect())
    }

    /// Последняя включённая волна (десктоп так подписывает главную кнопку).
    pub async fn wave_last(&self) -> Result<WheelWave> {
        self.send(self.req_as(reqwest::Method::GET, "/rotor/wave/last", DESKTOP_CLIENT)).await
    }

    /// Программа офлайн-волны под скачанные треки (как в мобильном клиенте).
    pub async fn offline_recommender(&self, liked: &[String], recommender: &[String]) -> Result<Vec<i32>> {
        let v: Value = self
            .post_json(
                "/rotor/get-offline-recommender",
                &json!({
                    "autoCachedLikedTracks": liked,
                    "autoCachedRecommenderTracks": recommender,
                    "userCachedTracks": [],
                }),
            )
            .await?;
        Ok(serde_json::from_value(v["offlineRecommenderSource"].clone())?)
    }

    /// Умное скачивание (как в мобильном клиенте): что докачать для офлайн-волны и что удалить.
    /// `body` — `SmartDownloadingTracksRequestDto` официального клиента.
    pub async fn cache_recommendation(&self, body: &Value) -> Result<CacheRecommendation> {
        self.post_json("/rotor/get-cache-recommendation", body).await
    }

    pub async fn rotor_new(&self, seeds: &[String]) -> Result<RotorSession> {
        self.post_json(
            "/rotor/session/new",
            &json!({ "seeds": seeds, "includeTracksInResponse": true, "includeWaveModel": false, "interactive": true }),
        )
        .await
    }

    pub async fn rotor_tracks(&self, session: &str, queue: &[String]) -> Result<RotorTracks> {
        self.post_json(&format!("/rotor/session/{session}/tracks"), &json!({ "queue": queue }))
            .await
    }

    pub async fn rotor_feedback(
        &self,
        session: &str,
        batch_id: Option<&str>,
        kind: &str,
        track_id: Option<&str>,
        played: Option<f64>,
    ) -> Result<()> {
        let mut event = json!({
            "type": kind,
            "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        });
        if let Some(t) = track_id {
            event["trackId"] = json!(t);
        }
        if let Some(p) = played {
            event["totalPlayedSeconds"] = json!(p);
        }
        let mut body = json!({ "event": event });
        if let Some(b) = batch_id {
            body["batchId"] = json!(b);
        }
        let _: Value = self
            .post_json(&format!("/rotor/session/{session}/feedback"), &body)
            .await?;
        Ok(())
    }
}

/// `trackId:albumId` → `{"id", "albumId"}` для diff плейлиста.
fn diff_track(full_id: &str) -> Value {
    match full_id.split_once(':') {
        Some((id, album)) => json!({ "id": id, "albumId": album }),
        None => json!({ "id": full_id }),
    }
}

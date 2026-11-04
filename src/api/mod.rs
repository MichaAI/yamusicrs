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
}

impl Quality {
    fn as_str(self) -> &'static str {
        match self {
            Quality::Lossless => "lossless",
            Quality::Hq => "hq",
            Quality::Nq => "nq",
        }
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("yamusicrs/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("reqwest client")
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
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
    }

    async fn send<T: DeserializeOwned>(&self, rb: reqwest::RequestBuilder) -> Result<T> {
        let resp = rb.send().await?;
        let status = resp.status();
        let body = resp.bytes().await?;
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
        let (brief, albums) = tokio::join!(self.get::<ArtistBrief>(&bp, &[]), self.get::<ArtistAlbums>(&ap, &q));
        let mut brief = brief?;
        match albums {
            Ok(a) => brief.discography = a.albums,
            Err(e) => tracing::warn!("дискография {id}: {e}"),
        }
        Ok(brief)
    }

    pub async fn search(&self, text: &str) -> Result<SearchResult> {
        self.get(
            "/search",
            &[
                ("text", text.into()),
                ("type", "all".into()),
                ("page", "0".into()),
                ("nocorrect", "false".into()),
            ],
        )
        .await
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

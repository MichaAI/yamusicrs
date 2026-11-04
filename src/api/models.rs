//! Модели ответов API. Описаны только поля, которые реально используются клиентом.

use serde::{Deserialize, Deserializer, Serialize};

/// Яндекс отдаёт идентификаторы то строкой, то числом — принимаем оба варианта.
pub fn id_str<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        S(String),
        N(i64),
    }
    Ok(match Id::deserialize(d)? {
        Id::S(s) => s,
        Id::N(n) => n.to_string(),
    })
}

fn opt_id_str<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        S(String),
        N(i64),
    }
    Ok(Option::<Id>::deserialize(d)?.map(|id| match id {
        Id::S(s) => s,
        Id::N(n) => n.to_string(),
    }))
}

fn id_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(opt_id_str(d)?.unwrap_or_default())
}

/// Разбирает массив поштучно: один кривой элемент не должен ронять весь список.
pub fn lenient_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = Vec::<serde_json::Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|v| match serde_json::from_value(v) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!("пропускаю элемент ответа: {e}");
                None
            }
        })
        .collect())
}

/// Превращает `avatars.yandex.net/.../%%` в полноценный URL нужного размера.
pub fn cover_url(uri: &str, size: u32) -> String {
    let u = uri.replace("%%", &format!("{size}x{size}"));
    // Обложки агентов волны приходят уже с протоколом.
    if u.starts_with("http") {
        u
    } else {
        format!("https://{u}")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    /// У исполнителей загруженных пользователем треков id нет — только имя.
    #[serde(default, deserialize_with = "id_or_empty")]
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cover {
    pub uri: Option<String>,
    #[serde(default)]
    pub items_uri: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct TrackPosition {
    #[serde(default)]
    pub volume: u32,
    #[serde(default)]
    pub index: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    #[serde(default, deserialize_with = "id_or_empty")]
    pub id: String,
    pub title: String,
    pub version: Option<String>,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    pub genre: Option<String>,
    pub cover_uri: Option<String>,
    pub og_image: Option<String>,
    pub track_count: Option<u32>,
    pub track_position: Option<TrackPosition>,
    /// `single`, `compilation`, `podcast`…; у обычного альбома поля нет.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// `explicit` — с ненормативной лексикой.
    pub content_warning: Option<String>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    /// Только в `/albums/{id}/with-tracks`.
    #[serde(default)]
    pub volumes: Vec<Vec<serde_json::Value>>,
}

impl Album {
    pub fn cover_uri(&self) -> Option<&str> {
        self.cover_uri.as_deref().or(self.og_image.as_deref())
    }

    pub fn full_title(&self) -> String {
        match &self.version {
            Some(v) if !v.is_empty() => format!("{} ({v})", self.title),
            _ => self.title.clone(),
        }
    }

    pub fn artist_names(&self) -> String {
        join_artists(&self.artists)
    }

    /// Тип релиза для подписи под обложкой.
    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_deref() {
            Some("single") => "Сингл",
            Some("compilation") => "Сборник",
            Some("podcast") => "Подкаст",
            Some("audiobook") => "Аудиокнига",
            _ => "Альбом",
        }
    }

    pub fn explicit(&self) -> bool {
        self.content_warning.as_deref() == Some("explicit")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewReleases {
    #[serde(default)]
    pub new_releases: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LyricsInfo {
    #[serde(default)]
    pub has_available_sync_lyrics: bool,
    #[serde(default)]
    pub has_available_text_lyrics: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct R128 {
    /// Интегральная громкость, LUFS.
    pub i: f32,
    /// True peak, dBTP.
    pub tp: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    #[serde(deserialize_with = "id_str")]
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub version: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default = "yes")]
    pub available: bool,
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub albums: Vec<Album>,
    pub cover_uri: Option<String>,
    pub og_image: Option<String>,
    pub lyrics_info: Option<LyricsInfo>,
    pub content_warning: Option<String>,
    pub r128: Option<R128>,
}

fn yes() -> bool {
    true
}

impl Track {
    pub fn full_title(&self) -> String {
        match &self.version {
            Some(v) if !v.is_empty() => format!("{} ({v})", self.title),
            _ => self.title.clone(),
        }
    }

    pub fn artist_names(&self) -> String {
        join_artists(&self.artists)
    }

    pub fn album(&self) -> Option<&Album> {
        self.albums.first()
    }

    pub fn cover_uri(&self) -> Option<&str> {
        self.cover_uri
            .as_deref()
            .or_else(|| self.album().and_then(|a| a.cover_uri()))
            .or(self.og_image.as_deref())
    }

    /// Идентификатор в форме `trackId:albumId`, которую ждут rotor и likes.
    pub fn full_id(&self) -> String {
        match self.album() {
            Some(a) => format!("{}:{}", self.id, a.id),
            None => self.id.clone(),
        }
    }

    pub fn duration_secs(&self) -> f64 {
        self.duration_ms.unwrap_or(0) as f64 / 1000.0
    }

    pub fn explicit(&self) -> bool {
        self.content_warning.as_deref() == Some("explicit")
    }

    pub fn has_sync_lyrics(&self) -> bool {
        self.lyrics_info.as_ref().is_some_and(|l| l.has_available_sync_lyrics)
    }

    pub fn has_text_lyrics(&self) -> bool {
        self.lyrics_info.as_ref().is_some_and(|l| l.has_available_text_lyrics)
    }
}

pub fn join_artists(artists: &[Artist]) -> String {
    let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
    if names.is_empty() {
        "Неизвестный исполнитель".into()
    } else {
        names.join(", ")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub login: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackShort {
    #[serde(deserialize_with = "id_str")]
    pub id: String,
    #[serde(default, deserialize_with = "opt_id_str")]
    pub album_id: Option<String>,
    pub track: Option<Track>,
}

impl TrackShort {
    pub fn full_id(&self) -> String {
        match &self.album_id {
            Some(a) => format!("{}:{a}", self.id),
            None => self.id.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    #[serde(deserialize_with = "id_str")]
    pub uid: String,
    #[serde(deserialize_with = "id_str")]
    pub kind: String,
    pub title: String,
    pub track_count: Option<u32>,
    /// Версия плейлиста: каждое изменение (`change-relative`) должно знать текущую.
    #[serde(default)]
    pub revision: u32,
    pub owner: Option<Owner>,
    pub cover: Option<Cover>,
    pub og_image: Option<String>,
    #[serde(default)]
    pub tracks: Vec<TrackShort>,
    pub description: Option<String>,
    /// `public` / `private`.
    pub visibility: Option<String>,
}

impl Playlist {
    pub fn cover_uri(&self) -> Option<&str> {
        self.cover
            .as_ref()
            .and_then(|c| c.uri.as_deref().or(c.items_uri.first().map(String::as_str)))
            .or(self.og_image.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    #[serde(default, deserialize_with = "opt_id_str")]
    pub uid: Option<String>,
    pub login: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plus {
    #[serde(default)]
    pub has_plus: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountStatus {
    pub account: Account,
    pub plus: Option<Plus>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LikesLibrary {
    pub library: LikesTracks,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LikesTracks {
    #[serde(default)]
    pub tracks: Vec<TrackShort>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SearchBlock<T: serde::de::DeserializeOwned> {
    #[serde(default = "Vec::new", deserialize_with = "lenient_vec")]
    pub results: Vec<T>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SearchResult {
    #[serde(default)]
    pub tracks: Option<SearchBlock<Track>>,
    #[serde(default)]
    pub albums: Option<SearchBlock<Album>>,
    #[serde(default)]
    pub artists: Option<SearchBlock<Artist>>,
    #[serde(default)]
    pub playlists: Option<SearchBlock<Playlist>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistBrief {
    #[serde(default)]
    pub artist: Option<ArtistInfo>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub albums: Vec<Album>,
    /// Сборники и чужие релизы с участием исполнителя.
    #[serde(default, deserialize_with = "lenient_vec")]
    pub also_albums: Vec<Album>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub popular_tracks: Vec<Track>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub similar_artists: Vec<Artist>,
    #[serde(default)]
    pub stats: Option<ArtistStats>,
    /// Полная дискография (`direct-albums`), догружается отдельным запросом.
    #[serde(skip)]
    pub discography: Vec<Album>,
    /// «Знакомое вам»: треки исполнителя из коллекции и из волны (`familiar-you`).
    #[serde(skip)]
    pub familiar: Vec<Track>,
    #[serde(skip)]
    pub concerts: Vec<Concert>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Concert {
    pub concert_title: Option<String>,
    pub afisha_url: Option<String>,
    pub city: Option<String>,
    pub place: Option<String>,
    /// RFC 3339 с часовым поясом площадки.
    pub datetime: Option<String>,
    pub min_price: Option<Price>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub value: f64,
    pub currency_symbol: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistInfo {
    #[serde(default, deserialize_with = "id_or_empty")]
    pub id: String,
    pub cover: Option<Cover>,
    pub og_image: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub countries: Vec<String>,
    pub likes_count: Option<u64>,
    pub description: Option<ArtistDescription>,
    pub counts: Option<ArtistCounts>,
}

impl ArtistInfo {
    pub fn cover_uri(&self) -> Option<&str> {
        self.cover.as_ref().and_then(|c| c.uri.as_deref()).or(self.og_image.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtistDescription {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtistCounts {
    pub tracks: u32,
    pub direct_albums: u32,
    pub also_albums: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistStats {
    pub last_month_listeners: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtistAlbums {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub albums: Vec<Album>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub download_info: DownloadInfo,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    pub codec: String,
    pub transport: String,
    pub url: Option<String>,
    #[serde(default)]
    pub urls: Vec<String>,
    pub key: Option<String>,
    pub size: Option<u64>,
}

impl DownloadInfo {
    pub fn urls(&self) -> Vec<String> {
        let mut v = self.urls.clone();
        if let Some(u) = &self.url
            && !v.contains(u)
        {
            v.insert(0, u.clone());
        }
        v
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsDownload {
    pub download_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceItem {
    pub track: Option<Track>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotorSession {
    pub radio_session_id: String,
    pub batch_id: Option<String>,
    #[serde(default)]
    pub sequence: Vec<SequenceItem>,
    /// Подсказка офлайн-рекомендателю (TinyMl) — если сервер её дал.
    #[serde(default)]
    pub offline_recommender_data: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotorTracks {
    pub batch_id: Option<String>,
    #[serde(default)]
    pub sequence: Vec<SequenceItem>,
    #[serde(default)]
    pub offline_recommender_data: Option<Vec<i32>>,
}

/// «Колесо» Моей волны (`POST api.music.yandex.ru/wheel/new`) — персональные пресеты.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Wheel {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub items: Vec<WheelItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WheelItem {
    /// `DEFAULT` или `CONTROL_ACCENT` (служебные: «Назад к привычному», «Встряхнуть»).
    #[serde(default)]
    pub style: String,
    pub data: WheelData,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WheelData {
    pub wave: Option<WheelWave>,
    pub agent: Option<WheelAgent>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WheelWave {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub seeds: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WheelAgent {
    pub cover: Option<AgentCover>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentCover {
    pub uri: Option<String>,
    /// Фирменный цвет агента, `#RRGGBB`.
    pub color: Option<String>,
}

impl WheelItem {
    pub fn name(&self) -> &str {
        self.data.wave.as_ref().map_or("", |w| w.name.as_str())
    }

    pub fn seeds(&self) -> Vec<String> {
        self.data.wave.as_ref().map(|w| w.seeds.clone()).unwrap_or_default()
    }

    pub fn icon(&self) -> Option<&str> {
        self.data.agent.as_ref()?.cover.as_ref()?.uri.as_deref()
    }

    pub fn color(&self) -> Option<(u8, u8, u8)> {
        let c = self
            .data
            .agent
            .as_ref()?
            .cover
            .as_ref()?
            .color
            .as_deref()?
            .trim_start_matches('#');
        let v = u32::from_str_radix(c, 16).ok()?;
        Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    pub fn accent(&self) -> bool {
        self.style == "CONTROL_ACCENT"
    }
}

/// `library/all-ids`: id → 1 (лайк) или -1 (скрыт/дизлайк).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryIds {
    #[serde(default)]
    pub albums: std::collections::HashMap<String, i64>,
    #[serde(default)]
    pub artists: std::collections::HashMap<String, i64>,
}

impl LibraryIds {
    pub fn with(map: &std::collections::HashMap<String, i64>, v: i64) -> std::collections::HashSet<String> {
        map.iter().filter(|(_, x)| **x == v).map(|(k, _)| k.clone()).collect()
    }
}

/// «Настроили для вас»: «Плейлист дня», «Дежавю», «Премьера»…
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PersonalPlaylist {
    pub playlist: Playlist,
    pub description: Option<String>,
}

/// Ветка навигации «Настроения / Занятия / Жанры / Эпохи» (`landing3/metatags`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaTree {
    pub title: String,
    #[serde(default)]
    pub leaves: Vec<MetaLeaf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaLeaf {
    pub tag: String,
    pub title: String,
    #[serde(default)]
    pub leaves: Vec<MetaLeaf>,
}

/// Страница тега: волна по нему и подборки.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metatag {
    pub station_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub playlists: Vec<Playlist>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub albums: Vec<Album>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub artists: Vec<Artist>,
}

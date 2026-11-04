//! Состояние приложения и вся логика: авторизация, очередь, волна, реакции на события.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use tokio::sync::mpsc::UnboundedSender;

use crate::api::auth::{self, DeviceCode, Poll, StoredToken};
use crate::api::{self, AccountStatus, ApiError, ArtistBrief, Client, Playlist, Quality, SearchResult, Track};
use crate::audio::{self, Player, Token};
use crate::config::{Config, Paths, State};
use crate::fetch::{Fetcher, Origin};
use crate::library::Library;
use crate::lyrics::Lyrics;
use crate::mpris;
use crate::offline::{self, Snapshot};
use crate::offline_wave;
use crate::imgcache;
use crate::tinyml::EventKind;
use crate::ui::kitty::Images;
use crate::ui::theme::{Palette, ThemeAnim};
use crate::ynison;

mod yn;

pub enum Msg {
    /// Обложка для уведомления о новом треке готова.
    Notify {
        token: Token,
        title: String,
        body: String,
        icon: Option<Vec<u8>>,
    },
    Wheel(Result<Vec<api::WheelItem>, String>),
    Resumed {
        resume: crate::config::Resume,
        result: Result<Vec<Track>, String>,
    },
    Authed(Result<(Client, AccountStatus, StoredToken), String>),
    /// Сеть/сервер недоступны при подключении — покажем и повторим сами.
    ConnectError(String),
    DeviceCode(Result<DeviceCode, String>),
    LoggedIn(Result<StoredToken, String>),
    Likes(Result<Vec<Track>, String>),
    LibraryIds(Result<api::LibraryIds, String>),
    LikedAlbums { view_id: u64, result: Result<Vec<api::Album>, String> },
    ArtistList {
        view_id: u64,
        /// Это лайкнутые — пополнить множество лайков.
        liked: bool,
        result: Result<Vec<(api::Artist, String)>, String>,
    },
    Metatags { view_id: u64, result: Result<Vec<api::MetaTree>, String> },
    Metatag { view_id: u64, result: Result<api::Metatag, String> },
    /// Лайк/дизлайк альбома или исполнителя не сохранился — откатить.
    EntityFailed { what: Entity, id: String, on: bool, error: String },
    Playlists(Result<Vec<Playlist>, String>),
    Personal(Result<Vec<api::PersonalPlaylist>, String>),
    Ynison(ynison::Event),
    /// Треки очереди, пришедшей с другого устройства.
    YnisonQueue {
        generation: u64,
        queue: ynison::QueueState,
        status: ynison::Status,
        result: Result<Vec<Track>, String>,
    },
    Found(Result<Vec<api::Found>, String>),
    Credits { track_id: String, result: Result<Vec<(String, String)>, String> },
    /// Программа офлайн-волны и треки, под которые она собрана.
    OfflineProgram(Result<(Vec<i32>, Vec<i32>), String>),
    /// Метаданные скачанных треков для офлайн-снимка.
    LibraryMeta(Vec<Track>),
    /// Треки плейлистов для офлайн-снимка: (`uid:kind`, ревизия, треки).
    PlaylistSnapshots(Vec<(String, u32, Vec<Track>)>),
    /// Правка плейлиста выполнена: тост; `kind` — что перечитать при ошибке.
    PlaylistOp {
        result: Result<String, String>,
        kind: Option<String>,
        reload_list: bool,
    },
    Releases(Result<Vec<api::Album>, String>),
    Tracks {
        view_id: u64,
        result: Result<Vec<Track>, String>,
    },
    Artist {
        view_id: u64,
        result: Result<ArtistBrief, String>,
    },
    Search {
        query: String,
        result: Result<SearchResult, String>,
    },
    WaveStarted(Result<api::RotorSession, String>),
    /// Догрузка волны `session` (очередь могла уже смениться).
    WaveMore {
        session: String,
        result: Result<api::RotorTracks, String>,
    },
    Opened {
        token: Token,
        next: bool,
        /// С какой секунды начать (перезагрузка трека после SeekMiss).
        start: f64,
        result: Result<(audio::Source, Origin), String>,
    },
    Cover {
        key: String,
        result: Result<(image::RgbaImage, Palette), String>,
    },
    Lyrics {
        track_id: String,
        lyrics: Option<Lyrics>,
        /// Исходный текст (LRC или строки) — для MPRIS-расширения шелла.
        raw: Option<String>,
    },
    Audio(audio::Event),
    /// Источник получше для играющего трека (подмена на ходу); `None` — lossless нет.
    Upgraded {
        token: Token,
        /// Сами решили после старта трека (а не пользователь нажал Q) — неудачи без тостов.
        auto: bool,
        result: Result<Option<(audio::Source, Origin)>, String>,
    },
    Mpris(mpris::Command),
    /// Лайк не сохранился на сервере — вернуть как было.
    LikeFailed { track: Track, like: bool, error: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Wave,
    Likes,
    Playlists,
    Search,
    Queue,
    Releases,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Wave,
        Tab::Likes,
        Tab::Playlists,
        Tab::Search,
        Tab::Queue,
        Tab::Releases,
        Tab::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Wave => "Волна",
            Tab::Likes => "Коллекция",
            Tab::Playlists => "Плейлисты",
            Tab::Search => "Поиск",
            Tab::Queue => "Очередь",
            Tab::Releases => "Новинки",
            Tab::Settings => "Настройки",
        }
    }
}

/// Что лайкаем/скрываем кроме треков.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    AlbumLike,
    ArtistLike,
    ArtistDislike,
}

/// Строка вкладки «Настройки».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Quality,
    Normalize,
    TargetLufs,
    Crossfade,
    ReportPlays,
    Ynison,
    Fps,
    Dither,
    /// Действие: перепроверить качество скачанных треков.
    Recheck,
}

impl Setting {
    pub const ALL: [Setting; 9] = [
        Setting::Quality,
        Setting::Normalize,
        Setting::TargetLufs,
        Setting::Crossfade,
        Setting::ReportPlays,
        Setting::Ynison,
        Setting::Fps,
        Setting::Dither,
        Setting::Recheck,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Setting::Quality => "Качество",
            Setting::Normalize => "Выравнивание громкости",
            Setting::TargetLufs => "Целевая громкость",
            Setting::Crossfade => "Плавный переход",
            Setting::ReportPlays => "Отправлять прослушивания",
            Setting::Ynison => "Ynison (Слушать на…)",
            Setting::Fps => "Кадров в секунду",
            Setting::Dither => "Дизеринг блоба",
            Setting::Recheck => "Перепроверить качество скачанного",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Setting::Quality => "Для следующих треков. Скачанные похуже тихо заменятся при проигрывании, если сервер даст лучше. Клавиша Q.",
            Setting::Normalize => "Подгоняет громкость треков по R128 из метаданных Яндекса. Выключено — bit-perfect. Клавиша N.",
            Setting::TargetLufs => "К какой громкости выравнивать, LUFS. Работает, только когда выравнивание включено.",
            Setting::Crossfade => "Наплыв между треками; внутри одного альбома всегда встык. Клавиша x.",
            Setting::ReportPlays => "Передавать Яндексу историю прослушиваний (влияет на рекомендации и «недавно слушали»).",
            Setting::Ynison => "yamusicrs появляется в «Слушать на…» в приложениях Яндекс Музыки: оттуда можно переключить музыку сюда и управлять ею. Включённое здесь забирает воспроизведение у телефона.",
            Setting::Fps => "Частота кадров анимаций (блоб, прокрутка, перебор символов).",
            Setting::Dither => "Шум против «колец» на градиентах блоба, в ступенях 8 бит; 0 — выключен.",
            Setting::Recheck => "Сервер не дал lossless — трек помечается и не перепроверяется 30 дней. Enter снимает пометки: каждый трек хуже lossless спросит сервер заново при следующем проигрывании и, если дадут лучше, перекачается.",
        }
    }
}

const CROSSFADE_STEPS: [f32; 4] = [0.0, 3.0, 6.0, 10.0];
const FPS_STEPS: [u32; 5] = [15, 24, 30, 60, 120];

pub enum Loadable<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> From<Result<T, String>> for Loadable<T> {
    fn from(r: Result<T, String>) -> Self {
        match r {
            Ok(v) => Loadable::Ready(v),
            Err(e) => Loadable::Failed(e),
        }
    }
}

impl<T> Loadable<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Loadable::Ready(t) => Some(t),
            _ => None,
        }
    }

    pub fn ready_mut(&mut self) -> Option<&mut T> {
        match self {
            Loadable::Ready(t) => Some(t),
            _ => None,
        }
    }
}

/// Строка в результатах поиска.
#[derive(Clone)]
pub enum SearchItem {
    Header(&'static str),
    Track(Track),
    Album(api::Album),
    Artist(api::Artist),
    Playlist(Playlist),
    /// Запрос из истории поиска.
    Query(String),
}

pub enum Popup {
    /// Метаданные трека; файл в библиотеке ищется один раз при открытии.
    TrackInfo {
        track: Track,
        file: Option<(std::path::PathBuf, crate::library::Entry)>,
    },
    /// У трека несколько исполнителей — к кому перейти.
    Artists { list: Vec<api::Artist>, sel: usize },
    /// В какой из своих плейлистов добавить трек; последняя строка — новый плейлист.
    AddToPlaylist {
        track: Track,
        /// (kind, название) своих плейлистов.
        list: Vec<(String, String)>,
        sel: usize,
    },
    /// Ввод строки (название плейлиста).
    Input {
        title: &'static str,
        text: String,
        action: InputAction,
    },
    /// Подтверждение необратимого: y/Enter — да.
    Confirm { text: String, action: ConfirmAction },
}

pub enum InputAction {
    /// Новый плейлист; трек, который сразу в него добавить.
    Create(Option<Track>),
    Rename(String),
    /// Описание своего плейлиста (`kind`); пустое — убрать.
    Describe(String),
}

pub enum ConfirmAction {
    DeletePlaylist(String, String),
    RemoveTrack { kind: String, at: usize, track: Track },
}

/// Правки плейлистов исполняются по одной: каждой нужна свежая ревизия.
pub enum PlOp {
    Create { title: String, add: Option<String> },
    Rename { kind: String, title: String },
    Describe { kind: String, text: String },
    Visibility { kind: String, title: String, public: bool },
    Delete { kind: String, title: String },
    Append { kind: String, title: String, track: String },
    /// `at` — позиция в показанном списке (подсказка: настоящую ищем по id в заголовке).
    Remove { kind: String, at: usize, track: String },
    /// Поставить `track` на место `neighbor` (соседа сверху или снизу).
    Move { kind: String, at: usize, track: String, neighbor: String },
}

/// Позиция трека `id` в плейлисте, ближайшая к подсказке `hint` (трек может повторяться).
fn find_pos(pl: &Playlist, full_id: &str, hint: usize) -> Option<usize> {
    let id = full_id.split(':').next().unwrap_or(full_id);
    pl.tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.id == id)
        .min_by_key(|(i, _)| i.abs_diff(hint))
        .map(|(i, _)| i)
}

async fn run_pl_op(c: &Client, op: PlOp) -> (Result<String, String>, Option<String>, bool) {
    let (kind, reload) = match &op {
        PlOp::Create { .. } => (None, true),
        PlOp::Rename { kind, .. }
        | PlOp::Delete { kind, .. }
        | PlOp::Append { kind, .. }
        | PlOp::Describe { kind, .. }
        | PlOp::Visibility { kind, .. } => (Some(kind.clone()), true),
        PlOp::Remove { kind, .. } | PlOp::Move { kind, .. } => (Some(kind.clone()), false),
    };
    let r: api::Result<String> = async {
        match op {
            PlOp::Create { title, add } => {
                let pl = c.playlist_create(&title).await?;
                if let Some(t) = add {
                    c.playlist_insert(&pl.kind, pl.revision, 0, &t).await?;
                    return Ok(format!("Создан «{title}», трек добавлен"));
                }
                Ok(format!("Создан плейлист «{title}»"))
            }
            PlOp::Rename { kind, title } => {
                c.playlist_rename(&kind, &title).await?;
                Ok(format!("Плейлист переименован: «{title}»"))
            }
            PlOp::Describe { kind, text } => {
                c.playlist_describe(&kind, &text).await?;
                Ok(if text.is_empty() { "Описание убрано".into() } else { "Описание сохранено".into() })
            }
            PlOp::Visibility { kind, title, public } => {
                c.playlist_visibility(&kind, public).await?;
                Ok(if public {
                    format!("«{title}» теперь виден всем")
                } else {
                    format!("«{title}» теперь приватный")
                })
            }
            PlOp::Delete { kind, title } => {
                c.playlist_delete(&kind).await?;
                Ok(format!("Плейлист «{title}» удалён"))
            }
            PlOp::Append { kind, title, track } => {
                let h = c.playlist_header(&kind).await?;
                let at = h.track_count.map(|n| n as usize).unwrap_or(h.tracks.len());
                c.playlist_insert(&kind, h.revision, at, &track).await?;
                Ok(format!("Добавлено в «{title}»"))
            }
            PlOp::Remove { kind, at, track } => {
                let h = c.playlist_header(&kind).await?;
                let pos = find_pos(&h, &track, at).ok_or_else(|| api::ApiError::Api {
                    status: 0,
                    name: "not-found".into(),
                    message: "трека уже нет в плейлисте".into(),
                })?;
                let id = h.tracks[pos].full_id();
                c.playlist_remove(&kind, h.revision, pos, &id).await?;
                Ok("Убрано из плейлиста".into())
            }
            PlOp::Move {
                kind,
                at,
                track,
                neighbor,
            } => {
                let h = c.playlist_header(&kind).await?;
                let from = find_pos(&h, &track, at);
                let to = find_pos(&h, &neighbor, at);
                let (Some(from), Some(to)) = (from, to) else {
                    return Err(api::ApiError::Api {
                        status: 0,
                        name: "not-found".into(),
                        message: "плейлист изменился".into(),
                    });
                };
                let id = h.tracks[from].full_id();
                c.playlist_move(&kind, h.revision, from, to, &id).await?;
                Ok(String::new())
            }
        }
    }
    .await;
    (r.map_err(err_str), kind, reload)
}

/// Идентификатор уведомления «сейчас играет» (OSC 99): с тем же id kitty обновляет его на месте.
const NOTIFY_ID: &str = "yamusicrs-now";

/// Сторож вывода: первая повторная попытка через 30 с, дальше вдвое реже, но не реже раза в 5 мин.
const STALL_FIRST: Duration = Duration::from_secs(30);
const STALL_MAX: Duration = Duration::from_secs(300);

/// Строк прокрутки сетки на одно событие колеса/тачпада.
const GRID_WHEEL_LINES: f32 = 2.0;

/// Сколько запросов помнит история поиска.
const SEARCH_HISTORY_LEN: usize = 30;

pub enum View {
    Wave,
    Likes,
    Playlists,
    Search,
    Queue,
    /// Сетка «Новые релизы».
    Releases,
    Settings,
    Tracks {
        id: u64,
        title: String,
        subtitle: String,
        cover: Option<String>,
        tracks: Loadable<Vec<Track>>,
        /// `kind` своего плейлиста — тогда треки можно убирать и переставлять.
        playlist: Option<String>,
        /// id альбома, если это страница альбома (для лайка альбома).
        album: Option<String>,
    },
    Artist {
        id: u64,
        name: String,
        artist_id: String,
        brief: Loadable<ArtistBrief>,
    },
    /// Лайкнутые альбомы.
    Albums { id: u64, albums: Loadable<Vec<api::Album>> },
    /// Список исполнителей (в коллекции, топ месяца): исполнитель и пояснение справа.
    Artists {
        id: u64,
        title: String,
        artists: Loadable<Vec<(api::Artist, String)>>,
    },
    /// «Жанры и настроения»: деревья тегов.
    Metatags { id: u64, trees: Loadable<Vec<api::MetaTree>> },
    /// Страница одного тега.
    Metatag {
        id: u64,
        title: String,
        page: Loadable<api::Metatag>,
    },
}

/// Строка «Жанров и настроений»: заголовок ветки или тег (с глубиной вложенности).
pub enum TagRow<'a> {
    Header(&'a str),
    Leaf(&'a api::MetaLeaf, usize),
}

pub fn tag_rows(trees: &[api::MetaTree]) -> Vec<TagRow<'_>> {
    fn leaves<'a>(v: &mut Vec<TagRow<'a>>, ls: &'a [api::MetaLeaf], depth: usize) {
        for l in ls {
            v.push(TagRow::Leaf(l, depth));
            leaves(v, &l.leaves, depth + 1);
        }
    }
    let mut v = Vec::new();
    for t in trees {
        v.push(TagRow::Header(&t.title));
        leaves(&mut v, &t.leaves, 0);
    }
    v
}

/// Строка страницы тега.
pub enum MetaRow<'a> {
    Header(&'static str),
    Wave(&'a str),
    Playlist(&'a Playlist),
    Album(&'a api::Album),
    Artist(&'a api::Artist),
}

pub fn meta_rows(m: &api::Metatag) -> Vec<MetaRow<'_>> {
    let mut v = Vec::new();
    if let Some(s) = m.station_id.as_deref() {
        v.push(MetaRow::Wave(s));
    }
    if !m.playlists.is_empty() {
        v.push(MetaRow::Header("Плейлисты"));
        v.extend(m.playlists.iter().map(MetaRow::Playlist));
    }
    if !m.albums.is_empty() {
        v.push(MetaRow::Header("Альбомы"));
        v.extend(m.albums.iter().map(MetaRow::Album));
    }
    if !m.artists.is_empty() {
        v.push(MetaRow::Header("Исполнители"));
        v.extend(m.artists.iter().map(MetaRow::Artist));
    }
    v
}

/// Вид + курсор в нём.
pub struct Page {
    pub view: View,
    pub sel: usize,
    pub offset: usize,
    /// Фильтр списка треков (`f`): строка и индексы подходящих треков.
    pub filter: String,
    pub filter_editing: bool,
    pub filtered: Option<Vec<usize>>,
    /// Плавная позиция прокрутки и то, что сейчас реально на экране.
    pub anim: f32,
    pub view_offset: usize,
}

impl Page {
    fn new(view: View) -> Self {
        Self {
            view,
            sel: 0,
            offset: 0,
            filter: String::new(),
            filter_editing: false,
            filtered: None,
            anim: 0.0,
            view_offset: 0,
        }
    }
}

/// Индексы треков, где каждое слово запроса встречается в названии, артистах или альбоме.
pub fn filter_tracks(tracks: &[Track], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            let hay = format!(
                "{} {} {}",
                t.full_title(),
                t.artist_names(),
                t.album().map(|a| a.title.as_str()).unwrap_or("")
            )
            .to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

#[derive(Clone, PartialEq, Eq)]
pub enum QueueKind {
    List,
    Wave { session: String, batch: Option<String> },
    /// Моя волна без сети: следующий трек выбирает TinyMl из скачанного.
    OfflineWave,
}

pub struct Queue {
    pub items: Vec<Track>,
    pub pos: Option<usize>,
    pub kind: QueueKind,
    pub title: String,
    /// Поколение очереди: токены старой очереди не двигают курсор новой.
    pub generation: u64,
}

pub struct Now {
    pub token: Token,
    pub track: Track,
    pub origin: Option<Origin>,
    pub info: audio::SourceInfo,
    pub pos: f64,
}

pub struct Toast {
    pub text: String,
    pub at: Instant,
    pub error: bool,
}

pub struct LoginState {
    pub code: Option<DeviceCode>,
    pub status: String,
    pub input: String,
    pub error: Option<String>,
}

pub enum Screen {
    /// Подключение; `Some` — последняя ошибка сети, идут повторы.
    Connecting(Option<String>),
    Login(LoginState),
    Main,
}

pub struct WaveUi {
    pub loading: bool,
    /// Идёт догрузка следующей пачки играющей волны.
    pub extending: bool,
    /// Персональные пресеты («колесо») с `wheel/new`.
    pub items: Loadable<Vec<api::WheelItem>>,
    /// Плавная позиция дуги (догоняет курсор).
    pub arc: f32,
    /// Сиды и название играющей волны.
    pub active_seeds: Option<Vec<String>>,
    pub active_name: Option<String>,
    /// Пункты дуги на экране — для мыши.
    pub hits: Vec<(ratatui::layout::Rect, usize)>,
}

/// Сиды «просто Моей волны».
const MY_WAVE: &str = "user:onyourwave";

const HISTORY_QUERIES: &str = "Недавние запросы";
const HISTORY_FOUND: &str = "Недавно находили (все устройства)";

/// Строка вкладки «Плейлисты».
pub enum PlaylistRow<'a> {
    /// «Настроили для вас» (Плейлист дня, Дежавю, Премьера).
    Personal(&'a api::PersonalPlaylist),
    Own(&'a Playlist),
    LikedAlbums,
    LikedArtists,
    TopArtists,
}

pub struct App {
    pub tx: UnboundedSender<Msg>,
    pub cfg: Config,
    paths: Paths,
    pub state: State,
    pub screen: Screen,
    client: Option<Client>,
    fetcher: Option<Fetcher>,
    /// Нет сети: играем только из локальной библиотеки по снимку.
    pub offline: bool,
    snapshot: Snapshot,
    snapshot_path: std::path::PathBuf,
    /// Офлайн-волна: программа, идущая сессия и треки, из которых она выбирает.
    owave_store: Option<offline_wave::Store>,
    owave: Option<offline_wave::Session>,
    owave_tracks: HashMap<i32, Track>,
    owave_fetching: bool,
    data_dir: std::path::PathBuf,
    pub library: Arc<Library>,
    pub player: Player,
    pub account: Option<AccountStatus>,
    pub tab: Tab,
    pub stack: Vec<Page>,
    pub liked: HashSet<String>,
    pub personal: Vec<api::PersonalPlaylist>,
    /// Серверная история поиска (найденное на любом устройстве).
    pub found: Vec<api::Found>,
    /// «О треке»: авторы и лейбл по id трека (грузятся при открытии окна).
    pub credits: HashMap<String, Vec<(String, String)>>,
    /// Лайкнутые альбомы и исполнители, скрытые исполнители (id из `library/all-ids`).
    pub liked_albums: HashSet<String>,
    pub liked_artists: HashSet<String>,
    pub disliked_artists: HashSet<String>,
    pub likes: Loadable<Vec<Track>>,
    pub playlists: Loadable<Vec<Playlist>>,
    /// Новые релизы; None — ещё не запрашивали (грузим при первом открытии).
    pub releases: Option<Loadable<Vec<api::Album>>>,
    /// Колонок в сетке релизов (знает отрисовка) и карточки на экране — для мыши.
    pub grid_cols: usize,
    /// Высота карточки сетки в строках и накопленная прокрутка колесом (в рядах).
    pub grid_card_h: u16,
    grid_wheel: f32,
    pub grid_hits: Vec<(ratatui::layout::Rect, usize)>,
    pub search_input: String,
    pub search_editing: bool,
    pub search: Option<Loadable<Vec<SearchItem>>>,
    pub queue: Queue,
    pub wave: WaveUi,
    /// Фаза анимации блоба (быстрее, когда играет).
    pub wave_phase: f32,
    /// Текущие (плавно догоняющие цель) цвета блоба.
    pub blob_colors: [[f32; 3]; 4],
    /// Плавные энергия и скорость блоба.
    pub blob_energy: f32,
    pub blob_speed: f32,
    pub now: Option<Now>,
    /// Токен последней явной загрузки (чтобы игнорировать устаревшие ответы).
    requested: Option<Token>,
    pub loading_track: Option<Track>,
    /// Токен → (поколение очереди, индекс, трек).
    tokens: HashMap<Token, (u64, usize, Track)>,
    next_token: Token,
    next_loaded: Option<Token>,
    /// Токен трека, который надо загрузить и оставить на паузе (восстановление сессии).
    start_paused: Option<Token>,
    /// Откуда открыт трек (библиотека/сеть) — до момента, когда он реально зазвучит.
    pending_origins: HashMap<Token, Origin>,
    /// Какая очередь записана в queue.json: (поколение, длина).
    queue_saved: Option<(u64, usize)>,
    /// Очередь восстановлена из кеша, а трек ещё не открыт (ждёт сети): (индекс, секунда).
    resume_at: Option<(usize, f64)>,
    /// Пока играет только сохранённый трек, а очередь грузится: (поколение заглушки, его место в полной очереди).
    resume_partial: Option<(u64, usize)>,
    /// Не открывшиеся треки очереди (поколение, индекс) — автопереход их пропускает.
    failed: HashSet<(u64, usize)>,
    /// Откуда новый источник, пока идёт подмена на ходу.
    swap_origin: Option<(Token, Origin, bool)>,
    /// Подмена склеена, но ещё не зазвучала (декодер впереди): сообщение покажем, когда дойдёт.
    swap_toast: Option<(Token, String)>,
    pub images: Images,
    pub gfx: crate::render::Gfx,
    /// Кнопки ⏮ ▶ ⏭ (в боковой панели): прямоугольник и действие 0/1/2.
    pub controls: Vec<(ratatui::layout::Rect, u8)>,
    pub theme: ThemeAnim,
    /// Моменты запуска эффекта «перебор символов»: старт, смена вида, смена трека.
    pub fx_start: Option<Instant>,
    pub fx_view: Option<Instant>,
    pub fx_track: Option<Instant>,
    /// Ручная прокрутка текста без таймингов (строки).
    pub plain_scroll: u16,
    /// Предел прокрутки текста без таймингов (строк после переноса), считает отрисовка.
    pub plain_max: std::cell::Cell<u16>,
    covers_loading: HashSet<String>,
    /// Не нашлись ни в кеше, ни в файлах, а сети нет — не дёргаем каждый кадр.
    covers_failed: HashSet<String>,
    /// Обложка → скачанный трек с ней (картинка вшита в файл); строится по запросу.
    cover_files: Option<HashMap<String, String>>,
    /// Палитры уже загруженных обложек: тема должна меняться и на треках из кеша.
    palettes: HashMap<String, Palette>,
    pub lyrics: Option<(String, Lyrics)>,
    pub lyrics_scroll: f32,
    /// Ручной сдвиг текста колесом (в репликах) и когда его трогали.
    lyrics_user_offset: f32,
    lyrics_user_at: Option<Instant>,
    /// Текст ещё едет к цели — нужны кадры даже на паузе.
    lyrics_moving: bool,
    /// Строки текста на экране: (y, номер реплики) — для клика.
    pub lyrics_hits: Vec<(u16, usize)>,
    pub lyrics_mode: bool,
    /// «Сцена»: полноэкранный «сейчас играет».
    pub stage_mode: bool,
    pub toasts: Vec<Toast>,
    pub show_help: bool,
    pub repeat_one: bool,
    pub shuffle: bool,
    mpris: UnboundedSender<mpris::Update>,
    /// Очередь правок плейлистов (исполняются по одной).
    pl_ops: Option<UnboundedSender<PlOp>>,
    pub quit: bool,
    /// Пользователь сам переключил трек — смена токена не «естественный» конец.
    user_switch: bool,
    error_streak: u32,
    view_seq: u64,
    pub started: Instant,
    /// Прямоугольники последнего кадра — для эффектов.
    pub areas: Areas,
    last_mpris_pos: Instant,
    /// Ynison: соединение, последнее известное состояние и активны ли мы.
    ynison: Option<ynison::Handle>,
    pub yn: Option<ynison::Remote>,
    pub yn_active: bool,
    pub yn_connected: bool,
    /// Трек запущен по команде другого устройства — не перехватывать активность.
    yn_remote_start: bool,
    yn_last_sent: Instant,
    /// Поколение загрузки очереди с другого устройства (устаревшие ответы отбрасываются).
    yn_fetch_gen: u64,
    last_session_save: Instant,
    last_stall: Option<Instant>,
    /// Пауза до следующей попытки оживить вывод: растёт, пока он не отвечает.
    stall_backoff: Duration,
    /// Последний клик по строке списка — для двойного клика.
    last_click: Option<(Instant, usize)>,
    /// Строка списка под мышью.
    pub hover: Option<usize>,
    /// Последняя позиция мыши (колонка, строка) — к ней привязан попап обложки.
    pub mouse: (u16, u16),
    /// Окно терминала в фокусе: без фокуса рисуем раз в секунду.
    pub focused: bool,
    /// Оверлеи кадра (крупный текст, ссылки) — пересобираются при каждой отрисовке.
    pub overlays: Vec<crate::ui::term::Overlay>,
    /// Всплывашки поверх кадра: OSC-оверлеи под ними не рисуются.
    pub occluders: Vec<ratatui::layout::Rect>,
    /// Всплывающее окно поверх всего (метаданные трека, выбор исполнителя).
    pub popup: Option<Popup>,
    /// Кликабельные строки всплывающего окна и его рамка — для мыши.
    pub popup_hits: Vec<(ratatui::layout::Rect, usize)>,
    pub popup_area: ratatui::layout::Rect,
    /// Разовые OSC-команды терминалу (курсор, буфер обмена, уведомления).
    pub osc_out: Vec<u8>,
    pointer: bool,
    /// Протоколы kitty доступны.
    pub kitty_text: bool,
    sel_moved_at: Instant,
    stored_token: Option<StoredToken>,
}

#[derive(Default, Clone, Copy)]
pub struct Areas {
    /// Подписи под обложкой на сцене: название, исполнители, альбом.
    pub stage_title: ratatui::layout::Rect,
    pub stage_artist: ratatui::layout::Rect,
    pub stage_album: ratatui::layout::Rect,
    pub info: ratatui::layout::Rect,
    /// Обложка «сейчас играет» (боковая панель или сцена).
    pub cover: ratatui::layout::Rect,
    pub lyrics: ratatui::layout::Rect,
    /// Видимая область текущего списка (для мыши).
    pub list: ratatui::layout::Rect,
    pub progress: ratatui::layout::Rect,
    pub player: ratatui::layout::Rect,
    /// Вкладки в шапке: (x начала, x конца, вкладка).
    pub tabs: [(u16, u16); Tab::ALL.len()],
    pub main: ratatui::layout::Rect,
    pub header: ratatui::layout::Rect,
}

/// Через сколько ручная прокрутка текста возвращается к текущей строке.
const LYRICS_RETURN: Duration = Duration::from_secs(4);

/// Длительность эффекта перебора символов.
pub const SCRAMBLE: Duration = Duration::from_millis(650);

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl App {
    pub fn new(tx: UnboundedSender<Msg>, cfg: Config, paths: Paths, offline: bool) -> color_eyre::Result<Self> {
        let state = State::load(&paths);
        let cache = paths.cache_dir.clone();
        tokio::task::spawn_blocking(move || imgcache::prune(&cache));
        let data_dir = paths.index_file.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
        let snapshot_path = Snapshot::path(&data_dir);
        let volume = state.volume.unwrap_or(cfg.volume);
        let audio_tx = tx.clone();
        let (atx, mut arx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(ev) = arx.recv().await {
                if audio_tx.send(Msg::Audio(ev)).is_err() {
                    break;
                }
            }
        });
        let player = Player::new(atx, volume)?;
        player.set_crossfade(state.crossfade.unwrap_or(cfg.crossfade_secs));
        let library = Library::open(cfg.library_dir.clone(), paths.index_file.clone(), cfg.library_max_gb);

        let (mtx, mut mrx) = tokio::sync::mpsc::unbounded_channel();
        let mpris = mpris::spawn(mtx);
        let mpris_tx = tx.clone();
        tokio::spawn(async move {
            while let Some(c) = mrx.recv().await {
                if mpris_tx.send(Msg::Mpris(c)).is_err() {
                    break;
                }
            }
        });
        let _ = mpris.send(mpris::Update::Volume(volume as f64));

        let images = Images::new(cfg.kitty_enabled());
        let gfx = crate::render::Gfx::new(cfg.gpu && cfg.kitty_enabled());
        let kitty_text = cfg.kitty_text_enabled();
        let mut app = Self {
            tx,
            paths,
            state,
            screen: Screen::Connecting(None),
            client: None,
            fetcher: None,
            offline: false,
            snapshot: Snapshot::load(&snapshot_path),
            snapshot_path,
            owave_store: offline_wave::Store::load(&data_dir),
            owave: None,
            owave_tracks: HashMap::new(),
            owave_fetching: false,
            data_dir,
            library,
            player,
            account: None,
            tab: Tab::Wave,
            stack: vec![Page::new(View::Wave)],
            liked: HashSet::new(),
            personal: Vec::new(),
            found: Vec::new(),
            credits: HashMap::new(),
            liked_albums: HashSet::new(),
            liked_artists: HashSet::new(),
            disliked_artists: HashSet::new(),
            likes: Loadable::Loading,
            playlists: Loadable::Loading,
            releases: None,
            grid_cols: 1,
            grid_card_h: 1,
            grid_wheel: 0.0,
            grid_hits: Vec::new(),
            search_input: String::new(),
            search_editing: false,
            search: None,
            queue: Queue {
                items: Vec::new(),
                pos: None,
                kind: QueueKind::List,
                title: String::new(),
                generation: 0,
            },
            wave: WaveUi {
                loading: false,
                extending: false,
                items: Loadable::Loading,
                arc: 0.0,
                active_seeds: None,
                active_name: None,
                hits: Vec::new(),
            },
            wave_phase: 0.0,
            blob_energy: 0.0,
            blob_speed: 0.3,
            blob_colors: [
                [255.0, 204.0, 0.0],
                [255.0, 107.0, 61.0],
                [240.0, 42.0, 79.0],
                [120.0, 90.0, 220.0],
            ],
            now: None,
            requested: None,
            loading_track: None,
            tokens: HashMap::new(),
            next_token: 1,
            next_loaded: None,
            start_paused: None,
            pending_origins: HashMap::new(),
            swap_origin: None,
            swap_toast: None,
            failed: HashSet::new(),
            queue_saved: None,
            resume_at: None,
            resume_partial: None,
            images,
            gfx,
            controls: Vec::new(),
            theme: ThemeAnim::new(),
            fx_start: None,
            fx_view: None,
            fx_track: None,
            plain_scroll: 0,
            plain_max: std::cell::Cell::new(u16::MAX),
            covers_loading: HashSet::new(),
            covers_failed: HashSet::new(),
            cover_files: None,
            palettes: HashMap::new(),
            lyrics: None,
            lyrics_scroll: 0.0,
            lyrics_user_offset: 0.0,
            lyrics_user_at: None,
            lyrics_moving: false,
            lyrics_hits: Vec::new(),
            lyrics_mode: false,
            stage_mode: false,
            toasts: Vec::new(),
            show_help: false,
            repeat_one: false,
            shuffle: false,
            mpris,
            pl_ops: None,
            quit: false,
            user_switch: false,
            error_streak: 0,
            view_seq: 0,
            started: Instant::now(),
            areas: Areas::default(),
            last_mpris_pos: Instant::now(),
            ynison: None,
            yn: None,
            yn_active: false,
            yn_connected: false,
            yn_remote_start: false,
            yn_last_sent: Instant::now(),
            yn_fetch_gen: 0,
            last_session_save: Instant::now(),
            last_stall: None,
            stall_backoff: STALL_FIRST,
            last_click: None,
            hover: None,
            mouse: (0, 0),
            focused: true,
            overlays: Vec::new(),
            occluders: Vec::new(),
            popup: None,
            popup_hits: Vec::new(),
            popup_area: ratatui::layout::Rect::default(),
            osc_out: Vec::new(),
            pointer: false,
            kitty_text,
            sel_moved_at: Instant::now(),
            stored_token: None,
            cfg,
        };
        app.resume_local();
        if offline {
            app.enter_offline();
        } else {
            app.start_auth();
        }
        Ok(app)
    }

    // ── авторизация ──────────────────────────────────────────────────────────

    fn start_auth(&mut self) {
        let Some(tok) = StoredToken::load(&self.paths.token_file) else {
            self.begin_login(None);
            return;
        };
        self.screen = Screen::Connecting(None);
        tokio::spawn(connect(tok, self.tx.clone()));
    }

    fn begin_login(&mut self, error: Option<String>) {
        self.screen = Screen::Login(LoginState {
            code: None,
            status: "Запрашиваю код…".into(),
            input: String::new(),
            error,
        });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = auth::request_device_code(&api::http_client()).await.map_err(err_str);
            let _ = tx.send(Msg::DeviceCode(r));
        });
    }

    fn poll_device(&self, code: DeviceCode) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let http = api::http_client();
            let deadline = Instant::now() + Duration::from_secs(code.expires_in);
            while Instant::now() < deadline {
                tokio::time::sleep(Duration::from_secs(code.interval.max(1))).await;
                match auth::poll_device_token(&http, &code).await {
                    Ok(Poll::Pending) => continue,
                    Ok(Poll::Done(t)) => {
                        let _ = tx.send(Msg::LoggedIn(Ok(t)));
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::LoggedIn(Err(e.to_string())));
                        return;
                    }
                }
            }
            let _ = tx.send(Msg::LoggedIn(Err("код истёк, нажми r для нового".into())));
        });
    }

    fn submit_pasted_token(&mut self) {
        let Screen::Login(l) = &mut self.screen else {
            return;
        };
        match auth::parse_pasted_token(&l.input) {
            Some(tok) => {
                l.status = "Проверяю токен…".into();
                l.error = None;
                let _ = self.tx.send(Msg::LoggedIn(Ok(StoredToken {
                    access_token: tok,
                    refresh_token: None,
                    expires_at: None,
                })));
            }
            None => l.error = Some("Это не похоже на токен".into()),
        }
    }

    fn on_authed(&mut self, client: Client, status: AccountStatus, tok: StoredToken) {
        if let Err(e) = tok.save(&self.paths.token_file) {
            self.toast_err(format!("не удалось сохранить токен: {e}"));
        }
        self.stored_token = Some(tok);
        if self.offline {
            self.offline = false;
            self.covers_failed.clear();
            self.toast("Сеть вернулась — снова онлайн".into());
        }
        let fetcher = Fetcher::new(client.clone(), self.library.clone(), self.cfg.quality);
        if let Some(q) = self.state.quality {
            fetcher.set_quality(q);
        }
        self.fetcher = Some(fetcher);
        let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel::<PlOp>();
        let (pc, mtx) = (client.clone(), self.tx.clone());
        tokio::spawn(async move {
            while let Some(op) = prx.recv().await {
                let (result, kind, reload_list) = run_pl_op(&pc, op).await;
                if mtx
                    .send(Msg::PlaylistOp {
                        result,
                        kind,
                        reload_list,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.pl_ops = Some(ptx);
        self.client = Some(client);
        if status.plus.as_ref().is_some_and(|p| !p.has_plus) {
            self.toast_err("Без Плюса будут только 30-секундные превью".into());
        }
        self.account = Some(status);
        self.screen = Screen::Main;
        self.fx_start = Some(Instant::now());
        self.load_likes();
        self.load_playlists();
        self.load_wheel();
        self.restore_session();
        self.ynison_start();
    }

    /// Восстанавливает очередь прошлого запуска и ставит трек на паузу с той же секунды.
    /// Очередь из queue.json — сразу при запуске, до входа. Скачанный трек загружается
    /// на паузу тут же, остальные откроются после входа без запроса метаданных.
    fn resume_local(&mut self) {
        let Some(r) = self.state.resume.clone().filter(|r| !r.ids.is_empty()) else {
            return;
        };
        let kind = match r.wave.clone() {
            Some((session, batch)) => QueueKind::Wave { session, batch },
            None => QueueKind::List,
        };
        let tracks = crate::offline::load_queue(&self.data_dir);
        let pos = if tracks.len() == r.ids.len() && tracks.iter().zip(&r.ids).all(|(t, id)| t.full_id() == *id) {
            let pos = r.pos.min(tracks.len() - 1);
            self.replace_queue(tracks, kind, r.title);
            self.queue_saved = Some((self.queue.generation, self.queue.items.len()));
            pos
        } else if let Some(t) = r.track.filter(|t| r.ids.get(r.pos) == Some(&t.full_id())) {
            // Кеш очереди не подошёл: сперва один играющий трек, полная очередь — после входа.
            self.replace_queue(vec![t], kind, r.title);
            self.resume_partial = Some((self.queue.generation, r.pos));
            0
        } else {
            return;
        };
        self.queue.pos = Some(pos);
        if self.library.contains(&self.queue.items[pos].id) {
            self.play_index_with(pos, r.secs, true, Opener::Offline(self.library.clone()));
        } else {
            self.loading_track = Some(self.queue.items[pos].clone());
            self.resume_at = Some((pos, r.secs));
        }
    }

    fn restore_session(&mut self) {
        // Очередь уже из кеша: открыть трек (если ещё ждал сети) и подготовить следующий.
        if !self.queue.items.is_empty() && self.resume_partial.is_none() {
            match self.resume_at.take() {
                Some((pos, secs)) => {
                    self.play_index_at(pos, secs, true);
                    self.toast("Продолжаем с того же места — Space".into());
                }
                None => self.schedule_next(),
            }
            return;
        }
        // Играет заглушка из одного трека: открыть его, если ждал сети, и догрузить очередь.
        if self.resume_partial.is_some()
            && let Some((pos, secs)) = self.resume_at.take()
        {
            self.play_index_at(pos, secs, true);
        }
        let Some(r) = self.state.resume.clone().filter(|r| !r.ids.is_empty()) else {
            return;
        };
        self.spawn(move |c, tx| async move {
            let result = c.tracks(&r.ids).await.map_err(err_str);
            let _ = tx.send(Msg::Resumed { resume: r, result });
        });
    }

    /// Полная очередь пришла с сервера — встаёт на место заглушки из одного трека, не трогая звук.
    fn fill_partial_queue(
        &mut self,
        generation: u64,
        full_pos: usize,
        resume: crate::config::Resume,
        result: Result<Vec<Track>, String>,
    ) {
        // Пользователь уже включил другое — заглушка не актуальна.
        if self.queue.generation != generation {
            return;
        }
        let tracks = match result {
            Ok(t) => t,
            Err(e) => return tracing::warn!("очередь не догрузилась: {e}"),
        };
        let cur_id = self.queue.items.first().map(|t| t.id.clone());
        // Недоступные сервер может не вернуть — позицию ищем по id рядом с сохранённой.
        let Some(pos) = tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| Some(&t.id) == cur_id.as_ref())
            .min_by_key(|(i, _)| i.abs_diff(full_pos))
            .map(|(i, _)| i)
        else {
            return;
        };
        let kind = std::mem::replace(&mut self.queue.kind, QueueKind::List);
        self.replace_queue(tracks, kind, resume.title);
        self.queue.pos = Some(pos);
        // Токены заглушки (играющий трек) — на новое место в новой очереди.
        let g = self.queue.generation;
        for (old_gen, idx, _) in self.tokens.values_mut() {
            if *old_gen == generation && *idx == 0 {
                *old_gen = g;
                *idx = pos;
            }
        }
        self.schedule_next();
        self.persist_session();
    }

    /// Сессия на диск: при смене трека и периодически — переживёт и kill терминала.
    fn persist_session(&mut self) {
        self.save_session();
        let mark = (self.queue.generation, self.queue.items.len());
        if !self.queue.items.is_empty() && self.queue_saved != Some(mark) && !self.partial_active() {
            self.queue_saved = Some(mark);
            crate::offline::save_queue(&self.data_dir, self.queue.items.clone());
        }
        self.state.save(&self.paths);
        self.last_session_save = Instant::now();
    }

    /// Играет заглушка из одного трека, а полная очередь ещё не пришла.
    fn partial_active(&self) -> bool {
        self.resume_partial.is_some_and(|(g, _)| g == self.queue.generation)
    }

    fn save_session(&mut self) {
        // Заглушка: сохранённую полную очередь не трогаем, обновляем только место в треке.
        if self.partial_active() {
            if let Some(r) = self.state.resume.as_mut() {
                r.secs = self.now.as_ref().map_or(r.secs, |n| n.pos);
            }
            return;
        }
        self.state.resume = (!self.queue.items.is_empty()).then(|| crate::config::Resume {
            title: self.queue.title.clone(),
            ids: self.queue.items.iter().map(Track::full_id).collect(),
            pos: self.queue.pos.unwrap_or(0),
            secs: self.now.as_ref().map_or(0.0, |n| n.pos),
            wave: match &self.queue.kind {
                QueueKind::Wave { session, batch } => Some((session.clone(), batch.clone())),
                QueueKind::List | QueueKind::OfflineWave => None,
            },
            track: self
                .now
                .as_ref()
                .map(|n| n.track.clone())
                .or_else(|| self.queue.pos.and_then(|p| self.queue.items.get(p).cloned())),
        });

    }

    // ── загрузки данных ──────────────────────────────────────────────────────

    fn client(&self) -> Option<Client> {
        self.client.clone()
    }

    fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Client, UnboundedSender<Msg>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if let Some(c) = self.client() {
            tokio::spawn(f(c, self.tx.clone()));
        }
    }

    fn load_likes(&mut self) {
        self.likes = Loadable::Loading;
        self.spawn(|c, tx| async move {
            let r = async {
                let ids = c.liked_track_ids().await?;
                c.tracks(&ids).await
            }
            .await
            .map_err(err_str);
            let _ = tx.send(Msg::Likes(r));
        });
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::LibraryIds(c.library_ids().await.map_err(err_str)));
        });
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::Found(c.search_history().await.map_err(err_str)));
        });
    }

    fn load_playlists(&mut self) {
        self.playlists = Loadable::Loading;
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::Playlists(c.playlists().await.map_err(err_str)));
        });
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::Personal(c.personal_playlists().await.map_err(err_str)));
        });
    }

    /// Строки вкладки «Плейлисты» (пусто, пока свои плейлисты не загрузились).
    pub fn playlist_rows(&self) -> Vec<PlaylistRow<'_>> {
        let Some(own) = self.playlists.ready() else {
            return Vec::new();
        };
        let mut v: Vec<PlaylistRow> = self.personal.iter().map(PlaylistRow::Personal).collect();
        v.extend(own.iter().map(PlaylistRow::Own));
        if !self.offline {
            v.push(PlaylistRow::LikedAlbums);
            v.push(PlaylistRow::LikedArtists);
            v.push(PlaylistRow::TopArtists);
        }
        v
    }

    fn load_releases(&mut self) {
        self.releases = Some(Loadable::Loading);
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::Releases(c.new_releases().await.map_err(err_str)));
        });
    }

    fn new_view_id(&mut self) -> u64 {
        self.view_seq += 1;
        self.view_seq
    }

    fn open_playlist(&mut self, pl: Playlist) {
        let id = self.new_view_id();
        let own = self.is_own(&pl);
        let subtitle = format!(
            "{} · {} треков",
            pl.owner
                .as_ref()
                .and_then(|o| o.name.clone().or(o.login.clone()))
                .unwrap_or_default(),
            pl.track_count.unwrap_or(0)
        );
        self.push(View::Tracks {
            id,
            title: pl.title.clone(),
            subtitle,
            cover: pl.cover_uri().map(str::to_owned),
            tracks: Loadable::Loading,
            playlist: own.then(|| pl.kind.clone()),
            album: None,
        });
        if let Some(c) = pl.cover_uri() {
            let c = c.to_string();
            self.fetch_cover(&c);
        }
        if self.offline {
            let r = self
                .snapshot
                .tracks_of(&pl)
                .map(|ts| self.offline_tracks(ts.clone()))
                .ok_or_else(|| "этот плейлист не успел сохраниться для офлайна".to_string());
            let _ = self.tx.send(Msg::Tracks { view_id: id, result: r });
            return;
        }
        self.spawn(move |c, tx| async move {
            let r = async {
                let full = c.playlist(&pl.uid, &pl.kind).await?;
                c.playlist_tracks(&full).await
            }
            .await
            .map_err(err_str);
            let _ = tx.send(Msg::Tracks { view_id: id, result: r });
        });
    }

    fn open_history(&mut self) {
        let id = self.new_view_id();
        self.push(View::Tracks {
            id,
            title: "История".into(),
            subtitle: "недавно слушали, свежие сверху".into(),
            cover: None,
            tracks: Loadable::Loading,
            playlist: None,
            album: None,
        });
        self.spawn(move |c, tx| async move {
            let r = c.history().await.map_err(err_str);
            let _ = tx.send(Msg::Tracks { view_id: id, result: r });
        });
    }

    fn open_similar(&mut self, track: Track) {
        let id = self.new_view_id();
        self.push(View::Tracks {
            id,
            title: format!("Похожие на «{}»", track.title),
            subtitle: track.artist_names(),
            cover: track.cover_uri().map(str::to_owned),
            tracks: Loadable::Loading,
            playlist: None,
            album: None,
        });
        if let Some(c) = track.cover_uri() {
            let c = c.to_string();
            self.fetch_cover(&c);
        }
        self.spawn(move |c, tx| async move {
            let r = c.similar_tracks(&track.id).await.map_err(err_str).and_then(|ts| {
                if ts.is_empty() {
                    Err("похожих треков нет".to_string())
                } else {
                    Ok(ts)
                }
            });
            let _ = tx.send(Msg::Tracks { view_id: id, result: r });
        });
    }

    fn open_chart(&mut self) {
        let id = self.new_view_id();
        self.push(View::Tracks {
            id,
            title: "Чарт".into(),
            subtitle: "популярное на Яндекс Музыке прямо сейчас".into(),
            cover: None,
            tracks: Loadable::Loading,
            playlist: None,
            album: None,
        });
        self.spawn(move |c, tx| async move {
            let r = async {
                let pl = c.chart().await?;
                c.playlist_tracks(&pl).await
            }
            .await
            .map_err(err_str);
            let _ = tx.send(Msg::Tracks { view_id: id, result: r });
        });
    }

    fn open_album(&mut self, album: api::Album) {
        let id = self.new_view_id();
        let subtitle = match album.year {
            Some(y) => format!("{} · {y}", album.artist_names()),
            None => album.artist_names(),
        };
        self.push(View::Tracks {
            id,
            title: album.full_title(),
            subtitle,
            cover: album.cover_uri().map(str::to_owned),
            tracks: Loadable::Loading,
            playlist: None,
            album: (!album.id.is_empty()).then(|| album.id.clone()),
        });
        if let Some(c) = album.cover_uri() {
            let c = c.to_string();
            self.fetch_cover(&c);
        }
        let aid = album.id.clone();
        self.spawn(move |c, tx| async move {
            let r = c
                .album(&aid)
                .await
                .map(|a| {
                    a.volumes
                        .into_iter()
                        .flatten()
                        .filter_map(|v| serde_json::from_value::<Track>(v).ok())
                        .collect()
                })
                .map_err(err_str);
            let _ = tx.send(Msg::Tracks { view_id: id, result: r });
        });
    }

    fn open_artist(&mut self, artist: api::Artist) {
        let id = self.new_view_id();
        self.push(View::Artist {
            id,
            name: artist.name.clone(),
            artist_id: artist.id.clone(),
            brief: Loadable::Loading,
        });
        self.spawn(move |c, tx| async move {
            let r = c.artist(&artist.id).await.map_err(err_str);
            let _ = tx.send(Msg::Artist { view_id: id, result: r });
        });
    }

    /// Список «Недавние запросы» (свои) и «Недавно находили» (с сервера, со всех устройств)
    /// вместо результатов; пусто, если нет ни того, ни другого.
    fn show_search_history(&mut self) {
        let h = &self.state.search_history;
        let mut v = Vec::new();
        if !h.is_empty() {
            v.push(SearchItem::Header(HISTORY_QUERIES));
            v.extend(h.iter().cloned().map(SearchItem::Query));
        }
        if !self.found.is_empty() {
            v.push(SearchItem::Header(HISTORY_FOUND));
            v.extend(self.found.iter().cloned().map(|f| match f {
                api::Found::Track(t) => SearchItem::Track(t),
                api::Found::Album(a) => SearchItem::Album(a),
                api::Found::Artist(a) => SearchItem::Artist(a),
                api::Found::Playlist(p) => SearchItem::Playlist(p),
            }));
        }
        self.search = (!v.is_empty()).then_some(Loadable::Ready(v));
        if matches!(self.page().view, View::Search) {
            self.page_mut().sel = 0;
            self.skip_headers(1);
        }
    }

    /// Сейчас в поиске показана история, а не результаты.
    pub fn search_shows_history(&self) -> bool {
        matches!(
            &self.search,
            Some(Loadable::Ready(v)) if matches!(v.first(), Some(SearchItem::Header(h)) if *h == HISTORY_QUERIES || *h == HISTORY_FOUND)
        )
    }

    fn forget_search(&mut self, q: &str) {
        self.state.search_history.retain(|h| h != q);
        self.state.save(&self.paths);
        let sel = self.page().sel;
        self.show_search_history();
        let len = self.page_len();
        if len > 0 {
            self.page_mut().sel = sel.min(len - 1);
            self.skip_headers(-1);
            self.skip_headers(1);
        }
    }

    fn run_search(&mut self) {
        let q = self.search_input.trim().to_string();
        if q.is_empty() {
            self.show_search_history();
            return;
        }
        let h = &mut self.state.search_history;
        h.retain(|x| x.to_lowercase() != q.to_lowercase());
        h.insert(0, q.clone());
        h.truncate(SEARCH_HISTORY_LEN);
        self.state.save(&self.paths);
        self.search = Some(Loadable::Loading);
        self.page_mut().sel = 0;
        self.spawn(move |c, tx| async move {
            let r = c.search(&q).await.map_err(err_str);
            let _ = tx.send(Msg::Search { query: q, result: r });
        });
    }

    fn fetch_cover(&mut self, uri: &str) {
        self.fetch_image(uri.to_string(), uri, 1000, true);
    }

    /// Тема под обложку трека: сразу, если палитра уже посчитана, иначе — когда обложка придёт.
    fn apply_theme_for(&mut self, t: &Track) {
        if let Some(p) = t.cover_uri().and_then(|u| self.palettes.get(u)).copied() {
            self.theme.set(p);
        }
    }

    /// Иконка агента волны (догружается, если её вытеснили из кеша).
    pub fn want_agent(&mut self, uri: &str) {
        self.fetch_image(agent_key(uri), uri, 200, false);
    }

    /// Миниатюра обложки для строки списка (под курсором или мышью).
    pub fn want_thumb(&mut self, uri: &str) {
        self.fetch_image(thumb_key(uri), uri, 300, false);
    }

    /// Файл библиотеки, в который вшита обложка `uri`.
    fn cover_file(&mut self, uri: &str) -> Option<std::path::PathBuf> {
        if self.cover_files.is_none() {
            let mut m = HashMap::new();
            for id in self.library.ids() {
                if let Some(u) = self.snapshot.find(&id).and_then(|t| t.cover_uri()) {
                    m.entry(u.to_string()).or_insert(id);
                }
            }
            self.cover_files = Some(m);
        }
        let id = self.cover_files.as_ref()?.get(uri)?.clone();
        self.library.file(&id)
    }

    /// Картинка: дисковый кеш → обложка из тегов скачанного трека → сеть (и в кеш).
    fn fetch_image(&mut self, key: String, uri: &str, size: u32, palette: bool) {
        if self.images.contains(&key) || self.covers_failed.contains(&key) || !self.covers_loading.insert(key.clone())
        {
            return;
        }
        let url = api::cover_url(uri, size);
        let cache = self.paths.cache_dir.clone();
        let file = self.cover_file(uri);
        let (client, tx) = (self.client.clone(), self.tx.clone());
        tokio::spawn(async move {
            let r = async {
                let (c2, u2) = (cache.clone(), url.clone());
                let cached = tokio::task::spawn_blocking(move || {
                    imgcache::read(&c2, &u2).or_else(|| file.and_then(|p| crate::library::embedded_cover(&p)))
                })
                .await?;
                let bytes = match cached {
                    Some(b) => b,
                    None => {
                        let c = client.ok_or_else(|| color_eyre::eyre::eyre!("нет сети, а в кеше нет"))?;
                        let b = crate::fetch::fetch_bytes(&c, &url).await?;
                        let b2 = b.clone();
                        tokio::task::spawn_blocking(move || imgcache::write(&cache, &url, &b2));
                        b
                    }
                };
                let img = tokio::task::spawn_blocking(move || -> color_eyre::Result<_> {
                    let img = image::load_from_memory(&bytes)?.to_rgba8();
                    let pal = if palette {
                        Palette::from_image(&img)
                    } else {
                        Palette::default()
                    };
                    Ok((img, pal))
                })
                .await??;
                Ok::<_, color_eyre::Report>(img)
            }
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Cover { key, result: r });
        });
    }

    fn fetch_lyrics(&mut self, t: &Track) {
        self.lyrics = None;
        self.lyrics_scroll = 0.0;
        self.lyrics_user_offset = 0.0;
        self.lyrics_user_at = None;
        self.plain_scroll = 0;
        let (sync, text) = (t.has_sync_lyrics(), t.has_text_lyrics());
        if !sync && !text {
            return;
        }
        let id = t.id.clone();
        // Скачанный трек несёт текст в тегах — без сети и без лишнего запроса.
        let file = self.library.file(&id);
        let (client, tx) = (self.client.clone(), self.tx.clone());
        tokio::spawn(async move {
            let embedded = match file {
                Some(p) => tokio::task::spawn_blocking(move || crate::library::embedded_lyrics(&p))
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            // Старые файлы несут только обычный текст: при сети синхронный берём с сервера.
            let lrc = |s: &str| matches!(Lyrics::parse_lrc(s), Lyrics::Synced(_));
            let raw = match (embedded, client) {
                (Some(s), c) if !sync || c.is_none() || lrc(&s) => Some(s),
                (_, Some(c)) => c.lyrics(&id, sync).await.ok(),
                (e, None) => e,
            };
            let lyrics = raw
                .as_deref()
                .map(|s| if sync { Lyrics::parse_lrc(s) } else { Lyrics::plain(s) });
            let _ = tx.send(Msg::Lyrics {
                track_id: id,
                lyrics,
                raw,
            });
        });
    }

    // ── навигация ────────────────────────────────────────────────────────────

    pub fn page(&self) -> &Page {
        self.stack.last().expect("stack")
    }

    pub fn page_mut(&mut self) -> &mut Page {
        self.stack.last_mut().expect("stack")
    }

    fn push(&mut self, v: View) {
        self.stack.push(Page::new(v));
        self.view_fx();
    }

    fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
            self.view_fx();
        }
    }

    fn set_tab(&mut self, t: Tab) {
        if self.tab == t && self.stack.len() == 1 {
            return;
        }
        self.tab = t;
        let v = match t {
            Tab::Wave => View::Wave,
            Tab::Likes => View::Likes,
            Tab::Playlists => View::Playlists,
            Tab::Search => View::Search,
            Tab::Queue => View::Queue,
            Tab::Releases => View::Releases,
            Tab::Settings => View::Settings,
        };
        self.stack = vec![Page::new(v)];
        if t == Tab::Releases && self.releases.is_none() {
            self.load_releases();
        }
        if t == Tab::Queue {
            self.page_mut().sel = self.queue.pos.unwrap_or(0);
        }
        if t == Tab::Search && self.search.is_none() {
            self.show_search_history();
            self.search_editing = true;
        }
        self.view_fx();
    }

    fn view_fx(&mut self) {
        self.hover = None;
        self.fx_view = Some(Instant::now());
    }

    /// Список треков текущего вида, если это вид-список треков (их можно фильтровать).
    pub fn page_tracks(&self) -> Option<&[Track]> {
        match &self.page().view {
            View::Likes => self.likes.ready().map(Vec::as_slice),
            View::Queue => Some(&self.queue.items),
            View::Tracks { tracks, .. } => tracks.ready().map(Vec::as_slice),
            _ => None,
        }
    }

    /// Исходный индекс трека под курсором с учётом фильтра.
    fn sel_index(&self) -> usize {
        let p = self.page();
        match &p.filtered {
            Some(f) => f.get(p.sel).copied().unwrap_or(usize::MAX),
            None => p.sel,
        }
    }

    /// Пересчёт фильтра после изменения списка, без сброса курсора.
    fn refilter_keep(&mut self) {
        if self.page().filtered.is_none() {
            return;
        }
        let f = self.page().filter.clone();
        let filtered = self.page_tracks().map(|t| filter_tracks(t, &f));
        self.page_mut().filtered = filtered;
    }

    fn refilter(&mut self) {
        let f = self.page().filter.clone();
        let filtered = if f.trim().is_empty() {
            None
        } else {
            self.page_tracks().map(|t| filter_tracks(t, &f))
        };
        let p = self.page_mut();
        p.filtered = filtered;
        p.sel = 0;
        p.offset = 0;
    }

    /// Количество строк в текущем виде (для курсора).
    pub fn page_len(&self) -> usize {
        if let Some(f) = &self.page().filtered {
            return f.len();
        }
        match &self.page().view {
            View::Wave => self.wave.items.ready().map_or(0, Vec::len),
            View::Likes => self.likes.ready().map_or(0, Vec::len),
            View::Playlists => self.playlist_rows().len(),
            View::Search => match &self.search {
                Some(Loadable::Ready(items)) => items.len(),
                _ => 0,
            },
            View::Queue => self.queue.items.len(),
            View::Releases => self.releases.as_ref().and_then(Loadable::ready).map_or(0, Vec::len),
            View::Settings => Setting::ALL.len(),
            View::Tracks { tracks, .. } => tracks.ready().map_or(0, Vec::len),
            View::Artist { brief, .. } => brief.ready().map_or(0, |b| artist_rows(b).len()),
            View::Albums { albums, .. } => albums.ready().map_or(0, Vec::len),
            View::Artists { artists, .. } => artists.ready().map_or(0, Vec::len),
            View::Metatags { trees, .. } => trees.ready().map_or(0, |t| tag_rows(t).len()),
            View::Metatag { page, .. } => page.ready().map_or(0, |m| meta_rows(m).len()),
        }
    }

    /// Курсор постоял на месте — можно показывать всплывающую обложку.
    pub fn selection_settled(&self) -> bool {
        self.sel_moved_at.elapsed() > Duration::from_millis(450)
    }

    fn move_sel(&mut self, delta: isize) {
        self.sel_moved_at = Instant::now();
        let len = self.page_len();
        if len == 0 {
            return;
        }
        let p = self.page_mut();
        let mut sel = p.sel as isize + delta;
        if matches!(
            p.view,
            View::Search | View::Artist { .. } | View::Metatags { .. } | View::Metatag { .. }
        ) {
            // Заголовки секций пропускаем.
            sel = sel.clamp(0, len as isize - 1);
            p.sel = sel as usize;
            self.skip_headers(delta.signum());
            return;
        }
        p.sel = sel.clamp(0, len as isize - 1) as usize;
    }

    /// Строка `i` текущего вида — заголовок секции (на него курсор не ставим).
    fn is_header(&self, i: usize) -> bool {
        match &self.page().view {
            View::Search => matches!(
                self.search.as_ref().and_then(Loadable::ready).and_then(|v| v.get(i)),
                Some(SearchItem::Header(_))
            ),
            View::Artist { brief, .. } => brief
                .ready()
                .is_some_and(|b| matches!(artist_rows(b).get(i), Some(ArtistRow::Header(_)))),
            View::Metatags { trees, .. } => trees
                .ready()
                .is_some_and(|t| matches!(tag_rows(t).get(i), Some(TagRow::Header(_)))),
            View::Metatag { page, .. } => page
                .ready()
                .is_some_and(|m| matches!(meta_rows(m).get(i), Some(MetaRow::Header(_)))),
            _ => false,
        }
    }

    fn skip_headers(&mut self, dir: isize) {
        let n = self.page_len();
        let mut sel = self.page().sel;
        while sel < n && self.is_header(sel) {
            let next = sel as isize + if dir == 0 { 1 } else { dir };
            if next < 0 || next as usize >= n {
                break;
            }
            sel = next as usize;
        }
        self.page_mut().sel = sel;
    }

    /// Трек под курсором, если в текущем виде это трек.
    pub fn selected_track(&self) -> Option<&Track> {
        let sel = self.page().sel;
        if let Some(ts) = self.page_tracks() {
            return ts.get(self.sel_index());
        }
        match &self.page().view {
            View::Search => match self.search.as_ref()?.ready()?.get(sel)? {
                SearchItem::Track(t) => Some(t),
                _ => None,
            },
            View::Artist { brief, .. } => match artist_rows(brief.ready()?).get(sel)? {
                ArtistRow::Track(ts, i) => ts.get(*i),
                _ => None,
            },
            _ => None,
        }
    }

    /// Трек, к которому относятся действия «лайк/альбом/артист»: выделенный или играющий.
    fn target_track(&self) -> Option<Track> {
        self.selected_track()
            .cloned()
            .or_else(|| self.now.as_ref().map(|n| n.track.clone()))
    }

    fn activate(&mut self) {
        let sel = self.page().sel;
        let orig = self.sel_index();
        match &self.page().view {
            View::Wave if self.offline => self.start_offline_wave(),
            View::Wave => {
                let Some(item) = self.wave.items.ready().and_then(|v| v.get(sel)).cloned() else {
                    return;
                };
                // Этот пресет уже играет — Enter ставит на паузу, а не перезапускает.
                if self.is_active_preset(&item) {
                    self.toggle_pause();
                } else {
                    self.start_wave(item.seeds(), item.name().to_string());
                }
            }
            View::Likes => {
                if let Some(ts) = self.likes.ready() {
                    let ts = ts.clone();
                    self.play_list(ts, orig, "Коллекция".into());
                }
            }
            View::Playlists => match self.playlist_rows().get(sel) {
                Some(PlaylistRow::Personal(p)) => {
                    let pl = p.playlist.clone();
                    self.open_playlist(pl);
                }
                Some(PlaylistRow::Own(p)) => {
                    let pl = (*p).clone();
                    self.open_playlist(pl);
                }
                Some(PlaylistRow::LikedAlbums) => self.open_liked_albums(),
                Some(PlaylistRow::LikedArtists) => self.open_liked_artists(),
                Some(PlaylistRow::TopArtists) => self.open_top_artists(),
                None => {}
            },
            View::Albums { albums, .. } => {
                if let Some(a) = albums.ready().and_then(|v| v.get(sel)).cloned() {
                    self.open_album(a);
                }
            }
            View::Artists { artists, .. } => {
                if let Some((a, _)) = artists.ready().and_then(|v| v.get(sel)).cloned() {
                    self.open_artist(a);
                }
            }
            View::Metatags { trees, .. } => {
                if let Some(TagRow::Leaf(l, _)) = trees.ready().and_then(|t| tag_rows(t).into_iter().nth(sel)) {
                    let (tag, title) = (l.tag.clone(), l.title.clone());
                    self.open_metatag(tag, title);
                }
            }
            View::Metatag { page, title, .. } => {
                let Some(m) = page.ready() else { return };
                match meta_rows(m).into_iter().nth(sel) {
                    Some(MetaRow::Wave(s)) => {
                        let (seed, name) = (s.to_string(), format!("Волна: {title}"));
                        self.set_tab(Tab::Wave);
                        self.start_wave(vec![seed], name);
                    }
                    Some(MetaRow::Playlist(p)) => {
                        let p = p.clone();
                        self.open_playlist(p);
                    }
                    Some(MetaRow::Album(a)) => {
                        let a = a.clone();
                        self.open_album(a);
                    }
                    Some(MetaRow::Artist(a)) => {
                        let a = a.clone();
                        self.open_artist(a);
                    }
                    _ => {}
                }
            }
            View::Queue => self.play_index(orig),
            View::Settings => self.change_setting(sel, 0),
            View::Releases => {
                if let Some(a) = self
                    .releases
                    .as_ref()
                    .and_then(Loadable::ready)
                    .and_then(|r| r.get(sel))
                    .cloned()
                {
                    self.open_album(a);
                }
            }
            View::Tracks { tracks, title, .. } => {
                if let Some(ts) = tracks.ready() {
                    let (ts, title) = (ts.clone(), title.clone());
                    self.play_list(ts, orig, title);
                }
            }
            View::Search => {
                let Some(Loadable::Ready(items)) = &self.search else {
                    return;
                };
                match items.get(sel).cloned() {
                    Some(SearchItem::Track(_)) => {
                        let tracks: Vec<Track> = items
                            .iter()
                            .filter_map(|i| {
                                if let SearchItem::Track(t) = i {
                                    Some(t.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        let idx = items[..sel]
                            .iter()
                            .filter(|i| matches!(i, SearchItem::Track(_)))
                            .count();
                        self.play_list(tracks, idx, format!("Поиск: {}", self.search_input));
                    }
                    Some(SearchItem::Album(a)) => self.open_album(a),
                    Some(SearchItem::Artist(a)) => self.open_artist(a),
                    Some(SearchItem::Playlist(p)) => self.open_playlist(p),
                    Some(SearchItem::Query(q)) => {
                        self.search_input = q;
                        self.run_search();
                    }
                    _ => {}
                }
            }
            View::Artist { brief, name, .. } => {
                let Some(b) = brief.ready() else { return };
                let rows = artist_rows(b);
                match rows.get(sel) {
                    Some(ArtistRow::Track(ts, i)) => {
                        let (tracks, idx, name) = (ts.to_vec(), *i, name.clone());
                        self.play_list(tracks, idx, name);
                    }
                    Some(ArtistRow::Concert(c)) => {
                        if let Some(u) = c.afisha_url.clone() {
                            crate::ui::term::copy_to_clipboard(&mut self.osc_out, &u);
                            self.toast(format!("Ссылка на Афишу скопирована: {u}"));
                        }
                    }
                    Some(ArtistRow::Album(a)) => {
                        let a = (*a).clone();
                        self.open_album(a);
                    }
                    Some(ArtistRow::Artist(a)) => {
                        let a = (*a).clone();
                        self.open_artist(a);
                    }
                    Some(ArtistRow::Wave(id)) => {
                        let (seed, name) = (format!("artist:{id}"), format!("Волна: {name}"));
                        self.set_tab(Tab::Wave);
                        self.start_wave(vec![seed], name);
                    }
                    _ => {}
                }
            }
        }
    }

    // ── очередь и воспроизведение ────────────────────────────────────────────

    fn play_list(&mut self, mut tracks: Vec<Track>, start: usize, title: String) {
        if tracks.is_empty() {
            return;
        }
        let start = start.min(tracks.len() - 1);
        let start = if self.shuffle {
            let first = tracks.remove(start);
            tracks.shuffle(&mut rand::rng());
            tracks.insert(0, first);
            0
        } else {
            start
        };
        self.replace_queue(tracks, QueueKind::List, title);
        self.play_index(start);
    }

    fn replace_queue(&mut self, items: Vec<Track>, kind: QueueKind, title: String) {
        if !matches!(kind, QueueKind::OfflineWave) {
            self.owave = None;
        }
        self.queue = Queue {
            items,
            pos: None,
            kind,
            title,
            generation: self.queue.generation + 1,
        };
        self.next_loaded = None;
        self.failed.clear();
    }

    fn alloc_token(&mut self, idx: usize) -> Option<Token> {
        let track = self.queue.items.get(idx)?.clone();
        let t = self.next_token;
        self.next_token += 1;
        self.tokens.insert(t, (self.queue.generation, idx, track));
        Some(t)
    }

    pub fn play_index(&mut self, idx: usize) {
        self.play_index_at(idx, 0.0, false);
    }

    fn play_index_at(&mut self, idx: usize, start: f64, paused: bool) {
        let Some(opener) = self.opener() else {
            return;
        };
        self.play_index_with(idx, start, paused, opener);
    }

    fn play_index_with(&mut self, idx: usize, start: f64, paused: bool, fetcher: Opener) {
        self.resume_at = None;
        let Some(token) = self.alloc_token(idx) else {
            return;
        };
        if let Some(now) = &self.now {
            self.report_end(now.token, now.pos, true);
        }
        self.user_switch = true;
        self.requested = Some(token);
        self.start_paused = paused.then_some(token);
        self.next_loaded = None;
        self.queue.pos = Some(idx);
        let track = self.queue.items[idx].clone();
        self.loading_track = Some(track.clone());
        if let Some(c) = track.cover_uri() {
            let c = c.to_string();
            self.fetch_cover(&c);
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = fetcher.open(&track).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Opened {
                token,
                next: false,
                start,
                result: r,
            });
        });
    }

    /// Чем открывать треки: загрузчик, а без сети — только локальная библиотека.
    fn opener(&self) -> Option<Opener> {
        match &self.fetcher {
            Some(f) => Some(Opener::Online(f.clone())),
            None if self.offline => Some(Opener::Offline(self.library.clone())),
            None => None,
        }
    }

    fn normalize(&self) -> bool {
        self.state.normalize.unwrap_or(self.cfg.normalize)
    }

    fn crossfade(&self) -> f32 {
        self.state.crossfade.unwrap_or(self.cfg.crossfade_secs)
    }

    fn toggle_normalize(&mut self) {
        let on = !self.normalize();
        self.state.normalize = Some(on);
        self.reapply_gain();
        self.toast(
            if on {
                "Выравнивание громкости: вкл"
            } else {
                "Выравнивание громкости: выкл (bit-perfect)"
            }
            .into(),
        );
    }

    /// Усиление поменялось: играющему треку — новое и перемотка на текущее место
    /// (в кольце звук уже с прежним усилением); следующий открываем заново.
    fn reapply_gain(&mut self) {
        if let Some(n) = &self.now {
            let (token, pos) = (n.token, n.pos);
            let g = self.gain_for(&n.track.clone());
            self.player.set_gain(token, g);
            self.seek_to(pos);
        }
        self.player.clear_next();
        self.next_loaded = None;
        self.schedule_next();
    }

    fn cycle_crossfade(&mut self) {
        let cur = self.crossfade();
        let next = CROSSFADE_STEPS.iter().copied().find(|s| *s > cur + 0.01).unwrap_or(0.0);
        self.set_crossfade(next);
    }

    fn set_crossfade(&mut self, next: f32) {
        self.state.crossfade = Some(next);
        self.player.set_crossfade(next);
        self.toast(if next > 0.0 {
            format!("Плавный переход: {next:.0} с (кроме треков одного альбома)")
        } else {
            "Плавный переход: выкл (встык)".into()
        });
    }

    fn gain_for(&self, t: &Track) -> f32 {
        if !self.normalize() {
            return 1.0;
        }
        match t.r128 {
            Some(r) => {
                let db = (self.cfg.normalize_target_lufs - r.i).min(-r.tp - 1.0);
                10f32.powf(db / 20.0)
            }
            None => 1.0,
        }
    }

    fn next_index(&self, from: usize) -> Option<usize> {
        if self.repeat_one {
            return Some(from);
        }
        // Недоступные (а без сети — и не скачанные) и не открывшиеся пропускаем.
        (from + 1..self.queue.items.len()).find(|&i| self.playable(i))
    }

    fn playable(&self, i: usize) -> bool {
        self.queue.items[i].available && !self.failed.contains(&(self.queue.generation, i))
    }

    fn mark_failed(&mut self, token: Token) {
        if let Some((generation, idx, _)) = self.tokens.get(&token) {
            self.failed.insert((*generation, *idx));
        }
    }

    /// Предзагруженный следующий не открылся: текущий не трогаем, готовим следующий за ним.
    fn next_failed(&mut self, token: Token) {
        self.mark_failed(token);
        self.next_loaded = None;
        self.schedule_next();
    }

    // ── офлайн ───────────────────────────────────────────────────────────────

    /// Подсказка рекомендателю из ответа ротора — для следующего старта офлайн-волны.
    fn keep_server_data(&mut self, data: Option<Vec<i32>>) {
        if let (Some(d), Some(store)) = (data.filter(|d| !d.is_empty()), self.owave_store.as_mut()) {
            store.server_data = d;
            store.save(&self.data_dir);
        }
    }

    /// Программа офлайн-волны под текущую библиотеку: перезапрос, если библиотека заметно
    /// изменилась или программа старая. Лайкнутые и остальные — разными списками, как в клиенте.
    fn refresh_offline_program(&mut self) {
        if self.offline || self.owave_fetching {
            return;
        }
        let lib: Vec<String> = self.library.ids();
        let ids: HashSet<i32> = lib.iter().filter_map(|i| i.parse().ok()).collect();
        if ids.len() < 3 || self.owave_store.as_ref().is_some_and(|s| !s.stale(&ids)) {
            return;
        }
        let (liked, rest): (Vec<String>, Vec<String>) = lib.into_iter().partition(|i| self.liked.contains(i));
        self.owave_fetching = true;
        let ids: Vec<i32> = ids.into_iter().collect();
        self.spawn(move |c, tx| async move {
            let r = c
                .offline_recommender(&liked, &rest)
                .await
                .map(|p| (p, ids))
                .map_err(err_str);
            let _ = tx.send(Msg::OfflineProgram(r));
        });
    }

    /// Метаданные скачанных треков, которых нет в снимке, — пачками, пока есть сеть.
    fn backfill_library_meta(&mut self) {
        let lib: HashSet<String> = self.library.ids().into_iter().collect();
        self.snapshot.library_tracks.retain(|id, _| lib.contains(id));
        let missing: Vec<String> = lib.into_iter().filter(|id| self.snapshot.find(id).is_none()).collect();
        if missing.is_empty() {
            return;
        }
        self.spawn(move |c, tx| async move {
            let mut out = Vec::new();
            for chunk in missing.chunks(200) {
                match c.tracks(chunk).await {
                    Ok(ts) => out.extend(ts),
                    Err(e) => {
                        tracing::warn!("метаданные библиотеки: {e}");
                        break;
                    }
                }
            }
            let _ = tx.send(Msg::LibraryMeta(out));
        });
    }

    /// Моя волна без сети: TinyMl выбирает из скачанного.
    fn start_offline_wave(&mut self) {
        let Some(store) = self.owave_store.as_ref() else {
            self.toast("Офлайн-волна ещё не готова: программа скачивается, когда есть сеть".into());
            return;
        };
        let mut pool: HashMap<i32, Track> = HashMap::new();
        for id in self.library.ids() {
            if let (Ok(n), Some(t)) = (id.parse::<i32>(), self.snapshot.find(&id)) {
                pool.insert(n, t.clone());
            }
        }
        if pool.len() < 3 {
            self.toast("Для офлайн-волны нужно хотя бы 3 скачанных трека".into());
            return;
        }
        let available: Vec<i32> = pool.keys().copied().collect();
        let context: Vec<(i32, Option<i32>)> = self
            .queue
            .items
            .iter()
            .rev()
            .take(20)
            .rev()
            .filter_map(|t| {
                let album = t.full_id().split_once(':').and_then(|(_, a)| a.parse().ok());
                Some((t.id.parse().ok()?, album))
            })
            .collect();
        match offline_wave::Session::start(store, &self.data_dir, &available, &context) {
            Ok((session, first)) => {
                let tracks: Vec<Track> = first.iter().filter_map(|id| pool.get(id).cloned()).collect();
                if tracks.is_empty() {
                    self.toast_err("Офлайн-волна ничего не выбрала".into());
                    return;
                }
                self.owave_tracks = pool;
                self.replace_queue(tracks, QueueKind::OfflineWave, "Моя волна · офлайн".into());
                self.owave = Some(session);
                self.play_index(0);
            }
            Err(e) => self.toast_err(format!("Офлайн-волна: {e}")),
        }
    }

    /// Держит в очереди один трек после играющего: его выбирает TinyMl.
    fn extend_offline_wave(&mut self) {
        if !matches!(self.queue.kind, QueueKind::OfflineWave) {
            return;
        }
        let Some(pos) = self.queue.pos else { return };
        if pos + 2 < self.queue.items.len() {
            return;
        }
        let id = |t: &Track| t.id.parse::<i32>().unwrap_or(0);
        let last = self.queue.items.get(pos).map_or(0, id);
        let queued = self.queue.items.last().map_or(last, id);
        let Some(session) = self.owave.as_mut() else { return };
        match session.next(last, queued) {
            Ok(n) => match self.owave_tracks.get(&n).cloned() {
                Some(t) => {
                    let was_at_end = self.next_loaded.is_none();
                    self.queue.items.push(t);
                    if was_at_end {
                        self.schedule_next();
                    }
                }
                None => tracing::warn!("офлайн-волна выбрала трек {n}, которого нет в библиотеке"),
            },
            Err(e) => self.toast_err(format!("Офлайн-волна: {e}")),
        }
    }

    /// Без сети играет только скачанное: остальное помечается недоступным.
    fn offline_tracks(&self, mut ts: Vec<Track>) -> Vec<Track> {
        for t in &mut ts {
            t.available &= self.library.contains(&t.id);
        }
        ts
    }

    /// Сети нет, но есть снимок — открываем библиотеку и играем скачанное.
    /// Подключение тем временем повторяется; когда пройдёт, приложение станет онлайн.
    fn enter_offline(&mut self) {
        self.offline = true;
        self.screen = Screen::Main;
        let likes = self.offline_tracks(self.snapshot.likes.clone());
        self.liked = likes.iter().map(|t| t.id.clone()).collect();
        self.likes = Loadable::Ready(likes);
        self.playlists = Loadable::Ready(self.snapshot.playlists.clone());
        self.wave.items = Loadable::Failed(if self.owave_store.is_some() {
            "нет сети — Enter или w: офлайн-волна по скачанному".into()
        } else {
            "нет сети, а программа офлайн-волны ещё не скачана".into()
        });
        // Очередь из кеша: без сети доступно только скачанное.
        if !self.queue.items.is_empty() && self.now.is_none() {
            let items = std::mem::take(&mut self.queue.items);
            self.queue.items = self.offline_tracks(items);
            let from = self.resume_at.take().map_or(0, |(p, _)| p);
            if let Some(pos) = (from..self.queue.items.len()).chain(0..from).find(|&i| self.queue.items[i].available) {
                self.play_index_at(pos, 0.0, true);
            }
        }
        if self.queue.items.is_empty()
            && let Some(r) = self.state.resume.clone()
        {
            let ts: Vec<Track> = r.ids.iter().filter_map(|id| self.snapshot.find(id).cloned()).collect();
            let ts = self.offline_tracks(ts);
            if let Some(pos) = ts.iter().position(|t| t.available) {
                self.replace_queue(ts, QueueKind::List, r.title);
                self.play_index_at(pos, 0.0, true);
            }
        }
        let (n, _) = self.library.stats();
        self.toast(format!("Нет сети — офлайн: играет скачанное ({n} треков)"));
    }

    /// Треки плейлистов для снимка: только те, у кого сменилась ревизия, по одному запросу.
    fn sync_snapshot_playlists(&mut self, pls: &[Playlist]) {
        let keep: HashSet<String> = pls.iter().map(offline::key).collect();
        self.snapshot.playlist_tracks.retain(|k, _| keep.contains(k));
        let stale: Vec<Playlist> = pls.iter().filter(|p| self.snapshot.stale(p)).cloned().collect();
        if stale.is_empty() {
            self.snapshot.save(self.snapshot_path.clone());
            return;
        }
        self.spawn(move |c, tx| async move {
            let mut out = Vec::new();
            for pl in stale {
                let r = async {
                    let full = c.playlist(&pl.uid, &pl.kind).await?;
                    let rev = full.revision;
                    Ok::<_, ApiError>((rev, c.playlist_tracks(&full).await?))
                }
                .await;
                match r {
                    Ok((rev, ts)) => out.push((offline::key(&pl), rev, ts)),
                    Err(e) => tracing::warn!("офлайн-снимок «{}»: {e}", pl.title),
                }
            }
            let _ = tx.send(Msg::PlaylistSnapshots(out));
        });
    }

    /// Заранее открывает следующий трек (старт загрузки) и отдаёт его декодеру для gapless.
    fn schedule_next(&mut self) {
        let Some(cur) = self.now.as_ref().and_then(|n| self.tokens.get(&n.token)).map(|t| t.1) else {
            return;
        };
        let Some(gen_ok) = self
            .now
            .as_ref()
            .and_then(|n| self.tokens.get(&n.token))
            .map(|t| t.0 == self.queue.generation)
        else {
            return;
        };
        if !gen_ok {
            return;
        }
        self.maybe_extend_wave(cur);
        let Some(idx) = self.next_index(cur) else {
            return;
        };
        let Some(token) = self.alloc_token(idx) else {
            return;
        };
        let Some(fetcher) = self.opener() else {
            return;
        };
        self.next_loaded = Some(token);
        let track = self.queue.items[idx].clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = fetcher.open(&track).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Opened {
                token,
                next: true,
                start: 0.0,
                result: r,
            });
        });
    }

    pub fn next(&mut self) {
        let cur = self.queue.pos.unwrap_or(0);
        let idx = if self.repeat_one {
            (cur + 1 < self.queue.items.len()).then_some(cur + 1)
        } else {
            self.next_index(cur)
        };
        match idx {
            Some(i) => self.play_index(i),
            None => {
                if matches!(self.queue.kind, QueueKind::Wave { .. }) {
                    self.toast("Волна подгружается…".into());
                    self.maybe_extend_wave(cur);
                } else if matches!(self.queue.kind, QueueKind::OfflineWave) {
                    self.extend_offline_wave();
                    if cur + 1 < self.queue.items.len() {
                        self.play_index(cur + 1);
                    }
                }
            }
        }
    }

    pub fn prev(&mut self) {
        if self.now.as_ref().is_some_and(|n| n.pos > 4.0) {
            self.seek_to(0.0);
            return;
        }
        if let Some(p) = self.queue.pos
            && let Some(i) = (0..p).rev().find(|&i| self.playable(i))
        {
            self.play_index(i);
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.now.is_none() && self.loading_track.is_none() {
            if let Some(p) = self.queue.pos.or((!self.queue.items.is_empty()).then_some(0)) {
                self.play_index(p);
            }
            return;
        }
        let p = !self.player.paused();
        self.player.set_paused(p);
        let pos = self.now.as_ref().map_or(0.0, |n| n.pos);
        let _ = self.mpris.send(mpris::Update::Status { playing: !p, pos });
        self.yn_local_play_state();
    }

    fn seek_to(&mut self, secs: f64) {
        let Some(now) = &mut self.now else { return };
        let dur = now.track.duration_secs();
        let s = if dur > 0.0 {
            secs.clamp(0.0, dur - 0.5)
        } else {
            secs.max(0.0)
        };
        self.player.seek(now.token, s);
        now.pos = s;
        let _ = self.mpris.send(mpris::Update::Seeked(s));
        self.yn_publish_status();
    }

    fn seek_by(&mut self, d: f64) {
        if let Some(p) = self.now.as_ref().map(|n| n.pos) {
            self.seek_to(p + d);
        }
    }

    fn set_volume(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.player.set_volume(v);
        self.state.volume = Some(v);
        let _ = self.mpris.send(mpris::Update::Volume(v as f64));
    }

    fn cycle_quality(&mut self) {
        let Some(f) = &self.fetcher else { return };
        let q = match f.quality() {
            Quality::Lossless => Quality::Hq,
            Quality::Hq => Quality::Nq,
            Quality::Nq => Quality::Lossless,
        };
        self.set_quality(q);
    }

    fn set_quality(&mut self, q: Quality) {
        let Some(f) = &self.fetcher else {
            return self.toast_err("Без сети качество не выбрать".into());
        };
        f.set_quality(q);
        if q == Quality::Lossless {
            self.hot_upgrade(false);
        }
        self.state.quality = Some(q);
        self.toast(format!("Качество: {} — для следующих треков", quality_label(q)));
    }

    /// Текущее значение строки настроек.
    pub fn setting_value(&self, s: Setting) -> String {
        match s {
            Setting::Quality => quality_label(self.quality()).into(),
            Setting::Normalize => if self.normalize() { "вкл" } else { "выкл · bit-perfect" }.into(),
            Setting::TargetLufs => format!("{:.0} LUFS", self.cfg.normalize_target_lufs),
            Setting::Crossfade => match self.crossfade() {
                c if c > 0.0 => format!("{c:.0} с"),
                _ => "выкл".into(),
            },
            Setting::ReportPlays => if self.cfg.report_plays { "да" } else { "нет" }.into(),
            Setting::Ynison => self.ynison_label(),
            Setting::Fps => self.cfg.fps.to_string(),
            Setting::Dither => match self.cfg.blob_dither {
                0 => "выкл".into(),
                d => d.to_string(),
            },
            Setting::Recheck => match self.library.below_lossless() {
                0 => "всё уже в lossless".into(),
                n => format!("{n} тр. хуже lossless"),
            },
        }
    }

    /// Меняет настройку `i`: `dir` ±1 — стрелки, 0 — Enter/клик (следующее значение или действие).
    fn change_setting(&mut self, i: usize, dir: isize) {
        let Some(&s) = Setting::ALL.get(i) else { return };
        let step = |len: usize, cur: usize| -> usize {
            if dir == 0 {
                (cur + 1) % len
            } else {
                (cur as isize + dir).clamp(0, len as isize - 1) as usize
            }
        };
        match s {
            Setting::Quality => {
                const QS: [Quality; 3] = [Quality::Nq, Quality::Hq, Quality::Lossless];
                let cur = QS.iter().position(|q| *q == self.quality()).unwrap_or(2);
                let next = QS[step(QS.len(), cur)];
                if next != self.quality() {
                    self.set_quality(next);
                }
            }
            Setting::Normalize => self.toggle_normalize(),
            Setting::TargetLufs => {
                let d = if dir == 0 { 1.0 } else { dir as f32 };
                let v = (self.cfg.normalize_target_lufs.round() + d).clamp(-23.0, -8.0);
                if v != self.cfg.normalize_target_lufs {
                    self.cfg.normalize_target_lufs = v;
                    self.cfg.save(&self.paths);
                    if self.normalize() {
                        self.reapply_gain();
                    }
                }
            }
            Setting::Crossfade => {
                let cur = CROSSFADE_STEPS
                    .iter()
                    .position(|c| (*c - self.crossfade()).abs() < 0.01)
                    .unwrap_or(0);
                let next = CROSSFADE_STEPS[step(CROSSFADE_STEPS.len(), cur)];
                if (next - self.crossfade()).abs() > 0.01 {
                    self.set_crossfade(next);
                }
            }
            Setting::ReportPlays => {
                self.cfg.report_plays = !self.cfg.report_plays;
                self.cfg.save(&self.paths);
            }
            Setting::Ynison => {
                self.cfg.ynison = !self.cfg.ynison;
                self.cfg.save(&self.paths);
                if self.cfg.ynison {
                    self.ynison_start();
                } else {
                    self.ynison_stop();
                }
            }
            Setting::Fps => {
                let cur = FPS_STEPS.iter().position(|f| *f >= self.cfg.fps).unwrap_or(FPS_STEPS.len() - 1);
                self.cfg.fps = FPS_STEPS[step(FPS_STEPS.len(), cur)];
                self.cfg.save(&self.paths);
            }
            Setting::Dither => {
                self.cfg.blob_dither = step(5, self.cfg.blob_dither.min(4) as usize) as u32;
                self.cfg.save(&self.paths);
            }
            Setting::Recheck if dir == 0 => match self.library.reset_checks() {
                0 => self.toast("Все скачанные треки уже в lossless".into()),
                n => self.toast(format!("{n} тр. перепроверятся на lossless при проигрывании")),
            },
            Setting::Recheck => {}
        }
    }

    /// Играющий трек не lossless — попросить FLAC и подменить его на ходу, без паузы.
    fn hot_upgrade(&mut self, auto: bool) {
        let Some(n) = &self.now else { return };
        // Подмена этого трека уже идёт (отметки от прошлых треков не в счёт).
        if matches!(n.info.codec.as_str(), "FLAC" | "ALAC") || self.swap_origin.as_ref().is_some_and(|(t, ..)| *t == n.token) {
            return;
        }
        let Some(f) = self.fetcher.clone() else { return };
        let (token, track) = (n.token, n.track.clone());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = f.upgrade_source(&track).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Upgraded { token, auto, result });
        });
    }

    fn on_upgraded(&mut self, token: Token, auto: bool, result: Result<Option<(audio::Source, Origin)>, String>) {
        let playing = self.now.as_ref().is_some_and(|n| n.token == token);
        match result {
            Ok(Some((src, origin))) if playing => {
                self.swap_origin = Some((token, origin, auto));
                self.player.swap(token, src);
            }
            Ok(Some((src, _))) => src.cancel(),
            Ok(None) if playing && !auto => self.toast("У этого трека нет lossless".into()),
            Err(e) if playing && !auto => self.toast_err(format!("Lossless: {e}")),
            Err(e) => tracing::warn!("lossless для подмены: {e}"),
            _ => {}
        }
    }

    fn on_swapped(&mut self, token: Token, result: Result<audio::SourceInfo, String>) {
        tracing::info!("подмена {token}: {}", match &result {
            Ok(i) => format!("готово, {} {} Гц", i.codec, i.rate),
            Err(e) => format!("не вышло: {e}"),
        });
        let taken = self.swap_origin.take_if(|(t, ..)| *t == token);
        let auto = taken.as_ref().is_some_and(|s| s.2);
        let origin = taken.map(|(_, o, _)| o);
        let Some(n) = self.now.as_mut().filter(|n| n.token == token) else { return };
        match result {
            Ok(info) => {
                if origin.is_some() {
                    n.origin = origin;
                }
                let fmt = match info.bits {
                    Some(b) => format!("{} {b}/{}", info.codec, khz(info.rate)),
                    None => format!("{} {}", info.codec, khz(info.rate)),
                };
                let text = match info.resampled_to {
                    Some(to) => format!("{fmt} без паузы, ресемплинг в {} · B — перезапустить bit-perfect", khz(to)),
                    None => format!("{fmt} без паузы"),
                };
                self.swap_toast = Some((token, text));
            }
            Err(e) => {
                tracing::warn!("подмена на ходу: {e}");
                if !auto {
                    self.toast_err(format!("Lossless на ходу не вышел: {e} — будет со следующего раза"));
                }
            }
        }
    }

    /// Звук ресемплится после подмены — перезапустить трек с той же секунды в родном формате.
    fn restart_bit_perfect(&mut self) {
        let Some(n) = &self.now else { return };
        if n.info.resampled_to.is_none() {
            return self.toast("Звук и так bit-perfect".into());
        }
        let (token, pos) = (n.token, n.pos);
        self.reload(token, pos);
        self.toast("Перезапуск в родном формате".into());
    }

    fn enqueue(&mut self, t: Track, play_next: bool) {
        let title = t.full_title();
        let at = match (play_next, self.queue.pos) {
            (true, Some(p)) => p + 1,
            _ => self.queue.items.len(),
        };
        self.queue.items.insert(at.min(self.queue.items.len()), t);
        // Индексы сдвинулись — заранее загруженный следующий трек мог устареть.
        self.queue.generation += 1;
        if let Some(now) = &self.now
            && let Some(e) = self.tokens.get_mut(&now.token)
        {
            e.0 = self.queue.generation;
        }
        self.player.clear_next();
        self.next_loaded = None;
        self.schedule_next();
        self.toast(if play_next {
            format!("Следующим: {title}")
        } else {
            format!("В очередь: {title}")
        });
    }

    // ── Моя волна ────────────────────────────────────────────────────────────

    fn load_wheel(&mut self) {
        self.spawn(|c, tx| async move {
            let (wheel, last) = tokio::join!(c.wheel(), c.wave_last());
            let r = wheel.map(|mut items| {
                // Последняя волна (например, по исполнителю), если её нет среди пресетов, —
                // первой на дуге, как «продолжить» в десктопе.
                if let Ok(last) = last
                    && !last.seeds.is_empty()
                    && last.seeds != [MY_WAVE]
                    && !items.iter().any(|i| i.seeds() == last.seeds)
                {
                    items.insert(
                        0,
                        api::WheelItem {
                            style: "LAST".into(),
                            data: api::WheelData {
                                wave: Some(api::WheelWave {
                                    description: Some("продолжить последнюю волну".into()),
                                    ..last
                                }),
                                agent: None,
                            },
                        },
                    );
                }
                items
            });
            let _ = tx.send(Msg::Wheel(r.map_err(err_str)));
        });
    }

    /// Пресет сейчас играет (волна запущена с его сидами).
    pub fn is_active_preset(&self, item: &api::WheelItem) -> bool {
        matches!(self.queue.kind, QueueKind::Wave { .. })
            && self.wave.active_seeds.as_ref().is_some_and(|s| *s == item.seeds())
    }

    fn start_wave(&mut self, seeds: Vec<String>, name: String) {
        if self.wave.loading {
            return;
        }
        self.wave.loading = true;
        self.wave.active_seeds = Some(seeds.clone());
        self.wave.active_name = Some(name);
        self.spawn(move |c, tx| async move {
            let r = c.rotor_new(&seeds).await.map_err(err_str);
            if let Ok(s) = &r {
                let _ = c
                    .rotor_feedback(&s.radio_session_id, s.batch_id.as_deref(), "radioStarted", None, None)
                    .await;
            }
            let _ = tx.send(Msg::WaveStarted(r));
        });
    }

    fn maybe_extend_wave(&mut self, cur: usize) {
        let QueueKind::Wave { session, .. } = &self.queue.kind else {
            return;
        };
        if cur + 3 < self.queue.items.len() || self.wave.extending {
            return;
        }
        self.wave.extending = true;
        let session = session.clone();
        let queue: Vec<String> = self
            .queue
            .items
            .iter()
            .rev()
            .take(20)
            .rev()
            .map(Track::full_id)
            .collect();
        self.spawn(move |c, tx| async move {
            let result = c.rotor_tracks(&session, &queue).await.map_err(err_str);
            let _ = tx.send(Msg::WaveMore { session, result });
        });
    }

    fn wave_feedback(&mut self, kind: &'static str, t: &Track, played: Option<f64>) {
        if matches!(self.queue.kind, QueueKind::OfflineWave) {
            let ev = match kind {
                "trackStarted" => EventKind::Started,
                "trackFinished" => EventKind::Finished,
                "skip" => EventKind::Skip,
                "like" => EventKind::Like,
                "unlike" => EventKind::Unlike,
                "dislike" => EventKind::Dislike,
                _ => return,
            };
            if let (Some(s), Ok(id)) = (self.owave.as_mut(), t.id.parse()) {
                s.event(id, ev, played.unwrap_or(0.0) as f32);
            }
            return;
        }
        let QueueKind::Wave { session, batch } = &self.queue.kind else {
            return;
        };
        if kind == "unlike" {
            return;
        }
        let (session, batch, id) = (session.clone(), batch.clone(), t.full_id());
        self.spawn(move |c, _| async move {
            if let Err(e) = c
                .rotor_feedback(&session, batch.as_deref(), kind, Some(&id), played)
                .await
            {
                tracing::debug!("rotor feedback {kind}: {e}");
            }
        });
    }

    /// Отчёт о прослушивании: история Яндекса + обратная связь волны.
    fn report_end(&mut self, token: Token, played: f64, skipped: bool) {
        let Some((_, _, t)) = self.tokens.get(&token).cloned() else {
            return;
        };
        let full = played >= t.duration_secs() - 2.0;
        self.wave_feedback(if skipped && !full { "skip" } else { "trackFinished" }, &t, Some(played));
        if self.cfg.report_plays && played > 1.0 {
            let t = t.clone();
            self.spawn(move |c, _| async move {
                let _ = c.play_audio(&t, played, "desktop_win-own_tracks-track-default").await;
            });
        }
    }

    // ── правка плейлистов ────────────────────────────────────────────────────

    fn is_own(&self, pl: &Playlist) -> bool {
        self.client.as_ref().is_some_and(|c| c.uid == pl.uid)
    }

    fn pl_op(&self, op: PlOp) {
        if let Some(tx) = &self.pl_ops {
            let _ = tx.send(op);
        }
    }

    /// P: выбрать свой плейлист для выделенного (или играющего) трека.
    fn add_to_playlist(&mut self) {
        let Some(track) = self.target_track() else { return };
        let list: Vec<(String, String)> = self
            .playlists
            .ready()
            .map(|v| {
                v.iter()
                    .filter(|p| self.is_own(p))
                    .map(|p| (p.kind.clone(), p.title.clone()))
                    .collect()
            })
            .unwrap_or_default();
        self.popup = Some(Popup::AddToPlaylist { track, list, sel: 0 });
    }

    /// Плейлист под курсором на вкладке «Плейлисты», если он свой.
    fn selected_own_playlist(&self) -> Option<Playlist> {
        if !matches!(self.page().view, View::Playlists) {
            return None;
        }
        match self.playlist_rows().get(self.page().sel)? {
            PlaylistRow::Own(pl) => self.is_own(pl).then(|| (*pl).clone()),
            _ => None,
        }
    }

    /// Свой плейлист, открытый на текущей странице: (kind, позиция трека, трек).
    fn playlist_track_at_cursor(&self) -> Option<(String, usize, Track)> {
        let View::Tracks {
            playlist: Some(kind),
            tracks,
            ..
        } = &self.page().view
        else {
            return None;
        };
        let at = self.sel_index();
        Some((kind.clone(), at, tracks.ready()?.get(at)?.clone()))
    }

    fn confirm_remove_track(&mut self) {
        let Some((kind, at, track)) = self.playlist_track_at_cursor() else {
            return;
        };
        self.popup = Some(Popup::Confirm {
            text: format!("Убрать «{}» из плейлиста?", track.full_title()),
            action: ConfirmAction::RemoveTrack { kind, at, track },
        });
    }

    /// Shift+↑/↓ (K/J) в своём плейлисте: сдвинуть трек на одну позицию.
    fn move_track(&mut self, d: isize) {
        if self.page().filtered.is_some() {
            self.toast("Сначала сними фильтр — переставлять можно только в полном списке".into());
            return;
        }
        let Some((kind, at, track)) = self.playlist_track_at_cursor() else {
            return;
        };
        let to = at as isize + d;
        let View::Tracks { tracks, .. } = &mut self.page_mut().view else {
            return;
        };
        let Some(ts) = tracks.ready_mut() else { return };
        if to < 0 || to as usize >= ts.len() {
            return;
        }
        let to = to as usize;
        let neighbor = ts[to].full_id();
        ts.swap(at, to);
        self.page_mut().sel = to;
        self.pl_op(PlOp::Move {
            kind,
            at,
            track: track.full_id(),
            neighbor,
        });
    }

    /// Перечитать открытые страницы плейлиста `kind` (после неудачной правки).
    fn reload_playlist_views(&mut self, kind: &str) {
        let Some(pl) = self
            .playlists
            .ready()
            .and_then(|v| v.iter().find(|p| p.kind == kind && self.is_own(p)))
            .cloned()
        else {
            return;
        };
        let ids: Vec<u64> = self
            .stack
            .iter()
            .filter_map(|p| match &p.view {
                View::Tracks {
                    id,
                    playlist: Some(k),
                    ..
                } if k == kind => Some(*id),
                _ => None,
            })
            .collect();
        for id in ids {
            let pl = pl.clone();
            self.spawn(move |c, tx| async move {
                let r = async {
                    let full = c.playlist(&pl.uid, &pl.kind).await?;
                    c.playlist_tracks(&full).await
                }
                .await
                .map_err(err_str);
                let _ = tx.send(Msg::Tracks { view_id: id, result: r });
            });
        }
    }

    fn input_submit(&mut self, text: String, action: InputAction) {
        let title = text.trim().to_string();
        if let InputAction::Describe(kind) = action {
            if let Loadable::Ready(v) = &mut self.playlists
                && let Some(p) = v.iter_mut().find(|p| p.kind == kind)
            {
                p.description = (!title.is_empty()).then(|| title.clone());
            }
            return self.pl_op(PlOp::Describe { kind, text: title });
        }
        if title.is_empty() {
            return;
        }
        match action {
            InputAction::Describe(_) => {}
            InputAction::Create(track) => self.pl_op(PlOp::Create {
                title,
                add: track.map(|t| t.full_id()),
            }),
            InputAction::Rename(kind) => {
                if let Loadable::Ready(v) = &mut self.playlists
                    && let Some(p) = v.iter_mut().find(|p| p.kind == kind)
                {
                    p.title = title.clone();
                }
                self.pl_op(PlOp::Rename { kind, title });
            }
        }
    }

    fn confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeletePlaylist(kind, title) => {
                if let Loadable::Ready(v) = &mut self.playlists {
                    v.retain(|p| p.kind != kind || p.uid != self.client.as_ref().map_or("", |c| c.uid.as_str()));
                }
                self.pl_op(PlOp::Delete { kind, title });
            }
            ConfirmAction::RemoveTrack { kind, at, track } => {
                if let View::Tracks { tracks, .. } = &mut self.page_mut().view
                    && let Some(ts) = tracks.ready_mut()
                    && ts.get(at).is_some_and(|t| t.id == track.id)
                {
                    ts.remove(at);
                }
                self.refilter_keep();
                let n = self.page_len();
                let p = self.page_mut();
                p.sel = p.sel.min(n.saturating_sub(1));
                self.pl_op(PlOp::Remove {
                    kind,
                    at,
                    track: track.full_id(),
                });
            }
        }
    }

    // ── лайки ────────────────────────────────────────────────────────────────

    pub fn is_liked(&self, t: &Track) -> bool {
        self.liked.contains(&t.id)
    }

    fn toggle_like(&mut self, t: Track) {
        if self.client.is_none() {
            return self.toast_err("Без сети лайк не сохранить".into());
        }
        let like = !self.is_liked(&t);
        self.apply_like(&t, like);
        if like {
            self.wave_feedback("like", &t, None);
            self.toast(format!("\u{f004} {}", t.full_title()));
        } else {
            self.wave_feedback("unlike", &t, None);
            self.toast(format!("\u{f08a} {}", t.full_title()));
        }
        let id = t.full_id();
        self.spawn(move |c, tx| async move {
            if let Err(e) = c.set_like(&id, like).await {
                let _ = tx.send(Msg::LikeFailed {
                    track: t,
                    like,
                    error: e.to_string(),
                });
            }
        });
    }

    /// Локальное состояние лайка: множество, коллекция, фильтр, MPRIS.
    fn apply_like(&mut self, t: &Track, like: bool) {
        if like {
            self.liked.insert(t.id.clone());
            if let Loadable::Ready(v) = &mut self.likes
                && !v.iter().any(|x| x.id == t.id)
            {
                v.insert(0, t.clone());
            }
        } else {
            self.liked.remove(&t.id);
            if let Loadable::Ready(v) = &mut self.likes {
                v.retain(|x| x.id != t.id);
            }
        }
        self.refilter_keep();
        self.mpris_liked();
    }

    /// Иконка лайка в MPRIS-расширении шелла.
    fn mpris_liked(&self) {
        if let Some(n) = &self.now {
            let _ = self.mpris.send(mpris::Update::Liked(self.is_liked(&n.track)));
        }
    }

    fn entity_set(&mut self, what: Entity) -> &mut HashSet<String> {
        match what {
            Entity::AlbumLike => &mut self.liked_albums,
            Entity::ArtistLike => &mut self.liked_artists,
            Entity::ArtistDislike => &mut self.disliked_artists,
        }
    }

    /// Лайк альбома/исполнителя или «не рекомендовать исполнителя» — переключатель.
    fn toggle_entity(&mut self, what: Entity, id: String, name: String) {
        if self.client.is_none() {
            return self.toast_err("Без сети не сохранить".into());
        }
        if id.is_empty() {
            return self.toast_err("У этого исполнителя нет страницы на Яндекс Музыке".into());
        }
        let on = !self.entity_set(what).contains(&id);
        self.apply_entity(what, &id, on);
        let msg = match (what, on) {
            (Entity::AlbumLike, true) => format!("\u{f004} Альбом в коллекции: {name}"),
            (Entity::AlbumLike, false) => format!("\u{f08a} Альбом убран из коллекции: {name}"),
            (Entity::ArtistLike, true) => format!("\u{f004} Исполнитель в коллекции: {name}"),
            (Entity::ArtistLike, false) => format!("\u{f08a} Исполнитель убран из коллекции: {name}"),
            (Entity::ArtistDislike, true) => format!("⊘ Не рекомендовать: {name} (D — вернуть)"),
            (Entity::ArtistDislike, false) => format!("Снова рекомендовать: {name}"),
        };
        self.toast(msg);
        self.spawn(move |c, tx| async move {
            let r = match what {
                Entity::AlbumLike => c.set_album_like(&id, on).await,
                Entity::ArtistLike => c.set_artist_like(&id, on).await,
                Entity::ArtistDislike => c.set_artist_dislike(&id, on).await,
            };
            if let Err(e) = r {
                let _ = tx.send(Msg::EntityFailed {
                    what,
                    id,
                    on,
                    error: e.to_string(),
                });
            }
        });
    }

    fn apply_entity(&mut self, what: Entity, id: &str, on: bool) {
        if on {
            self.entity_set(what).insert(id.to_string());
            // Лайк и «не рекомендовать» взаимоисключают друг друга, как в официальных клиентах.
            match what {
                Entity::ArtistLike => self.disliked_artists.remove(id),
                Entity::ArtistDislike => self.liked_artists.remove(id),
                Entity::AlbumLike => false,
            };
        } else {
            self.entity_set(what).remove(id);
        }
    }

    /// Альбом или исполнитель под курсором, иначе — страница, на которой стоим.
    fn target_entity(&self) -> Option<(Entity, String, String)> {
        match &self.page().view {
            View::Artist {
                brief, artist_id, name, ..
            } => {
                let sel = self.page().sel;
                match brief.ready().and_then(|b| artist_rows(b).get(sel).map(|r| match r {
                    ArtistRow::Album(a) => Some((Entity::AlbumLike, a.id.clone(), a.title.clone())),
                    ArtistRow::Artist(a) => Some((Entity::ArtistLike, a.id.clone(), a.name.clone())),
                    _ => None,
                })) {
                    Some(Some(t)) => Some(t),
                    _ => Some((Entity::ArtistLike, artist_id.clone(), name.clone())),
                }
            }
            View::Tracks {
                album: Some(a), title, ..
            } => Some((Entity::AlbumLike, a.clone(), title.clone())),
            View::Albums { albums, .. } => albums
                .ready()
                .and_then(|v| v.get(self.page().sel))
                .map(|a| (Entity::AlbumLike, a.id.clone(), a.title.clone())),
            View::Artists { artists, .. } => artists
                .ready()
                .and_then(|v| v.get(self.page().sel))
                .map(|(a, _)| (Entity::ArtistLike, a.id.clone(), a.name.clone())),
            View::Releases => self
                .releases
                .as_ref()
                .and_then(Loadable::ready)
                .and_then(|v| v.get(self.page().sel))
                .map(|a| (Entity::AlbumLike, a.id.clone(), a.title.clone())),
            View::Metatag { page, .. } => match page.ready().and_then(|m| meta_rows(m).into_iter().nth(self.page().sel)) {
                Some(MetaRow::Album(a)) => Some((Entity::AlbumLike, a.id.clone(), a.title.clone())),
                Some(MetaRow::Artist(a)) => Some((Entity::ArtistLike, a.id.clone(), a.name.clone())),
                _ => None,
            },
            _ => None,
        }
    }

    fn open_metatags(&mut self) {
        let id = self.new_view_id();
        self.push(View::Metatags {
            id,
            trees: Loadable::Loading,
        });
        self.spawn(move |c, tx| async move {
            let r = c.metatags().await.map_err(err_str);
            let _ = tx.send(Msg::Metatags { view_id: id, result: r });
        });
    }

    fn open_metatag(&mut self, tag: String, title: String) {
        let id = self.new_view_id();
        self.push(View::Metatag {
            id,
            title,
            page: Loadable::Loading,
        });
        self.spawn(move |c, tx| async move {
            let r = c.metatag(&tag).await.map_err(err_str);
            let _ = tx.send(Msg::Metatag { view_id: id, result: r });
        });
    }

    fn open_liked_albums(&mut self) {
        let id = self.new_view_id();
        self.push(View::Albums {
            id,
            albums: Loadable::Loading,
        });
        self.spawn(move |c, tx| async move {
            let r = c.liked_albums().await.map_err(err_str);
            let _ = tx.send(Msg::LikedAlbums { view_id: id, result: r });
        });
    }

    fn open_liked_artists(&mut self) {
        let id = self.new_view_id();
        self.push(View::Artists {
            id,
            title: "Исполнители в коллекции".into(),
            artists: Loadable::Loading,
        });
        self.spawn(move |c, tx| async move {
            let r = c
                .liked_artists()
                .await
                .map(|v| v.into_iter().map(|a| (a, String::new())).collect())
                .map_err(err_str);
            let _ = tx.send(Msg::ArtistList {
                view_id: id,
                liked: true,
                result: r,
            });
        });
    }

    fn open_top_artists(&mut self) {
        let id = self.new_view_id();
        self.push(View::Artists {
            id,
            title: "Топ исполнителей за месяц".into(),
            artists: Loadable::Loading,
        });
        self.spawn(move |c, tx| async move {
            let r = c
                .top_artists()
                .await
                .map(|v| {
                    v.into_iter()
                        .map(|(a, secs, progress)| {
                            let mark = match progress.as_str() {
                                "up" => " ↑",
                                "down" => " ↓",
                                "new" => " new",
                                _ => "",
                            };
                            let (h, m) = (secs / 3600, secs / 60 % 60);
                            let time = if h > 0 { format!("{h} ч {m} мин") } else { format!("{m} мин") };
                            (a, format!("{time}{mark}"))
                        })
                        .collect()
                })
                .map_err(err_str);
            let _ = tx.send(Msg::ArtistList {
                view_id: id,
                liked: false,
                result: r,
            });
        });
    }

    fn dislike_current(&mut self) {
        let Some(t) = self.now.as_ref().map(|n| n.track.clone()) else {
            return;
        };
        let id = t.full_id();
        self.wave_feedback("dislike", &t, None);
        self.spawn(move |c, _| async move {
            let _ = c.dislike(&id).await;
        });
        self.toast(format!("Не рекомендовать: {}", t.full_title()));
        self.next();
    }

    // ── события ──────────────────────────────────────────────────────────────

    pub fn toast(&mut self, text: String) {
        self.toasts.push(Toast {
            text,
            at: Instant::now(),
            error: false,
        });
    }

    fn toast_err(&mut self, text: String) {
        self.toasts.push(Toast {
            text,
            at: Instant::now(),
            error: true,
        });
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Authed(Ok((c, s, t))) => self.on_authed(c, s, t),
            Msg::Notify {
                token,
                title,
                body,
                icon,
            } => {
                if self.now.as_ref().is_some_and(|n| n.token == token) {
                    // Один id на все треки: kitty заменяет показанное уведомление, а не копит стопку.
                    crate::ui::term::notify(
                        &mut self.osc_out,
                        NOTIFY_ID,
                        &title,
                        &body,
                        icon.as_deref(),
                        &["⏭ Дальше", "♥ Нравится"],
                    );
                }
            }
            Msg::Wheel(r) => match r {
                Ok(items) => {
                    // Курсор — на «Назад к привычному» (обычная Моя волна), как на вебе.
                    let home = items.iter().position(|i| i.seeds() == [MY_WAVE]).unwrap_or(0);
                    for icon in items
                        .iter()
                        .filter_map(|i| i.icon())
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                    {
                        self.fetch_image(agent_key(&icon), &icon, 200, false);
                    }
                    self.wave.items = Loadable::Ready(items);
                    if let Some(p) = self.stack.iter_mut().find(|p| matches!(p.view, View::Wave)) {
                        p.sel = home;
                    }
                    self.wave.arc = home as f32;
                }
                Err(e) => self.wave.items = Loadable::Failed(e),
            },
            Msg::Resumed { resume, result } => {
                if let Some((generation, full_pos)) = self.resume_partial.take() {
                    return self.fill_partial_queue(generation, full_pos, resume, result);
                }
                // Пользователь уже что-то включил сам — не перебиваем.
                if !self.queue.items.is_empty() {
                    return;
                }
                match result {
                    Ok(tracks) if !tracks.is_empty() => {
                        let kind = match resume.wave {
                            Some((session, batch)) => QueueKind::Wave { session, batch },
                            None => QueueKind::List,
                        };
                        let pos = resume.pos.min(tracks.len() - 1);
                        self.replace_queue(tracks, kind, resume.title);
                        self.play_index_at(pos, resume.secs, true);
                        self.toast("Продолжаем с того же места — Space".into());
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("не удалось восстановить сессию: {e}"),
                }
            }
            Msg::Authed(Err(e)) => {
                tracing::warn!("вход сброшен: {e}");
                self.begin_login(Some(e));
            }
            Msg::ConnectError(e) => {
                if let Screen::Connecting(err) = &mut self.screen {
                    *err = Some(e);
                    if !self.snapshot.is_empty() {
                        self.enter_offline();
                    }
                }
            }
            Msg::DeviceCode(r) => {
                let poll = if let Screen::Login(l) = &mut self.screen {
                    match r {
                        Ok(code) => {
                            l.status = "Жду подтверждения…".into();
                            l.code = Some(code.clone());
                            Some(code)
                        }
                        Err(e) => {
                            l.status = "Не удалось получить код".into();
                            l.error = Some(e);
                            None
                        }
                    }
                } else {
                    None
                };
                if let Some(code) = poll {
                    self.poll_device(code);
                }
            }
            Msg::LoggedIn(Ok(tok)) => {
                self.screen = Screen::Connecting(None);
                tokio::spawn(connect(tok, self.tx.clone()));
            }
            Msg::LoggedIn(Err(e)) => {
                if let Screen::Login(l) = &mut self.screen {
                    l.error = Some(e);
                }
            }
            Msg::Likes(r) => match r {
                Ok(ts) => {
                    self.liked = ts.iter().map(|t| t.id.clone()).collect();
                    self.snapshot.likes = ts.clone();
                    self.snapshot.save(self.snapshot_path.clone());
                    self.likes = Loadable::Ready(ts);
                    self.backfill_library_meta();
                    self.refresh_offline_program();
                    self.refilter_keep();
                    self.mpris_liked();
                }
                Err(e) => self.likes = Loadable::Failed(e),
            },
            Msg::Releases(r) => {
                self.releases = Some(match r {
                    Ok(a) => Loadable::Ready(a),
                    Err(e) => Loadable::Failed(e),
                });
            }
            Msg::Ynison(ev) => self.on_ynison(ev),
            Msg::YnisonQueue {
                generation,
                queue,
                status,
                result,
            } => self.on_ynison_queue(generation, queue, status, result),
            Msg::Credits { track_id, result } => match result {
                Ok(v) => {
                    self.credits.insert(track_id, v);
                }
                Err(e) => tracing::warn!("credits {track_id}: {e}"),
            },
            Msg::Found(r) => match r {
                Ok(v) => {
                    self.found = v;
                    // История уже на экране — перерисовать с серверной частью.
                    if self.search.is_none() || self.search_shows_history() {
                        let sel = self.page().sel;
                        self.show_search_history();
                        if matches!(self.page().view, View::Search) && sel > 0 {
                            self.page_mut().sel = sel.min(self.page_len().saturating_sub(1));
                        }
                    }
                }
                Err(e) => tracing::warn!("search-history: {e}"),
            },
            Msg::Personal(r) => match r {
                Ok(v) => {
                    // Курсор остаётся на той же строке, хотя сверху добавились подборки.
                    let shift = v.len() as isize - self.personal.len() as isize;
                    self.personal = v;
                    if let Some(p) = self.stack.iter_mut().find(|p| matches!(p.view, View::Playlists))
                        && p.sel > 0
                    {
                        p.sel = (p.sel as isize + shift).max(0) as usize;
                    }
                }
                Err(e) => tracing::warn!("personal-playlists: {e}"),
            },
            Msg::Playlists(r) => {
                self.playlists = match r {
                    Ok(p) => {
                        self.snapshot.playlists = p.clone();
                        self.sync_snapshot_playlists(&p);
                        Loadable::Ready(p)
                    }
                    Err(e) => Loadable::Failed(e),
                }
            }
            Msg::OfflineProgram(r) => {
                self.owave_fetching = false;
                match r {
                    Ok((program, ids)) => {
                        let store = offline_wave::Store {
                            program,
                            ids,
                            fetched_at: chrono::Utc::now().timestamp(),
                            server_data: self.owave_store.take().map(|s| s.server_data).unwrap_or_default(),
                        };
                        store.save(&self.data_dir);
                        tracing::info!("офлайн-волна: программа {} слов под {} треков", store.program.len(), store.ids.len());
                        self.owave_store = Some(store);
                    }
                    Err(e) => tracing::warn!("программа офлайн-волны: {e}"),
                }
            }
            Msg::LibraryMeta(ts) => {
                self.cover_files = None;
                for t in ts {
                    self.snapshot.library_tracks.insert(t.id.clone(), t);
                }
                self.snapshot.save(self.snapshot_path.clone());
            }
            Msg::PlaylistSnapshots(v) => {
                for (k, rev, ts) in v {
                    self.snapshot.playlist_tracks.insert(k, (rev, ts));
                }
                self.snapshot.save(self.snapshot_path.clone());
            }
            Msg::PlaylistOp {
                result,
                kind,
                reload_list,
            } => {
                match result {
                    Ok(t) if !t.is_empty() => self.toast(t),
                    Ok(_) => {}
                    Err(e) => {
                        self.toast_err(format!("плейлист не изменился: {e}"));
                        if let Some(k) = &kind {
                            self.reload_playlist_views(k);
                        }
                    }
                }
                if reload_list {
                    self.load_playlists();
                }
            }
            Msg::LibraryIds(r) => match r {
                Ok(ids) => {
                    self.liked_albums = api::LibraryIds::with(&ids.albums, 1);
                    self.liked_artists = api::LibraryIds::with(&ids.artists, 1);
                    self.disliked_artists = api::LibraryIds::with(&ids.artists, -1);
                }
                Err(e) => tracing::warn!("library/all-ids: {e}"),
            },
            Msg::LikedAlbums { view_id, result } => {
                for p in &mut self.stack {
                    if let View::Albums { id, albums } = &mut p.view
                        && *id == view_id
                    {
                        if let Ok(v) = &result {
                            self.liked_albums.extend(v.iter().map(|a| a.id.clone()));
                        }
                        *albums = result.clone().into();
                    }
                }
            }
            Msg::ArtistList { view_id, liked, result } => {
                for p in &mut self.stack {
                    if let View::Artists { id, artists, .. } = &mut p.view
                        && *id == view_id
                    {
                        if let (true, Ok(v)) = (liked, &result) {
                            self.liked_artists.extend(v.iter().map(|(a, _)| a.id.clone()));
                        }
                        *artists = result.clone().into();
                    }
                }
            }
            Msg::Metatags { view_id, result } => {
                for p in &mut self.stack {
                    if let View::Metatags { id, trees } = &mut p.view
                        && *id == view_id
                    {
                        *trees = result.clone().into();
                    }
                }
                if matches!(&self.page().view, View::Metatags { id, .. } if *id == view_id) {
                    self.page_mut().sel = 0;
                    self.skip_headers(1);
                }
            }
            Msg::Metatag { view_id, result } => {
                for p in &mut self.stack {
                    if let View::Metatag { id, page, .. } = &mut p.view
                        && *id == view_id
                    {
                        *page = result.clone().into();
                    }
                }
            }
            Msg::EntityFailed { what, id, on, error } => {
                self.apply_entity(what, &id, !on);
                self.toast_err(format!("не сохранилось: {error}"));
            }
            Msg::Tracks { view_id, result } => {
                for p in &mut self.stack {
                    if let View::Tracks { id, tracks, .. } = &mut p.view
                        && *id == view_id
                    {
                        *tracks = match result {
                            Ok(t) => Loadable::Ready(t),
                            Err(e) => Loadable::Failed(e),
                        };
                        break;
                    }
                }
            }
            Msg::Artist { view_id, result } => {
                for p in &mut self.stack {
                    if let View::Artist { id, brief, .. } = &mut p.view
                        && *id == view_id
                    {
                        let photo = result
                            .as_ref()
                            .ok()
                            .and_then(|b| b.artist.as_ref()?.cover_uri().map(str::to_owned));
                        *brief = match result {
                            Ok(b) => Loadable::Ready(b),
                            Err(e) => Loadable::Failed(e),
                        };
                        if let Some(u) = photo {
                            self.fetch_cover(&u);
                        }
                        if matches!(&self.page().view, View::Artist { id, .. } if *id == view_id) {
                            self.page_mut().sel = 0;
                            self.skip_headers(1);
                        }
                        break;
                    }
                }
            }
            Msg::Search { query, result } => {
                if query == self.search_input.trim() {
                    self.search = Some(match result {
                        Ok(r) => Loadable::Ready(search_items(r)),
                        Err(e) => Loadable::Failed(e),
                    });
                    if matches!(self.page().view, View::Search) {
                        self.page_mut().sel = 0;
                        self.skip_headers(1);
                    }
                }
            }
            Msg::WaveStarted(r) => {
                self.wave.loading = false;
                match r {
                    Ok(s) => {
                        self.keep_server_data(s.offline_recommender_data.clone());
                        let tracks: Vec<Track> = s.sequence.into_iter().filter_map(|i| i.track).collect();
                        if tracks.is_empty() {
                            self.toast_err("Волна пустая — попробуй другие настройки".into());
                            return;
                        }
                        self.replace_queue(
                            tracks,
                            QueueKind::Wave {
                                session: s.radio_session_id,
                                batch: s.batch_id,
                            },
                            "Моя волна".into(),
                        );
                        self.play_index(0);
                    }
                    Err(e) => self.toast_err(format!("Волна: {e}")),
                }
            }
            Msg::WaveMore { session, result } => {
                self.wave.extending = false;
                // Пока догружали, включили другое — эти треки уже не нужны.
                if !matches!(&self.queue.kind, QueueKind::Wave { session: s, .. } if *s == session) {
                    return;
                }
                match result {
                    Ok(more) => {
                        self.keep_server_data(more.offline_recommender_data.clone());
                        let have: HashSet<String> = self.queue.items.iter().map(|t| t.id.clone()).collect();
                        if let QueueKind::Wave { batch, .. } = &mut self.queue.kind {
                            *batch = more.batch_id.or(batch.take());
                        }
                        let was_at_end = self.next_loaded.is_none();
                        self.queue.items.extend(
                            more.sequence
                                .into_iter()
                                .filter_map(|i| i.track)
                                .filter(|t| !have.contains(&t.id)),
                        );
                        if was_at_end {
                            self.schedule_next();
                        }
                    }
                    Err(e) => self.toast_err(format!("Волна: {e}")),
                }
            }
            Msg::Opened {
                token,
                next,
                start,
                result,
            } => self.on_opened(token, next, start, result),
            Msg::Cover { key, result } => {
                self.covers_loading.remove(&key);
                if result.is_err() && self.client.is_none() {
                    self.covers_failed.insert(key.clone());
                }
                match result {
                    Ok((img, pal)) => {
                        self.images.insert(key.clone(), img);
                        self.palettes.insert(key.clone(), pal);
                        let now_cover = self.now.as_ref().and_then(|n| n.track.cover_uri()).map(str::to_owned);
                        let loading_cover = self
                            .loading_track
                            .as_ref()
                            .and_then(|t| t.cover_uri())
                            .map(str::to_owned);
                        if now_cover.as_deref() == Some(&key)
                            || (self.now.is_none() && loading_cover.as_deref() == Some(&key))
                        {
                            self.theme.set(pal);
                        }
                    }
                    Err(e) => tracing::debug!("обложка {key}: {e}"),
                }
            }
            Msg::Lyrics { track_id, lyrics, raw } => {
                if self.now.as_ref().is_some_and(|n| n.track.id == track_id)
                    && let Some(l) = lyrics
                {
                    self.lyrics = Some((track_id, l));
                    if let Some(r) = raw {
                        let _ = self.mpris.send(mpris::Update::Lyrics(r));
                    }
                }
            }
            Msg::Audio(ev) => self.on_audio(ev),
            Msg::Upgraded { token, auto, result } => self.on_upgraded(token, auto, result),
            Msg::Mpris(c) => match c {
                mpris::Command::PlayPause => self.toggle_pause(),
                mpris::Command::Play => {
                    if self.player.paused() {
                        self.toggle_pause()
                    }
                }
                mpris::Command::Pause => {
                    if !self.player.paused() {
                        self.toggle_pause()
                    }
                }
                mpris::Command::Next => self.next(),
                mpris::Command::Previous => self.prev(),
                mpris::Command::Stop => {
                    self.player.stop();
                    self.now = None;
                }
                mpris::Command::Seek(d) => self.seek_by(d),
                mpris::Command::SetPosition(p) => self.seek_to(p),
                mpris::Command::Volume(v) => self.set_volume(v as f32),
                mpris::Command::Like => {
                    if let Some(t) = self.now.as_ref().map(|n| n.track.clone()) {
                        self.toggle_like(t);
                    }
                }
                mpris::Command::Dislike => self.dislike_current(),
            },
            Msg::LikeFailed { track, like, error } => {
                self.apply_like(&track, !like);
                self.toast_err(format!("Лайк не сохранился: {error}"));
            }
        }
    }

    fn on_opened(&mut self, token: Token, next: bool, start: f64, result: Result<(audio::Source, Origin), String>) {
        let track = self.tokens.get(&token).map(|t| t.2.clone());
        match result {
            Ok((src, origin)) => {
                let gain = track.as_ref().map(|t| self.gain_for(t)).unwrap_or(1.0);
                if next {
                    if self.next_loaded == Some(token) {
                        self.origins_insert(token, origin);
                        // Треки одного альбома подряд (концерт, сюита) сводить нельзя — только встык.
                        let album = |t: &Track| t.full_id().split_once(':').map(|(_, a)| a.to_string());
                        let cur = self.now.as_ref().and_then(|n| album(&n.track));
                        let crossfade = !self.repeat_one
                            && track.as_ref().map(album).is_none_or(|a| a.is_none() || a != cur);
                        self.player.enqueue_next(token, src, gain, crossfade);
                    } else {
                        src.cancel();
                    }
                } else if self.requested == Some(token) {
                    self.origins_insert(token, origin);
                    // Пауза ставится до загрузки, чтобы при восстановлении не проскочил звук.
                    let paused = self.start_paused.take() == Some(token);
                    if paused != self.player.paused() {
                        self.player.set_paused(paused);
                    }
                    self.player.load(token, src, gain, start);
                    // Перезагрузка играющего трека: on_track_started не сработает — источник
                    // (библиотека/сеть) и следующий трек ставим сами.
                    if self.now.as_ref().is_some_and(|n| n.token == token) {
                        let origin = self.pending_origins.remove(&token);
                        if let Some(n) = self.now.as_mut() {
                            n.origin = origin;
                        }
                        self.schedule_next();
                    }
                } else {
                    src.cancel();
                }
            }
            Err(e) => {
                let name = track.map(|t| t.full_title()).unwrap_or_default();
                tracing::warn!("{name}: {e}");
                if self.requested == Some(token) {
                    self.toast_err(format!("{name}: {e}"));
                    self.skip_broken(token);
                } else if self.next_loaded == Some(token) {
                    self.toast_err(format!("{name}: {e}"));
                    self.next_failed(token);
                }
            }
        }
    }

    /// Заново открывает играющий трек и запускает его с позиции `secs`.
    fn reload(&mut self, token: Token, secs: f64) {
        if self.now.as_ref().is_none_or(|n| n.token != token) {
            return;
        }
        let Some(track) = self.tokens.get(&token).map(|t| t.2.clone()) else {
            return;
        };
        let Some(fetcher) = self.opener() else {
            return;
        };
        self.requested = Some(token);
        self.next_loaded = None;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let r = fetcher.open(&track).await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Opened {
                token,
                next: false,
                start: secs,
                result: r,
            });
        });
    }

    /// Трек ещё нужен: его ждём, он играет или стоит следующим.
    fn token_relevant(&self, token: Token) -> bool {
        self.requested == Some(token)
            || self.next_loaded == Some(token)
            || self.now.as_ref().is_some_and(|n| n.token == token)
    }

    fn origins_insert(&mut self, token: Token, origin: Origin) {
        self.pending_origins.insert(token, origin);
    }

    fn skip_broken(&mut self, token: Token) {
        self.mark_failed(token);
        self.error_streak += 1;
        if self.error_streak > 5 {
            self.toast_err("Слишком много ошибок подряд, остановился".into());
            self.loading_track = None;
            return;
        }
        if let Some((generation, idx, _)) = self.tokens.get(&token).cloned()
            && generation == self.queue.generation
            && let Some(n) = self.next_index(idx).filter(|n| *n != idx)
        {
            self.play_index(n);
        }
    }

    fn on_audio(&mut self, ev: audio::Event) {
        match ev {
            audio::Event::NeedNext => {
                if self.next_loaded.is_none() {
                    self.schedule_next();
                }
            }
            audio::Event::Drained => {
                // Следующий трек ещё грузится — декодер подхватит его сам.
                if self.next_loaded.is_some() {
                    return;
                }
                let next = self.queue.pos.and_then(|p| self.next_index(p));
                match next {
                    Some(i) => self.play_index(i),
                    // Волна догружается: ждём WaveMore, он сам поставит следующий.
                    None if self.wave.extending => {}
                    None => {
                        if let Some(now) = self.now.take() {
                            self.report_end(now.token, now.pos, false);
                        }
                        self.player.stop();
                        let _ = self.mpris.send(mpris::Update::Status {
                            playing: false,
                            pos: 0.0,
                        });
                        self.yn_publish_status();
                    }
                }
            }
            audio::Event::SeekMiss(token, secs) => self.reload(token, secs),
            audio::Event::Swapped(token, result) => self.on_swapped(token, result),
            audio::Event::Error(token, e) => {
                let name = self.tokens.get(&token).map(|t| t.2.full_title()).unwrap_or_default();
                tracing::warn!("{name}: {e}");
                // Трек, с которого уже ушли, — молча.
                if !self.token_relevant(token) {
                    return;
                }
                self.toast_err(format!("{name}: {e}"));
                if self.requested == Some(token) && self.now.as_ref().is_none_or(|n| n.token != token) {
                    self.skip_broken(token);
                } else if self.next_loaded == Some(token) {
                    self.next_failed(token);
                }
            }
        }
    }

    /// Вызывается каждый кадр: синхронизирует «что играет» с аудиодвижком.
    pub fn tick(&mut self) {
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(4));
        // Дуга волны догоняет курсор.
        if let Some(p) = self.stack.iter().find(|p| matches!(p.view, View::Wave)) {
            let target = p.sel as f32;
            self.wave.arc += (target - self.wave.arc) * 0.22;
            if (target - self.wave.arc).abs() < 0.01 {
                self.wave.arc = target;
            }
        }
        let Some(pos) = self.player.position() else {
            return;
        };
        match &mut self.now {
            Some(n) if n.token == pos.token => {
                n.pos = pos.secs;
                let changed = n.info.codec != pos.info.codec;
                n.info = pos.info;
                if changed && let Some((_, text)) = self.swap_toast.take_if(|(t, _)| *t == pos.token) {
                    self.toast(text);
                }
            }
            _ => self.on_track_started(pos),
        }
        // Сторож вывода: звук есть, а PipeWire его не берёт — сообщаем и пробуем переподключиться.
        if self.player.stalled(Duration::from_secs(2)) {
            if self.last_stall.is_none_or(|t| t.elapsed() > self.stall_backoff) {
                // Сообщаем один раз за эпизод, дальше пробуем тише и реже.
                if self.last_stall.is_none() {
                    self.toast_err("Устройство вывода не отвечает — переподключаюсь".into());
                }
                tracing::warn!("PipeWire не забирает звук: переподключаюсь (следующая попытка через {:?})", self.stall_backoff);
                if self.last_stall.is_some() {
                    self.stall_backoff = (self.stall_backoff * 2).min(STALL_MAX);
                }
                self.last_stall = Some(Instant::now());
                self.player.reconnect_output();
            }
        } else if self.last_stall.is_some() && !self.player.stalled(Duration::from_millis(500)) {
            // Звук снова идёт (или пауза) — эпизод закончен.
            self.last_stall = None;
            self.stall_backoff = STALL_FIRST;
        }
        if self.last_session_save.elapsed() > Duration::from_secs(15) && self.now.is_some() {
            self.persist_session();
        }
        if self.last_mpris_pos.elapsed() > Duration::from_millis(500)
            && let Some(n) = &self.now
        {
            let _ = self.mpris.send(mpris::Update::Position(n.pos));
            self.last_mpris_pos = Instant::now();
        }
        self.yn_tick();
        // Плавная прокрутка текста к текущей строке.
        if let (Some((_, Lyrics::Synced(lines))), Some(n)) = (&self.lyrics, &self.now) {
            let cur = crate::lyrics::current(lines, n.pos + 0.25).map_or(-1.0, |i| i as f32);
            // Ручная прокрутка держится, пока её трогают, потом текст сам возвращается.
            if self.lyrics_user_at.is_some_and(|t| t.elapsed() > LYRICS_RETURN) {
                self.lyrics_user_at = None;
                self.lyrics_user_offset = 0.0;
            }
            let max = lines.len() as f32 - 1.0;
            let target = (cur + self.lyrics_user_offset).clamp(-1.0, max.max(-1.0));
            self.lyrics_moving = (target - self.lyrics_scroll).abs() >= 0.01 || self.lyrics_user_at.is_some();
            self.lyrics_scroll += (target - self.lyrics_scroll) * 0.18;
            if (target - self.lyrics_scroll).abs() < 0.01 {
                self.lyrics_scroll = target;
            }
        }
    }

    fn on_track_started(&mut self, pos: audio::Position) {
        let Some((generation, idx, track)) = self.tokens.get(&pos.token).cloned() else {
            return;
        };
        if let Some(prev) = self.now.take()
            && !self.user_switch
        {
            self.report_end(prev.token, prev.pos, false);
        }
        self.user_switch = false;
        self.error_streak = 0;
        if self.next_loaded == Some(pos.token) {
            self.next_loaded = None;
        }
        if self.requested == Some(pos.token) || self.loading_track.as_ref().is_some_and(|t| t.id == track.id) {
            self.loading_track = None;
        }
        if generation == self.queue.generation {
            self.queue.pos = Some(idx);
        }
        let origin = self.pending_origins.remove(&pos.token);
        self.now = Some(Now {
            token: pos.token,
            track: track.clone(),
            origin,
            info: pos.info,
            pos: pos.secs,
        });
        // Старые токены больше не нужны.
        let keep = pos.token;
        self.tokens.retain(|t, _| *t >= keep || Some(*t) == self.requested);
        // Буфер предзагруженного, но так и не сыгранного трека держит весь файл в памяти.
        self.pending_origins.retain(|t, _| self.tokens.contains_key(t));

        if let Some(c) = track.cover_uri() {
            let c = c.to_string();
            self.fetch_cover(&c);
        }
        self.apply_theme_for(&track);
        self.fetch_lyrics(&track);
        self.wave_feedback("trackStarted", &track, None);
        if self.library.contains(&track.id) {
            self.snapshot.library_tracks.insert(track.id.clone(), track.clone());
        }
        self.extend_offline_wave();
        self.notify_track(pos.token, &track);
        self.persist_session();
        let _ = self.mpris.send(mpris::Update::Track {
            id: track.full_id(),
            title: track.full_title(),
            artists: track.artists.iter().map(|a| a.name.clone()).collect(),
            album: track.album().map(|a| a.full_title()).unwrap_or_default(),
            length: track.duration_secs(),
            art: track.cover_uri().map(|u| api::cover_url(u, 400)),
            pos: pos.secs,
            liked: self.is_liked(&track),
        });
        let _ = self.mpris.send(mpris::Update::Status {
            playing: !self.player.paused(),
            pos: pos.secs,
        });
        self.yn_track_started();
        self.fx_track = Some(Instant::now());
        self.schedule_next();
        // Скачанный хуже lossless: играет сразу, а FLAC подменит его на ходу, если он есть.
        if matches!(self.now.as_ref().and_then(|n| n.origin.as_ref()), Some(Origin::Library))
            && self.fetcher.as_ref().is_some_and(|f| f.should_upgrade(&track))
        {
            self.hot_upgrade(true);
        }
    }

    // ── клавиатура ───────────────────────────────────────────────────────────

    // ── всплывающие окна ─────────────────────────────────────────────────────

    fn show_track_info(&mut self) {
        let Some(track) = self.now.as_ref().map(|n| n.track.clone()) else {
            return;
        };
        let file = self.library.lookup(&track.id);
        if !self.credits.contains_key(&track.id) {
            let id = track.id.clone();
            self.spawn(move |c, tx| async move {
                let result = c.credits(&id).await.map_err(err_str);
                let _ = tx.send(Msg::Credits { track_id: id, result });
            });
        }
        self.popup = Some(Popup::TrackInfo { track, file });
    }

    /// Один исполнитель — сразу к нему, несколько — сперва выбор.
    fn pick_artist(&mut self, t: &Track) {
        let list: Vec<api::Artist> = t.artists.iter().filter(|a| !a.id.is_empty()).cloned().collect();
        match list.len() {
            0 => self.toast_err("У исполнителя нет страницы".into()),
            1 => {
                let a = list.into_iter().next().expect("один");
                self.go_artist(a);
            }
            _ => self.popup = Some(Popup::Artists { list, sel: 0 }),
        }
    }

    /// Открыть страницу из сцены/текста: сначала вернуться к обычному виду.
    fn leave_fullscreen(&mut self) {
        self.popup = None;
        if self.stage_mode {
            self.toggle_stage();
        }
        self.lyrics_mode = false;
    }

    fn go_artist(&mut self, a: api::Artist) {
        self.leave_fullscreen();
        self.open_artist(a);
    }

    fn go_album(&mut self, a: api::Album) {
        self.leave_fullscreen();
        self.open_album(a);
    }

    fn popup_move(&mut self, d: isize) {
        match &mut self.popup {
            Some(Popup::Artists { list, sel }) => {
                *sel = (*sel as isize + d).clamp(0, list.len() as isize - 1) as usize;
            }
            // Последняя строка — «новый плейлист».
            Some(Popup::AddToPlaylist { list, sel, .. }) => {
                *sel = (*sel as isize + d).clamp(0, list.len() as isize) as usize;
            }
            _ => {}
        }
    }

    /// Строка окна `i`: в выборе — исполнитель; в метаданных 0 — исполнители, 1 — альбом, 2 — ссылка.
    fn popup_activate(&mut self, i: usize) {
        match self.popup.take() {
            Some(Popup::Artists { list, .. }) => {
                if let Some(a) = list.into_iter().nth(i) {
                    self.go_artist(a);
                }
            }
            Some(Popup::TrackInfo { track, file }) => match i {
                0 => self.pick_artist(&track),
                1 => {
                    if let Some(a) = track.albums.first().cloned() {
                        self.go_album(a);
                    }
                }
                2 => {
                    crate::ui::term::copy_to_clipboard(&mut self.osc_out, &Self::track_url(&track));
                    self.toast("Ссылка скопирована".into());
                    self.popup = Some(Popup::TrackInfo { track, file });
                }
                _ => self.popup = Some(Popup::TrackInfo { track, file }),
            },
            Some(Popup::AddToPlaylist { track, list, .. }) => match list.into_iter().nth(i) {
                Some((kind, title)) => self.pl_op(PlOp::Append {
                    kind,
                    title,
                    track: track.full_id(),
                }),
                None => {
                    self.popup = Some(Popup::Input {
                        title: " Новый плейлист ",
                        text: String::new(),
                        action: InputAction::Create(Some(track)),
                    })
                }
            },
            Some(Popup::Confirm { action, .. }) => self.confirm(action),
            Some(p @ Popup::Input { .. }) => self.popup = Some(p),
            None => {}
        }
    }

    fn on_popup_key(&mut self, raw: KeyEvent) {
        // Ввод названия — сырые клавиши (кириллица), остальное — по латинской раскладке.
        if let Some(Popup::Input { text, .. }) = &mut self.popup {
            match raw.code {
                KeyCode::Esc => self.popup = None,
                KeyCode::Enter => {
                    if let Some(Popup::Input { text, action, .. }) = self.popup.take() {
                        self.input_submit(text, action);
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) if !raw.modifiers.contains(KeyModifiers::CONTROL) => text.push(c),
                _ => {}
            }
            return;
        }
        let k = latin_key(raw);
        if matches!(self.popup, Some(Popup::Confirm { .. })) {
            match k.code {
                KeyCode::Enter | KeyCode::Char('y') => self.popup_activate(0),
                _ => self.popup = None,
            }
            return;
        }
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') => self.popup = None,
            KeyCode::Up | KeyCode::Char('k') => self.popup_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.popup_move(1),
            KeyCode::Enter => match &self.popup {
                Some(Popup::Artists { sel, .. }) | Some(Popup::AddToPlaylist { sel, .. }) => self.popup_activate(*sel),
                _ => self.popup = None,
            },
            KeyCode::Char('i') if matches!(self.popup, Some(Popup::TrackInfo { .. })) => self.popup_activate(0),
            KeyCode::Char('o') if matches!(self.popup, Some(Popup::TrackInfo { .. })) => self.popup_activate(1),
            KeyCode::Char('y') if matches!(self.popup, Some(Popup::TrackInfo { .. })) => self.popup_activate(2),
            _ => {}
        }
    }

    fn toggle_stage(&mut self) {
        self.stage_mode = !self.stage_mode;
        self.fx_view = Some(Instant::now());
    }

    /// Курсор-рука над всем, что кликается.
    fn update_pointer(&mut self, m: MouseEvent) {
        if !self.kitty_text {
            return;
        }
        let inside = |r: ratatui::layout::Rect| {
            m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height
        };
        let on_tab =
            m.row == self.areas.header.y && self.areas.tabs.iter().any(|(a, b)| m.column >= *a && m.column < *b);
        let on_lyric = inside(self.areas.lyrics) && self.lyrics_hits.iter().any(|(y, _)| *y == m.row);
        let stage = self.stage_mode && self.now.is_some();
        let on_wave =
            (!stage && matches!(self.page().view, View::Wave) && self.wave.hits.iter().any(|(r, _)| inside(*r)))
                || self.controls.iter().any(|(r, _)| inside(*r));
        let on_stage_text = stage
            && (inside(self.areas.stage_title) || inside(self.areas.stage_artist) || inside(self.areas.stage_album));
        let on_popup = self.popup.is_some() && self.popup_hits.iter().any(|(r, _)| inside(*r));
        if self.popup.is_some() {
            if on_popup != self.pointer {
                self.pointer = on_popup;
                crate::ui::term::pointer_shape(&mut self.osc_out, on_popup);
            }
            return;
        }
        let pointer = self.hover.is_some()
            || on_stage_text
            || on_tab
            || on_lyric
            || on_wave
            || inside(self.areas.progress)
            || inside(self.areas.cover);
        if pointer != self.pointer {
            self.pointer = pointer;
            crate::ui::term::pointer_shape(&mut self.osc_out, pointer);
        }
    }

    /// Уведомление о новом треке (kitty покажет его, только если окно не в фокусе).
    fn notify_track(&self, token: Token, t: &Track) {
        if !self.kitty_text || self.player.paused() {
            return;
        }
        let title = t.full_title();
        let body = match t.album() {
            Some(a) => format!("{} · {}", t.artist_names(), a.full_title()),
            None => t.artist_names(),
        };
        let cover = t.cover_uri().map(|u| api::cover_url(u, 200));
        self.spawn(move |c, tx| async move {
            let icon = match cover {
                Some(u) => crate::fetch::fetch_bytes(&c, &u).await.ok(),
                None => None,
            };
            let _ = tx.send(Msg::Notify {
                token,
                title,
                body,
                icon,
            });
        });
    }

    /// Ссылка на трек в Яндекс Музыке.
    pub fn track_url(t: &Track) -> String {
        match t.album() {
            Some(a) if !a.id.is_empty() => format!("https://music.yandex.ru/album/{}/track/{}", a.id, t.id),
            _ => format!("https://music.yandex.ru/track/{}", t.id),
        }
    }

    /// Ответ на кнопку уведомления: 1 — следующий, 2 — лайк.
    pub fn on_notify_reply(&mut self, r: crate::ui::term::NotifyReply) {
        // Уведомление одно и всегда про текущий трек.
        if r.id != NOTIFY_ID || self.now.is_none() {
            return;
        }
        match r.button {
            Some(1) => self.next(),
            Some(2) => {
                if let Some(t) = self.now.as_ref().map(|n| n.track.clone())
                    && !self.is_liked(&t)
                {
                    self.toggle_like(t);
                }
            }
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if !matches!(self.screen, Screen::Main) || self.show_help {
            return;
        }
        let inside = |r: ratatui::layout::Rect| {
            m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height
        };
        if self.popup.is_some() {
            self.update_pointer(m);
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(i) = self.popup_hits.iter().find(|(r, _)| inside(*r)).map(|h| h.1) {
                        self.popup_activate(i);
                    } else if !inside(self.popup_area) {
                        self.popup = None;
                    }
                }
                MouseEventKind::Down(MouseButton::Right) => self.popup = None,
                MouseEventKind::ScrollUp => self.popup_move(-1),
                MouseEventKind::ScrollDown => self.popup_move(1),
                MouseEventKind::Moved => {
                    if let (Some(i), Some(Popup::Artists { sel, .. })) = (
                        self.popup_hits.iter().find(|(r, _)| inside(*r)).map(|h| h.1),
                        self.popup.as_mut(),
                    ) {
                        *sel = i;
                    }
                }
                _ => {}
            }
            return;
        }
        // Экран волны: клик по пресету — выбрать (повторный — включить), кнопки ⏮ ▶ ⏭, колесо — по дуге.
        let stage = self.stage_mode && self.now.is_some();
        // В сцене экран волны не виден — его пресеты и кнопки не должны ловить клики.
        if !stage && matches!(self.page().view, View::Wave) && inside(self.areas.main) {
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(i) = self.wave.hits.iter().find(|(r, _)| inside(*r)).map(|h| h.1) {
                        if self.page().sel == i {
                            self.activate();
                        } else {
                            self.page_mut().sel = i;
                        }
                    }
                    return;
                }
                MouseEventKind::ScrollUp => return self.move_sel(-1),
                MouseEventKind::ScrollDown => return self.move_sel(1),
                _ => {}
            }
        }
        // Сетка релизов: клик выбирает карточку, повторный — открывает; колесо — по рядам.
        if !stage && !self.lyrics_mode && matches!(self.page().view, View::Releases) && inside(self.areas.main) {
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(i) = self.grid_hits.iter().find(|(r, _)| inside(*r)).map(|h| h.1) {
                        if self.page().sel == i {
                            self.activate();
                        } else {
                            self.page_mut().sel = i;
                        }
                    }
                    return;
                }
                // Тачпад шлёт много мелких событий: каждое — пара строк, ряд — когда наберётся высота карточки.
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    let d = if m.kind == MouseEventKind::ScrollUp { -1.0 } else { 1.0 };
                    if self.grid_wheel.signum() != d && self.grid_wheel != 0.0 {
                        self.grid_wheel = 0.0;
                    }
                    self.grid_wheel += d * GRID_WHEEL_LINES / self.grid_card_h.max(1) as f32;
                    while self.grid_wheel.abs() >= 1.0 {
                        let step = self.grid_wheel.signum();
                        self.grid_wheel -= step;
                        self.move_sel(step as isize * self.grid_cols as isize);
                    }
                    return;
                }
                _ => {}
            }
        }
        self.mouse = (m.column, m.row);
        if m.kind == MouseEventKind::Moved {
            self.hover = inside(self.areas.list)
                .then(|| self.page().view_offset + (m.row - self.areas.list.y) as usize)
                .filter(|r| *r < self.page_len());
        }
        self.update_pointer(m);
        match m.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                if inside(self.areas.lyrics) && matches!(self.lyrics, Some((_, Lyrics::Plain(_)))) {
                    let max = self.plain_max.get();
                    self.plain_scroll = if up {
                        self.plain_scroll.min(max).saturating_sub(2)
                    } else {
                        (self.plain_scroll + 2).min(max)
                    };
                } else if inside(self.areas.lyrics) {
                    self.lyrics_user_offset += if up { -1.0 } else { 1.0 };
                    self.lyrics_user_at = Some(Instant::now());
                } else if inside(self.areas.player) || stage {
                    self.set_volume(self.player.volume() + if up { 0.05 } else { -0.05 });
                } else if inside(self.areas.main) {
                    self.move_sel(if up { -3 } else { 3 });
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(a) = self.controls.iter().find(|(r, _)| inside(*r)).map(|c| c.1) {
                    match a {
                        0 => self.prev(),
                        1 => self.toggle_pause(),
                        _ => self.next(),
                    }
                } else if stage && inside(self.areas.stage_title) {
                    self.show_track_info();
                } else if stage && inside(self.areas.stage_artist) {
                    if let Some(t) = self.now.as_ref().map(|n| n.track.clone()) {
                        self.pick_artist(&t);
                    }
                } else if stage && inside(self.areas.stage_album) {
                    if let Some(a) = self.now.as_ref().and_then(|n| n.track.albums.first().cloned()) {
                        self.go_album(a);
                    }
                } else if inside(self.areas.cover) {
                    // Клик по обложке — «Сцена» и обратно.
                    self.toggle_stage();
                } else if inside(self.areas.lyrics) {
                    // Клик по строке текста — перемотка на неё.
                    let line = self.lyrics_hits.iter().find(|(y, _)| *y == m.row).map(|(_, i)| *i);
                    let at = match (&self.lyrics, line) {
                        (Some((_, Lyrics::Synced(lines))), Some(i)) => lines.get(i).map(|l| l.at),
                        _ => None,
                    };
                    if let Some(at) = at {
                        self.lyrics_user_offset = 0.0;
                        self.lyrics_user_at = None;
                        self.seek_to(at);
                    }
                } else if m.row == self.areas.header.y {
                    if let Some(i) = self
                        .areas
                        .tabs
                        .iter()
                        .position(|(a, b)| m.column >= *a && m.column < *b)
                    {
                        self.set_tab(Tab::ALL[i]);
                    }
                } else if inside(self.areas.progress) {
                    if let Some(dur) = self.now.as_ref().map(|n| n.track.duration_secs()) {
                        let frac = (m.column - self.areas.progress.x) as f64 / self.areas.progress.width.max(1) as f64;
                        self.seek_to(frac * dur);
                    }
                } else if inside(self.areas.list) {
                    let row = self.page().view_offset + (m.row - self.areas.list.y) as usize;
                    if row >= self.page_len() {
                        return;
                    }
                    self.page_mut().sel = row;
                    if matches!(self.page().view, View::Search) {
                        self.search_editing = false;
                    }
                    let double = self
                        .last_click
                        .is_some_and(|(t, r)| r == row && t.elapsed() < Duration::from_millis(400));
                    if double {
                        self.last_click = None;
                        self.activate();
                    } else {
                        self.last_click = Some((Instant::now(), row));
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => self.pop(),
            _ => {}
        }
    }

    pub fn on_paste(&mut self, s: String) {
        match &mut self.screen {
            Screen::Login(l) => l.input.push_str(s.trim()),
            Screen::Main if self.search_editing => self.search_input.push_str(&s.replace('\n', " ")),
            _ => {}
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(latin_key(k).code, KeyCode::Char('c') | KeyCode::Char('q')) {
            self.quit = true;
            return;
        }
        match &mut self.screen {
            Screen::Connecting(_) => match latin_key(k).code {
                KeyCode::Char('q') => self.quit = true,
                // Не ждать сеть, а войти заново (старый токен останется, пока не войдёшь).
                KeyCode::Char('l') => self.begin_login(None),
                _ => {}
            },
            Screen::Login(l) => match k.code {
                KeyCode::Esc => self.quit = true,
                KeyCode::Enter => self.submit_pasted_token(),
                KeyCode::Backspace => {
                    l.input.pop();
                }
                KeyCode::Char('r') if l.input.is_empty() => self.begin_login(None),
                KeyCode::Char(c) => l.input.push(c),
                _ => {}
            },
            Screen::Main => self.on_main_key(k),
        }
    }

    fn on_main_key(&mut self, k: KeyEvent) {
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.popup.is_some() {
            self.on_popup_key(k);
            return;
        }
        if self.page().filter_editing {
            match k.code {
                KeyCode::Esc => {
                    let p = self.page_mut();
                    p.filter.clear();
                    p.filter_editing = false;
                    self.refilter();
                }
                KeyCode::Enter => self.page_mut().filter_editing = false,
                KeyCode::Backspace => {
                    self.page_mut().filter.pop();
                    self.refilter();
                }
                KeyCode::Up => self.move_sel(-1),
                KeyCode::Down => self.move_sel(1),
                KeyCode::Char(c) => {
                    self.page_mut().filter.push(c);
                    self.refilter();
                }
                _ => {}
            }
            return;
        }
        if self.search_editing {
            match k.code {
                KeyCode::Esc => self.search_editing = false,
                KeyCode::Enter => {
                    self.search_editing = false;
                    self.run_search();
                }
                KeyCode::Backspace => {
                    self.search_input.pop();
                    if self.search_input.is_empty() {
                        self.show_search_history();
                    }
                }
                KeyCode::Char(c) if k.modifiers.contains(KeyModifiers::CONTROL) && qwerty(c) == 'u' => {
                    self.search_input.clear();
                    self.show_search_history();
                }
                KeyCode::Char(c) => self.search_input.push(c),
                KeyCode::Down => self.search_editing = false,
                _ => {}
            }
            return;
        }
        let k = latin_key(k);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let page_rows = (self.areas.main.height as isize - 4).max(1);
        // В сцене списка не видно: стрелки и Enter управляют плеером, а не скрытой страницей.
        if self.stage_mode && self.now.is_some() {
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => return self.set_volume(self.player.volume() + 0.05),
                KeyCode::Down | KeyCode::Char('j') => return self.set_volume(self.player.volume() - 0.05),
                KeyCode::Enter => return self.toggle_pause(),
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => return,
                KeyCode::Char('g' | 'G' | 'a' | 'A' | 'L' | 'f') => return,
                _ => {}
            }
        }
        // Настройки: ←→ меняют значение (перемотка — с Shift).
        if matches!(self.page().view, View::Settings) && !self.lyrics_mode && !(self.stage_mode && self.now.is_some()) {
            let sel = self.page().sel;
            match k.code {
                KeyCode::Left if !shift => return self.change_setting(sel, -1),
                KeyCode::Right if !shift => return self.change_setting(sel, 1),
                _ => {}
            }
        }
        // Сетка релизов: ↑↓ — по рядам, ←→ — по карточкам (перемотка — с Shift).
        if matches!(self.page().view, View::Releases) && !self.lyrics_mode && !(self.stage_mode && self.now.is_some()) {
            let cols = self.grid_cols.max(1) as isize;
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => return self.move_sel(-cols),
                KeyCode::Down | KeyCode::Char('j') => return self.move_sel(cols),
                KeyCode::Left if !shift => return self.move_sel(-1),
                KeyCode::Right if !shift => return self.move_sel(1),
                KeyCode::PageUp => return self.move_sel(-cols * 2),
                KeyCode::PageDown => return self.move_sel(cols * 2),
                _ => {}
            }
        }
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('1') => self.set_tab(Tab::Wave),
            KeyCode::Char('2') => self.set_tab(Tab::Likes),
            KeyCode::Char('3') => self.set_tab(Tab::Playlists),
            KeyCode::Char('4') => self.set_tab(Tab::Search),
            KeyCode::Char('5') => self.set_tab(Tab::Queue),
            KeyCode::Char('6') => self.set_tab(Tab::Releases),
            KeyCode::Char('7') => self.set_tab(Tab::Settings),
            KeyCode::Tab | KeyCode::BackTab => {
                let i = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
                let n = Tab::ALL.len();
                let j = if k.code == KeyCode::BackTab {
                    (i + n - 1) % n
                } else {
                    (i + 1) % n
                };
                self.set_tab(Tab::ALL[j]);
            }
            KeyCode::Char('/') => {
                self.set_tab(Tab::Search);
                self.search_editing = true;
            }
            KeyCode::Char('f') => {
                if self.page_tracks().is_some() {
                    self.page_mut().filter_editing = true;
                } else {
                    self.set_tab(Tab::Search);
                    self.search_editing = true;
                }
            }
            KeyCode::Esc | KeyCode::Backspace => {
                if self.stage_mode {
                    self.toggle_stage();
                } else if self.lyrics_mode {
                    self.lyrics_mode = false;
                } else if !self.page().filter.is_empty() {
                    self.page_mut().filter.clear();
                    self.refilter();
                } else {
                    self.pop();
                }
            }
            KeyCode::Delete if matches!(self.page().view, View::Search) => {
                let q = match self
                    .search
                    .as_ref()
                    .and_then(Loadable::ready)
                    .and_then(|v| v.get(self.page().sel))
                {
                    Some(SearchItem::Query(q)) => Some(q.clone()),
                    _ => None,
                };
                if let Some(q) = q {
                    self.forget_search(&q);
                }
            }
            KeyCode::Up if k.modifiers.contains(KeyModifiers::SHIFT) => self.move_track(-1),
            KeyCode::Down if k.modifiers.contains(KeyModifiers::SHIFT) => self.move_track(1),
            KeyCode::Up | KeyCode::Char('k') => {
                if matches!(self.page().view, View::Search) && self.page().sel <= 1 {
                    self.search_editing = true;
                } else {
                    self.move_sel(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::PageUp => self.move_sel(-page_rows),
            KeyCode::PageDown => self.move_sel(page_rows),
            KeyCode::Home | KeyCode::Char('g') => self.move_sel(-(1 << 20)),
            KeyCode::End | KeyCode::Char('G') => self.move_sel(1 << 20),
            KeyCode::Left => self.seek_by(if shift { -30.0 } else { -5.0 }),
            KeyCode::Right => self.seek_by(if shift { 30.0 } else { 5.0 }),
            KeyCode::Enter => self.activate(),
            KeyCode::Char(' ') => self.toggle_pause(),
            KeyCode::Char('n') => self.next(),
            KeyCode::Char('p') => self.prev(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.set_volume(self.player.volume() + 0.05),
            KeyCode::Char('-') | KeyCode::Char('_') => self.set_volume(self.player.volume() - 0.05),
            KeyCode::Char('l') => {
                if let Some(t) = self.now.as_ref().map(|n| n.track.clone()) {
                    self.toggle_like(t);
                }
            }
            KeyCode::Char('L') => {
                if let Some(t) = self.target_track() {
                    self.toggle_like(t);
                }
            }
            KeyCode::Char('d') => self.dislike_current(),
            KeyCode::Char('a') => {
                if let Some(t) = self.selected_track().cloned() {
                    self.enqueue(t, false);
                }
            }
            KeyCode::Char('A') => {
                if let Some(t) = self.selected_track().cloned() {
                    self.enqueue(t, true);
                }
            }
            KeyCode::Char('o') => {
                if let Some(a) = self.target_track().and_then(|t| t.albums.first().cloned()) {
                    self.go_album(a);
                }
            }
            KeyCode::Char('i') => {
                if let Some(t) = self.target_track() {
                    self.pick_artist(&t);
                }
            }
            KeyCode::Char('w') => {
                self.set_tab(Tab::Wave);
                if self.offline {
                    self.start_offline_wave();
                } else {
                    self.start_wave(vec![MY_WAVE.into()], "Моя волна".into());
                }
            }
            KeyCode::Char('t') => self.lyrics_mode = !self.lyrics_mode,
            KeyCode::Char('v') => self.toggle_stage(),
            KeyCode::Char('y') => {
                if let Some(t) = self.target_track() {
                    crate::ui::term::copy_to_clipboard(&mut self.osc_out, &Self::track_url(&t));
                    self.toast(format!("Ссылка скопирована: {}", t.full_title()));
                }
            }
            KeyCode::Char('r') => {
                self.repeat_one = !self.repeat_one;
                self.player.clear_next();
                self.next_loaded = None;
                self.schedule_next();
                self.toast(
                    if self.repeat_one {
                        "Повтор трека: вкл"
                    } else {
                        "Повтор трека: выкл"
                    }
                    .into(),
                );
            }
            KeyCode::Char('s') => {
                self.shuffle = !self.shuffle;
                self.toast(
                    if self.shuffle {
                        "Перемешивание: вкл"
                    } else {
                        "Перемешивание: выкл"
                    }
                    .into(),
                );
            }
            KeyCode::Char('Q') => self.cycle_quality(),
            KeyCode::Char('B') => self.restart_bit_perfect(),
            KeyCode::Char('N') => self.toggle_normalize(),
            KeyCode::Char('P') => self.add_to_playlist(),
            KeyCode::Char('h') => self.open_history(),
            KeyCode::Char('C') => self.open_chart(),
            KeyCode::Char('M') if !self.offline => self.open_metatags(),
            KeyCode::Char('F') => {
                if let Some((what, id, name)) = self.target_entity() {
                    self.toggle_entity(what, id, name);
                }
            }
            KeyCode::Char('D') => {
                if let Some((Entity::ArtistLike, id, name)) = self.target_entity() {
                    self.toggle_entity(Entity::ArtistDislike, id, name);
                }
            }
            KeyCode::Char('m') if !self.offline => {
                if let Some(t) = self.target_track() {
                    self.open_similar(t);
                }
            }
            KeyCode::Char('c') if matches!(self.page().view, View::Playlists) => {
                self.popup = Some(Popup::Input {
                    title: " Новый плейлист ",
                    text: String::new(),
                    action: InputAction::Create(None),
                });
            }
            KeyCode::Char('E') => {
                if let Some(pl) = self.selected_own_playlist() {
                    self.popup = Some(Popup::Input {
                        title: " Описание плейлиста (пусто — убрать) ",
                        text: pl.description.clone().unwrap_or_default(),
                        action: InputAction::Describe(pl.kind.clone()),
                    });
                }
            }
            KeyCode::Char('H') => {
                if let Some(pl) = self.selected_own_playlist() {
                    let public = pl.visibility.as_deref() == Some("private");
                    if let Loadable::Ready(v) = &mut self.playlists
                        && let Some(p) = v.iter_mut().find(|p| p.kind == pl.kind)
                    {
                        p.visibility = Some(if public { "public" } else { "private" }.into());
                    }
                    self.pl_op(PlOp::Visibility {
                        kind: pl.kind,
                        title: pl.title,
                        public,
                    });
                }
            }
            KeyCode::Char('e') => {
                if let Some(pl) = self.selected_own_playlist() {
                    self.popup = Some(Popup::Input {
                        title: " Переименовать плейлист ",
                        text: pl.title.clone(),
                        action: InputAction::Rename(pl.kind.clone()),
                    });
                }
            }
            KeyCode::Delete if matches!(self.page().view, View::Playlists) => {
                if let Some(pl) = self.selected_own_playlist() {
                    self.popup = Some(Popup::Confirm {
                        text: format!("Удалить плейлист «{}»?", pl.title),
                        action: ConfirmAction::DeletePlaylist(pl.kind.clone(), pl.title.clone()),
                    });
                }
            }
            KeyCode::Delete => self.confirm_remove_track(),
            KeyCode::Char('K') => self.move_track(-1),
            KeyCode::Char('J') => self.move_track(1),
            KeyCode::Char('x') => self.cycle_crossfade(),
            KeyCode::Char('R') => {
                self.load_likes();
                self.load_playlists();
                self.load_wheel();
                if self.releases.is_some() {
                    self.load_releases();
                }
                self.toast("Обновляю библиотеку…".into());
            }
            _ => {}
        }
    }

    /// Заголовок окна терминала: что играет, или просто имя приложения.
    pub fn window_title(&self) -> String {
        let t = self.now.as_ref().map(|n| &n.track).or(self.loading_track.as_ref());
        match t {
            Some(t) => {
                let state = if self.loading_track.is_some() {
                    "…"
                } else if self.player.paused() {
                    "⏸"
                } else {
                    "▶"
                };
                format!(
                    "{state} {} — {} · {}",
                    t.full_title(),
                    t.artist_names(),
                    crate::ui::TITLE
                )
            }
            None => crate::ui::TITLE.into(),
        }
    }

    pub fn quality(&self) -> Quality {
        self.fetcher.as_ref().map(|f| f.quality()).unwrap_or(self.cfg.quality)
    }

    /// Нужна ли перерисовка с полной частотой кадров.
    /// Сколько ждать до следующего кадра без ввода: полные fps только когда что-то
    /// действительно движется; просто играет — 8 fps; окно не в фокусе — раз в секунду.
    /// Идёт ли сейчас эффект перебора символов.
    pub fn scrambling(&self) -> bool {
        [self.fx_start, self.fx_view, self.fx_track]
            .iter()
            .flatten()
            .any(|t| t.elapsed() < SCRAMBLE)
    }

    pub fn frame_interval(&self, fps: u32) -> Duration {
        // Без фокуса окно может быть видно (соседний сплит) — плавность сохраняем,
        // только поменьше кадров.
        let fps = if self.focused { fps } else { fps.min(12) };
        let heavy = self.scrambling()
            || self.theme.animating()
            || !self.toasts.is_empty()
            || self.loading_track.is_some()
            || matches!(self.screen, Screen::Connecting(_) | Screen::Login(_))
            || matches!(self.page().view, View::Wave)
            || self.stage_mode
            || self.lyrics_moving
            || (self.page().anim - self.page().offset as f32).abs() > 0.01;
        if heavy {
            Duration::from_secs_f64(1.0 / fps as f64)
        } else if self.now.is_some() && !self.player.paused() {
            Duration::from_millis(125)
        } else {
            Duration::from_millis(500)
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(now) = &self.now {
            self.report_end(now.token, now.pos, true);
        }
        self.save_session();
        self.state.save(&self.paths);
        self.player.stop();
    }

    pub fn token_needs_refresh(&self) -> Option<StoredToken> {
        self.stored_token.clone().filter(StoredToken::needs_refresh)
    }
}

/// Символ с той же физической клавиши в раскладке QWERTY: управление работает и на ЙЦУКЕН.
fn qwerty(c: char) -> char {
    // Только кириллица и `.`/`,` с клавиши `/`: латинские знаки (`?`, `+`, `-`…) остаются как есть,
    // иначе `?` на латинской раскладке превращался бы в `&`.
    const RU: &str = "йцукенгшщзхъфывапролджэячсмитьбюёЙЦУКЕНГШЩЗХЪФЫВАПРОЛДЖЭЯЧСМИТЬБЮЁ.,";
    const EN: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`QWERTYUIOP{}ASDFGHJKL:\"ZXCVBNM<>~/?";
    RU.chars()
        .position(|r| r == c)
        .and_then(|i| EN.chars().nth(i))
        .unwrap_or(c)
}

fn latin_key(mut k: KeyEvent) -> KeyEvent {
    if let KeyCode::Char(c) = k.code {
        k.code = KeyCode::Char(qwerty(c));
    }
    k
}

pub fn agent_key(uri: &str) -> String {
    format!("agent:{uri}")
}

pub fn thumb_key(uri: &str) -> String {
    format!("thumb:{uri}")
}

pub fn quality_label(q: Quality) -> &'static str {
    match q {
        Quality::Lossless => "Lossless",
        Quality::Hq => "Высокое",
        Quality::Nq => "Экономия",
    }
}

enum VerifyError {
    /// Токен отвергнут (401) — нужен вход.
    Invalid(String),
    /// Сеть, таймаут, 5xx — токен ни при чём, стоит повторить.
    Transient(String),
}

/// Проверка токена (с обновлением, если скоро истекает) и загрузка аккаунта.
async fn verify(mut tok: StoredToken) -> Result<(Client, AccountStatus, StoredToken), VerifyError> {
    if tok.needs_refresh()
        && let Some(r) = tok.refresh_token.clone()
    {
        match auth::refresh(&api::http_client(), &r).await {
            Ok(t) => tok = t,
            Err(e) => tracing::warn!("refresh: {e}"),
        }
    }
    let mut c = Client::new(tok.access_token.clone());
    match c.account_status().await {
        Ok(s) => Ok((c, s, tok)),
        Err(ApiError::Unauthorized) => {
            // 401 — последний шанс: обновить токен по refresh_token и проверить ещё раз.
            if let Some(r) = tok.refresh_token.clone() {
                match auth::refresh(&api::http_client(), &r).await {
                    Ok(t) => {
                        let mut c = Client::new(t.access_token.clone());
                        match c.account_status().await {
                            Ok(s) => return Ok((c, s, t)),
                            Err(ApiError::Unauthorized) => {}
                            Err(e) => return Err(VerifyError::Transient(e.to_string())),
                        }
                    }
                    Err(e) => tracing::warn!("refresh после 401: {e}"),
                }
            }
            Err(VerifyError::Invalid("Токен недействителен, нужен вход".into()))
        }
        Err(e) => Err(VerifyError::Transient(e.to_string())),
    }
}

/// Подключение с повторами: из-за сети/сервера на экран входа не выкидываем.
async fn connect(tok: StoredToken, tx: UnboundedSender<Msg>) {
    let mut delay = 2u64;
    loop {
        match verify(tok.clone()).await {
            Ok(r) => {
                let _ = tx.send(Msg::Authed(Ok(r)));
                return;
            }
            Err(VerifyError::Invalid(e)) => {
                let _ = tx.send(Msg::Authed(Err(e)));
                return;
            }
            Err(VerifyError::Transient(e)) => {
                tracing::warn!("подключение: {e}; повтор через {delay} с");
                if tx
                    .send(Msg::ConnectError(format!("{e} — повтор через {delay} с")))
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(delay)).await;
                delay = (delay * 2).min(60);
            }
        }
    }
}

fn search_items(r: SearchResult) -> Vec<SearchItem> {
    let mut v = Vec::new();
    if let Some(b) = r.tracks.filter(|b| !b.results.is_empty()) {
        v.push(SearchItem::Header("Треки"));
        v.extend(b.results.into_iter().map(SearchItem::Track));
    }
    if let Some(b) = r.artists.filter(|b| !b.results.is_empty()) {
        v.push(SearchItem::Header("Исполнители"));
        v.extend(b.results.into_iter().take(8).map(SearchItem::Artist));
    }
    if let Some(b) = r.albums.filter(|b| !b.results.is_empty()) {
        v.push(SearchItem::Header("Альбомы"));
        v.extend(b.results.into_iter().take(10).map(SearchItem::Album));
    }
    if let Some(b) = r.playlists.filter(|b| !b.results.is_empty()) {
        v.push(SearchItem::Header("Плейлисты"));
        v.extend(b.results.into_iter().take(10).map(SearchItem::Playlist));
    }
    v
}

pub enum ArtistRow<'a> {
    Header(&'static str),
    /// «Моя волна по исполнителю» (id исполнителя).
    Wave(&'a str),
    /// Трек секции: вся секция и номер в ней (Enter играет секцию с этого места).
    Track(&'a [Track], usize),
    Concert(&'a api::Concert),
    Album(&'a api::Album),
    Artist(&'a api::Artist),
}

pub fn artist_rows(b: &ArtistBrief) -> Vec<ArtistRow<'_>> {
    let mut v = Vec::new();
    if let Some(a) = b.artist.as_ref().filter(|a| !a.id.is_empty()) {
        v.push(ArtistRow::Wave(&a.id));
    }
    if !b.familiar.is_empty() {
        v.push(ArtistRow::Header("Знакомое вам"));
        v.extend((0..b.familiar.len()).map(|i| ArtistRow::Track(&b.familiar, i)));
    }
    if !b.popular_tracks.is_empty() {
        v.push(ArtistRow::Header("Популярные треки"));
        v.extend((0..b.popular_tracks.len()).map(|i| ArtistRow::Track(&b.popular_tracks, i)));
    }
    if !b.concerts.is_empty() {
        v.push(ArtistRow::Header("Концерты"));
        v.extend(b.concerts.iter().map(ArtistRow::Concert));
    }
    // Дискография целиком, если догрузилась; иначе — то, что пришло в карточке.
    let disc: &[api::Album] = if b.discography.is_empty() {
        &b.albums
    } else {
        &b.discography
    };
    let single = |a: &api::Album| a.kind.as_deref() == Some("single");
    let mut albums = disc.iter().filter(|a| !single(a)).peekable();
    if albums.peek().is_some() {
        v.push(ArtistRow::Header("Альбомы"));
        v.extend(albums.map(ArtistRow::Album));
    }
    let mut singles = disc.iter().filter(|a| single(a)).peekable();
    if singles.peek().is_some() {
        v.push(ArtistRow::Header("Синглы и EP"));
        v.extend(singles.map(ArtistRow::Album));
    }
    if !b.also_albums.is_empty() {
        v.push(ArtistRow::Header("Участие и сборники"));
        v.extend(b.also_albums.iter().map(ArtistRow::Album));
    }
    if !b.similar_artists.is_empty() {
        v.push(ArtistRow::Header("Похожие исполнители"));
        v.extend(b.similar_artists.iter().filter(|a| !a.id.is_empty()).map(ArtistRow::Artist));
    }
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn russian_layout() {
        assert_eq!(super::qwerty('й'), 'q');
        assert_eq!(super::qwerty('Й'), 'Q');
        assert_eq!(super::qwerty('д'), 'l');
        assert_eq!(super::qwerty('.'), '/');
        assert_eq!(super::qwerty(','), '?');
        assert_eq!(super::qwerty('ё'), '`');
        assert_eq!(super::qwerty('n'), 'n');
        assert_eq!(super::qwerty('?'), '?');
        assert_eq!(super::qwerty('+'), '+');
    }
}

#[derive(Clone)]
enum Opener {
    Online(Fetcher),
    Offline(Arc<Library>),
}

impl Opener {
    async fn open(&self, t: &Track) -> color_eyre::Result<(audio::Source, Origin)> {
        match self {
            Opener::Online(f) => f.open(t).await,
            Opener::Offline(lib) => lib
                .lookup(&t.id)
                .map(|(p, _)| (audio::Source::File(p), Origin::Library))
                .ok_or_else(|| color_eyre::eyre::eyre!("нет в библиотеке, а сети нет")),
        }
    }
}

/// «44.1 кГц» / «96 кГц».
fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} кГц", rate / 1000)
    } else {
        format!("{:.1} кГц", rate as f32 / 1000.0)
    }
}

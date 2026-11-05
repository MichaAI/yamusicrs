//! Состояние интерфейса: экраны, навигация, ввод, картинки и анимации. Плеер, очередь,
//! вход и волна — в движке ядра ([`Engine`]); его просьбы выполняет [`App::pump`].

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use tokio::sync::mpsc::UnboundedSender;
pub use yamusic_core::engine::{Loadable, LoginState, QueueKind, Screen, quality_label};
use yamusic_core::engine::{self, CROSSFADE_STEPS, Effect, Engine, Entity, MY_WAVE, PlOp, err_str};

use crate::api::{self, ArtistBrief, Client, Playlist, Quality, SearchResult, Track};
use crate::audio::{self, Token};
use crate::config::{Config, Paths};
use crate::imgcache;
use crate::lyrics::Lyrics;
use crate::mpris;
use crate::ui::kitty::Images;
use crate::ui::theme::{Palette, ThemeAnim};

pub enum Msg {
    /// Ответ фоновой задачи движка.
    Engine(engine::Msg),
    /// Обложка для уведомления о новом треке готова.
    Notify {
        token: Token,
        title: String,
        body: String,
        icon: Option<Vec<u8>>,
    },
    LikedAlbums { view_id: u64, result: Result<Vec<api::Album>, String> },
    ArtistList {
        view_id: u64,
        /// Это лайкнутые — пополнить множество лайков.
        liked: bool,
        result: Result<Vec<(api::Artist, String)>, String>,
    },
    Metatags { view_id: u64, result: Result<Vec<api::MetaTree>, String> },
    Metatag { view_id: u64, result: Result<api::Metatag, String> },
    Credits { track_id: String, result: Result<Vec<(String, String)>, String> },
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
    Mpris(mpris::Command),
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
            Setting::Crossfade => "Наплыв между треками, 0 — встык. Клавиша x.",
            Setting::ReportPlays => "Передавать Яндексу историю прослушиваний (влияет на рекомендации и «недавно слушали»).",
            Setting::Ynison => "yamusicrs появляется в «Слушать на…» в приложениях Яндекс Музыки: оттуда можно переключить музыку сюда и управлять ею. Включённое здесь забирает воспроизведение у телефона.",
            Setting::Fps => "Частота кадров анимаций (блоб, прокрутка, перебор символов).",
            Setting::Dither => "Шум против «колец» на градиентах блоба, в ступенях 8 бит; 0 — выключен.",
            Setting::Recheck => "Сервер не дал lossless — трек помечается и не перепроверяется 30 дней. Enter снимает пометки: каждый трек хуже lossless спросит сервер заново при следующем проигрывании и, если дадут лучше, перекачается.",
        }
    }
}

const FPS_STEPS: [u32; 5] = [15, 24, 30, 60, 120];

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
    /// Ynison «Слушать на»: (id, название, тип, активно); первая строка — этот компьютер (id пустой).
    Devices { list: Vec<(String, String, String, bool)>, sel: usize },
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

/// Идентификатор уведомления «сейчас играет» (OSC 99): с тем же id kitty обновляет его на месте.
const NOTIFY_ID: &str = "yamusicrs-now";

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

pub struct Toast {
    pub text: String,
    pub at: Instant,
    pub error: bool,
}

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
    /// Плеер, очередь, вход, волна, Ynison — всё, что не про экран.
    pub engine: Engine,
    /// Плавная позиция дуги волны (догоняет курсор) и пункты дуги на экране — для мыши.
    pub wave_arc: f32,
    pub wave_hits: Vec<(ratatui::layout::Rect, usize)>,
    pub tab: Tab,
    pub stack: Vec<Page>,
    /// «О треке»: авторы и лейбл по id трека (грузятся при открытии окна).
    pub credits: HashMap<String, Vec<(String, String)>>,
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
    /// Фаза анимации блоба (быстрее, когда играет).
    pub wave_phase: f32,
    /// Текущие (плавно догоняющие цель) цвета блоба.
    pub blob_colors: [[f32; 3]; 4],
    /// Плавные энергия и скорость блоба.
    pub blob_energy: f32,
    pub blob_speed: f32,
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
    mpris: UnboundedSender<mpris::Update>,
    pub quit: bool,
    view_seq: u64,
    pub started: Instant,
    /// Прямоугольники последнего кадра — для эффектов.
    pub areas: Areas,
    last_mpris_pos: Instant,
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

impl App {
    pub fn new(tx: UnboundedSender<Msg>, cfg: Config, paths: Paths, offline: bool) -> color_eyre::Result<Self> {
        let cache = paths.cache_dir.clone();
        tokio::task::spawn_blocking(move || imgcache::prune(&cache));
        let (etx, mut erx) = tokio::sync::mpsc::unbounded_channel();
        let engine_tx = tx.clone();
        tokio::spawn(async move {
            while let Some(m) = erx.recv().await {
                if engine_tx.send(Msg::Engine(m)).is_err() {
                    break;
                }
            }
        });
        let images = Images::new(cfg.kitty_enabled());
        let gfx = crate::render::Gfx::new(cfg.gpu && cfg.kitty_enabled());
        let kitty_text = cfg.kitty_text_enabled();
        let engine = Engine::new(etx, cfg, paths, offline, Box::new(audio::pipewire::spawn))?;

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
        let _ = mpris.send(mpris::Update::Volume(engine.player.volume() as f64));

        let mut app = Self {
            tx,
            engine,
            wave_arc: 0.0,
            wave_hits: Vec::new(),
            tab: Tab::Wave,
            stack: vec![Page::new(View::Wave)],
            credits: HashMap::new(),
            releases: None,
            grid_cols: 1,
            grid_card_h: 1,
            grid_wheel: 0.0,
            grid_hits: Vec::new(),
            search_input: String::new(),
            search_editing: false,
            search: None,
            wave_phase: 0.0,
            blob_energy: 0.0,
            blob_speed: 0.3,
            blob_colors: [
                [255.0, 204.0, 0.0],
                [255.0, 107.0, 61.0],
                [240.0, 42.0, 79.0],
                [120.0, 90.0, 220.0],
            ],
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
            mpris,
            quit: false,
            view_seq: 0,
            started: Instant::now(),
            areas: Areas::default(),
            last_mpris_pos: Instant::now(),
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
        };
        app.pump();
        Ok(app)
    }

    // ── авторизация ──────────────────────────────────────────────────────────

    // ── загрузки данных ──────────────────────────────────────────────────────

    /// Строки вкладки «Плейлисты» (пусто, пока свои плейлисты не загрузились).
    pub fn playlist_rows(&self) -> Vec<PlaylistRow<'_>> {
        let Some(own) = self.engine.playlists.ready() else {
            return Vec::new();
        };
        let mut v: Vec<PlaylistRow> = self.engine.personal.iter().map(PlaylistRow::Personal).collect();
        v.extend(own.iter().map(PlaylistRow::Own));
        if !self.engine.offline {
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
        let own = self.engine.is_own(&pl);
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
        if self.engine.offline {
            let r = self
                .engine.snapshot
                .tracks_of(&pl)
                .map(|ts| self.engine.offline_tracks(ts.clone()))
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
        let h = &self.engine.state.search_history;
        let mut v = Vec::new();
        if !h.is_empty() {
            v.push(SearchItem::Header(HISTORY_QUERIES));
            v.extend(h.iter().cloned().map(SearchItem::Query));
        }
        if !self.engine.found.is_empty() {
            v.push(SearchItem::Header(HISTORY_FOUND));
            v.extend(self.engine.found.iter().cloned().map(|f| match f {
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
        self.engine.state.search_history.retain(|h| h != q);
        self.engine.state.save(&self.engine.paths);
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
        let h = &mut self.engine.state.search_history;
        h.retain(|x| x.to_lowercase() != q.to_lowercase());
        h.insert(0, q.clone());
        h.truncate(SEARCH_HISTORY_LEN);
        self.engine.state.save(&self.engine.paths);
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
            for id in self.engine.library.ids() {
                if let Some(u) = self.engine.snapshot.find(&id).and_then(|t| t.cover_uri()) {
                    m.entry(u.to_string()).or_insert(id);
                }
            }
            self.cover_files = Some(m);
        }
        let id = self.cover_files.as_ref()?.get(uri)?.clone();
        self.engine.library.file(&id)
    }

    /// Картинка: дисковый кеш → обложка из тегов скачанного трека → сеть (и в кеш).
    fn fetch_image(&mut self, key: String, uri: &str, size: u32, palette: bool) {
        if self.images.contains(&key) || self.covers_failed.contains(&key) || !self.covers_loading.insert(key.clone())
        {
            return;
        }
        let url = api::cover_url(uri, size);
        let cache = self.engine.paths.cache_dir.clone();
        let file = self.cover_file(uri);
        let (client, tx) = (self.engine.client.clone(), self.tx.clone());
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
        let file = self.engine.library.file(&id);
        let (client, tx) = (self.engine.client.clone(), self.tx.clone());
        tokio::spawn(async move {
            let (lyrics, raw) = crate::lyrics::load(client, file, id.clone(), sync).await;
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
            self.page_mut().sel = self.engine.queue.pos.unwrap_or(0);
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
            View::Likes => self.engine.likes.ready().map(Vec::as_slice),
            View::Queue => Some(&self.engine.queue.items),
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
            View::Wave => self.engine.wave.items.ready().map_or(0, Vec::len),
            View::Likes => self.engine.likes.ready().map_or(0, Vec::len),
            View::Playlists => self.playlist_rows().len(),
            View::Search => match &self.search {
                Some(Loadable::Ready(items)) => items.len(),
                _ => 0,
            },
            View::Queue => self.engine.queue.items.len(),
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
            .or_else(|| self.engine.now.as_ref().map(|n| n.track.clone()))
    }

    fn activate(&mut self) {
        let sel = self.page().sel;
        let orig = self.sel_index();
        match &self.page().view {
            View::Wave if self.engine.offline => self.engine.start_offline_wave(),
            View::Wave => {
                let Some(item) = self.engine.wave.items.ready().and_then(|v| v.get(sel)).cloned() else {
                    return;
                };
                // Этот пресет уже играет — Enter ставит на паузу, а не перезапускает.
                if self.engine.is_active_preset(&item) {
                    self.engine.toggle_pause();
                } else {
                    self.engine.start_wave(item.seeds(), item.name().to_string());
                }
            }
            View::Likes => {
                if let Some(ts) = self.engine.likes.ready() {
                    let ts = ts.clone();
                    self.engine.play_list(ts, orig, "Коллекция".into());
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
                        self.engine.start_wave(vec![seed], name);
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
            View::Queue => self.engine.play_index(orig),
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
                    self.engine.play_list(ts, orig, title);
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
                        self.engine.play_list(tracks, idx, format!("Поиск: {}", self.search_input));
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
                        self.engine.play_list(tracks, idx, name);
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
                        self.engine.start_wave(vec![seed], name);
                    }
                    _ => {}
                }
            }
        }
    }

    // ── очередь и воспроизведение ────────────────────────────────────────────

    // ── офлайн ───────────────────────────────────────────────────────────────

    /// Текущее значение строки настроек.
    pub fn setting_value(&self, s: Setting) -> String {
        match s {
            Setting::Quality => quality_label(self.engine.quality()).into(),
            Setting::Normalize => if self.engine.normalize() { "вкл" } else { "выкл · bit-perfect" }.into(),
            Setting::TargetLufs => format!("{:.0} LUFS", self.engine.cfg.normalize_target_lufs),
            Setting::Crossfade => match self.engine.crossfade() {
                c if c > 0.0 => format!("{c:.0} с"),
                _ => "выкл".into(),
            },
            Setting::ReportPlays => if self.engine.cfg.report_plays { "да" } else { "нет" }.into(),
            Setting::Ynison => self.engine.ynison_label(),
            Setting::Fps => self.engine.cfg.fps.to_string(),
            Setting::Dither => match self.engine.cfg.blob_dither {
                0 => "выкл".into(),
                d => d.to_string(),
            },
            Setting::Recheck => match self.engine.library.below_lossless() {
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
                let cur = QS.iter().position(|q| *q == self.engine.quality()).unwrap_or(2);
                let next = QS[step(QS.len(), cur)];
                if next != self.engine.quality() {
                    self.engine.set_quality(next);
                }
            }
            Setting::Normalize => self.engine.toggle_normalize(),
            Setting::TargetLufs => {
                let d = if dir == 0 { 1.0 } else { dir as f32 };
                let v = (self.engine.cfg.normalize_target_lufs.round() + d).clamp(-23.0, -8.0);
                self.engine.set_target_lufs(v);
            }
            Setting::Crossfade => {
                let cur = CROSSFADE_STEPS
                    .iter()
                    .position(|c| (*c - self.engine.crossfade()).abs() < 0.01)
                    .unwrap_or(0);
                let next = CROSSFADE_STEPS[step(CROSSFADE_STEPS.len(), cur)];
                if (next - self.engine.crossfade()).abs() > 0.01 {
                    self.engine.set_crossfade(next);
                }
            }
            Setting::ReportPlays => self.engine.toggle_report_plays(),
            Setting::Ynison => self.engine.toggle_ynison(),
            Setting::Fps => {
                let cur = FPS_STEPS.iter().position(|f| *f >= self.engine.cfg.fps).unwrap_or(FPS_STEPS.len() - 1);
                self.engine.cfg.fps = FPS_STEPS[step(FPS_STEPS.len(), cur)];
                self.engine.cfg.save(&self.engine.paths);
            }
            Setting::Dither => {
                self.engine.cfg.blob_dither = step(5, self.engine.cfg.blob_dither.min(4) as usize) as u32;
                self.engine.cfg.save(&self.engine.paths);
            }
            Setting::Recheck if dir == 0 => self.engine.recheck_library(),
            Setting::Recheck => {}
        }
    }

    // ── Моя волна ────────────────────────────────────────────────────────────

    // ── правка плейлистов ────────────────────────────────────────────────────

    /// P: выбрать свой плейлист для выделенного (или играющего) трека.
    fn add_to_playlist(&mut self) {
        let Some(track) = self.target_track() else { return };
        let list: Vec<(String, String)> = self
            .engine.playlists
            .ready()
            .map(|v| {
                v.iter()
                    .filter(|p| self.engine.is_own(p))
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
            PlaylistRow::Own(pl) => self.engine.is_own(pl).then(|| (*pl).clone()),
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
        self.engine.pl_op(PlOp::Move {
            kind,
            at,
            track: track.full_id(),
            neighbor,
        });
    }

    /// Перечитать открытые страницы плейлиста `kind` (после неудачной правки).
    fn reload_playlist_views(&mut self, kind: &str) {
        let Some(pl) = self
            .engine.playlists
            .ready()
            .and_then(|v| v.iter().find(|p| p.kind == kind && self.engine.is_own(p)))
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
            if let Loadable::Ready(v) = &mut self.engine.playlists
                && let Some(p) = v.iter_mut().find(|p| p.kind == kind)
            {
                p.description = (!title.is_empty()).then(|| title.clone());
            }
            return self.engine.pl_op(PlOp::Describe { kind, text: title });
        }
        if title.is_empty() {
            return;
        }
        match action {
            InputAction::Describe(_) => {}
            InputAction::Create(track) => self.engine.pl_op(PlOp::Create {
                title,
                add: track.map(|t| t.full_id()),
            }),
            InputAction::Rename(kind) => {
                if let Loadable::Ready(v) = &mut self.engine.playlists
                    && let Some(p) = v.iter_mut().find(|p| p.kind == kind)
                {
                    p.title = title.clone();
                }
                self.engine.pl_op(PlOp::Rename { kind, title });
            }
        }
    }

    fn confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeletePlaylist(kind, title) => {
                if let Loadable::Ready(v) = &mut self.engine.playlists {
                    v.retain(|p| p.kind != kind || p.uid != self.engine.client.as_ref().map_or("", |c| c.uid.as_str()));
                }
                self.engine.pl_op(PlOp::Delete { kind, title });
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
                self.engine.pl_op(PlOp::Remove {
                    kind,
                    at,
                    track: track.full_id(),
                });
            }
        }
    }

    // ── лайки ────────────────────────────────────────────────────────────────

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
            Msg::Engine(m) => self.engine.on_msg(m),
            Msg::Notify {
                token,
                title,
                body,
                icon,
            } => {
                if self.engine.now.as_ref().is_some_and(|n| n.token == token) {
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
            Msg::Releases(r) => {
                self.releases = Some(match r {
                    Ok(a) => Loadable::Ready(a),
                    Err(e) => Loadable::Failed(e),
                });
            }
            Msg::Credits { track_id, result } => match result {
                Ok(v) => {
                    self.credits.insert(track_id, v);
                }
                Err(e) => tracing::warn!("credits {track_id}: {e}"),
            },
            Msg::LikedAlbums { view_id, result } => {
                for p in &mut self.stack {
                    if let View::Albums { id, albums } = &mut p.view
                        && *id == view_id
                    {
                        if let Ok(v) = &result {
                            self.engine.liked_albums.extend(v.iter().map(|a| a.id.clone()));
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
                            self.engine.liked_artists.extend(v.iter().map(|(a, _)| a.id.clone()));
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
            Msg::Cover { key, result } => {
                self.covers_loading.remove(&key);
                if result.is_err() && self.engine.client.is_none() {
                    self.covers_failed.insert(key.clone());
                }
                match result {
                    Ok((img, pal)) => {
                        self.images.insert(key.clone(), img);
                        self.palettes.insert(key.clone(), pal);
                        let now_cover = self.engine.now.as_ref().and_then(|n| n.track.cover_uri()).map(str::to_owned);
                        let loading_cover = self
                            .engine.loading_track
                            .as_ref()
                            .and_then(|t| t.cover_uri())
                            .map(str::to_owned);
                        if now_cover.as_deref() == Some(&key)
                            || (self.engine.now.is_none() && loading_cover.as_deref() == Some(&key))
                        {
                            self.theme.set(pal);
                        }
                    }
                    Err(e) => tracing::debug!("обложка {key}: {e}"),
                }
            }
            Msg::Lyrics { track_id, lyrics, raw } => {
                if self.engine.now.as_ref().is_some_and(|n| n.track.id == track_id)
                    && let Some(l) = lyrics
                {
                    self.lyrics = Some((track_id, l));
                    if let Some(r) = raw {
                        let _ = self.mpris.send(mpris::Update::Lyrics(r));
                    }
                }
            }
            Msg::Mpris(c) => match c {
                mpris::Command::PlayPause => self.engine.toggle_pause(),
                mpris::Command::Play => {
                    if self.engine.player.paused() {
                        self.engine.toggle_pause()
                    }
                }
                mpris::Command::Pause => {
                    if !self.engine.player.paused() {
                        self.engine.toggle_pause()
                    }
                }
                mpris::Command::Next => self.engine.next(),
                mpris::Command::Previous => self.engine.prev(),
                mpris::Command::Stop => {
                    self.engine.player.stop();
                    self.engine.now = None;
                }
                mpris::Command::Seek(d) => self.engine.seek_by(d),
                mpris::Command::SetPosition(p) => self.engine.seek_to(p),
                mpris::Command::Volume(v) => self.engine.set_volume(v as f32),
                mpris::Command::Like => {
                    if let Some(t) = self.engine.now.as_ref().map(|n| n.track.clone()) {
                        self.engine.toggle_like(t);
                    }
                }
                mpris::Command::Dislike => self.engine.dislike_current(),
            },
        }
        self.pump();
    }

    /// Вызывается каждый кадр: движок синхронизирует «что играет», UI — анимации.
    pub fn tick(&mut self) {
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(4));
        // Дуга волны догоняет курсор.
        if let Some(p) = self.stack.iter().find(|p| matches!(p.view, View::Wave)) {
            let target = p.sel as f32;
            self.wave_arc += (target - self.wave_arc) * 0.22;
            if (target - self.wave_arc).abs() < 0.01 {
                self.wave_arc = target;
            }
        }
        self.engine.tick();
        self.pump();
        if self.last_mpris_pos.elapsed() > Duration::from_millis(500)
            && let Some(n) = &self.engine.now
        {
            let _ = self.mpris.send(mpris::Update::Position(n.pos));
            self.last_mpris_pos = Instant::now();
        }
        // Плавная прокрутка текста к текущей строке.
        if let (Some((_, Lyrics::Synced(lines))), Some(n)) = (&self.lyrics, &self.engine.now) {
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

    /// Выполняет просьбы движка: тосты, обложки, MPRIS, уведомления, обновление списков.
    pub fn pump(&mut self) {
        loop {
            let effects = self.engine.take_effects();
            if effects.is_empty() {
                return;
            }
            for e in effects {
                self.on_effect(e);
            }
        }
    }

    fn on_effect(&mut self, e: Effect) {
        match e {
            Effect::Toast { text, error, action } => self.toasts.push(Toast {
                text: match action {
                    None => text,
                    Some(engine::ToastAction::Resume) => format!("{text} — Space"),
                    Some(engine::ToastAction::RestartBitPerfect) => format!("{text} · B — перезапустить bit-perfect"),
                },
                at: Instant::now(),
                error,
            }),
            Effect::Cover(uri) => self.fetch_cover(&uri),
            Effect::TrackStarted { token, track, pos } => {
                if let Some(c) = track.cover_uri() {
                    let c = c.to_string();
                    self.fetch_cover(&c);
                }
                self.apply_theme_for(&track);
                self.fetch_lyrics(&track);
                self.notify_track(token, &track);
                let _ = self.mpris.send(mpris::Update::Track {
                    id: track.full_id(),
                    title: track.full_title(),
                    artists: track.artists.iter().map(|a| a.name.clone()).collect(),
                    album: track.album().map(|a| a.full_title()).unwrap_or_default(),
                    length: track.duration_secs(),
                    art: track.cover_uri().map(|u| api::cover_url(u, 400)),
                    pos,
                    liked: self.engine.is_liked(&track),
                });
                let _ = self.mpris.send(mpris::Update::Status {
                    playing: !self.engine.player.paused(),
                    pos,
                });
                self.fx_track = Some(Instant::now());
            }
            Effect::Playback { playing, pos } => {
                let _ = self.mpris.send(mpris::Update::Status { playing, pos });
            }
            Effect::Seeked(s) => {
                let _ = self.mpris.send(mpris::Update::Seeked(s));
            }
            Effect::Volume(v) => {
                let _ = self.mpris.send(mpris::Update::Volume(v as f64));
            }
            Effect::LikesChanged => {
                self.refilter_keep();
                // Иконка лайка в MPRIS-расширении шелла.
                if let Some(n) = &self.engine.now {
                    let _ = self.mpris.send(mpris::Update::Liked(self.engine.is_liked(&n.track)));
                }
            }
            Effect::Connected { was_offline } => {
                if was_offline {
                    self.covers_failed.clear();
                }
                self.fx_start = Some(Instant::now());
            }
            Effect::Wheel { home } => {
                let icons: Vec<String> = self
                    .engine
                    .wave
                    .items
                    .ready()
                    .into_iter()
                    .flatten()
                    .filter_map(|i| i.icon())
                    .map(str::to_owned)
                    .collect();
                for icon in icons {
                    self.fetch_image(agent_key(&icon), &icon, 200, false);
                }
                if let Some(p) = self.stack.iter_mut().find(|p| matches!(p.view, View::Wave)) {
                    p.sel = home;
                }
                self.wave_arc = home as f32;
            }
            Effect::Found => {
                // История уже на экране — перерисовать с серверной частью.
                if self.search.is_none() || self.search_shows_history() {
                    let sel = self.page().sel;
                    self.show_search_history();
                    if matches!(self.page().view, View::Search) && sel > 0 {
                        self.page_mut().sel = sel.min(self.page_len().saturating_sub(1));
                    }
                }
            }
            Effect::Personal { shift } => {
                // Курсор остаётся на той же строке, хотя сверху добавились подборки.
                if let Some(p) = self.stack.iter_mut().find(|p| matches!(p.view, View::Playlists))
                    && p.sel > 0
                {
                    p.sel = (p.sel as isize + shift).max(0) as usize;
                }
            }
            Effect::PlaylistOpFailed { kind } => self.reload_playlist_views(&kind),
            Effect::LibraryMeta => self.cover_files = None,
        }
    }

    /// Фоновая задача с клиентом API (если вошли); ответ — в канал интерфейса.
    fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Client, UnboundedSender<Msg>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if let Some(c) = self.engine.client.clone() {
            tokio::spawn(f(c, self.tx.clone()));
        }
    }

    // ── клавиатура ───────────────────────────────────────────────────────────

    // ── всплывающие окна ─────────────────────────────────────────────────────

    fn show_track_info(&mut self) {
        let Some(track) = self.engine.now.as_ref().map(|n| n.track.clone()) else {
            return;
        };
        let file = self.engine.library.lookup(&track.id);
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

    /// Окно «Слушать на» (Ynison): этот компьютер и устройства аккаунта.
    fn open_devices(&mut self) {
        if !self.engine.cfg.ynison {
            return self.engine.toast("Ynison выключен — включите в настройках «Ynison (Слушать на…)»".into());
        }
        let here = self.engine.yn_active || self.engine.yn_other_device().is_none();
        let mut list = vec![(String::new(), "Этот компьютер".to_string(), "WEB".to_string(), here)];
        list.extend(
            self.engine
                .yn_devices()
                .into_iter()
                .filter(|d| d.4)
                .map(|(id, title, kind, active, _)| (id, if title.is_empty() { "Устройство без имени".into() } else { title }, kind, active)),
        );
        let sel = list.iter().position(|d| d.3).unwrap_or(0);
        self.popup = Some(Popup::Devices { list, sel });
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
            Some(Popup::Devices { list, sel }) => {
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
                Some((kind, title)) => self.engine.pl_op(PlOp::Append {
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
            Some(Popup::Devices { list, .. }) => {
                if let Some((id, ..)) = list.into_iter().nth(i) {
                    let id = if id.is_empty() {
                        self.engine.ynison.as_ref().map(|h| h.device_id.clone()).unwrap_or_default()
                    } else {
                        id
                    };
                    self.engine.yn_transfer(&id);
                }
            }
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
                Some(Popup::Artists { sel, .. }) | Some(Popup::AddToPlaylist { sel, .. }) | Some(Popup::Devices { sel, .. }) => {
                    self.popup_activate(*sel)
                }
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
        let stage = self.stage_mode && self.engine.now.is_some();
        let on_wave =
            (!stage && matches!(self.page().view, View::Wave) && self.wave_hits.iter().any(|(r, _)| inside(*r)))
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
        if !self.kitty_text || self.engine.player.paused() {
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
        if r.id != NOTIFY_ID || self.engine.now.is_none() {
            return;
        }
        match r.button {
            Some(1) => self.engine.next(),
            Some(2) => {
                if let Some(t) = self.engine.now.as_ref().map(|n| n.track.clone())
                    && !self.engine.is_liked(&t)
                {
                    self.engine.toggle_like(t);
                }
            }
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if !matches!(self.engine.screen, Screen::Main) || self.show_help {
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
        let stage = self.stage_mode && self.engine.now.is_some();
        // В сцене экран волны не виден — его пресеты и кнопки не должны ловить клики.
        if !stage && matches!(self.page().view, View::Wave) && inside(self.areas.main) {
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(i) = self.wave_hits.iter().find(|(r, _)| inside(*r)).map(|h| h.1) {
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
                    self.engine.set_volume(self.engine.player.volume() + if up { 0.05 } else { -0.05 });
                } else if inside(self.areas.main) {
                    self.move_sel(if up { -3 } else { 3 });
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(a) = self.controls.iter().find(|(r, _)| inside(*r)).map(|c| c.1) {
                    match a {
                        0 => self.engine.prev(),
                        1 => self.engine.toggle_pause(),
                        _ => self.engine.next(),
                    }
                } else if stage && inside(self.areas.stage_title) {
                    self.show_track_info();
                } else if stage && inside(self.areas.stage_artist) {
                    if let Some(t) = self.engine.now.as_ref().map(|n| n.track.clone()) {
                        self.pick_artist(&t);
                    }
                } else if stage && inside(self.areas.stage_album) {
                    if let Some(a) = self.engine.now.as_ref().and_then(|n| n.track.albums.first().cloned()) {
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
                        self.engine.seek_to(at);
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
                    if let Some(dur) = self.engine.now.as_ref().map(|n| n.track.duration_secs()) {
                        let frac = (m.column - self.areas.progress.x) as f64 / self.areas.progress.width.max(1) as f64;
                        self.engine.seek_to(frac * dur);
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
        match &mut self.engine.screen {
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
        match &mut self.engine.screen {
            Screen::Connecting(_) => match latin_key(k).code {
                KeyCode::Char('q') => self.quit = true,
                // Не ждать сеть, а войти заново (старый токен останется, пока не войдёшь).
                KeyCode::Char('l') => self.engine.begin_login(None),
                _ => {}
            },
            Screen::Login(l) => match k.code {
                KeyCode::Esc => self.quit = true,
                KeyCode::Enter => self.engine.submit_pasted_token(),
                KeyCode::Backspace => {
                    l.input.pop();
                }
                KeyCode::Char('r') if l.input.is_empty() => self.engine.begin_login(None),
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
        if self.stage_mode && self.engine.now.is_some() {
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => return self.engine.set_volume(self.engine.player.volume() + 0.05),
                KeyCode::Down | KeyCode::Char('j') => return self.engine.set_volume(self.engine.player.volume() - 0.05),
                KeyCode::Enter => return self.engine.toggle_pause(),
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => return,
                KeyCode::Char('g' | 'G' | 'a' | 'A' | 'L' | 'f') => return,
                _ => {}
            }
        }
        // Настройки: ←→ меняют значение (перемотка — с Shift).
        if matches!(self.page().view, View::Settings) && !self.lyrics_mode && !(self.stage_mode && self.engine.now.is_some()) {
            let sel = self.page().sel;
            match k.code {
                KeyCode::Left if !shift => return self.change_setting(sel, -1),
                KeyCode::Right if !shift => return self.change_setting(sel, 1),
                _ => {}
            }
        }
        // Сетка релизов: ↑↓ — по рядам, ←→ — по карточкам (перемотка — с Shift).
        if matches!(self.page().view, View::Releases) && !self.lyrics_mode && !(self.stage_mode && self.engine.now.is_some()) {
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
            // Музыка на другом устройстве (Ynison) — клавиши плеера управляют им.
            KeyCode::Left | KeyCode::Right if self.engine.yn_remote().is_some() => {
                let s = self.engine.yn_remote().map(|r| r.2);
                if let Some(s) = s {
                    let d: i64 = match (k.code, shift) {
                        (KeyCode::Left, true) => -30_000,
                        (KeyCode::Left, false) => -5_000,
                        (_, true) => 30_000,
                        _ => 5_000,
                    };
                    let pos = self.engine.yn_remote_pos(s) as i64;
                    self.engine.yn_remote_seek((pos + d).max(0) as u64);
                }
            }
            KeyCode::Char(' ') if self.engine.yn_remote().is_some() => self.engine.yn_remote_toggle(),
            KeyCode::Char('n') if self.engine.yn_remote().is_some() => self.engine.yn_remote_skip(1),
            KeyCode::Char('p') if self.engine.yn_remote().is_some() => self.engine.yn_remote_skip(-1),
            KeyCode::Left => self.engine.seek_by(if shift { -30.0 } else { -5.0 }),
            KeyCode::Right => self.engine.seek_by(if shift { 30.0 } else { 5.0 }),
            KeyCode::Enter => self.activate(),
            KeyCode::Char(' ') => self.engine.toggle_pause(),
            KeyCode::Char('n') => self.engine.next(),
            KeyCode::Char('p') => self.engine.prev(),
            KeyCode::Char('Y') => self.open_devices(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.engine.set_volume(self.engine.player.volume() + 0.05),
            KeyCode::Char('-') | KeyCode::Char('_') => self.engine.set_volume(self.engine.player.volume() - 0.05),
            KeyCode::Char('l') => {
                if let Some(t) = self.engine.now.as_ref().map(|n| n.track.clone()) {
                    self.engine.toggle_like(t);
                }
            }
            KeyCode::Char('L') => {
                if let Some(t) = self.target_track() {
                    self.engine.toggle_like(t);
                }
            }
            KeyCode::Char('d') => self.engine.dislike_current(),
            KeyCode::Char('a') => {
                if let Some(t) = self.selected_track().cloned() {
                    self.engine.enqueue(t, false);
                }
            }
            KeyCode::Char('A') => {
                if let Some(t) = self.selected_track().cloned() {
                    self.engine.enqueue(t, true);
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
                if self.engine.offline {
                    self.engine.start_offline_wave();
                } else {
                    self.engine.start_wave(vec![MY_WAVE.into()], "Моя волна".into());
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
                self.engine.repeat_one = !self.engine.repeat_one;
                self.engine.player.clear_next();
                self.engine.next_loaded = None;
                self.engine.schedule_next();
                self.toast(
                    if self.engine.repeat_one {
                        "Повтор трека: вкл"
                    } else {
                        "Повтор трека: выкл"
                    }
                    .into(),
                );
            }
            KeyCode::Char('s') => {
                self.engine.shuffle = !self.engine.shuffle;
                self.toast(
                    if self.engine.shuffle {
                        "Перемешивание: вкл"
                    } else {
                        "Перемешивание: выкл"
                    }
                    .into(),
                );
            }
            KeyCode::Char('Q') => self.engine.cycle_quality(),
            KeyCode::Char('B') => self.engine.restart_bit_perfect(),
            KeyCode::Char('N') => self.engine.toggle_normalize(),
            KeyCode::Char('P') => self.add_to_playlist(),
            KeyCode::Char('h') => self.open_history(),
            KeyCode::Char('C') => self.open_chart(),
            KeyCode::Char('M') if !self.engine.offline => self.open_metatags(),
            KeyCode::Char('F') => {
                if let Some((what, id, name)) = self.target_entity() {
                    self.engine.toggle_entity(what, id, name);
                }
            }
            KeyCode::Char('D') => {
                if let Some((Entity::ArtistLike, id, name)) = self.target_entity() {
                    self.engine.toggle_entity(Entity::ArtistDislike, id, name);
                }
            }
            KeyCode::Char('m') if !self.engine.offline => {
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
                    if let Loadable::Ready(v) = &mut self.engine.playlists
                        && let Some(p) = v.iter_mut().find(|p| p.kind == pl.kind)
                    {
                        p.visibility = Some(if public { "public" } else { "private" }.into());
                    }
                    self.engine.pl_op(PlOp::Visibility {
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
            KeyCode::Char('x') => self.engine.cycle_crossfade(),
            KeyCode::Char('R') => {
                self.engine.load_likes();
                self.engine.load_playlists();
                self.engine.load_wheel();
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
        let t = self.engine.now.as_ref().map(|n| &n.track).or(self.engine.loading_track.as_ref());
        match t {
            Some(t) => {
                let state = if self.engine.loading_track.is_some() {
                    "…"
                } else if self.engine.player.paused() {
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
            || self.engine.loading_track.is_some()
            || matches!(self.engine.screen, Screen::Connecting(_) | Screen::Login(_))
            || matches!(self.page().view, View::Wave)
            || self.stage_mode
            || self.lyrics_moving
            || (self.page().anim - self.page().offset as f32).abs() > 0.01;
        if heavy {
            Duration::from_secs_f64(1.0 / fps as f64)
        } else if self.engine.now.is_some() && !self.engine.player.paused() {
            Duration::from_millis(125)
        } else {
            Duration::from_millis(500)
        }
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

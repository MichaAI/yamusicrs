//! Состояние приложения и вся логика: авторизация, очередь, волна, реакции на события.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use rand::seq::SliceRandom;
use tachyonfx::{EffectManager, Interpolation, Motion, fx};
use tokio::sync::mpsc::UnboundedSender;

use crate::api::auth::{self, DeviceCode, Poll, StoredToken};
use crate::api::{self, AccountStatus, ApiError, ArtistBrief, Client, Playlist, Quality, SearchResult, Track};
use crate::audio::{self, Player, Token};
use crate::config::{Config, Paths, State};
use crate::fetch::{Fetcher, Origin};
use crate::library::Library;
use crate::lyrics::Lyrics;
use crate::mpris;
use crate::ui::kitty::Images;
use crate::ui::theme::{Palette, ThemeAnim};

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
    DeviceCode(Result<DeviceCode, String>),
    LoggedIn(Result<StoredToken, String>),
    Likes(Result<Vec<Track>, String>),
    Playlists(Result<Vec<Playlist>, String>),
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
    WaveMore(Result<api::RotorTracks, String>),
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
    },
    Audio(audio::Event),
    Mpris(mpris::Command),
    Toast(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Wave,
    Likes,
    Playlists,
    Search,
    Queue,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Wave, Tab::Likes, Tab::Playlists, Tab::Search, Tab::Queue];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Wave => "Волна",
            Tab::Likes => "Коллекция",
            Tab::Playlists => "Плейлисты",
            Tab::Search => "Поиск",
            Tab::Queue => "Очередь",
        }
    }
}

pub enum Loadable<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Loadable<T> {
    pub fn ready(&self) -> Option<&T> {
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
}

pub enum View {
    Wave,
    Likes,
    Playlists,
    Search,
    Queue,
    Tracks {
        id: u64,
        title: String,
        subtitle: String,
        cover: Option<String>,
        tracks: Loadable<Vec<Track>>,
    },
    Artist {
        id: u64,
        name: String,
        brief: Loadable<ArtistBrief>,
    },
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
    Connecting,
    Login(LoginState),
    Main,
}

pub struct WaveUi {
    pub loading: bool,
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

pub struct App {
    pub tx: UnboundedSender<Msg>,
    pub cfg: Config,
    paths: Paths,
    pub state: State,
    pub screen: Screen,
    client: Option<Client>,
    fetcher: Option<Fetcher>,
    pub library: Arc<Library>,
    pub player: Player,
    pub account: Option<AccountStatus>,
    pub tab: Tab,
    pub stack: Vec<Page>,
    pub liked: HashSet<String>,
    pub likes: Loadable<Vec<Track>>,
    pub playlists: Loadable<Vec<Playlist>>,
    pub search_input: String,
    pub search_editing: bool,
    pub search: Option<Loadable<Vec<SearchItem>>>,
    pub queue: Queue,
    pub wave: WaveUi,
    /// Фаза анимации блоба (быстрее, когда играет).
    pub wave_phase: f32,
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
    pub images: Images,
    pub gfx: crate::render::Gfx,
    /// Кнопки ⏮ ▶ ⏭ (в боковой панели): прямоугольник и действие 0/1/2.
    pub controls: Vec<(ratatui::layout::Rect, u8)>,
    pub theme: ThemeAnim,
    pub fx: EffectManager<&'static str>,
    covers_loading: HashSet<String>,
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
    pub quit: bool,
    /// Пользователь сам переключил трек — смена токена не «естественный» конец.
    user_switch: bool,
    error_streak: u32,
    view_seq: u64,
    pub started: Instant,
    /// Прямоугольники последнего кадра — для эффектов.
    pub areas: Areas,
    last_mpris_pos: Instant,
    last_session_save: Instant,
    last_stall: Option<Instant>,
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
    pub info: ratatui::layout::Rect,
    /// Обложка «сейчас играет» (боковая панель или сцена).
    pub cover: ratatui::layout::Rect,
    pub lyrics: ratatui::layout::Rect,
    /// Видимая область текущего списка (для мыши).
    pub list: ratatui::layout::Rect,
    pub progress: ratatui::layout::Rect,
    pub player: ratatui::layout::Rect,
    /// Вкладки в шапке: (x начала, x конца, вкладка).
    pub tabs: [(u16, u16); 5],
    pub main: ratatui::layout::Rect,
    pub header: ratatui::layout::Rect,
}

/// Через сколько ручная прокрутка текста возвращается к текущей строке.
const LYRICS_RETURN: Duration = Duration::from_secs(4);

fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl App {
    pub fn new(tx: UnboundedSender<Msg>, cfg: Config, paths: Paths) -> color_eyre::Result<Self> {
        let state = State::load(&paths);
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
            screen: Screen::Connecting,
            client: None,
            fetcher: None,
            library,
            player,
            account: None,
            tab: Tab::Wave,
            stack: vec![Page::new(View::Wave)],
            liked: HashSet::new(),
            likes: Loadable::Loading,
            playlists: Loadable::Loading,
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
                items: Loadable::Loading,
                arc: 0.0,
                active_seeds: None,
                active_name: None,
                hits: Vec::new(),
            },
            wave_phase: 0.0,
            now: None,
            requested: None,
            loading_track: None,
            tokens: HashMap::new(),
            next_token: 1,
            next_loaded: None,
            start_paused: None,
            pending_origins: HashMap::new(),
            images,
            gfx,
            controls: Vec::new(),
            theme: ThemeAnim::new(),
            fx: EffectManager::default(),
            covers_loading: HashSet::new(),
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
            quit: false,
            user_switch: false,
            error_streak: 0,
            view_seq: 0,
            started: Instant::now(),
            areas: Areas::default(),
            last_mpris_pos: Instant::now(),
            last_session_save: Instant::now(),
            last_stall: None,
            last_click: None,
            hover: None,
            mouse: (0, 0),
            focused: true,
            overlays: Vec::new(),
            osc_out: Vec::new(),
            pointer: false,
            kitty_text,
            sel_moved_at: Instant::now(),
            stored_token: None,
            cfg,
        };
        app.start_auth();
        Ok(app)
    }

    // ── авторизация ──────────────────────────────────────────────────────────

    fn start_auth(&mut self) {
        let Some(tok) = StoredToken::load(&self.paths.token_file) else {
            self.begin_login(None);
            return;
        };
        self.screen = Screen::Connecting;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Msg::Authed(verify(tok).await));
        });
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
        let fetcher = Fetcher::new(client.clone(), self.library.clone(), self.cfg.quality);
        if let Some(q) = self.state.quality {
            fetcher.set_quality(q);
        }
        self.fetcher = Some(fetcher);
        self.client = Some(client);
        if status.plus.as_ref().is_some_and(|p| !p.has_plus) {
            self.toast_err("Без Плюса будут только 30-секундные превью".into());
        }
        self.account = Some(status);
        self.screen = Screen::Main;
        self.fx.add_unique_effect(
            "startup",
            fx::coalesce((900, Interpolation::QuadOut))
                .with_area(screen_rect())
                .with_filter(not_image()),
        );
        self.load_likes();
        self.load_playlists();
        self.load_wheel();
        self.restore_session();
    }

    /// Восстанавливает очередь прошлого запуска и ставит трек на паузу с той же секунды.
    fn restore_session(&mut self) {
        let Some(r) = self.state.resume.clone().filter(|r| !r.ids.is_empty()) else {
            return;
        };
        self.spawn(move |c, tx| async move {
            let result = c.tracks(&r.ids).await.map_err(err_str);
            let _ = tx.send(Msg::Resumed { resume: r, result });
        });
    }

    /// Сессия на диск: при смене трека и периодически — переживёт и kill терминала.
    fn persist_session(&mut self) {
        self.save_session();
        self.state.save(&self.paths);
        self.last_session_save = Instant::now();
    }

    fn save_session(&mut self) {
        self.state.resume = (!self.queue.items.is_empty()).then(|| crate::config::Resume {
            title: self.queue.title.clone(),
            ids: self.queue.items.iter().map(Track::full_id).collect(),
            pos: self.queue.pos.unwrap_or(0),
            secs: self.now.as_ref().map_or(0.0, |n| n.pos),
            wave: match &self.queue.kind {
                QueueKind::Wave { session, batch } => Some((session.clone(), batch.clone())),
                QueueKind::List => None,
            },
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
    }

    fn load_playlists(&mut self) {
        self.playlists = Loadable::Loading;
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::Playlists(c.playlists().await.map_err(err_str)));
        });
    }

    fn new_view_id(&mut self) -> u64 {
        self.view_seq += 1;
        self.view_seq
    }

    fn open_playlist(&mut self, pl: Playlist) {
        let id = self.new_view_id();
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
        });
        if let Some(c) = pl.cover_uri() {
            let c = c.to_string();
            self.fetch_cover(&c);
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
            brief: Loadable::Loading,
        });
        self.spawn(move |c, tx| async move {
            let r = c.artist(&artist.id).await.map_err(err_str);
            let _ = tx.send(Msg::Artist { view_id: id, result: r });
        });
    }

    fn run_search(&mut self) {
        let q = self.search_input.trim().to_string();
        if q.is_empty() {
            return;
        }
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

    fn fetch_image(&mut self, key: String, uri: &str, size: u32, palette: bool) {
        if self.images.contains(&key) || !self.covers_loading.insert(key.clone()) {
            return;
        }
        let url = api::cover_url(uri, size);
        self.spawn(move |c, tx| async move {
            let r = async {
                let bytes = crate::fetch::fetch_bytes(&c, &url).await?;
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
        let (sync, text) = (t.has_sync_lyrics(), t.has_text_lyrics());
        if !sync && !text {
            return;
        }
        let id = t.id.clone();
        self.spawn(move |c, tx| async move {
            let lyrics = if sync {
                c.lyrics(&id, true).await.ok().map(|s| Lyrics::parse_lrc(&s))
            } else {
                c.lyrics(&id, false).await.ok().map(|s| Lyrics::plain(&s))
            };
            let _ = tx.send(Msg::Lyrics { track_id: id, lyrics });
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
        };
        self.stack = vec![Page::new(v)];
        if t == Tab::Queue {
            self.page_mut().sel = self.queue.pos.unwrap_or(0);
        }
        if t == Tab::Search && self.search.is_none() {
            self.search_editing = true;
        }
        self.view_fx();
    }

    fn view_fx(&mut self) {
        self.hover = None;
        let bg = self.theme.current().surface.color();
        self.fx.add_unique_effect(
            "view",
            fx::parallel(&[
                fx::fade_from_fg(bg, (260, Interpolation::QuadOut)),
                fx::sweep_in(Motion::UpToDown, 8, 0, bg, (220, Interpolation::QuadOut)),
            ])
            .with_area(self.areas.main)
            .with_filter(not_image()),
        );
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
            View::Playlists => self.playlists.ready().map_or(0, Vec::len),
            View::Search => match &self.search {
                Some(Loadable::Ready(items)) => items.len(),
                _ => 0,
            },
            View::Queue => self.queue.items.len(),
            View::Tracks { tracks, .. } => tracks.ready().map_or(0, Vec::len),
            View::Artist { brief, .. } => brief.ready().map_or(0, |b| artist_rows(b).len()),
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
        if matches!(p.view, View::Search) {
            // Заголовки секций пропускаем.
            sel = sel.clamp(0, len as isize - 1);
            p.sel = sel as usize;
            self.skip_headers(delta.signum());
            return;
        }
        p.sel = sel.clamp(0, len as isize - 1) as usize;
    }

    fn skip_headers(&mut self, dir: isize) {
        let Some(Loadable::Ready(items)) = &self.search else {
            return;
        };
        let n = items.len();
        let mut sel = self.page().sel;
        while matches!(items.get(sel), Some(SearchItem::Header(_))) {
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
                ArtistRow::Track(t) => Some(t),
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
            View::Playlists => {
                if let Some(pl) = self.playlists.ready().and_then(|p| p.get(sel)).cloned() {
                    self.open_playlist(pl);
                }
            }
            View::Queue => self.play_index(orig),
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
                    _ => {}
                }
            }
            View::Artist { brief, name, .. } => {
                let Some(b) = brief.ready() else { return };
                let rows = artist_rows(b);
                match rows.get(sel) {
                    Some(ArtistRow::Track(_)) => {
                        let tracks = b.popular_tracks.clone();
                        let idx = rows[..sel].iter().filter(|r| matches!(r, ArtistRow::Track(_))).count();
                        let name = name.clone();
                        self.play_list(tracks, idx, name);
                    }
                    Some(ArtistRow::Album(a)) => {
                        let a = (*a).clone();
                        self.open_album(a);
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
        self.queue = Queue {
            items,
            pos: None,
            kind,
            title,
            generation: self.queue.generation + 1,
        };
        self.next_loaded = None;
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
        let Some(token) = self.alloc_token(idx) else {
            return;
        };
        let Some(fetcher) = self.fetcher.clone() else {
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

    fn gain_for(&self, t: &Track) -> f32 {
        if !self.cfg.normalize {
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
        (from + 1 < self.queue.items.len()).then_some(from + 1)
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
        let Some(fetcher) = self.fetcher.clone() else {
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
            && p > 0
        {
            self.play_index(p - 1);
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
        let _ = self.mpris.send(mpris::Update::Status { playing: !p });
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
        f.set_quality(q);
        self.state.quality = Some(q);
        self.toast(format!("Качество: {} — для следующих треков", quality_label(q)));
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
            let _ = tx.send(Msg::Wheel(c.wheel().await.map_err(err_str)));
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
        if cur + 3 < self.queue.items.len() || self.wave.loading {
            return;
        }
        self.wave.loading = true;
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
            let _ = tx.send(Msg::WaveMore(c.rotor_tracks(&session, &queue).await.map_err(err_str)));
        });
    }

    fn wave_feedback(&self, kind: &'static str, t: &Track, played: Option<f64>) {
        let QueueKind::Wave { session, batch } = &self.queue.kind else {
            return;
        };
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
    fn report_end(&self, token: Token, played: f64, skipped: bool) {
        let Some((_, _, t)) = self.tokens.get(&token) else {
            return;
        };
        let full = played >= t.duration_secs() - 2.0;
        self.wave_feedback(if skipped && !full { "skip" } else { "trackFinished" }, t, Some(played));
        if self.cfg.report_plays && played > 1.0 {
            let t = t.clone();
            self.spawn(move |c, _| async move {
                let _ = c.play_audio(&t, played, "desktop_win-own_tracks-track-default").await;
            });
        }
    }

    // ── лайки ────────────────────────────────────────────────────────────────

    pub fn is_liked(&self, t: &Track) -> bool {
        self.liked.contains(&t.id)
    }

    fn toggle_like(&mut self, t: Track) {
        let like = !self.is_liked(&t);
        if like {
            self.liked.insert(t.id.clone());
            if let Loadable::Ready(v) = &mut self.likes {
                v.insert(0, t.clone());
            }
            self.wave_feedback("like", &t, None);
            self.toast(format!("♥ {}", t.full_title()));
        } else {
            self.liked.remove(&t.id);
            if let Loadable::Ready(v) = &mut self.likes {
                v.retain(|x| x.id != t.id);
            }
            self.toast(format!("♡ {}", t.full_title()));
        }
        self.refilter_keep();
        let id = t.full_id();
        self.spawn(move |c, tx| async move {
            if let Err(e) = c.set_like(&id, like).await {
                let _ = tx.send(Msg::Toast(format!("лайк не сохранился: {e}")));
            }
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
                    crate::ui::term::notify(
                        &mut self.osc_out,
                        &format!("yam-{token}"),
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
            Msg::Authed(Err(e)) => self.begin_login(Some(e)),
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
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(Msg::Authed(verify(tok).await));
                });
            }
            Msg::LoggedIn(Err(e)) => {
                if let Screen::Login(l) = &mut self.screen {
                    l.error = Some(e);
                }
            }
            Msg::Likes(r) => match r {
                Ok(ts) => {
                    self.liked = ts.iter().map(|t| t.id.clone()).collect();
                    self.likes = Loadable::Ready(ts);
                    self.refilter_keep();
                }
                Err(e) => self.likes = Loadable::Failed(e),
            },
            Msg::Playlists(r) => {
                self.playlists = match r {
                    Ok(p) => Loadable::Ready(p),
                    Err(e) => Loadable::Failed(e),
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
                        *brief = match result {
                            Ok(b) => Loadable::Ready(b),
                            Err(e) => Loadable::Failed(e),
                        };
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
            Msg::WaveMore(r) => {
                self.wave.loading = false;
                match r {
                    Ok(more) => {
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
            Msg::Lyrics { track_id, lyrics } => {
                if self.now.as_ref().is_some_and(|n| n.track.id == track_id)
                    && let Some(l) = lyrics
                {
                    self.lyrics = Some((track_id, l));
                }
            }
            Msg::Audio(ev) => self.on_audio(ev),
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
            },
            Msg::Toast(t) => self.toast_err(t),
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
                        self.player.enqueue_next(token, src, gain);
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
                    // Перезагрузка играющего трека: on_track_started не сработает, следующий ставим сами.
                    if self.now.as_ref().is_some_and(|n| n.token == token) {
                        self.schedule_next();
                    }
                } else {
                    src.cancel();
                }
            }
            Err(e) => {
                let name = track.map(|t| t.full_title()).unwrap_or_default();
                self.toast_err(format!("{name}: {e}"));
                if self.requested == Some(token) || self.next_loaded == Some(token) {
                    self.skip_broken(token);
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
        let Some(fetcher) = self.fetcher.clone() else {
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

    fn origins_insert(&mut self, token: Token, origin: Origin) {
        self.pending_origins.insert(token, origin);
    }

    fn skip_broken(&mut self, token: Token) {
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
                    None if self.wave.loading => {}
                    None => {
                        if let Some(now) = self.now.take() {
                            self.report_end(now.token, now.pos, false);
                        }
                        self.player.stop();
                        let _ = self.mpris.send(mpris::Update::Status { playing: false });
                    }
                }
            }
            audio::Event::SeekMiss(token, secs) => self.reload(token, secs),
            audio::Event::Error(token, e) => {
                let name = self.tokens.get(&token).map(|t| t.2.full_title()).unwrap_or_default();
                self.toast_err(format!("{name}: {e}"));
                if self.requested == Some(token) && self.now.as_ref().is_none_or(|n| n.token != token) {
                    self.skip_broken(token);
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
                n.info = pos.info;
            }
            _ => self.on_track_started(pos),
        }
        // Сторож вывода: звук есть, а PipeWire его не берёт — сообщаем и пробуем переподключиться.
        if self.player.stalled(Duration::from_secs(2))
            && self.last_stall.is_none_or(|t| t.elapsed() > Duration::from_secs(30))
        {
            self.last_stall = Some(Instant::now());
            tracing::warn!("PipeWire не забирает звук: пересоздаю поток");
            self.toast_err("Устройство вывода не отвечает — переподключаюсь".into());
            self.player.reconnect_output();
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

        if let Some(c) = track.cover_uri() {
            let c = c.to_string();
            self.fetch_cover(&c);
        }
        self.apply_theme_for(&track);
        self.fetch_lyrics(&track);
        self.wave_feedback("trackStarted", &track, None);
        self.notify_track(pos.token, &track);
        self.persist_session();
        let _ = self.mpris.send(mpris::Update::Track {
            id: track.full_id(),
            title: track.full_title(),
            artists: track.artists.iter().map(|a| a.name.clone()).collect(),
            album: track.album().map(|a| a.full_title()).unwrap_or_default(),
            length: track.duration_secs(),
            art: track.cover_uri().map(|u| api::cover_url(u, 400)),
        });
        let _ = self.mpris.send(mpris::Update::Status {
            playing: !self.player.paused(),
        });
        let bg = self.theme.current().surface.color();
        self.fx.add_unique_effect(
            "track",
            fx::parallel(&[
                fx::sweep_in(Motion::LeftToRight, 18, 4, bg, (700, Interpolation::QuadOut)),
                fx::fade_from_fg(bg, (500, Interpolation::SineOut)),
            ])
            .with_area(self.areas.info)
            .with_filter(not_image()),
        );
        self.schedule_next();
    }

    // ── клавиатура ───────────────────────────────────────────────────────────

    fn toggle_stage(&mut self) {
        self.stage_mode = !self.stage_mode;
        let bg = self.theme.current().surface.color();
        self.fx.add_unique_effect(
            "stage",
            fx::fade_from_fg(bg, (350, Interpolation::QuadOut))
                .with_area(screen_rect())
                .with_filter(not_image()),
        );
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
        let on_wave = matches!(self.page().view, View::Wave)
            && (self.wave.hits.iter().any(|(r, _)| inside(*r)) || self.controls.iter().any(|(r, _)| inside(*r)));
        let pointer = self.hover.is_some()
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
        let Some(token) = r.id.strip_prefix("yam-").and_then(|t| t.parse::<Token>().ok()) else {
            return;
        };
        if self.now.as_ref().is_none_or(|n| n.token != token) {
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
        // Экран волны: клик по пресету — выбрать (повторный — включить), кнопки ⏮ ▶ ⏭, колесо — по дуге.
        if matches!(self.page().view, View::Wave) && inside(self.areas.main) {
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
                if inside(self.areas.lyrics) {
                    self.lyrics_user_offset += if up { -1.0 } else { 1.0 };
                    self.lyrics_user_at = Some(Instant::now());
                } else if inside(self.areas.player) {
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
            Screen::Connecting => {
                if k.code == KeyCode::Char('q') {
                    self.quit = true;
                }
            }
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
                }
                KeyCode::Char(c) if k.modifiers.contains(KeyModifiers::CONTROL) && qwerty(c) == 'u' => {
                    self.search_input.clear()
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
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('1') => self.set_tab(Tab::Wave),
            KeyCode::Char('2') => self.set_tab(Tab::Likes),
            KeyCode::Char('3') => self.set_tab(Tab::Playlists),
            KeyCode::Char('4') => self.set_tab(Tab::Search),
            KeyCode::Char('5') => self.set_tab(Tab::Queue),
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
                    self.open_album(a);
                }
            }
            KeyCode::Char('i') => {
                if let Some(a) = self.target_track().and_then(|t| t.artists.first().cloned()) {
                    self.open_artist(a);
                }
            }
            KeyCode::Char('w') => {
                self.set_tab(Tab::Wave);
                self.start_wave(vec![MY_WAVE.into()], "Моя волна".into());
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
                let _ = self.mpris.send(mpris::Update::Repeat(self.repeat_one));
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
            KeyCode::Char('R') => {
                self.load_likes();
                self.load_playlists();
                self.load_wheel();
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
    pub fn frame_interval(&self, fps: u32) -> Duration {
        if !self.focused {
            return Duration::from_secs(1);
        }
        let heavy = self.fx.is_running()
            || self.theme.animating()
            || !self.toasts.is_empty()
            || self.loading_track.is_some()
            || matches!(self.screen, Screen::Connecting | Screen::Login(_))
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

/// Эффекты не должны трогать ячейки kitty-картинок: в их цвете закодирован id изображения.
fn not_image() -> tachyonfx::CellFilter {
    tachyonfx::CellFilter::eval_cell(|c: &ratatui::buffer::Cell| !c.symbol().starts_with('\u{10EEEE}'))
}

fn screen_rect() -> ratatui::layout::Rect {
    let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
    ratatui::layout::Rect::new(0, 0, w, h)
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

/// Проверка токена (с обновлением, если скоро истекает) и загрузка аккаунта.
async fn verify(mut tok: StoredToken) -> Result<(Client, AccountStatus, StoredToken), String> {
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
        Err(ApiError::Unauthorized) => Err("Токен недействителен, нужен вход".into()),
        Err(e) => Err(format!("Не удалось подключиться: {e}")),
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
    Track(&'a Track),
    Album(&'a api::Album),
}

pub fn artist_rows(b: &ArtistBrief) -> Vec<ArtistRow<'_>> {
    let mut v = Vec::new();
    if !b.popular_tracks.is_empty() {
        v.push(ArtistRow::Header("Популярные треки"));
        v.extend(b.popular_tracks.iter().map(ArtistRow::Track));
    }
    if !b.albums.is_empty() {
        v.push(ArtistRow::Header("Альбомы"));
        v.extend(b.albums.iter().map(ArtistRow::Album));
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

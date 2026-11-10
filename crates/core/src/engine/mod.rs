//! Движок плеера без UI: вход, сессия, очередь и токены треков, gapless и подмена на ходу,
//! лайки, Моя волна (в сети и офлайн), офлайн-снимок, правка плейлистов, Ynison.
//!
//! Интерфейс держит [`Engine`], передаёт ему ответы фоновых задач ([`Msg`]) и вызывает
//! методы по действиям пользователя, а после каждого вызова забирает [`Effect`]-ы
//! ([`Engine::take_effects`]) — что показать, какую обложку подгрузить, что сообщить системе.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::seq::SliceRandom;
use tokio::sync::mpsc::UnboundedSender;

use crate::api::auth::{self, DeviceCode, Poll, StoredToken};
use crate::api::{self, AccountStatus, ApiError, Client, Playlist, Quality, Track};
use crate::audio::{self, OutputFactory, Player, Token};
use crate::config::{Config, Paths, State};
use crate::fetch::{Fetcher, Origin};
use crate::library::Library;
use crate::offline::{self, Snapshot};
use crate::offline_wave;
use crate::tinyml::EventKind;
use crate::ynison;

mod smart;
mod yn;

pub use smart::SmartReport;

/// Ответы фоновых задач движка. Интерфейс получает их из канала, отданного в
/// [`Engine::new`], и возвращает в [`Engine::on_msg`].
pub enum Msg {
    /// Проход умного скачивания закончился.
    SmartDone(Result<SmartReport, String>),
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
    Audio(audio::Event),
    /// Источник получше для играющего трека (подмена на ходу); `None` — lossless нет.
    Upgraded {
        token: Token,
        /// Сами решили после старта трека (а не пользователь нажал Q) — неудачи без тостов.
        auto: bool,
        result: Result<Option<(audio::Source, Origin)>, String>,
    },
    /// Лайк не сохранился на сервере — вернуть как было.
    LikeFailed { track: Track, like: bool, error: String },
}

/// Действие, которое предлагает тост.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastAction {
    /// Продолжить восстановленную сессию (стоит на паузе).
    Resume,
    /// Перезапустить ресемплящийся трек в родном формате.
    RestartBitPerfect,
}

/// Что движок просит сделать интерфейс.
#[derive(Debug, Clone)]
pub enum Effect {
    /// `action` — что тост предлагает сделать: TUI подписывает клавишу, телефон рисует кнопку.
    Toast { text: String, error: bool, action: Option<ToastAction> },
    /// Подгрузить обложку заранее (трек открывается).
    Cover(String),
    /// Трек зазвучал (или встал на паузу в начале при восстановлении).
    TrackStarted { token: Token, track: Track, pos: f64 },
    /// Пауза/игра поменялась.
    Playback { playing: bool, pos: f64 },
    Seeked(f64),
    Volume(f32),
    /// Лайкнутые треки поменялись (множество или список).
    LikesChanged,
    /// Вход выполнен: главный экран; `was_offline` — до этого играли без сети.
    Connected { was_offline: bool },
    /// Пресеты волны пришли; `home` — «просто Моя волна».
    Wheel { home: usize },
    /// Серверная история поиска обновилась.
    Found,
    /// Подборки «Настроили для вас» обновились; `shift` — на сколько сдвинулся список.
    Personal { shift: isize },
    /// Правка плейлиста `kind` не прошла — показанное надо перечитать.
    PlaylistOpFailed { kind: String },
    /// Снимок узнал метаданные скачанных треков (обложки из файлов).
    LibraryMeta,
}

/// Что лайкаем/скрываем кроме треков.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    AlbumLike,
    ArtistLike,
    ArtistDislike,
}

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
            Loadable::Ready(v) => Some(v),
            _ => None,
        }
    }

    pub fn ready_mut(&mut self) -> Option<&mut T> {
        match self {
            Loadable::Ready(v) => Some(v),
            _ => None,
        }
    }
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
    /// Сиды волны «по этому списку» (плейлист, альбом, исполнитель); пусто — по последнему треку.
    pub seeds: Vec<String>,
}

pub struct Now {
    pub token: Token,
    pub track: Track,
    pub origin: Option<Origin>,
    pub info: audio::SourceInfo,
    pub pos: f64,
}

pub struct LoginState {
    pub code: Option<DeviceCode>,
    pub status: String,
    /// Вставленный вручную токен (поле ввода).
    pub input: String,
    pub error: Option<String>,
}

pub enum Screen {
    /// Подключение; `Some` — последняя ошибка сети, идут повторы.
    Connecting(Option<String>),
    Login(LoginState),
    Main,
}

pub struct WaveState {
    pub loading: bool,
    /// Идёт догрузка следующей пачки играющей волны.
    pub extending: bool,
    /// Персональные пресеты («колесо») с `wheel/new`.
    pub items: Loadable<Vec<api::WheelItem>>,
    /// Сиды и название играющей волны.
    pub active_seeds: Option<Vec<String>>,
    pub active_name: Option<String>,
}

/// Сиды «просто Моей волны».
pub const MY_WAVE: &str = "user:onyourwave";

pub const CROSSFADE_STEPS: [f32; 4] = [0.0, 3.0, 6.0, 10.0];

/// Сторож вывода: первая повторная попытка через 30 с, дальше вдвое реже, но не реже раза в 5 мин.
const STALL_FIRST: Duration = Duration::from_secs(30);
const STALL_MAX: Duration = Duration::from_secs(300);

pub struct Engine {
    pub tx: UnboundedSender<Msg>,
    pub cfg: Config,
    pub paths: Paths,
    pub state: State,
    pub screen: Screen,
    pub client: Option<Client>,
    pub fetcher: Option<Fetcher>,
    /// Нет сети: играем только из локальной библиотеки по снимку.
    pub offline: bool,
    pub snapshot: Snapshot,
    pub snapshot_path: std::path::PathBuf,
    /// Офлайн-волна: программа, идущая сессия и треки, из которых она выбирает.
    pub owave_store: Option<offline_wave::Store>,
    pub owave: Option<offline_wave::Session>,
    pub owave_tracks: HashMap<i32, Track>,
    pub owave_fetching: bool,
    /// Умное скачивание: идёт проход, когда последний раз проверяли, когда спросить сервер.
    smart_running: bool,
    smart_checked: Option<Instant>,
    smart: smart::SmartState,
    pub data_dir: std::path::PathBuf,
    pub library: Arc<Library>,
    pub player: Player,
    pub account: Option<AccountStatus>,
    pub liked: HashSet<String>,
    pub personal: Vec<api::PersonalPlaylist>,
    /// Серверная история поиска (найденное на любом устройстве).
    pub found: Vec<api::Found>,
    /// Лайкнутые альбомы и исполнители, скрытые исполнители (id из `library/all-ids`).
    pub liked_albums: HashSet<String>,
    pub liked_artists: HashSet<String>,
    pub disliked_artists: HashSet<String>,
    pub likes: Loadable<Vec<Track>>,
    pub playlists: Loadable<Vec<Playlist>>,
    pub queue: Queue,
    pub wave: WaveState,
    pub now: Option<Now>,
    /// Токен последней явной загрузки (чтобы игнорировать устаревшие ответы).
    pub requested: Option<Token>,
    pub loading_track: Option<Track>,
    /// Токен → (поколение очереди, индекс, трек).
    pub tokens: HashMap<Token, (u64, usize, Track)>,
    next_token: Token,
    pub next_loaded: Option<Token>,
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
    swap_toast: Option<(Token, String, Option<ToastAction>)>,
    pub repeat_one: bool,
    pub shuffle: bool,
    /// Очередь правок плейлистов (исполняются по одной).
    pl_ops: Option<UnboundedSender<PlOp>>,
    /// Пользователь сам переключил трек — смена токена не «естественный» конец.
    user_switch: bool,
    error_streak: u32,
    /// Ynison: соединение, последнее известное состояние и активны ли мы.
    pub ynison: Option<ynison::Handle>,
    pub yn: Option<ynison::Remote>,
    pub yn_active: bool,
    pub yn_connected: bool,
    /// Трек запущен по команде другого устройства — не перехватывать активность.
    yn_remote_start: bool,
    yn_last_sent: Instant,
    /// Поколение загрузки очереди с другого устройства (устаревшие ответы отбрасываются).
    yn_fetch_gen: u64,
    /// Название и тип устройства в «Слушать на…» (по умолчанию «yamusicrs · хост», WEB).
    pub yn_title: Option<String>,
    pub yn_kind: &'static str,
    /// Когда пришёл последний статус хаба (позиция чужого устройства тикает от него).
    yn_state_at: Instant,
    /// До какого момента сервер просил не подключаться (для подписи в настройках).
    yn_wait_until: Option<Instant>,
    last_session_save: Instant,
    last_stall: Option<Instant>,
    /// Пауза до следующей попытки оживить вывод: растёт, пока он не отвечает.
    stall_backoff: Duration,
    stored_token: Option<StoredToken>,
    /// Лайки и треки плейлистов пришли с сервера (а не из снимка): по ним можно решать,
    /// что из скачанного — не коллекция.
    likes_fresh: bool,
    playlists_fresh: bool,
    /// Треки, которые «не рекомендовать» в этом запуске (дизлайк снимает лайк и наоборот).
    pub disliked: HashSet<String>,
    /// Что включить, когда появится связь: нажали «играть» раньше, чем прошёл вход.
    play_when_online: bool,
    effects: Vec<Effect>,
}

impl Engine {
    /// Движок: звук через `output`, ответы фоновых задач — в `tx`. Сразу восстанавливает
    /// очередь прошлого запуска и начинает вход (или офлайн-режим, если `offline`).
    pub fn new(
        tx: UnboundedSender<Msg>,
        cfg: Config,
        paths: Paths,
        offline: bool,
        output: OutputFactory,
    ) -> color_eyre::Result<Self> {
        let state = State::load(&paths);
        audio::source::set_spill_dir(paths.cache_dir.clone());
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
        let player = Player::with_output(atx, volume, output)?;
        player.set_crossfade(state.crossfade.unwrap_or(cfg.crossfade_secs));
        player.set_eq(eq_params(&cfg));
        let library = Library::open(cfg.library_dir.clone(), paths.index_file.clone(), cfg.library_max_gb);
        let mut e = Self {
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
            smart_running: false,
            smart_checked: None,
            smart: smart::SmartState::load(&data_dir),
            data_dir,
            library,
            player,
            account: None,
            liked: HashSet::new(),
            personal: Vec::new(),
            found: Vec::new(),
            liked_albums: HashSet::new(),
            liked_artists: HashSet::new(),
            disliked_artists: HashSet::new(),
            likes: Loadable::Loading,
            playlists: Loadable::Loading,
            queue: Queue {
                items: Vec::new(),
                pos: None,
                kind: QueueKind::List,
                title: String::new(),
                generation: 0,
                seeds: Vec::new(),
            },
            wave: WaveState {
                loading: false,
                extending: false,
                items: Loadable::Loading,
                active_seeds: None,
                active_name: None,
            },
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
            repeat_one: false,
            shuffle: false,
            pl_ops: None,
            user_switch: false,
            error_streak: 0,
            ynison: None,
            yn: None,
            yn_active: false,
            yn_connected: false,
            yn_remote_start: false,
            yn_last_sent: Instant::now(),
            yn_fetch_gen: 0,
            yn_title: None,
            yn_kind: "ANDROID",
            yn_state_at: Instant::now(),
            yn_wait_until: None,
            last_session_save: Instant::now(),
            last_stall: None,
            stall_backoff: STALL_FIRST,
            stored_token: None,
            likes_fresh: false,
            playlists_fresh: false,
            disliked: HashSet::new(),
            play_when_online: false,
            effects: Vec::new(),
            cfg,
        };
        e.player.set_switch_fade(e.cfg.switch_fade_secs);
        e.library.set_stray_limit(e.cfg.cache_gb);
        e.resume_local();
        if offline || e.cfg.offline_mode {
            e.enter_offline();
        } else {
            e.start_auth();
            // Вход уже был и есть снимок коллекции: главный экран сразу, с прошлыми лайками и
            // плейлистами; сервер подтвердит токен и обновит их в фоне.
            if matches!(e.screen, Screen::Connecting(_)) && !e.snapshot.is_empty() {
                e.screen = Screen::Main;
                e.liked = e.snapshot.likes.iter().map(|t| t.id.clone()).collect();
                e.likes = Loadable::Ready(e.snapshot.likes.clone());
                e.playlists = Loadable::Ready(e.snapshot.playlists.clone());
                e.emit(Effect::LikesChanged);
            }
        }
        Ok(e)
    }

    /// Накопленные просьбы к интерфейсу (по порядку).
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn emit(&mut self, e: Effect) {
        self.effects.push(e);
    }

    pub fn toast(&mut self, text: String) {
        self.emit(Effect::Toast { text, error: false, action: None });
    }

    pub fn toast_with(&mut self, text: String, action: ToastAction) {
        self.emit(Effect::Toast { text, error: false, action: Some(action) });
    }

    pub fn toast_err(&mut self, text: String) {
        self.emit(Effect::Toast { text, error: true, action: None });
    }

    /// Остановить звук совсем (MPRIS Stop).
    pub fn stop(&mut self) {
        self.player.stop();
        self.now = None;
    }

    /// Поле ввода токена на экране входа.
    pub fn login_input(&mut self) -> Option<&mut String> {
        match &mut self.screen {
            Screen::Login(l) => Some(&mut l.input),
            _ => None,
        }
    }

    /// Целевая громкость нормализации, LUFS (сохраняется в конфиг).
    pub fn set_target_lufs(&mut self, v: f32) {
        if v != self.cfg.normalize_target_lufs {
            self.cfg.normalize_target_lufs = v;
            self.cfg.save(&self.paths);
            if self.normalize() {
                self.reapply_gain();
            }
        }
    }

    pub fn toggle_report_plays(&mut self) {
        self.cfg.report_plays = !self.cfg.report_plays;
        self.cfg.save(&self.paths);
    }

    pub fn toggle_ynison(&mut self) {
        self.cfg.ynison = !self.cfg.ynison;
        self.cfg.save(&self.paths);
        if self.cfg.ynison {
            self.ynison_start();
        } else {
            self.ynison_stop();
        }
    }

    /// Офлайн-режим из настроек: включённый рвёт связь с сервером и играет только скачанное,
    /// выключенный — подключается заново, не уходя с главного экрана.
    pub fn set_offline_mode(&mut self, on: bool) {
        if on == self.cfg.offline_mode {
            return;
        }
        self.cfg.offline_mode = on;
        self.cfg.save(&self.paths);
        if on {
            if !self.offline {
                self.ynison_stop();
                self.client = None;
                self.fetcher = None;
                self.pl_ops = None;
                self.enter_offline();
            }
        } else if self.offline {
            match StoredToken::load(&self.paths.token_file) {
                Some(tok) => {
                    self.toast("Подключаюсь…".into());
                    tokio::spawn(connect(tok, self.tx.clone()));
                }
                None => self.begin_login(None),
            }
        }
    }

    /// Снять пометки «lossless нет»: скачанное перепроверится при проигрывании.
    pub fn recheck_library(&mut self) {
        match self.library.reset_checks() {
            0 => self.toast("Все скачанные треки уже в lossless".into()),
            n => self.toast(format!("{n} тр. перепроверятся на lossless при проигрывании")),
        }
    }

    /// Вызывать часто (каждый кадр): синхронизирует «что играет» с аудиодвижком,
    /// сторожит вывод, сохраняет сессию, держит Ynison в курсе.
    pub fn tick(&mut self) {
        // Умное скачивание не зависит от того, играет ли что-то.
        self.smart_tick();
        let Some(pos) = self.player.position() else {
            return;
        };
        match &mut self.now {
            Some(n) if n.token == pos.token => {
                n.pos = pos.secs;
                let changed = n.info.codec != pos.info.codec;
                n.info = pos.info;
                if changed && let Some((_, text, action)) = self.swap_toast.take_if(|(t, ..)| *t == pos.token) {
                    self.emit(Effect::Toast { text, error: false, action });
                }
            }
            _ => self.on_track_started(pos),
        }
        // Сторож вывода: звук есть, а вывод его не берёт — сообщаем и пробуем переподключиться.
        if self.player.stalled(Duration::from_secs(2)) {
            if self.last_stall.is_none_or(|t| t.elapsed() > self.stall_backoff) {
                // Сообщаем один раз за эпизод, дальше пробуем тише и реже.
                if self.last_stall.is_none() {
                    self.toast_err("Устройство вывода не отвечает — переподключаюсь".into());
                }
                tracing::warn!("вывод не забирает звук: переподключаюсь (следующая попытка через {:?})", self.stall_backoff);
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
        self.yn_tick();
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            // Пока подключались, включили офлайн-режим — сеть не нужна.
            Msg::SmartDone(r) => self.on_smart_done(r),
            Msg::Authed(Ok(_)) if self.cfg.offline_mode => {}
            Msg::Authed(Ok((c, s, t))) => self.on_authed(c, s, t),
            Msg::Wheel(r) => match r {
                Ok(items) => {
                    // Курсор — на «Назад к привычному» (обычная Моя волна), как на вебе.
                    let home = items.iter().position(|i| i.seeds() == [MY_WAVE]).unwrap_or(0);
                    self.wave.items = Loadable::Ready(items);
                    self.emit(Effect::Wheel { home });
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
                        self.toast_with("Продолжаем с того же места".into(), ToastAction::Resume);
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
                } else if matches!(self.screen, Screen::Main) && self.client.is_none() && !self.offline {
                    // Главный экран показан заранее (по снимку), а сети так и нет.
                    self.play_when_online = false;
                    self.enter_offline();
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
                    self.likes_fresh = true;
                    self.sync_keep();
                    self.backfill_library_meta();
                    self.refresh_offline_program();
                    self.emit(Effect::LikesChanged);
                }
                // Обновление не прошло — показанное (из снимка или прошлого ответа) остаётся.
                Err(e) if self.likes.ready().is_some() => tracing::warn!("лайки не обновились: {e}"),
                Err(e) => self.likes = Loadable::Failed(e),
            },
            Msg::Ynison(ev) => self.on_ynison(ev),
            Msg::YnisonQueue {
                generation,
                queue,
                status,
                result,
            } => self.on_ynison_queue(generation, queue, status, result),
            Msg::Found(r) => match r {
                Ok(v) => {
                    self.found = v;
                    self.emit(Effect::Found);
                }
                Err(e) => tracing::warn!("search-history: {e}"),
            },
            Msg::Personal(r) => match r {
                Ok(v) => {
                    let shift = v.len() as isize - self.personal.len() as isize;
                    self.personal = v;
                    self.emit(Effect::Personal { shift });
                }
                Err(e) => tracing::warn!("personal-playlists: {e}"),
            },
            Msg::Playlists(r) => match r {
                Ok(p) => {
                    self.snapshot.playlists = p.clone();
                    self.sync_snapshot_playlists(&p);
                    self.playlists = Loadable::Ready(p);
                }
                Err(e) if self.playlists.ready().is_some() => tracing::warn!("плейлисты не обновились: {e}"),
                Err(e) => self.playlists = Loadable::Failed(e),
            },
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
                for t in ts {
                    self.snapshot.library_tracks.insert(t.id.clone(), t);
                }
                self.snapshot.save(self.snapshot_path.clone());
                self.emit(Effect::LibraryMeta);
            }
            Msg::PlaylistSnapshots(v) => {
                for (k, rev, ts) in v {
                    self.snapshot.playlist_tracks.insert(k, (rev, ts));
                }
                self.snapshot.save(self.snapshot_path.clone());
                self.playlists_fresh = true;
                self.sync_keep();
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
                        if let Some(kind) = kind {
                            self.emit(Effect::PlaylistOpFailed { kind });
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
            Msg::EntityFailed { what, id, on, error } => {
                self.apply_entity(what, &id, !on);
                self.toast_err(format!("не сохранилось: {error}"));
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
                            self.wave.active_name.clone().unwrap_or_else(|| "Моя волна".into()),
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
            Msg::Audio(ev) => self.on_audio(ev),
            Msg::Upgraded { token, auto, result } => self.on_upgraded(token, auto, result),
            Msg::LikeFailed { track, like, error } => {
                self.apply_like(&track, !like);
                self.toast_err(format!("Лайк не сохранился: {error}"));
            }
        }
    }

    pub fn start_auth(&mut self) {
        let Some(tok) = StoredToken::load(&self.paths.token_file) else {
            self.begin_login(None);
            return;
        };
        self.screen = Screen::Connecting(None);
        tokio::spawn(connect(tok, self.tx.clone()));
    }

    pub fn begin_login(&mut self, error: Option<String>) {
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

    pub fn poll_device(&self, code: DeviceCode) {
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

    pub fn submit_pasted_token(&mut self) {
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

    pub fn on_authed(&mut self, client: Client, status: AccountStatus, tok: StoredToken) {
        if let Err(e) = tok.save(&self.paths.token_file) {
            self.toast_err(format!("не удалось сохранить токен: {e}"));
        }
        self.stored_token = Some(tok);
        let was_offline = self.offline;
        if self.offline {
            self.offline = false;
            self.toast("Сеть вернулась — снова онлайн".into());
        }
        let fetcher = Fetcher::new(client.clone(), self.library.clone(), self.cfg.quality);
        fetcher.set_fast_start(self.cfg.fast_start);
        if let Some(q) = self.state.quality {
            fetcher.set_quality(q);
        }
        self.fetcher = Some(fetcher);
        self.sync_keep();
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
        self.emit(Effect::Connected { was_offline });
        self.load_likes();
        self.load_playlists();
        self.load_wheel();
        self.restore_session();
        self.ynison_start();
    }

    /// Восстанавливает очередь прошлого запуска и ставит трек на паузу с той же секунды.
    /// Очередь из queue.json — сразу при запуске, до входа. Скачанный трек загружается
    /// на паузу тут же, остальные откроются после входа без запроса метаданных.
    pub fn resume_local(&mut self) {
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

    pub fn restore_session(&mut self) {
        // Очередь уже из кеша: открыть трек (если ещё ждал сети) и подготовить следующий.
        if !self.queue.items.is_empty() && self.resume_partial.is_none() {
            let play = std::mem::take(&mut self.play_when_online);
            match self.resume_at.take() {
                Some((pos, secs)) if play => self.play_index_at(pos, secs, false),
                Some((pos, secs)) => {
                    self.play_index_at(pos, secs, true);
                    self.toast_with("Продолжаем с того же места".into(), ToastAction::Resume);
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
    pub fn fill_partial_queue(
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
    pub fn persist_session(&mut self) {
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
    pub fn partial_active(&self) -> bool {
        self.resume_partial.is_some_and(|(g, _)| g == self.queue.generation)
    }

    pub fn save_session(&mut self) {
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

    pub fn client(&self) -> Option<Client> {
        self.client.clone()
    }

    pub fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Client, UnboundedSender<Msg>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if let Some(c) = self.client() {
            tokio::spawn(f(c, self.tx.clone()));
        }
    }

    pub fn load_likes(&mut self) {
        // Уже показанное (снимок, прошлый ответ) не прячем за индикатором: обновится на месте.
        if self.likes.ready().is_none() {
            self.likes = Loadable::Loading;
        }
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

    pub fn load_playlists(&mut self) {
        if self.playlists.ready().is_none() {
            self.playlists = Loadable::Loading;
        }
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::Playlists(c.playlists().await.map_err(err_str)));
        });
        self.spawn(|c, tx| async move {
            let _ = tx.send(Msg::Personal(c.personal_playlists().await.map_err(err_str)));
        });
    }

    pub fn play_list(&mut self, mut tracks: Vec<Track>, start: usize, title: String) {
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

    pub fn replace_queue(&mut self, items: Vec<Track>, kind: QueueKind, title: String) {
        if !matches!(kind, QueueKind::OfflineWave) {
            self.owave = None;
        }
        self.queue = Queue {
            items,
            pos: None,
            kind,
            title,
            generation: self.queue.generation + 1,
            seeds: Vec::new(),
        };
        self.next_loaded = None;
        self.failed.clear();
    }

    pub fn alloc_token(&mut self, idx: usize) -> Option<Token> {
        let track = self.queue.items.get(idx)?.clone();
        let t = self.next_token;
        self.next_token += 1;
        self.tokens.insert(t, (self.queue.generation, idx, track));
        Some(t)
    }

    pub fn play_index(&mut self, idx: usize) {
        self.play_index_at(idx, 0.0, false);
    }

    pub fn play_index_at(&mut self, idx: usize, start: f64, paused: bool) {
        let Some(opener) = self.opener() else {
            // Вход ещё идёт (главный экран показан по снимку): запомним и включим, как появится связь.
            if let Some(t) = self.queue.items.get(idx).cloned() {
                self.queue.pos = Some(idx);
                self.loading_track = Some(t);
                self.resume_at = Some((idx, start));
                self.play_when_online = !paused;
            }
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
        // Заранее загруженный следующий больше не следующий: иначе, если текущий успеет
        // доиграть до загрузки выбранного, наплыв уйдёт в него.
        self.player.clear_next();
        self.queue.pos = Some(idx);
        let track = self.queue.items[idx].clone();
        self.loading_track = Some(track.clone());
        if let Some(c) = track.cover_uri() {
            self.emit(Effect::Cover(c.to_string()));
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

    pub fn normalize(&self) -> bool {
        self.state.normalize.unwrap_or(self.cfg.normalize)
    }

    pub fn crossfade(&self) -> f32 {
        self.state.crossfade.unwrap_or(self.cfg.crossfade_secs)
    }

    pub fn toggle_normalize(&mut self) {
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
    pub fn reapply_gain(&mut self) {
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

    pub fn cycle_crossfade(&mut self) {
        let cur = self.crossfade();
        let next = CROSSFADE_STEPS.iter().copied().find(|s| *s > cur + 0.01).unwrap_or(0.0);
        self.set_crossfade(next);
    }

    /// Применяет и сохраняет настройки эквалайзера из конфига.
    pub fn apply_eq(&mut self) {
        self.player.set_eq(eq_params(&self.cfg));
        self.cfg.save(&self.paths);
    }

    pub fn set_crossfade(&mut self, next: f32) {
        self.state.crossfade = Some(next);
        self.player.set_crossfade(next);
        self.toast(if next > 0.0 {
            format!("Плавный переход: {next:.0} с")
        } else {
            "Плавный переход: выкл (встык)".into()
        });
    }

    pub fn gain_for(&self, t: &Track) -> f32 {
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

    pub fn next_index(&self, from: usize) -> Option<usize> {
        if self.repeat_one {
            return Some(from);
        }
        // Недоступные (а без сети — и не скачанные) и не открывшиеся пропускаем.
        (from + 1..self.queue.items.len()).find(|&i| self.playable(i))
    }

    pub fn playable(&self, i: usize) -> bool {
        self.queue.items[i].available && !self.failed.contains(&(self.queue.generation, i))
    }

    pub fn mark_failed(&mut self, token: Token) {
        if let Some((generation, idx, _)) = self.tokens.get(&token) {
            self.failed.insert((*generation, *idx));
        }
    }

    /// Предзагруженный следующий не открылся: текущий не трогаем, готовим следующий за ним.
    pub fn next_failed(&mut self, token: Token) {
        self.mark_failed(token);
        self.next_loaded = None;
        self.schedule_next();
    }

    /// Подсказка рекомендателю из ответа ротора — для следующего старта офлайн-волны.
    pub fn keep_server_data(&mut self, data: Option<Vec<i32>>) {
        if let (Some(d), Some(store)) = (data.filter(|d| !d.is_empty()), self.owave_store.as_mut()) {
            store.server_data = d;
            store.save(&self.data_dir);
        }
    }

    /// Программа офлайн-волны под текущую библиотеку: перезапрос, если библиотека заметно
    /// изменилась или программа старая. Лайкнутые и остальные — разными списками, как в клиенте.
    pub fn refresh_offline_program(&mut self) {
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
    pub fn backfill_library_meta(&mut self) {
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
    pub fn start_offline_wave(&mut self) {
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
    pub fn extend_offline_wave(&mut self) {
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
    pub fn offline_tracks(&self, mut ts: Vec<Track>) -> Vec<Track> {
        for t in &mut ts {
            t.available &= self.library.contains(&t.id);
        }
        ts
    }

    /// Сети нет, но есть снимок — открываем библиотеку и играем скачанное.
    /// Подключение тем временем повторяется; когда пройдёт, приложение станет онлайн.
    pub fn enter_offline(&mut self) {
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
    pub fn sync_snapshot_playlists(&mut self, pls: &[Playlist]) {
        let keep: HashSet<String> = pls.iter().map(offline::key).collect();
        self.snapshot.playlist_tracks.retain(|k, _| keep.contains(k));
        let stale: Vec<Playlist> = pls.iter().filter(|p| self.snapshot.stale(p)).cloned().collect();
        if stale.is_empty() {
            self.snapshot.save(self.snapshot_path.clone());
            self.playlists_fresh = true;
            self.sync_keep();
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
    pub fn schedule_next(&mut self) {
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
                if self.wave_after_list() {
                    return;
                }
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

    /// Список доиграл: дальше волна по нему (если включено). `true` — волна запускается.
    fn wave_after_list(&mut self) -> bool {
        if !self.cfg.wave_after_list || self.offline || self.wave.loading || !matches!(self.queue.kind, QueueKind::List) {
            return false;
        }
        let seeds = if self.queue.seeds.is_empty() {
            let last = self.queue.pos.and_then(|p| self.queue.items.get(p)).or(self.queue.items.last());
            match last {
                Some(t) => vec![format!("track:{}", t.id)],
                None => return false,
            }
        } else {
            self.queue.seeds.clone()
        };
        let name = if self.queue.title.is_empty() { "Моя волна".to_string() } else { format!("Волна по «{}»", self.queue.title) };
        self.start_wave(seeds, name);
        true
    }

    pub fn prev(&mut self) {
        // Пока грузится выбранный трек, `now` — ещё прежний: его не перематываем, а листаем дальше назад.
        if self.loading_track.is_none() && self.now.as_ref().is_some_and(|n| n.pos > 4.0) {
            self.seek_to(0.0);
            return;
        }
        self.prev_track();
    }

    /// Предыдущий трек очереди без «сначала в начало текущего» — для свайпа.
    pub fn prev_track(&mut self) {
        if let Some(p) = self.queue.pos
            && let Some(i) = (0..p).rev().find(|&i| self.playable(i))
        {
            self.play_index(i);
        }
    }

    pub fn toggle_pause(&mut self) {
        // Трек ждёт связи (вход ещё идёт): «играть» значит «включи, как подключишься».
        if self.now.is_none() && self.resume_at.is_some() && self.opener().is_none() {
            self.play_when_online = !self.play_when_online;
            return;
        }
        if self.now.is_none() && self.loading_track.is_none() {
            if let Some(p) = self.queue.pos.or((!self.queue.items.is_empty()).then_some(0)) {
                self.play_index(p);
            }
            return;
        }
        let p = !self.player.paused();
        self.player.set_paused(p);
        let pos = self.now.as_ref().map_or(0.0, |n| n.pos);
        self.emit(Effect::Playback { playing: !p, pos });
        self.yn_local_play_state();
    }

    pub fn seek_to(&mut self, secs: f64) {
        let Some(now) = &mut self.now else { return };
        let dur = now.track.duration_secs();
        let s = if dur > 0.0 {
            secs.clamp(0.0, dur - 0.5)
        } else {
            secs.max(0.0)
        };
        self.player.seek(now.token, s);
        now.pos = s;
        self.emit(Effect::Seeked(s));
        self.yn_publish_status();
    }

    pub fn seek_by(&mut self, d: f64) {
        if let Some(p) = self.now.as_ref().map(|n| n.pos) {
            self.seek_to(p + d);
        }
    }

    pub fn set_volume(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.player.set_volume(v);
        self.state.volume = Some(v);
        self.emit(Effect::Volume(v));
    }

    pub fn cycle_quality(&mut self) {
        let Some(f) = &self.fetcher else { return };
        let q = match f.quality() {
            Quality::Lossless => Quality::Auto,
            Quality::Auto => Quality::Hq,
            Quality::Hq => Quality::Nq,
            Quality::Nq => Quality::Lossless,
        };
        self.set_quality(q);
    }

    pub fn set_quality(&mut self, q: Quality) {
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

    /// Играющий трек не lossless — попросить FLAC и подменить его на ходу, без паузы.
    pub fn hot_upgrade(&mut self, auto: bool) {
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

    pub fn on_upgraded(&mut self, token: Token, auto: bool, result: Result<Option<(audio::Source, Origin)>, String>) {
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

    pub fn on_swapped(&mut self, token: Token, result: Result<audio::SourceInfo, String>) {
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
                let (text, action) = match info.resampled_to {
                    Some(to) => (format!("{fmt} без паузы, ресемплинг в {}", khz(to)), Some(ToastAction::RestartBitPerfect)),
                    None => (format!("{fmt} без паузы"), None),
                };
                self.swap_toast = Some((token, text, action));
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
    pub fn restart_bit_perfect(&mut self) {
        let Some(n) = &self.now else { return };
        if n.info.resampled_to.is_none() {
            return self.toast("Звук и так bit-perfect".into());
        }
        let (token, pos) = (n.token, n.pos);
        self.reload(token, pos);
        self.toast("Перезапуск в родном формате".into());
    }

    pub fn enqueue(&mut self, t: Track, play_next: bool) {
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

    pub fn load_wheel(&mut self) {
        self.spawn(|c, tx| async move {
            let (wheel, last) = tokio::join!(c.wheel(), c.wave_last());
            // Последняя волна по исполнителю, альбому, треку или плейлисту, если её нет среди
            // пресетов, — первой на дуге, как «продолжить» в десктопе. Служебные («Встряхнуть»,
            // настройки характера) сюда не берём: их сиды каждый раз новые, а иконки у них нет.
            let last = last.ok().filter(|l| {
                let [seed] = l.seeds.as_slice() else { return false };
                ["artist:", "album:", "track:", "playlist:"].iter().any(|p| seed.starts_with(p))
            });
            // Иконка — фото исполнителя или обложка альбома/трека.
            let icon = match last.as_ref().map(|l| l.seeds[0].split_once(':').unwrap_or_default()) {
                Some(("artist", id)) => c.artist_photo(id).await.ok().flatten(),
                Some(("album", id)) => c.album(id).await.ok().and_then(|a| a.cover_uri().map(str::to_string)),
                Some(("track", id)) => c
                    .tracks(&[id.to_string()])
                    .await
                    .ok()
                    .and_then(|v| v.first().and_then(|t| t.cover_uri().map(str::to_string))),
                _ => None,
            };
            let r = wheel.map(|mut items| {
                if let Some(last) = last
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
                                agent: icon.map(|uri| api::WheelAgent {
                                    cover: Some(api::AgentCover { uri: Some(uri), color: None }),
                                }),
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

    pub fn start_wave(&mut self, seeds: Vec<String>, name: String) {
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

    pub fn maybe_extend_wave(&mut self, cur: usize) {
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

    pub fn wave_feedback(&mut self, kind: &'static str, t: &Track, played: Option<f64>) {
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
    pub fn report_end(&mut self, token: Token, played: f64, skipped: bool) {
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

    pub fn is_own(&self, pl: &Playlist) -> bool {
        self.client.as_ref().is_some_and(|c| c.uid == pl.uid)
    }

    pub fn pl_op(&self, op: PlOp) {
        if let Some(tx) = &self.pl_ops {
            let _ = tx.send(op);
        }
    }

    pub fn is_liked(&self, t: &Track) -> bool {
        self.liked.contains(&t.id)
    }

    pub fn toggle_like(&mut self, t: Track) {
        if self.client.is_none() {
            return self.toast_err("Без сети лайк не сохранить".into());
        }
        let like = !self.is_liked(&t);
        self.apply_like(&t, like);
        if like {
            // Лайк снимает «не рекомендовать» (сервер делает то же сам).
            self.disliked.remove(&t.id);
            if let Some(f) = self.fetcher.clone() {
                let liked = t.clone();
                tokio::spawn(async move { f.promote(liked).await });
            }
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
    pub fn apply_like(&mut self, t: &Track, like: bool) {
        if let Some(f) = &self.fetcher {
            f.keep_one(&t.id, like);
        }
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
        self.emit(Effect::LikesChanged);
    }

    pub fn entity_set(&mut self, what: Entity) -> &mut HashSet<String> {
        match what {
            Entity::AlbumLike => &mut self.liked_albums,
            Entity::ArtistLike => &mut self.liked_artists,
            Entity::ArtistDislike => &mut self.disliked_artists,
        }
    }

    /// Лайк альбома/исполнителя или «не рекомендовать исполнителя» — переключатель.
    pub fn toggle_entity(&mut self, what: Entity, id: String, name: String) {
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

    pub fn apply_entity(&mut self, what: Entity, id: &str, on: bool) {
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

    pub fn dislike_current(&mut self) {
        let Some(t) = self.now.as_ref().map(|n| n.track.clone()) else {
            return;
        };
        let id = t.full_id();
        self.wave_feedback("dislike", &t, None);
        // «Не рекомендовать» и «нравится» исключают друг друга: лайк снимается и здесь, и на сервере.
        let was_liked = self.is_liked(&t);
        if was_liked {
            self.apply_like(&t, false);
        }
        self.disliked.insert(t.id.clone());
        self.spawn(move |c, _| async move {
            if was_liked && let Err(e) = c.set_like(&id, false).await {
                tracing::warn!("лайк при дизлайке не снялся: {e}");
            }
            if let Err(e) = c.dislike(&id).await {
                tracing::warn!("дизлайк не сохранился: {e}");
            }
        });
        self.toast(format!("Не рекомендовать: {}", t.full_title()));
        self.next();
    }

    pub fn on_opened(&mut self, token: Token, next: bool, start: f64, result: Result<(audio::Source, Origin), String>) {
        let track = self.tokens.get(&token).map(|t| t.2.clone());
        match result {
            Ok((src, origin)) => {
                let gain = track.as_ref().map(|t| self.gain_for(t)).unwrap_or(1.0);
                if next {
                    if self.next_loaded == Some(token) {
                        self.origins_insert(token, origin);
                        // Наплыв — между любыми треками (кроме повтора одного).
                        self.player.enqueue_next(token, src, gain, !self.repeat_one);
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
    pub fn reload(&mut self, token: Token, secs: f64) {
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
    pub fn token_relevant(&self, token: Token) -> bool {
        self.requested == Some(token)
            || self.next_loaded == Some(token)
            || self.now.as_ref().is_some_and(|n| n.token == token)
    }

    pub fn origins_insert(&mut self, token: Token, origin: Origin) {
        self.pending_origins.insert(token, origin);
    }

    pub fn skip_broken(&mut self, token: Token) {
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

    pub fn on_audio(&mut self, ev: audio::Event) {
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
                        self.wave_after_list();
                        if let Some(now) = self.now.take() {
                            self.report_end(now.token, now.pos, false);
                        }
                        self.player.stop();
                        self.emit(Effect::Playback {
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

    pub fn on_track_started(&mut self, pos: audio::Position) {
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

        self.wave_feedback("trackStarted", &track, None);
        if self.library.contains(&track.id) {
            self.snapshot.library_tracks.insert(track.id.clone(), track.clone());
        }
        self.extend_offline_wave();
        self.persist_session();
        self.emit(Effect::TrackStarted {
            token: pos.token,
            track: track.clone(),
            pos: pos.secs,
        });
        self.yn_track_started();
        self.schedule_next();
        self.warm_ahead(idx);
        // Скачанный хуже lossless: играет сразу, а FLAC подменит его на ходу, если он есть.
        // Так же и быстрый старт из сети: начали с AAC — просим lossless.
        let origin = self.now.as_ref().and_then(|n| n.origin.as_ref());
        let lossy = self.now.as_ref().is_some_and(|n| !matches!(n.info.codec.as_str(), "FLAC" | "ALAC"));
        if let Some(f) = &self.fetcher
            && (matches!(origin, Some(Origin::Library)) && f.should_upgrade(&track)
                || matches!(origin, Some(Origin::Network { .. }))
                    && lossy
                    && f.fast_start()
                    && f.quality() == Quality::Lossless
                    && !self.library.checked_no_lossless(&track.id))
        {
            self.hot_upgrade(true);
        }
    }

    /// Сохранять ли в библиотеку всё сыгранное (иначе — только коллекцию, остальное в кеш).
    pub fn keep_played(&self) -> bool {
        self.cfg.keep_played.unwrap_or(true)
    }

    pub fn set_keep_played(&mut self, on: bool) {
        self.cfg.keep_played = Some(on);
        self.cfg.save(&self.paths);
        self.sync_keep();
    }

    /// Наплыв при ручной смене трека, сек (0 — резко).
    pub fn set_switch_fade(&mut self, secs: f32) {
        self.cfg.switch_fade_secs = secs.clamp(0.0, 3.0);
        self.player.set_switch_fade(self.cfg.switch_fade_secs);
        self.cfg.save(&self.paths);
    }

    /// Коллекция (лайки и треки своих плейлистов) — загрузчику и библиотеке: что из сыгранного
    /// сохранять насовсем, а что держать кешем. Скачанное раньше, но не из коллекции, уходит
    /// в кеш, только когда и лайки, и плейлисты подтверждены сервером.
    pub fn sync_keep(&mut self) {
        let all = self.keep_played();
        let mut ids: HashSet<String> = self.liked.clone();
        for (_, ts) in self.snapshot.playlist_tracks.values() {
            ids.extend(ts.iter().map(|t| t.id.clone()));
        }
        if let Some(f) = &self.fetcher {
            f.set_keep(all, ids.clone());
        }
        let flags = self.library.stray_flags();
        // Свои, лежащие в кеше, — в библиотеку (с тегами, если знаем метаданные).
        let promote: Vec<String> = flags.iter().filter(|(id, stray)| *stray && (all || ids.contains(id))).map(|(id, _)| id.clone()).collect();
        if !promote.is_empty() {
            let known: Vec<Track> = promote.iter().filter_map(|id| self.snapshot.find(id).cloned()).collect();
            let unknown: Vec<String> = promote.iter().filter(|id| self.snapshot.find(id).is_none()).cloned().collect();
            self.library.set_stray(&unknown, false);
            if let Some(f) = self.fetcher.clone() {
                tokio::spawn(async move {
                    for t in known {
                        f.promote(t).await;
                    }
                });
            }
        }
        if !all && self.likes_fresh && self.playlists_fresh {
            let demote: Vec<String> = flags.into_iter().filter(|(id, stray)| !stray && !ids.contains(id)).map(|(id, _)| id).collect();
            if !demote.is_empty() {
                let n = self.library.set_stray(&demote, true);
                tracing::info!("{n} сыгранных треков не из коллекции — теперь кеш");
            }
        }
    }

    /// Перечитать с сервера всё, что показано: лайки, плейлисты, подборки, пресеты волны.
    pub fn refresh(&mut self) {
        if self.client.is_none() {
            return;
        }
        self.load_likes();
        self.load_playlists();
        self.load_wheel();
    }

    /// Прогреть следующие треки очереди после `cur` (ссылка и начало файла).
    pub fn warm_ahead(&self, cur: usize) {
        let n = self.cfg.warm_ahead;
        if n == 0 || self.offline {
            return;
        }
        let Some(f) = &self.fetcher else { return };
        let ahead: Vec<Track> = self.queue.items.iter().skip(cur + 1).take(n).filter(|t| t.available).cloned().collect();
        f.warm(&ahead);
    }

    pub fn quality(&self) -> Quality {
        self.fetcher.as_ref().map(|f| f.quality()).unwrap_or(self.cfg.quality)
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

/// «44.1 кГц» / «96 кГц».
fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} кГц", rate / 1000)
    } else {
        format!("{:.1} кГц", rate as f32 / 1000.0)
    }
}

pub fn quality_label(q: Quality) -> &'static str {
    match q {
        Quality::Lossless => "Lossless",
        Quality::Hq => "Высокое",
        Quality::Auto => "Авто",
        Quality::Nq => "Экономия",
    }
}

pub fn err_str(e: impl std::fmt::Display) -> String {
    e.to_string()
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

enum VerifyError {
    /// Токен отвергнут (401) — нужен вход.
    Invalid(String),
    /// Сеть, таймаут, 5xx — токен ни при чём, стоит повторить.
    Transient(String),
}

fn eq_params(cfg: &Config) -> crate::audio::dsp::EqParams {
    let n = cfg.eq_gains.len().clamp(3, 31);
    crate::audio::dsp::EqParams {
        enabled: cfg.eq_enabled,
        freqs: crate::audio::dsp::eq_freqs(n),
        gains: cfg.eq_gains.iter().take(n).map(|g| g.clamp(-12.0, 12.0)).collect(),
    }
}

//! Мост ядра yamusicrs в Android: UniFFI-объект [`Core`].
//!
//! Движок живёт в своём потоке с рантаймом tokio. Kotlin шлёт команды методами `Core`,
//! а состояние получает колбэками [`CoreListener`]: снимок плеера (не чаще ~4 раз в секунду,
//! если меняется только позиция), список «Мне нравится», очередь и тосты.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use yamusic_core::api::{self, Track};
use yamusic_core::config::{Config, Paths};
use yamusic_core::engine::{self, Effect, Engine, Loadable, MY_WAVE, Screen};

#[cfg(target_os = "android")]
mod aaudio;
#[cfg(target_os = "android")]
mod logcat;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct TrackItem {
    pub id: String,
    pub title: String,
    pub artists: String,
    pub album: String,
    /// Обложка 400×400 (https), если есть.
    pub cover: Option<String>,
    /// Миниатюра 200×200 для списков.
    pub thumb: Option<String>,
    pub duration_ms: u64,
    pub available: bool,
}

impl From<&Track> for TrackItem {
    fn from(t: &Track) -> Self {
        Self {
            id: t.id.clone(),
            title: t.full_title(),
            artists: t.artist_names(),
            album: t.album().map(|a| a.full_title()).unwrap_or_default(),
            cover: t.cover_uri().map(|u| api::cover_url(u, 400)),
            thumb: t.cover_uri().map(|u| api::cover_url(u, 200)),
            duration_ms: t.duration_ms.unwrap_or(0),
            available: t.available,
        }
    }
}

/// Пресет «колеса» Моей волны.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct WavePreset {
    pub name: String,
    pub description: Option<String>,
    /// Иконка агента (https, 200×200).
    pub icon: Option<String>,
    /// Цвет агента 0xRRGGBB.
    pub color: Option<u32>,
    pub seeds: Vec<String>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LyricLine {
    /// Начало строки, мс (для обычного текста — 0).
    pub time_ms: u64,
    pub text: String,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LyricsData {
    pub synced: bool,
    pub lines: Vec<LyricLine>,
}

#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum Phase {
    /// Подключение; `error` — последняя ошибка сети, идут повторы.
    Connecting { error: Option<String> },
    /// Вход: код для ya.ru/device или вставка токена.
    Login {
        code: Option<String>,
        url: Option<String>,
        status: String,
        error: Option<String>,
    },
    Main,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct PlayerState {
    pub phase: Phase,
    /// Нет сети: играет только скачанное.
    pub offline: bool,
    /// Что звучит (или открывается, если `loading`).
    pub track: Option<TrackItem>,
    pub loading: bool,
    pub paused: bool,
    pub position_ms: u64,
    pub liked: bool,
    /// «FLAC 24/96», «AAC 256»… — пусто, пока не играет.
    pub format: String,
    pub queue_title: String,
    pub queue_pos: Option<u32>,
    pub queue_len: u32,
    pub wave: bool,
    /// Сиды и название играющей волны (пусто — играет не волна).
    pub wave_seeds: Vec<String>,
    pub wave_name: String,
    /// Волна запускается.
    pub wave_loading: bool,
    pub likes_loading: bool,
}

#[uniffi::export(with_foreign)]
pub trait CoreListener: Send + Sync {
    fn on_state(&self, state: PlayerState);
    fn on_likes(&self, likes: Vec<TrackItem>);
    fn on_queue(&self, title: String, items: Vec<TrackItem>);
    fn on_toast(&self, text: String, error: bool);
    fn on_wheel(&self, presets: Vec<WavePreset>);
    /// Текст играющего трека `track_id`; `None` — текста нет.
    fn on_lyrics(&self, track_id: String, lyrics: Option<LyricsData>);
}

enum Cmd {
    PlayPause,
    Play,
    Pause,
    Next,
    Prev,
    Seek(u64),
    PlayLikes(u32),
    PlayQueue(u32),
    ToggleLike,
    Dislike,
    Wave,
    WaveSeeds(Vec<String>, String),
    SubmitToken(String),
    NewCode,
    Shutdown,
}

#[derive(uniffi::Object)]
pub struct Core {
    tx: UnboundedSender<Cmd>,
}

#[uniffi::export]
impl Core {
    /// `files_dir` — токен, состояние, снимок; `cache_dir` — обложки и временные файлы;
    /// `music_dir` — скачанные треки.
    #[uniffi::constructor]
    pub fn new(files_dir: String, cache_dir: String, music_dir: String, listener: Arc<dyn CoreListener>) -> Arc<Self> {
        init_logging();
        let (tx, rx) = unbounded_channel();
        let files = PathBuf::from(files_dir);
        let paths = Paths {
            config_file: files.join("config.toml"),
            token_file: files.join("token.json"),
            index_file: files.join("library.json"),
            state_file: files.join("state.json"),
            log_dir: files.join("logs"),
            cache_dir: PathBuf::from(cache_dir),
        };
        let music = PathBuf::from(music_dir);
        std::thread::Builder::new()
            .name("engine".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio");
                rt.block_on(run(paths, music, rx, listener));
                // Фоновые задачи (загрузки, Ynison) не держат выход.
                rt.shutdown_timeout(Duration::from_millis(200));
            })
            .expect("engine thread");
        Arc::new(Self { tx })
    }

    pub fn play_pause(&self) {
        self.send(Cmd::PlayPause);
    }

    pub fn play(&self) {
        self.send(Cmd::Play);
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }

    pub fn next(&self) {
        self.send(Cmd::Next);
    }

    pub fn prev(&self) {
        self.send(Cmd::Prev);
    }

    pub fn seek(&self, position_ms: u64) {
        self.send(Cmd::Seek(position_ms));
    }

    /// Играть «Мне нравится» с трека `index` (по порядку из `on_likes`).
    pub fn play_likes(&self, index: u32) {
        self.send(Cmd::PlayLikes(index));
    }

    /// Перейти к треку очереди `index` (по порядку из `on_queue`).
    pub fn play_queue(&self, index: u32) {
        self.send(Cmd::PlayQueue(index));
    }

    pub fn toggle_like(&self) {
        self.send(Cmd::ToggleLike);
    }

    /// «Не рекомендовать» играющий и сразу следующий.
    pub fn dislike(&self) {
        self.send(Cmd::Dislike);
    }

    /// Моя волна (без сети — офлайн-волна по скачанному).
    pub fn start_wave(&self) {
        self.send(Cmd::Wave);
    }

    /// Волна по сидам пресета (`on_wheel`).
    pub fn start_wave_with(&self, seeds: Vec<String>, name: String) {
        self.send(Cmd::WaveSeeds(seeds, name));
    }

    pub fn submit_token(&self, token: String) {
        self.send(Cmd::SubmitToken(token));
    }

    /// Новый код для входа (старый истёк).
    pub fn new_login_code(&self) {
        self.send(Cmd::NewCode);
    }

    /// Сохранить сессию и остановить звук (приложение закрывается).
    pub fn shutdown(&self) {
        self.send(Cmd::Shutdown);
    }
}

impl Core {
    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

fn init_logging() {
    #[cfg(target_os = "android")]
    {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("info,yamusic_core=debug,yamusic_android=debug"))
            .with_writer(logcat::Logcat)
            .with_ansi(false)
            .without_time()
            .try_init();
    }
}

#[cfg(target_os = "android")]
fn output() -> yamusic_core::audio::OutputFactory {
    Box::new(aaudio::spawn)
}

/// Вне Android вывода нет (крейт собирается на хосте только ради генерации привязок).
#[cfg(not(target_os = "android"))]
fn output() -> yamusic_core::audio::OutputFactory {
    Box::new(|_| Err(color_eyre::eyre::eyre!("yamusic-android работает только на Android")))
}

async fn run(paths: Paths, music: PathBuf, mut rx: UnboundedReceiver<Cmd>, listener: Arc<dyn CoreListener>) {
    let mut cfg = Config::load(&paths);
    cfg.library_dir = music;
    // Ynison на телефоне пока не включаем: здесь и так официальный клиент рядом.
    cfg.ynison = false;
    let (etx, mut erx) = unbounded_channel();
    let mut engine = match Engine::new(etx, cfg, paths, false, output()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("движок не запустился: {e:#}");
            listener.on_toast(format!("Движок не запустился: {e:#}"), true);
            return;
        }
    };
    let mut publisher = Publisher::default();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            c = rx.recv() => match c {
                None | Some(Cmd::Shutdown) => {
                    engine.shutdown();
                    return;
                }
                Some(c) => command(&mut engine, c),
            },
            Some(m) = erx.recv() => engine.on_msg(m),
            _ = tick.tick() => {}
        }
        engine.tick();
        for e in engine.take_effects() {
            match e {
                Effect::Toast { text, error } => listener.on_toast(text, error),
                Effect::LikesChanged => publisher.likes_dirty = true,
                Effect::Wheel { .. } => {
                    if let Loadable::Ready(items) = &engine.wave.items {
                        listener.on_wheel(items.iter().map(preset).collect());
                    }
                }
                Effect::TrackStarted { track, .. } => load_lyrics(&engine, track, listener.clone()),
                _ => {}
            }
        }
        publisher.publish(&engine, listener.as_ref());
    }
}

fn preset(i: &api::WheelItem) -> WavePreset {
    WavePreset {
        name: i.name().to_string(),
        description: i.data.wave.as_ref().and_then(|w| w.description.clone()),
        icon: i.icon().map(|u| api::cover_url(u, 200)),
        color: i.color().map(|(r, g, b)| (r as u32) << 16 | (g as u32) << 8 | b as u32),
        seeds: i.seeds(),
    }
}

/// Текст зазвучавшего трека — в фоне; ответ уходит в Kotlin сам.
fn load_lyrics(e: &Engine, t: Track, l: Arc<dyn CoreListener>) {
    let (sync, text) = (t.has_sync_lyrics(), t.has_text_lyrics());
    if !sync && !text {
        return l.on_lyrics(t.id, None);
    }
    let (client, file) = (e.client.clone(), e.library.file(&t.id));
    tokio::spawn(async move {
        let (lyrics, _) = yamusic_core::lyrics::load(client, file, t.id.clone(), sync).await;
        let data = lyrics.map(|l| match l {
            yamusic_core::lyrics::Lyrics::Synced(v) => LyricsData {
                synced: true,
                lines: v
                    .into_iter()
                    .map(|x| LyricLine {
                        time_ms: (x.at * 1000.0) as u64,
                        text: x.text,
                    })
                    .collect(),
            },
            yamusic_core::lyrics::Lyrics::Plain(v) => LyricsData {
                synced: false,
                lines: v.into_iter().map(|text| LyricLine { time_ms: 0, text }).collect(),
            },
        });
        l.on_lyrics(t.id, data);
    });
}

fn command(e: &mut Engine, c: Cmd) {
    let idle = e.now.is_none() && e.loading_track.is_none();
    match c {
        Cmd::PlayPause => e.toggle_pause(),
        Cmd::Play => {
            if idle || e.player.paused() {
                e.toggle_pause();
            }
        }
        Cmd::Pause => {
            if !idle && !e.player.paused() {
                e.toggle_pause();
            }
        }
        Cmd::Next => e.next(),
        Cmd::Prev => e.prev(),
        Cmd::Seek(ms) => e.seek_to(ms as f64 / 1000.0),
        Cmd::PlayLikes(i) => {
            if let Loadable::Ready(v) = &e.likes {
                let v = v.clone();
                e.play_list(v, i as usize, "Мне нравится".into());
            }
        }
        Cmd::PlayQueue(i) => {
            if (i as usize) < e.queue.items.len() {
                e.play_index(i as usize);
            }
        }
        Cmd::ToggleLike => {
            if let Some(t) = e.now.as_ref().map(|n| n.track.clone()) {
                e.toggle_like(t);
            }
        }
        Cmd::Dislike => e.dislike_current(),
        Cmd::Wave => {
            if e.offline {
                e.start_offline_wave();
            } else {
                e.start_wave(vec![MY_WAVE.into()], "Моя волна".into());
            }
        }
        Cmd::WaveSeeds(seeds, name) => e.start_wave(seeds, name),
        Cmd::SubmitToken(t) => {
            if let Some(input) = e.login_input() {
                *input = t;
                e.submit_pasted_token();
            }
        }
        Cmd::NewCode => e.begin_login(None),
        Cmd::Shutdown => {}
    }
}

/// Шлёт в Kotlin только изменившееся.
#[derive(Default)]
struct Publisher {
    last: Option<PlayerState>,
    last_at: Option<Instant>,
    likes_dirty: bool,
    likes_sent: bool,
    queue_mark: Option<(u64, usize)>,
}

impl Publisher {
    fn publish(&mut self, e: &Engine, l: &dyn CoreListener) {
        let state = snapshot(e);
        // Позиция меняется каждый тик — её одну шлём не чаще 4 раз в секунду.
        let same_but_pos = self
            .last
            .as_ref()
            .is_some_and(|p| PlayerState { position_ms: state.position_ms, ..p.clone() } == state);
        let due = self.last_at.is_none_or(|t| t.elapsed() >= Duration::from_millis(250));
        if self.last.as_ref() != Some(&state) && (!same_but_pos || due) {
            l.on_state(state.clone());
            self.last = Some(state);
            self.last_at = Some(Instant::now());
        }
        if let Loadable::Ready(v) = &e.likes
            && (self.likes_dirty || !self.likes_sent)
        {
            l.on_likes(v.iter().map(TrackItem::from).collect());
            self.likes_dirty = false;
            self.likes_sent = true;
        }
        let mark = (e.queue.generation, e.queue.items.len());
        if self.queue_mark != Some(mark) {
            self.queue_mark = Some(mark);
            l.on_queue(e.queue.title.clone(), e.queue.items.iter().map(TrackItem::from).collect());
        }
    }
}

fn snapshot(e: &Engine) -> PlayerState {
    let phase = match &e.screen {
        Screen::Connecting(err) => Phase::Connecting { error: err.clone() },
        Screen::Login(l) => Phase::Login {
            code: l.code.as_ref().map(|c| c.user_code.clone()),
            url: l.code.as_ref().map(|c| c.verification_url.clone()),
            status: l.status.clone(),
            error: l.error.clone(),
        },
        Screen::Main => Phase::Main,
    };
    let track = e.now.as_ref().map(|n| &n.track).or(e.loading_track.as_ref());
    let format = e
        .now
        .as_ref()
        .filter(|n| n.info.rate > 0)
        .map(|n| match n.info.bits {
            Some(b) => format!("{} {b}/{}", n.info.codec, khz(n.info.rate)),
            None => format!("{} {}", n.info.codec, khz(n.info.rate)),
        })
        .unwrap_or_default();
    PlayerState {
        phase,
        offline: e.offline,
        track: track.map(TrackItem::from),
        loading: e.loading_track.is_some(),
        paused: e.player.paused() || (e.now.is_none() && e.loading_track.is_none()),
        position_ms: e.now.as_ref().map_or(0, |n| (n.pos * 1000.0) as u64),
        liked: track.is_some_and(|t| e.is_liked(t)),
        format,
        queue_title: e.queue.title.clone(),
        queue_pos: e.queue.pos.map(|p| p as u32),
        queue_len: e.queue.items.len() as u32,
        wave: matches!(e.queue.kind, engine::QueueKind::Wave { .. } | engine::QueueKind::OfflineWave),
        wave_seeds: match e.queue.kind {
            engine::QueueKind::Wave { .. } => e.wave.active_seeds.clone().unwrap_or_default(),
            _ => Vec::new(),
        },
        wave_name: e.wave.active_name.clone().unwrap_or_default(),
        wave_loading: e.wave.loading,
        likes_loading: matches!(e.likes, Loadable::Loading),
    }
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{}", rate / 1000)
    } else {
        format!("{:.1}", rate as f32 / 1000.0)
    }
}

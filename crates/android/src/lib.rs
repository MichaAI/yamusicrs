//! Мост ядра yamusicrs в Android: UniFFI-объект [`Core`].
//!
//! Движок живёт в своём потоке с рантаймом tokio. Kotlin шлёт команды методами `Core`,
//! а состояние получает колбэками [`CoreListener`]: снимок плеера (не чаще ~4 раз в секунду,
//! если меняется только позиция), список «Мне нравится», очередь и тосты.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use yamusic_core::api::{self, Track};
use yamusic_core::config::{Config, Paths};
use yamusic_core::engine::{self, Effect, Engine, Loadable, MY_WAVE, Screen};

pub use pages::{AlbumItem, ArtistItem, Page, PlaylistItem, Source};
use pages::{Pages, Reply, Req};

#[cfg(target_os = "android")]
mod aaudio;
#[cfg(target_os = "android")]
mod logcat;
mod pages;

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
    /// Для перехода к альбому и исполнителю (первому с id).
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub explicit: bool,
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
            album_id: t.album().map(|a| a.id.clone()).filter(|a| !a.is_empty()),
            artist_id: t.artists.iter().map(|a| a.id.clone()).find(|a| !a.is_empty()),
            explicit: t.explicit(),
        }
    }
}

/// Исполнитель играющего трека для главного экрана волны: фото и ближайший концерт.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct ArtistCard {
    pub artist_id: String,
    pub name: String,
    pub photo: Option<String>,
    /// «15 ноября, 19:00, Москва».
    pub concert: Option<String>,
    /// Билеты на Афише.
    pub concert_url: Option<String>,
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

/// Устройство аккаунта в Ynison («Слушать на…»), кроме нас.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct DeviceItem {
    pub id: String,
    pub title: String,
    /// `ANDROID`, `IOS`, `WEB`, `SMART_SPEAKER`…
    pub kind: String,
    pub active: bool,
    pub can_play: bool,
}

/// Что играет на другом устройстве (мы — пульт).
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct RemoteNow {
    pub device: String,
    pub title: String,
    pub cover: Option<String>,
    pub paused: bool,
    /// Позиция на момент последнего ответа хаба; дальше тикает сама, если не на паузе.
    pub position_ms: u64,
    pub duration_ms: u64,
    pub has_prev: bool,
    pub has_next: bool,
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
    /// Ynison: другие устройства и что играет на активном из них.
    pub devices: Vec<DeviceItem>,
    pub remote: Option<RemoteNow>,
}

/// Настройки звука и отчётов (как вкладка «Настройки» в TUI).
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct SettingsData {
    /// `lossless`, `hq` или `nq`.
    pub quality: String,
    pub normalize: bool,
    /// Целевая громкость выравнивания, LUFS.
    pub target_lufs: f32,
    /// Плавный переход, с (0 — встык).
    pub crossfade_secs: f32,
    pub report_plays: bool,
    /// Быстрый старт: из сети сначала AAC, lossless подменяет на ходу; следующие треки прогреваются.
    pub fast_start: bool,
    /// Ynison: видно в «Слушать на…», можно управлять другими устройствами.
    pub ynison: bool,
    /// Состояние Ynison для подписи («играет здесь», «Яндекс не пускает…»).
    pub ynison_status: String,
    /// Недавние поисковые запросы, свежие первыми.
    pub search_history: Vec<String>,
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
    /// Ответ на `open_*`, `search`, `releases`, `chart` — по id, который они вернули.
    fn on_page(&self, id: u64, page: Page);
    /// Свои плейлисты и «Настроили для вас».
    fn on_playlists(&self, own: Vec<PlaylistItem>, personal: Vec<PlaylistItem>);
    fn on_settings(&self, settings: SettingsData);
    /// Карточка исполнителя играющего трека (приходит после `on_state` с этим треком).
    fn on_artist(&self, card: ArtistCard);
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
    Open(u64, Req),
    ClosePage(u64),
    PlayFrom(Source, u32),
    EnqueueFrom(Source, u32, bool),
    LikeFrom(Source, u32),
    Quality(String),
    Normalize(bool),
    TargetLufs(f32),
    Crossfade(f32),
    ReportPlays(bool),
    FastStart(bool),
    Ynison(bool),
    Transfer(String),
    RemotePlayPause,
    RemoteSkip(i64),
    RemoteSeek(u64),
    ForgetSearch(String),
    Shutdown,
}

#[derive(uniffi::Object)]
pub struct Core {
    tx: UnboundedSender<Cmd>,
    next_page: AtomicU64,
}

#[uniffi::export]
impl Core {
    /// `files_dir` — токен, состояние, снимок; `cache_dir` — обложки и временные файлы;
    /// `music_dir` — скачанные треки.
    #[uniffi::constructor]
    /// `device_name` — модель телефона, для списка «Слушать на…» в Ynison.
    pub fn new(
        files_dir: String,
        cache_dir: String,
        music_dir: String,
        device_name: String,
        listener: Arc<dyn CoreListener>,
    ) -> Arc<Self> {
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
                rt.block_on(run(paths, music, device_name, rx, listener));
                // Фоновые задачи (загрузки, Ynison) не держат выход.
                rt.shutdown_timeout(Duration::from_millis(200));
            })
            .expect("engine thread");
        Arc::new(Self {
            tx,
            next_page: AtomicU64::new(1),
        })
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

    /// Плейлист (свой, «Настроили для вас», найденный).
    pub fn open_playlist(&self, uid: String, kind: String) -> u64 {
        self.open(Req::Playlist { uid, kind })
    }

    pub fn open_album(&self, id: String) -> u64 {
        self.open(Req::Album(id))
    }

    pub fn open_artist(&self, id: String) -> u64 {
        self.open(Req::Artist(id))
    }

    /// «Похожие треки»; `title` — для заголовка страницы.
    pub fn open_similar(&self, track_id: String, title: String) -> u64 {
        self.open(Req::Similar { track_id, title })
    }

    /// Поиск; запрос запоминается в истории.
    pub fn search(&self, query: String) -> u64 {
        self.open(Req::Search(query))
    }

    /// Новые релизы (страница альбомов).
    pub fn releases(&self) -> u64 {
        self.open(Req::Releases)
    }

    pub fn chart(&self) -> u64 {
        self.open(Req::Chart)
    }

    /// Страница закрыта — её треки больше не нужны.
    pub fn close_page(&self, id: u64) {
        self.send(Cmd::ClosePage(id));
    }

    /// Играть список `source` с трека `index` (очередь — перейти к треку).
    pub fn play_from(&self, source: Source, index: u32) {
        self.send(Cmd::PlayFrom(source, index));
    }

    /// В очередь: `next` — следующим, иначе в конец.
    pub fn enqueue_from(&self, source: Source, index: u32, next: bool) {
        self.send(Cmd::EnqueueFrom(source, index, next));
    }

    /// Лайк или снять лайк.
    pub fn like_from(&self, source: Source, index: u32) {
        self.send(Cmd::LikeFrom(source, index));
    }

    /// `lossless`, `hq` или `nq`.
    pub fn set_quality(&self, quality: String) {
        self.send(Cmd::Quality(quality));
    }

    pub fn set_normalize(&self, on: bool) {
        self.send(Cmd::Normalize(on));
    }

    pub fn set_target_lufs(&self, lufs: f32) {
        self.send(Cmd::TargetLufs(lufs));
    }

    pub fn set_crossfade(&self, secs: f32) {
        self.send(Cmd::Crossfade(secs));
    }

    pub fn set_report_plays(&self, on: bool) {
        self.send(Cmd::ReportPlays(on));
    }

    pub fn set_fast_start(&self, on: bool) {
        self.send(Cmd::FastStart(on));
    }

    pub fn set_ynison(&self, on: bool) {
        self.send(Cmd::Ynison(on));
    }

    /// Перевести воспроизведение на устройство; пустая строка — сюда.
    pub fn transfer(&self, device_id: String) {
        self.send(Cmd::Transfer(device_id));
    }

    /// Пульт: пауза/игра на активном устройстве.
    pub fn remote_play_pause(&self) {
        self.send(Cmd::RemotePlayPause);
    }

    pub fn remote_next(&self) {
        self.send(Cmd::RemoteSkip(1));
    }

    pub fn remote_prev(&self) {
        self.send(Cmd::RemoteSkip(-1));
    }

    pub fn remote_seek(&self, position_ms: u64) {
        self.send(Cmd::RemoteSeek(position_ms));
    }

    pub fn forget_search(&self, query: String) {
        self.send(Cmd::ForgetSearch(query));
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

    fn open(&self, req: Req) -> u64 {
        let id = self.next_page.fetch_add(1, Ordering::Relaxed);
        self.send(Cmd::Open(id, req));
        id
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

async fn run(paths: Paths, music: PathBuf, device_name: String, mut rx: UnboundedReceiver<Cmd>, listener: Arc<dyn CoreListener>) {
    let mut cfg = Config::load(&paths);
    cfg.library_dir = music;
    // На телефоне Ynison — только по явному включению в настройках (рядом и так официальное
    // приложение, а лишние попытки при отказе сервера продлевают его go-away).
    cfg.ynison = ynison_flag(&paths).exists();
    let (etx, mut erx) = unbounded_channel();
    let mut engine = match Engine::new(etx, cfg, paths, false, output()) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("движок не запустился: {e:#}");
            listener.on_toast(format!("Движок не запустился: {e:#}"), true);
            return;
        }
    };
    engine.yn_kind = "ANDROID";
    engine.yn_title = Some(format!("yamusicrs · {device_name}"));
    let mut publisher = Publisher::default();
    let mut pages = Pages::default();
    // Карточки исполнителей: один запрос на исполнителя за запуск.
    let mut artist_cache: std::collections::HashMap<String, std::sync::Arc<parking_lot::Mutex<Option<ArtistCard>>>> =
        std::collections::HashMap::new();
    let (ptx, mut prx) = unbounded_channel::<Reply>();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            c = rx.recv() => match c {
                None | Some(Cmd::Shutdown) => {
                    engine.shutdown();
                    return;
                }
                Some(c) => command(&mut engine, &mut pages, &ptx, c),
            },
            Some(m) = erx.recv() => engine.on_msg(m),
            Some((id, r)) = prx.recv() => {
                tracing::debug!("страница {id}: {}", if r.is_ok() { "готова" } else { "ошибка" });
                listener.on_page(id, pages.accept(id, r));
            }
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
                Effect::TrackStarted { track, .. } => {
                    load_artist(&engine, &track, &mut artist_cache, listener.clone());
                    load_lyrics(&engine, track, listener.clone());
                }
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

/// Фото и ближайший концерт первого исполнителя трека — в фоне, с кешем по исполнителю.
fn load_artist(
    e: &Engine,
    t: &Track,
    cache: &mut std::collections::HashMap<String, Arc<parking_lot::Mutex<Option<ArtistCard>>>>,
    l: Arc<dyn CoreListener>,
) {
    let Some(a) = t.artists.iter().find(|a| !a.id.is_empty()) else { return };
    if let Some(slot) = cache.get(&a.id) {
        if let Some(card) = slot.lock().clone() {
            l.on_artist(card);
        }
        return;
    }
    let Some(c) = e.client() else { return };
    let slot = Arc::new(parking_lot::Mutex::new(None));
    cache.insert(a.id.clone(), slot.clone());
    let (id, name) = (a.id.clone(), a.name.clone());
    tokio::spawn(async move {
        let (info, concerts) = match c.artist_card(&id).await {
            Ok(r) => r,
            Err(err) => return tracing::debug!("карточка исполнителя {id}: {err}"),
        };
        let now = chrono::Utc::now();
        let next = concerts
            .iter()
            .filter_map(|c| Some((chrono::DateTime::parse_from_rfc3339(c.datetime.as_deref()?).ok()?, c)))
            .filter(|(d, _)| *d > now)
            .min_by_key(|(d, _)| *d);
        let card = ArtistCard {
            artist_id: id,
            name,
            photo: info.as_ref().and_then(|i| i.cover_uri()).map(|u| api::cover_url(u, 1000)),
            concert: next.map(|(d, c)| {
                const M: [&str; 12] = [
                    "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября",
                    "ноября", "декабря",
                ];
                use chrono::{Datelike, Timelike};
                let mut s = format!("{} {}, {}:{:02}", d.day(), M[d.month0() as usize], d.hour(), d.minute());
                if let Some(city) = c.city.as_deref().filter(|x| !x.is_empty()) {
                    s.push_str(", ");
                    s.push_str(city);
                }
                s
            }),
            concert_url: next.and_then(|(_, c)| c.afisha_url.clone()),
        };
        *slot.lock() = Some(card.clone());
        l.on_artist(card);
    });
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

/// Трек источника по индексу (с названием списка для очереди).
fn source_tracks<'a>(e: &'a Engine, p: &'a Pages, s: &Source) -> Option<(String, &'a [Track])> {
    match s {
        Source::Likes => e.likes.ready().map(|v| ("Мне нравится".to_string(), v.as_slice())),
        Source::Queue => Some((e.queue.title.clone(), e.queue.items.as_slice())),
        Source::Page { id } => p.tracks.get(id).map(|(t, v)| (t.clone(), v.as_slice())),
    }
}

fn command(e: &mut Engine, pages: &mut Pages, ptx: &UnboundedSender<Reply>, c: Cmd) {
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
        Cmd::Open(id, req) => {
            if let Req::Search(q) = &req {
                let q = q.trim().to_string();
                let h = &mut e.state.search_history;
                h.retain(|x| x.to_lowercase() != q.to_lowercase());
                h.insert(0, q);
                h.truncate(30);
                e.state.save(&e.paths);
            }
            tracing::debug!("открываю страницу {id}");
            pages::open(e, id, req, ptx);
        }
        Cmd::ClosePage(id) => {
            pages.tracks.remove(&id);
        }
        Cmd::PlayFrom(Source::Queue, i) => {
            if (i as usize) < e.queue.items.len() {
                e.play_index(i as usize);
            }
        }
        Cmd::PlayFrom(s, i) => {
            if let Some((title, v)) = source_tracks(e, pages, &s) {
                let v = v.to_vec();
                e.play_list(v, i as usize, title);
            }
        }
        Cmd::EnqueueFrom(s, i, next) => {
            if let Some(t) = source_tracks(e, pages, &s).and_then(|(_, v)| v.get(i as usize).cloned()) {
                e.enqueue(t, next);
            }
        }
        Cmd::LikeFrom(s, i) => {
            if let Some(t) = source_tracks(e, pages, &s).and_then(|(_, v)| v.get(i as usize).cloned()) {
                e.toggle_like(t);
            }
        }
        Cmd::Quality(q) => {
            let q = match q.as_str() {
                "hq" => api::Quality::Hq,
                "nq" => api::Quality::Nq,
                _ => api::Quality::Lossless,
            };
            if q != e.quality() {
                e.set_quality(q);
            }
        }
        Cmd::Normalize(on) => {
            if on != e.normalize() {
                e.toggle_normalize();
            }
        }
        Cmd::TargetLufs(v) => e.set_target_lufs(v),
        Cmd::Crossfade(v) => {
            if (v - e.crossfade()).abs() > 0.01 {
                e.set_crossfade(v);
            }
        }
        Cmd::ReportPlays(on) => {
            if on != e.cfg.report_plays {
                e.toggle_report_plays();
            }
        }
        Cmd::FastStart(on) => {
            e.cfg.fast_start = on;
            e.cfg.save(&e.paths);
            if let Some(f) = &e.fetcher {
                f.set_fast_start(on);
            }
        }
        Cmd::Ynison(on) => {
            let flag = ynison_flag(&e.paths);
            let _ = if on { std::fs::write(&flag, b"1") } else { std::fs::remove_file(&flag) };
            if on != e.cfg.ynison {
                e.toggle_ynison();
            }
        }
        Cmd::Transfer(id) => {
            let id = if id.is_empty() {
                e.ynison.as_ref().map(|h| h.device_id.clone()).unwrap_or_default()
            } else {
                id
            };
            e.yn_transfer(&id);
        }
        Cmd::RemotePlayPause => e.yn_remote_toggle(),
        Cmd::RemoteSkip(d) => e.yn_remote_skip(d),
        Cmd::RemoteSeek(ms) => e.yn_remote_seek(ms),
        Cmd::ForgetSearch(q) => {
            e.state.search_history.retain(|h| h != &q);
            e.state.save(&e.paths);
        }
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
    playlists_mark: Option<(usize, u64, usize)>,
    settings: Option<SettingsData>,
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
        let pls = match &e.playlists {
            Loadable::Ready(v) => v.as_slice(),
            _ => e.snapshot.playlists.as_slice(),
        };
        let mark = (
            pls.len(),
            pls.iter().map(|p| p.revision as u64 + p.track_count.unwrap_or(0) as u64).sum(),
            e.personal.len(),
        );
        if self.playlists_mark != Some(mark) {
            self.playlists_mark = Some(mark);
            let (own, personal) = pages::playlists(e);
            l.on_playlists(own, personal);
        }
        let settings = SettingsData {
            quality: match e.quality() {
                api::Quality::Lossless => "lossless",
                api::Quality::Hq => "hq",
                api::Quality::Nq => "nq",
            }
            .into(),
            normalize: e.normalize(),
            target_lufs: e.cfg.normalize_target_lufs,
            crossfade_secs: e.crossfade(),
            report_plays: e.cfg.report_plays,
            fast_start: e.cfg.fast_start,
            ynison: e.cfg.ynison,
            ynison_status: e.ynison_label(),
            search_history: e.state.search_history.clone(),
        };
        if self.settings.as_ref() != Some(&settings) {
            l.on_settings(settings.clone());
            self.settings = Some(settings);
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
        devices: e
            .yn_devices()
            .into_iter()
            .map(|(id, title, kind, active, can_play)| DeviceItem { id, title, kind, active, can_play })
            .collect(),
        remote: e.yn_remote().and_then(|(device, q, st)| {
            let p = usize::try_from(q.index).ok().and_then(|i| q.items.get(i))?;
            Some(RemoteNow {
                device,
                title: p.title.clone(),
                cover: p.cover.as_deref().map(|u| api::cover_url(u, 400)),
                paused: st.paused,
                position_ms: st.progress_ms,
                duration_ms: st.duration_ms,
                has_prev: q.index > 0,
                has_next: (q.index as usize) + 1 < q.items.len(),
            })
        }),
    }
}

fn ynison_flag(p: &Paths) -> PathBuf {
    p.token_file.with_file_name("ynison-on")
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{}", rate / 1000)
    } else {
        format!("{:.1}", rate as f32 / 1000.0)
    }
}

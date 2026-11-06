use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::Quality;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Желаемое качество: lossless | hq | nq.
    pub quality: Quality,
    /// Корень локальной библиотеки (кеша).
    pub library_dir: PathBuf,
    /// Лимит размера библиотеки в ГиБ, 0 — без лимита. При превышении удаляются давно не игранные треки.
    pub library_max_gb: f64,
    /// Громкость при старте, 0..=1.
    pub volume: f32,
    /// Нормализация громкости по R128 из метаданных Яндекса (ломает bit-perfect).
    /// Переключается клавишей N — выбор запоминается в state.json и перекрывает конфиг.
    pub normalize: bool,
    /// Целевая громкость нормализации, LUFS.
    pub normalize_target_lufs: f32,
    /// Кроссфейд между треками, сек; 0 — встык. Клавиша x перебирает 0/3/6/10.
    pub crossfade_secs: f32,
    /// Рисовать обложки через kitty graphics protocol (auto определяется по $TERM).
    pub kitty_images: Option<bool>,
    /// Рисовать блоб на видеокарте (иначе — CPU).
    pub gpu: bool,
    /// Дизеринг блоба против «колец» на градиентах: сила в ступенях 8 бит, 0 — выключен.
    pub blob_dither: u32,
    /// Частота кадров анимаций.
    pub fps: u32,
    /// Отправлять историю прослушиваний в Яндекс.
    pub report_plays: bool,
    /// Ynison: телефон видит yamusicrs в «Слушать на…» и управляет им; включённое здесь
    /// забирает воспроизведение у других устройств.
    pub ynison: bool,
    /// Быстрый старт: трек из сети начинает играть в AAC, lossless подменяет его на ходу;
    /// следующие треки очереди прогреваются заранее.
    pub fast_start: bool,
    /// Сколько следующих треков прогревать.
    pub warm_ahead: usize,
    /// Список (плейлист, альбом…) доиграл — дальше волна по нему, а не тишина.
    pub wave_after_list: bool,
    /// Офлайн по своей воле: без сети, только скачанное (как флаг --offline, но запоминается).
    pub offline_mode: bool,
    /// В шапке альбома — полная дата выхода (день и месяц), а не только год.
    pub full_release_date: bool,
    /// В «Плейлистах» первыми показывать «Мне нравится» и «Скачанное».
    pub playlist_shortcuts: bool,
    /// Умное скачивание: сервер подбирает треки для офлайн-волны, они докачиваются сами.
    pub smart_download: bool,
    /// Качество умного скачивания — отдельно от качества стрима: lossless | hq | nq.
    pub smart_quality: Quality,
    /// Сколько места отдать умному скачиванию, ГиБ.
    pub smart_limit_gb: f64,
    /// Окно докачки по местному времени, часы [с, до); равные — круглые сутки.
    /// Окно через полночь (23 → 7) тоже можно.
    pub smart_from_hour: u8,
    pub smart_to_hour: u8,
    /// Докачивать только не по мобильной сети (как официальный клиент).
    pub smart_wifi_only: bool,
}

impl Default for Config {
    fn default() -> Self {
        let music = directories::UserDirs::new()
            .and_then(|u| u.audio_dir().map(PathBuf::from))
            .unwrap_or_else(|| home().join("Music"));
        Self {
            quality: Quality::Lossless,
            library_dir: music.join("YandexMusic"),
            library_max_gb: 0.0,
            volume: 0.8,
            normalize: true,
            normalize_target_lufs: -14.0,
            crossfade_secs: 3.0,
            kitty_images: None,
            gpu: true,
            blob_dither: 1,
            fps: 30,
            report_plays: true,
            ynison: true,
            fast_start: true,
            warm_ahead: 5,
            wave_after_list: true,
            offline_mode: false,
            full_release_date: false,
            playlist_shortcuts: true,
            smart_download: false,
            smart_quality: Quality::Hq,
            smart_limit_gb: 2.0,
            smart_from_hour: 0,
            smart_to_hour: 0,
            smart_wifi_only: true,
        }
    }
}

fn home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub struct Paths {
    pub config_file: PathBuf,
    pub token_file: PathBuf,
    pub index_file: PathBuf,
    pub state_file: PathBuf,
    pub log_dir: PathBuf,
    /// Кеш картинок (обложки, фото) — читается и без сети.
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        let pd = directories::ProjectDirs::from("", "", "yamusicrs").expect("home dir");
        let state = pd.state_dir().unwrap_or(pd.data_local_dir()).to_path_buf();
        Self {
            config_file: pd.config_dir().join("config.toml"),
            token_file: pd.data_dir().join("token.json"),
            index_file: pd.data_dir().join("library.json"),
            state_file: state.join("state.json"),
            log_dir: state,
            cache_dir: pd.cache_dir().to_path_buf(),
        }
    }
}

impl Config {
    /// Читает конфиг; если файла нет — пишет дефолтный, чтобы было что править.
    pub fn load(paths: &Paths) -> Self {
        match std::fs::read_to_string(&paths.config_file) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("config.toml не распарсился, беру дефолт: {e}");
                Self::default()
            }),
            Err(_) => {
                let cfg = Self::default();
                if let Some(dir) = paths.config_file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Ok(s) = toml::to_string_pretty(&cfg) {
                    let _ = std::fs::write(&paths.config_file, s);
                }
                cfg
            }
        }
    }

    /// Пишет конфиг (правки из вкладки «Настройки»).
    pub fn save(&self, paths: &Paths) {
        if let Some(dir) = paths.config_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match toml::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&paths.config_file, s) {
                    tracing::warn!("config.toml не записался: {e}");
                }
            }
            Err(e) => tracing::warn!("config.toml не сериализовался: {e}"),
        }
    }

    /// Протоколы kitty поверх текста (крупный текст, уведомления, курсор): только в самом kitty.
    pub fn kitty_text_enabled(&self) -> bool {
        std::env::var_os("TMUX").is_none()
            && (std::env::var("TERM").is_ok_and(|t| t.contains("kitty"))
                || std::env::var_os("KITTY_WINDOW_ID").is_some())
    }

    pub fn kitty_enabled(&self) -> bool {
        self.kitty_images.unwrap_or_else(|| {
            // tmux графику kitty не пропускает (а KITTY_WINDOW_ID внутри него наследуется).
            if std::env::var_os("TMUX").is_some() {
                return false;
            }
            std::env::var("TERM").is_ok_and(|t| t.contains("kitty"))
                || std::env::var_os("KITTY_WINDOW_ID").is_some()
                || std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "ghostty" || t == "WezTerm")
        })
    }
}

/// Состояние между запусками: громкость, настройки волны.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub volume: Option<f32>,
    /// Качество, выбранное в интерфейсе (перекрывает конфиг).
    pub quality: Option<Quality>,
    /// Нормализация и кроссфейд, выбранные клавишами (перекрывают конфиг).
    pub normalize: Option<bool>,
    pub crossfade: Option<f32>,
    /// Что играло при выходе — восстанавливается на паузе при следующем запуске.
    pub resume: Option<Resume>,
    /// Недавние поисковые запросы, свежие первыми.
    pub search_history: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resume {
    pub title: String,
    /// `trackId:albumId` всей очереди.
    pub ids: Vec<String>,
    pub pos: usize,
    pub secs: f64,
    /// Сессия волны и последний batch, если играла волна.
    pub wave: Option<(String, Option<String>)>,
    /// Сам играющий трек — стартует сразу, даже если очередь придётся грузить с сервера.
    #[serde(default)]
    pub track: Option<crate::api::Track>,
}

impl State {
    pub fn load(paths: &Paths) -> Self {
        std::fs::read(&paths.state_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) {
        if let Some(dir) = paths.state_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(&paths.state_file, b);
        }
    }
}

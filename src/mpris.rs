//! MPRIS2 на zbus: медиаклавиши, playerctl, виджеты окружения.
//! Объявляются только поддерживаемые свойства: `Shuffle` и `LoopStatus` по спеке
//! необязательны, и без них шелл не рисует кнопок, которые ничего не делают.
//! Плюс расширение `org.mpris.MediaPlayer2.MichaAIext` на том же объекте: две кнопки
//! («не рекомендовать» / лайк) с SVG-иконками и текст песни в LRC — его читает шелл.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::object_server::SignalEmitter;
use zbus::{OwnedVariant as OwnedValue, Variant as Value};
use zbus::{OwnedObjectPath, fdo, interface};

#[derive(Debug, Clone)]
pub enum Command {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    Stop,
    Seek(f64),
    SetPosition(f64),
    Volume(f64),
    /// Правая кнопка расширения: лайк текущего трека (переключение).
    Like,
    /// Левая кнопка расширения: «не рекомендовать» и дальше.
    Dislike,
}

#[derive(Debug, Clone)]
pub enum Update {
    Track {
        id: String,
        title: String,
        artists: Vec<String>,
        album: String,
        length: f64,
        art: Option<String>,
        /// Позиция в новом треке: без неё виджеты продолжают отсчёт со старой.
        pos: f64,
        liked: bool,
    },
    Status {
        playing: bool,
        pos: f64,
    },
    Position(f64),
    Seeked(f64),
    Volume(f64),
    /// Лайк текущего трека поменялся (из интерфейса или после загрузки «Мне нравится»).
    Liked(bool),
    /// Текст текущего трека: LRC или просто строки.
    Lyrics(String),
}

pub fn spawn(commands: UnboundedSender<Command>) -> UnboundedSender<Update> {
    let (tx, rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("mpris".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async move {
                if let Err(e) = run(commands, rx).await {
                    tracing::warn!("MPRIS недоступен: {e}");
                }
            });
        })
        .expect("mpris thread");
    tx
}

const PATH: &str = "/org/mpris/MediaPlayer2";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

fn path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p).unwrap_or_else(|_| OwnedObjectPath::try_from(NO_TRACK).expect("path"))
}

fn micros(s: f64) -> i64 {
    (s * 1e6) as i64
}

fn val<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    v.into().try_to_owned().expect("без fd")
}

// ── org.mpris.MediaPlayer2 ──────────────────────────────────────────────────

struct Root;

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}

    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> &str {
        "Яндекс Музыка (yamusicrs)"
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "yamusicrs"
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

// ── org.mpris.MediaPlayer2.Player ───────────────────────────────────────────

struct Player {
    cmd: UnboundedSender<Command>,
    status: &'static str,
    metadata: HashMap<String, OwnedValue>,
    trackid: OwnedObjectPath,
    volume: f64,
    /// Позиция меняется постоянно, поэтому без сигнала — её читают по запросу.
    position: Arc<Mutex<i64>>,
}

impl Player {
    fn send(&self, c: Command) {
        let _ = self.cmd.send(c);
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.send(Command::Next);
    }

    fn previous(&self) {
        self.send(Command::Previous);
    }

    fn pause(&self) {
        self.send(Command::Pause);
    }

    fn play_pause(&self) {
        self.send(Command::PlayPause);
    }

    fn stop(&self) {
        self.send(Command::Stop);
    }

    fn play(&self) {
        self.send(Command::Play);
    }

    fn seek(&self, offset: i64) {
        self.send(Command::Seek(offset as f64 / 1e6));
    }

    fn set_position(&self, track_id: OwnedObjectPath, position: i64) {
        // По спеке устаревший trackid игнорируется.
        if track_id == self.trackid && position >= 0 {
            self.send(Command::SetPosition(position as f64 / 1e6));
        }
    }

    fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("OpenUri".into()))
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> &str {
        self.status
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn set_rate(&mut self, _rate: f64) {}

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.metadata.clone()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.volume
    }

    #[zbus(property)]
    fn set_volume(&mut self, v: f64) {
        self.send(Command::Volume(v.clamp(0.0, 1.0)));
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        *self.position.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
}

// ── расширение MichaAIext ───────────────────────────────────────────────────

/// Иконки рисуются цветом темы шелла (`currentColor`), ~18px. Контуры — Tabler Icons (MIT):
/// только штрихи, без масок — их понимает и QtSvg.
const SVG_OPEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#;
const HEART_PATH: &str = "M19.5 12.572l-7.5 7.428l-7.5 -7.428a5 5 0 1 1 7.5 -6.566a5 5 0 1 1 7.5 6.572";

fn heart(full: bool) -> String {
    let fill = if full { r#" fill="currentColor""# } else { "" };
    format!(r#"{SVG_OPEN}<path{fill} d="{HEART_PATH}"/></svg>"#)
}

/// Зачёркнутое сердце (tabler heart-off).
const DISLIKE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3l18 18"/><path d="M19.5 12.572l-1.5 1.428m-2 2l-4 4l-7.5 -7.428a5 5 0 0 1 -1.288 -5.068a4.976 4.976 0 0 1 1.788 -2.504m3 -1c1.56 0 3.05 .727 4 2a5 5 0 1 1 7.5 6.572"/></svg>"#;

struct Ext {
    cmd: UnboundedSender<Command>,
    left_icon: String,
    right_icon: String,
    lyrics: (OwnedObjectPath, String),
}

#[interface(name = "org.mpris.MediaPlayer2.MichaAIext")]
impl Ext {
    fn left(&self) {
        let _ = self.cmd.send(Command::Dislike);
    }

    fn right(&self) {
        let _ = self.cmd.send(Command::Like);
    }

    #[zbus(property)]
    fn left_icon(&self) -> String {
        self.left_icon.clone()
    }

    #[zbus(property)]
    fn right_icon(&self) -> String {
        self.right_icon.clone()
    }

    #[zbus(property)]
    fn lyrics(&self) -> (OwnedObjectPath, String) {
        self.lyrics.clone()
    }
}

// ── цикл обновлений ─────────────────────────────────────────────────────────

async fn run(cmd: UnboundedSender<Command>, mut rx: UnboundedReceiver<Update>) -> zbus::Result<()> {
    // `org.mpris.MediaPlayer2.<имя>`; переопределяется для второго экземпляра или тестов.
    let name = std::env::var("YAMUSICRS_MPRIS_NAME").unwrap_or_else(|_| "yamusicrs".into());
    let position = Arc::new(Mutex::new(0i64));
    let conn = zbus::connection::Builder::session()
        .name(format!("org.mpris.MediaPlayer2.{name}"))
        .serve_at(PATH, Root)
        .serve_at(
            PATH,
            Player {
                cmd: cmd.clone(),
                status: "Stopped",
                metadata: HashMap::from([("mpris:trackid".to_string(), val(path(NO_TRACK)))]),
                trackid: path(NO_TRACK),
                volume: 1.0,
                position: position.clone(),
            },
        )
        .serve_at(
            PATH,
            Ext {
                cmd,
                left_icon: String::new(),
                right_icon: String::new(),
                lyrics: (path(NO_TRACK), String::new()),
            },
        )
        .build()
        .await?;
    let objects = conn.object_server();
    let player = objects.interface::<_, Player>(PATH).await?;
    let ext = objects.interface::<_, Ext>(PATH).await?;
    let pe = player.signal_emitter();
    let ee = ext.signal_emitter();
    let set_pos = |s: f64| *position.lock().unwrap_or_else(|e| e.into_inner()) = micros(s);

    while let Some(u) = rx.recv().await {
        let r: zbus::Result<()> = async {
            match u {
                Update::Track {
                    id,
                    title,
                    artists,
                    album,
                    length,
                    art,
                    pos,
                    liked,
                } => {
                    let tid = path(&format!("/ru/yandex/music/track/t{}", id.replace(':', "_")));
                    let mut m = HashMap::from([
                        ("mpris:trackid".to_string(), val(tid.clone())),
                        ("xesam:title".into(), val(title)),
                        ("xesam:artist".into(), val(artists)),
                        ("xesam:album".into(), val(album)),
                        ("mpris:length".into(), val(micros(length))),
                        (
                            "xesam:url".into(),
                            val(format!(
                                "https://music.yandex.ru/track/{}",
                                id.split(':').next().unwrap_or(&id)
                            )),
                        ),
                    ]);
                    if let Some(a) = art {
                        m.insert("mpris:artUrl".into(), val(a));
                    }
                    // Текст старого трека снимаем до новых Metadata: шелл не должен принять его за новый.
                    {
                        let mut e = ext.get_mut().await;
                        e.lyrics = (tid.clone(), String::new());
                        e.left_icon = DISLIKE.into();
                        e.right_icon = heart(liked);
                        e.lyrics_changed(ee).await?;
                        e.left_icon_changed(ee).await?;
                        e.right_icon_changed(ee).await?;
                    }
                    // Клиенты перечитывают Position при смене Metadata, поэтому позиция — до неё,
                    // а Seeked после — для тех, кто только экстраполирует.
                    set_pos(pos);
                    {
                        let mut p = player.get_mut().await;
                        p.trackid = tid;
                        p.metadata = m;
                        p.metadata_changed(pe).await?;
                    }
                    Player::seeked(pe, micros(pos)).await
                }
                Update::Status { playing, pos } => {
                    set_pos(pos);
                    let mut p = player.get_mut().await;
                    let s = if playing { "Playing" } else { "Paused" };
                    if p.status == s {
                        return Ok(());
                    }
                    p.status = s;
                    p.playback_status_changed(pe).await
                }
                Update::Position(p) => {
                    set_pos(p);
                    Ok(())
                }
                Update::Seeked(p) => {
                    set_pos(p);
                    Player::seeked(pe, micros(p)).await
                }
                Update::Volume(v) => {
                    let mut p = player.get_mut().await;
                    p.volume = v;
                    p.volume_changed(pe).await
                }
                Update::Liked(liked) => {
                    let icon = heart(liked);
                    let mut e = ext.get_mut().await;
                    if e.right_icon.is_empty() || e.right_icon == icon {
                        return Ok(());
                    }
                    e.right_icon = icon;
                    e.right_icon_changed(ee).await
                }
                Update::Lyrics(text) => {
                    let mut e = ext.get_mut().await;
                    e.lyrics.1 = text;
                    e.lyrics_changed(ee).await
                }
            }
        }
        .await;
        if let Err(e) = r {
            tracing::debug!("mpris: {e}");
        }
    }
    Ok(())
}

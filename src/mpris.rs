//! MPRIS2: медиаклавиши, playerctl, виджеты окружения.
//! Плюс расширение `org.mpris.MediaPlayer2.MichaAIext` на том же объекте: две кнопки
//! (лайк / «не рекомендовать») с SVG-иконками и текст песни в LRC — его читает шелл.

use std::sync::Mutex;

use mpris_server::zbus::{self, OwnedObjectPath, fdo};
use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Signal, Time,
    TrackId, Volume,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

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
    /// Левая кнопка расширения: лайк текущего трека (переключение).
    Like,
    /// Правая кнопка расширения: «не рекомендовать» и дальше.
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
    Repeat(bool),
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

struct State {
    status: PlaybackStatus,
    looping: LoopStatus,
    metadata: Metadata,
    volume: Volume,
    position: Time,
}

struct Imp {
    cmd: UnboundedSender<Command>,
    state: Mutex<State>,
}

impl Imp {
    fn send(&self, c: Command) -> fdo::Result<()> {
        let _ = self.cmd.send(c);
        Ok(())
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl RootInterface for Imp {
    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn quit(&self) -> fdo::Result<()> {
        Ok(())
    }
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_fullscreen(&self, _: bool) -> zbus::Result<()> {
        Ok(())
    }
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn identity(&self) -> fdo::Result<String> {
        Ok("Яндекс Музыка (yamusicrs)".into())
    }
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("yamusicrs".into())
    }
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl PlayerInterface for Imp {
    async fn next(&self) -> fdo::Result<()> {
        self.send(Command::Next)
    }
    async fn previous(&self) -> fdo::Result<()> {
        self.send(Command::Previous)
    }
    async fn pause(&self) -> fdo::Result<()> {
        self.send(Command::Pause)
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        self.send(Command::PlayPause)
    }
    async fn stop(&self) -> fdo::Result<()> {
        self.send(Command::Stop)
    }
    async fn play(&self) -> fdo::Result<()> {
        self.send(Command::Play)
    }
    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.send(Command::Seek(offset.as_micros() as f64 / 1e6))
    }
    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        // По спеке устаревший trackid игнорируется.
        if self.st().metadata.trackid().is_some_and(|t| t != track_id) {
            return Ok(());
        }
        self.send(Command::SetPosition(position.as_micros() as f64 / 1e6))
    }
    async fn open_uri(&self, _: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("OpenUri".into()))
    }
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self.st().status)
    }
    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(self.st().looping)
    }
    async fn set_loop_status(&self, _: LoopStatus) -> zbus::Result<()> {
        Ok(())
    }
    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn set_rate(&self, _: PlaybackRate) -> zbus::Result<()> {
        Ok(())
    }
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }
    async fn set_shuffle(&self, _: bool) -> zbus::Result<()> {
        Ok(())
    }
    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.st().metadata.clone())
    }
    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.st().volume)
    }
    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        let _ = self.cmd.send(Command::Volume(volume));
        Ok(())
    }
    async fn position(&self) -> fdo::Result<Time> {
        Ok(self.st().position)
    }
    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }
    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

// ── расширение MichaAIext ───────────────────────────────────────────────────

/// Иконки рисуются цветом темы шелла (`currentColor`), ~18px.
const HEART: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" d="M12 20.5s-7.6-4.7-9.3-9.5C1.5 7.6 3.8 4.3 7.3 4.3c2 0 3.6 1.1 4.7 2.9 1.1-1.8 2.7-2.9 4.7-2.9 3.5 0 5.8 3.3 4.6 6.7-1.7 4.8-9.3 9.5-9.3 9.5z"/></svg>"#;
const HEART_FULL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linejoin="round" d="M12 20.5s-7.6-4.7-9.3-9.5C1.5 7.6 3.8 4.3 7.3 4.3c2 0 3.6 1.1 4.7 2.9 1.1-1.8 2.7-2.9 4.7-2.9 3.5 0 5.8 3.3 4.6 6.7-1.7 4.8-9.3 9.5-9.3 9.5z"/></svg>"#;
const DISLIKE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5" fill="none" stroke="currentColor" stroke-width="2"/><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" d="M6 6l12 12"/></svg>"#;

struct Ext {
    cmd: UnboundedSender<Command>,
    left_icon: String,
    right_icon: String,
    lyrics: (OwnedObjectPath, String),
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.MichaAIext")]
impl Ext {
    fn left(&self) {
        let _ = self.cmd.send(Command::Like);
    }

    fn right(&self) {
        let _ = self.cmd.send(Command::Dislike);
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

fn no_track() -> OwnedObjectPath {
    OwnedObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack").expect("path")
}

fn micros(s: f64) -> Time {
    Time::from_micros((s * 1e6) as i64)
}

async fn run(cmd: UnboundedSender<Command>, mut rx: UnboundedReceiver<Update>) -> zbus::Result<()> {
    // `org.mpris.MediaPlayer2.<имя>`; переопределяется для второго экземпляра или тестов.
    let name = std::env::var("YAMUSICRS_MPRIS_NAME").unwrap_or_else(|_| "yamusicrs".into());
    let server = Server::new(
        &name,
        Imp {
            cmd: cmd.clone(),
            state: Mutex::new(State {
                status: PlaybackStatus::Stopped,
                looping: LoopStatus::None,
                metadata: Metadata::new(),
                volume: 1.0,
                position: Time::ZERO,
            }),
        },
    )
    .await?;
    let objects = server.connection().object_server();
    objects
        .at(
            PATH,
            Ext {
                cmd,
                left_icon: String::new(),
                right_icon: String::new(),
                lyrics: (no_track(), String::new()),
            },
        )
        .await?;
    let ext = objects.interface::<_, Ext>(PATH).await?;
    let imp = server.imp();

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
                    let path = format!("/ru/yandex/music/track/t{}", id.replace(':', "_"));
                    let tid = TrackId::try_from(path.as_str()).unwrap_or(TrackId::NO_TRACK);
                    let mut m = Metadata::builder()
                        .trackid(tid)
                        .title(title)
                        .artist(artists)
                        .album(album)
                        .length(micros(length))
                        .url(format!(
                            "https://music.yandex.ru/track/{}",
                            id.split(':').next().unwrap_or(&id)
                        ))
                        .build();
                    if let Some(a) = art {
                        m.set_art_url(Some(a));
                    }
                    // Клиенты перечитывают Position при смене Metadata, поэтому позиция — до неё,
                    // а Seeked после — для тех, кто только экстраполирует.
                    let t = micros(pos);
                    {
                        let mut s = imp.st();
                        s.position = t;
                        s.metadata = m.clone();
                    }
                    // Текст старого трека снимаем до новых Metadata: шелл не должен принять его за новый.
                    {
                        let mut e = ext.get_mut().await;
                        e.lyrics = (
                            OwnedObjectPath::try_from(path).unwrap_or_else(|_| no_track()),
                            String::new(),
                        );
                        e.left_icon = (if liked { HEART_FULL } else { HEART }).into();
                        e.right_icon = DISLIKE.into();
                    }
                    {
                        let e = ext.get().await;
                        let em = ext.signal_emitter();
                        e.lyrics_changed(em).await?;
                        e.left_icon_changed(em).await?;
                        e.right_icon_changed(em).await?;
                    }
                    server.properties_changed([Property::Metadata(m)]).await?;
                    server.emit(Signal::Seeked { position: t }).await
                }
                Update::Status { playing, pos } => {
                    let status = if playing {
                        PlaybackStatus::Playing
                    } else {
                        PlaybackStatus::Paused
                    };
                    {
                        let mut s = imp.st();
                        s.position = micros(pos);
                        s.status = status;
                    }
                    server.properties_changed([Property::PlaybackStatus(status)]).await
                }
                Update::Position(p) => {
                    imp.st().position = micros(p);
                    Ok(())
                }
                Update::Seeked(p) => {
                    imp.st().position = micros(p);
                    server.emit(Signal::Seeked { position: micros(p) }).await
                }
                Update::Volume(v) => {
                    imp.st().volume = v;
                    server.properties_changed([Property::Volume(v)]).await
                }
                Update::Repeat(r) => {
                    let l = if r { LoopStatus::Track } else { LoopStatus::None };
                    imp.st().looping = l;
                    server.properties_changed([Property::LoopStatus(l)]).await
                }
                Update::Liked(liked) => {
                    let icon = if liked { HEART_FULL } else { HEART };
                    let mut e = ext.get_mut().await;
                    if e.left_icon.is_empty() || e.left_icon == icon {
                        return Ok(());
                    }
                    e.left_icon = icon.into();
                    e.left_icon_changed(ext.signal_emitter()).await
                }
                Update::Lyrics(text) => {
                    let mut e = ext.get_mut().await;
                    e.lyrics.1 = text;
                    e.lyrics_changed(ext.signal_emitter()).await
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

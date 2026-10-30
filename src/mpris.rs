//! MPRIS2: медиаклавиши, playerctl, виджеты окружения.
//! `mpris_server::Player` не `Send`, поэтому живёт в своём потоке с локальным рантаймом.

use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};
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
    },
    Status {
        playing: bool,
    },
    Position(f64),
    Seeked(f64),
    Volume(f64),
    Repeat(bool),
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
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                if let Err(e) = run(commands, rx).await {
                    tracing::warn!("MPRIS недоступен: {e}");
                }
            });
        })
        .expect("mpris thread");
    tx
}

async fn run(cmd: UnboundedSender<Command>, mut rx: UnboundedReceiver<Update>) -> mpris_server::zbus::Result<()> {
    let player = Player::builder("yamusicrs")
        .identity("Яндекс Музыка (yamusicrs)")
        .desktop_entry("yamusicrs")
        .can_play(true)
        .can_pause(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_seek(true)
        .can_control(true)
        .build()
        .await?;

    let c = cmd.clone();
    player.connect_play_pause(move |_| drop(c.send(Command::PlayPause)));
    let c = cmd.clone();
    player.connect_play(move |_| drop(c.send(Command::Play)));
    let c = cmd.clone();
    player.connect_pause(move |_| drop(c.send(Command::Pause)));
    let c = cmd.clone();
    player.connect_next(move |_| drop(c.send(Command::Next)));
    let c = cmd.clone();
    player.connect_previous(move |_| drop(c.send(Command::Previous)));
    let c = cmd.clone();
    player.connect_stop(move |_| drop(c.send(Command::Stop)));
    let c = cmd.clone();
    player.connect_seek(move |_, off| drop(c.send(Command::Seek(off.as_micros() as f64 / 1e6))));
    let c = cmd.clone();
    player.connect_set_position(move |_, _, t| drop(c.send(Command::SetPosition(t.as_micros() as f64 / 1e6))));
    let c = cmd.clone();
    player.connect_set_volume(move |_, v| drop(c.send(Command::Volume(v))));

    tokio::task::spawn_local(player.run());

    while let Some(u) = rx.recv().await {
        let r = match u {
            Update::Track {
                id,
                title,
                artists,
                album,
                length,
                art,
            } => {
                let tid = TrackId::try_from(format!("/ru/yandex/music/track/t{}", id.replace(':', "_")))
                    .unwrap_or(TrackId::NO_TRACK);
                let mut m = Metadata::builder()
                    .trackid(tid)
                    .title(title)
                    .artist(artists)
                    .album(album)
                    .length(Time::from_micros((length * 1e6) as i64))
                    .url(format!(
                        "https://music.yandex.ru/track/{}",
                        id.split(':').next().unwrap_or(&id)
                    ))
                    .build();
                if let Some(a) = art {
                    m.set_art_url(Some(a));
                }
                player.set_metadata(m).await
            }
            Update::Status { playing } => {
                player
                    .set_playback_status(if playing {
                        PlaybackStatus::Playing
                    } else {
                        PlaybackStatus::Paused
                    })
                    .await
            }
            Update::Position(p) => {
                player.set_position(Time::from_micros((p * 1e6) as i64));
                Ok(())
            }
            Update::Seeked(p) => player.seeked(Time::from_micros((p * 1e6) as i64)).await,
            Update::Volume(v) => player.set_volume(v).await,
            Update::Repeat(r) => {
                player
                    .set_loop_status(if r { LoopStatus::Track } else { LoopStatus::None })
                    .await
            }
        };
        if let Err(e) = r {
            tracing::debug!("mpris: {e}");
        }
    }
    Ok(())
}

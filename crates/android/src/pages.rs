//! Страницы: плейлист, альбом, исполнитель, похожие, поиск, новинки.
//!
//! Kotlin открывает страницу методом `Core` и сразу получает её id; ответ приходит позже
//! колбэком `on_page(id, page)`. Треки страницы остаются в потоке движка (по id), чтобы
//! играть и ставить в очередь по индексу, не гоняя модели туда-обратно.

use std::collections::HashMap;

use yamusic_core::api::{self, Album, Artist, ArtistBrief, Client, Playlist, SearchResult, Track};
use tokio::sync::mpsc::UnboundedSender;
use yamusic_core::engine::{Engine, Loadable, err_str};

use crate::TrackItem;

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct AlbumItem {
    pub id: String,
    pub title: String,
    pub artists: String,
    pub year: Option<i32>,
    /// Полная дата выхода («12 марта 2024»), если сервер её отдал.
    pub released: Option<String>,
    /// «Альбом», «Сингл», «Сборник»…
    pub kind: String,
    pub cover: Option<String>,
    pub thumb: Option<String>,
    pub track_count: u32,
    pub explicit: bool,
}

impl From<&Album> for AlbumItem {
    fn from(a: &Album) -> Self {
        Self {
            id: a.id.clone(),
            title: a.full_title(),
            artists: a.artist_names(),
            year: a.year,
            released: a.release_label(),
            kind: a.kind_label().into(),
            cover: a.cover_uri().map(|u| api::cover_url(u, 400)),
            thumb: a.cover_uri().map(|u| api::cover_url(u, 200)),
            track_count: a.track_count.unwrap_or(0),
            explicit: a.explicit(),
        }
    }
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct ArtistItem {
    pub id: String,
    pub name: String,
    pub photo: Option<String>,
}

impl From<&Artist> for ArtistItem {
    fn from(a: &Artist) -> Self {
        Self {
            id: a.id.clone(),
            name: a.name.clone(),
            photo: a.cover_uri().map(|u| api::cover_url(u, 200)),
        }
    }
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct PlaylistItem {
    pub uid: String,
    pub kind: String,
    pub title: String,
    pub owner: String,
    pub track_count: u32,
    pub cover: Option<String>,
    pub description: Option<String>,
}

impl From<&Playlist> for PlaylistItem {
    fn from(p: &Playlist) -> Self {
        Self {
            uid: p.uid.clone(),
            kind: p.kind.clone(),
            title: p.title.clone(),
            owner: owner(p),
            track_count: p.track_count.unwrap_or(p.tracks.len() as u32),
            cover: p.cover_uri().map(|u| api::cover_url(u, 400)),
            description: p.description.clone().filter(|d| !d.is_empty()),
        }
    }
}

fn owner(p: &Playlist) -> String {
    p.owner
        .as_ref()
        .and_then(|o| o.name.clone().or(o.login.clone()))
        .unwrap_or_default()
}

#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum Page {
    /// Список треков: плейлист, альбом, похожие, чарт.
    Tracks {
        title: String,
        subtitle: String,
        cover: Option<String>,
        /// Есть у страницы альбома.
        album: Option<AlbumItem>,
        tracks: Vec<TrackItem>,
    },
    Search {
        query: String,
        tracks: Vec<TrackItem>,
        albums: Vec<AlbumItem>,
        artists: Vec<ArtistItem>,
        playlists: Vec<PlaylistItem>,
    },
    Albums {
        title: String,
        albums: Vec<AlbumItem>,
    },
    Artist {
        artist: ArtistItem,
        /// Слушателей за месяц.
        listeners: Option<u64>,
        genres: Vec<String>,
        about: Option<String>,
        /// Популярные треки (их и играет `play_from`).
        tracks: Vec<TrackItem>,
        albums: Vec<AlbumItem>,
        also: Vec<AlbumItem>,
        similar: Vec<ArtistItem>,
    },
    Failed {
        error: String,
    },
}

/// Откуда трек: для `play_from`, `enqueue_from`, `like_from`.
#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum Source {
    Likes,
    Queue,
    Page { id: u64 },
}

/// Что открыть.
pub enum Req {
    Playlist { uid: String, kind: String },
    Album(String),
    Artist(String),
    Similar { track_id: String, title: String },
    Search(String),
    Releases,
    Chart,
    /// Всё скачанное, недавно игранное сверху (без сети тоже).
    Downloaded,
}

/// Сырой ответ сети; в `Page` превращается уже в потоке движка.
pub enum Loaded {
    Tracks {
        title: String,
        subtitle: String,
        cover: Option<String>,
        album: Option<Album>,
        tracks: Vec<Track>,
        /// Сид волны «по этому списку».
        seed: Option<String>,
    },
    Search(String, SearchResult),
    Albums(String, Vec<Album>),
    Artist(String, Box<ArtistBrief>),
}

pub type Reply = (u64, Result<Loaded, String>);

/// Треки открытых страниц: id → (название очереди, треки); сиды волны «по этому списку».
#[derive(Default)]
pub struct Pages {
    pub tracks: HashMap<u64, (String, Vec<Track>)>,
    pub seeds: HashMap<u64, String>,
}

impl Pages {
    /// Ответ сети → страница для Kotlin; треки запоминаются под id.
    pub fn accept(&mut self, id: u64, r: Result<Loaded, String>) -> Page {
        let loaded = match r {
            Ok(l) => l,
            Err(error) => return Page::Failed { error },
        };
        match loaded {
            Loaded::Tracks {
                title,
                subtitle,
                cover,
                album,
                tracks,
                seed,
            } => {
                let items = tracks.iter().map(TrackItem::from).collect();
                self.tracks.insert(id, (title.clone(), tracks));
                if let Some(seed) = seed {
                    self.seeds.insert(id, seed);
                }
                Page::Tracks {
                    title,
                    subtitle,
                    cover,
                    album: album.as_ref().map(AlbumItem::from),
                    tracks: items,
                }
            }
            Loaded::Search(query, r) => {
                let tracks = r.tracks.map(|b| b.results).unwrap_or_default();
                let items = tracks.iter().map(TrackItem::from).collect();
                self.tracks.insert(id, (format!("Поиск: {query}"), tracks));
                Page::Search {
                    query,
                    tracks: items,
                    albums: r.albums.map(|b| b.results.iter().map(AlbumItem::from).collect()).unwrap_or_default(),
                    artists: r.artists.map(|b| b.results.iter().map(ArtistItem::from).collect()).unwrap_or_default(),
                    playlists: r
                        .playlists
                        .map(|b| b.results.iter().map(PlaylistItem::from).collect())
                        .unwrap_or_default(),
                }
            }
            Loaded::Albums(title, v) => Page::Albums {
                title,
                albums: v.iter().map(AlbumItem::from).collect(),
            },
            Loaded::Artist(name, b) => {
                let info = b.artist.as_ref();
                let tracks = if b.popular_tracks.is_empty() {
                    b.familiar.clone()
                } else {
                    b.popular_tracks.clone()
                };
                let items = tracks.iter().map(TrackItem::from).collect();
                self.tracks.insert(id, (name.clone(), tracks));
                if let Some(a) = info.filter(|a| !a.id.is_empty()) {
                    self.seeds.insert(id, format!("artist:{}", a.id));
                }
                let albums = if b.discography.is_empty() { &b.albums } else { &b.discography };
                Page::Artist {
                    artist: ArtistItem {
                        id: info.map(|i| i.id.clone()).unwrap_or_default(),
                        name,
                        // Во всю ширину шапки — крупно.
                        photo: info.and_then(|i| i.cover_uri()).map(|u| api::cover_url(u, 1000)),
                    },
                    listeners: b.stats.and_then(|s| s.last_month_listeners),
                    genres: info.map(|i| i.genres.clone()).unwrap_or_default(),
                    about: info
                        .and_then(|i| i.description.as_ref())
                        .map(|d| d.text.clone())
                        .filter(|t| !t.is_empty()),
                    tracks: items,
                    albums: albums.iter().map(AlbumItem::from).collect(),
                    also: b.also_albums.iter().map(AlbumItem::from).collect(),
                    similar: b.similar_artists.iter().map(ArtistItem::from).collect(),
                }
            }
        }
    }
}

/// Скачанное: треки из библиотеки, о которых знает снимок; свежие сверху.
fn downloaded(e: &Engine) -> Loaded {
    let mut entries = e.library.entries();
    entries.sort_by_key(|(_, ts, _)| std::cmp::Reverse(*ts));
    let smart = entries.iter().filter(|(.., s)| *s).count();
    let ts: Vec<Track> = entries.iter().filter_map(|(id, ..)| e.snapshot.find(id).cloned()).collect();
    let (_, bytes) = e.library.stats();
    let gb = format!("{:.1}", bytes as f64 / (1024.0 * 1024.0 * 1024.0)).replace('.', ",");
    let mut subtitle = format!("{}, {gb} ГБ на диске", tracks_word(ts.len()));
    if smart > 0 {
        subtitle.push_str(&format!("\n{smart} подобрано для офлайн-волны"));
    }
    Loaded::Tracks {
        title: "Скачанное".into(),
        subtitle,
        cover: None,
        album: None,
        tracks: e.offline_tracks(ts),
        seed: None,
    }
}

/// Открыть страницу: ответ уйдёт в `tx` (офлайн плейлист — из снимка сразу).
pub fn open(e: &Engine, id: u64, req: Req, tx: &UnboundedSender<Reply>) {
    if let Req::Downloaded = req {
        let _ = tx.send((id, Ok(downloaded(e))));
        return;
    }
    if let Req::Playlist { uid, kind } = &req
        && e.offline
    {
        let pl = e.snapshot.playlists.iter().find(|p| &p.uid == uid && &p.kind == kind);
        let r = match pl.and_then(|p| Some((p, e.snapshot.tracks_of(p)?))) {
            Some((p, ts)) => Ok(Loaded::Tracks {
                title: p.title.clone(),
                subtitle: owner(p),
                cover: p.cover_uri().map(|u| api::cover_url(u, 400)),
                album: None,
                tracks: e.offline_tracks(ts.clone()),
                seed: Some(format!("playlist:{}_{}", p.uid, p.kind)),
            }),
            None => Err("Этот плейлист не успел сохраниться для офлайна".into()),
        };
        let _ = tx.send((id, r));
        return;
    }
    let Some(c) = e.client() else {
        let _ = tx.send((id, Err("Нет сети".into())));
        return;
    };
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send((id, fetch(c, req).await));
    });
}

async fn fetch(c: Client, req: Req) -> Result<Loaded, String> {
    match req {
        // Отдаётся из библиотеки в `open`, сюда не доходит.
        Req::Downloaded => Err("Скачанное читается без сети".into()),
        Req::Playlist { uid, kind } => {
            let full = c.playlist(&uid, &kind).await.map_err(err_str)?;
            let tracks = c.playlist_tracks(&full).await.map_err(err_str)?;
            let n = tracks.len();
            Ok(Loaded::Tracks {
                title: full.title.clone(),
                subtitle: [owner(&full), tracks_word(n)]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", "),
                cover: full.cover_uri().map(|u| api::cover_url(u, 400)),
                album: None,
                tracks,
                seed: Some(format!("playlist:{uid}_{kind}")),
            })
        }
        Req::Album(id) => {
            let a = c.album(&id).await.map_err(err_str)?;
            let tracks = a.volume_tracks();
            Ok(Loaded::Tracks {
                title: a.full_title(),
                subtitle: a.artist_names(),
                cover: a.cover_uri().map(|u| api::cover_url(u, 400)),
                album: Some(Album { volumes: Vec::new(), ..a }),
                tracks,
                seed: Some(format!("album:{id}")),
            })
        }
        Req::Artist(id) => {
            let b = c.artist(&id).await.map_err(err_str)?;
            // Имя обычно есть в brief-info; если нет — из любого его альбома или трека.
            let name = b
                .artist
                .as_ref()
                .map(|a| a.name.clone())
                .filter(|n| !n.is_empty())
                .or_else(|| {
                    b.albums
                        .iter()
                        .chain(&b.discography)
                        .flat_map(|a| &a.artists)
                        .chain(b.popular_tracks.iter().flat_map(|t| &t.artists))
                        .find(|a| a.id == id)
                        .map(|a| a.name.clone())
                })
                .unwrap_or_default();
            Ok(Loaded::Artist(name, Box::new(b)))
        }
        Req::Similar { track_id, title } => {
            let tracks = c.similar_tracks(&track_id).await.map_err(err_str)?;
            if tracks.is_empty() {
                return Err("Похожих треков нет".into());
            }
            Ok(Loaded::Tracks {
                title: format!("Похожие на «{title}»"),
                subtitle: tracks_word(tracks.len()),
                cover: None,
                album: None,
                tracks,
                seed: Some(format!("track:{track_id}")),
            })
        }
        Req::Search(q) => Ok(Loaded::Search(q.clone(), c.search(&q).await.map_err(err_str)?)),
        Req::Releases => Ok(Loaded::Albums("Новые релизы".into(), c.new_releases().await.map_err(err_str)?)),
        Req::Chart => {
            let pl = c.chart().await.map_err(err_str)?;
            let tracks = c.playlist_tracks(&pl).await.map_err(err_str)?;
            Ok(Loaded::Tracks {
                title: "Чарт".into(),
                subtitle: "Популярное прямо сейчас".into(),
                cover: None,
                album: None,
                tracks,
                seed: None,
            })
        }
    }
}

fn tracks_word(n: usize) -> String {
    let w = match (n % 10, n % 100) {
        (1, x) if x != 11 => "трек",
        (2..=4, x) if !(12..=14).contains(&x) => "трека",
        _ => "треков",
    };
    format!("{n} {w}")
}

/// Свои плейлисты (офлайн — из снимка) и «Настроили для вас».
pub fn playlists(e: &Engine) -> (Vec<PlaylistItem>, Vec<PlaylistItem>) {
    let own = match &e.playlists {
        Loadable::Ready(v) => v.iter().map(PlaylistItem::from).collect(),
        _ => e.snapshot.playlists.iter().map(PlaylistItem::from).collect(),
    };
    let personal = e.personal.iter().map(|p| PlaylistItem::from(&p.playlist)).collect();
    (own, personal)
}

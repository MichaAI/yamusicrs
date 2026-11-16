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

/// Ссылка исполнителя (соцсети, сайт).
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LinkItem {
    pub title: String,
    pub subtitle: Option<String>,
    pub url: String,
}

/// Волна из поиска или «Похожего» → пресет для карточки волны.
pub fn wave_preset(w: &api::SearchWave) -> crate::WavePreset {
    crate::WavePreset {
        name: w.title.clone(),
        description: w.sub_title.clone(),
        icon: w.icon().map(|u| api::cover_url(u, 200)),
        color: w.color.as_deref().and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok()),
        seeds: w.seeds.clone(),
        home: false,
    }
}

/// Анонсированный релиз: страницы у него до выхода нет, только пресейв.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct UpcomingItem {
    pub id: String,
    pub title: String,
    pub artists: String,
    /// Первый исполнитель — куда ведёт нажатие на карточку.
    pub artist_id: Option<String>,
    /// «Сингл», «Альбом»…
    pub kind: String,
    /// «Через 3 дня, 20 ноября».
    pub due: String,
    pub cover: Option<String>,
    pub presaved: bool,
    pub explicit: bool,
    /// Момент выхода (мс от эпохи) — для уведомления в день выхода.
    pub release_ms: Option<i64>,
}

impl From<&api::Upcoming> for UpcomingItem {
    fn from(u: &api::Upcoming) -> Self {
        Self {
            id: u.id.clone(),
            title: u.title.clone(),
            artists: u.artist_names(),
            artist_id: u.artists.iter().map(|a| a.id.clone()).find(|id| !id.is_empty()),
            kind: u.kind_label().into(),
            due: u.when_label().unwrap_or_default(),
            cover: u.cover_uri.as_deref().map(|c| api::cover_url(c, 400)),
            presaved: u.presaved,
            explicit: u.content_warning.as_deref() == Some("explicit"),
            release_ms: u.release_ms(),
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
    /// Свой плейлист: в него можно добавлять треки.
    pub mine: bool,
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
            mine: false,
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
        /// Страница плейлиста (а не альбома, чарта, похожих).
        playlist: bool,
        /// Альбом в коллекции (для страницы альбома).
        liked: bool,
        tracks: Vec<TrackItem>,
        /// Список скрытых треков: их можно вернуть в рекомендации.
        hidden: bool,
        /// «Похожее» под альбомом.
        similar_albums: Vec<AlbumItem>,
        similar_artists: Vec<ArtistItem>,
        similar_waves: Vec<crate::WavePreset>,
    },
    /// «Об исполнителе» целиком.
    About {
        description: Option<String>,
        links: Vec<LinkItem>,
        /// Фото исполнителя (https, 1000×1000).
        photos: Vec<String>,
        listeners: Option<u64>,
        /// Прирост слушателей за месяц.
        delta: Option<i64>,
    },
    Playlists {
        title: String,
        playlists: Vec<PlaylistItem>,
    },
    Search {
        query: String,
        /// Волны под запрос (настроение, жанр, занятие) — как в поиске официального клиента.
        waves: Vec<crate::WavePreset>,
        tracks: Vec<TrackItem>,
        albums: Vec<AlbumItem>,
        artists: Vec<ArtistItem>,
        playlists: Vec<PlaylistItem>,
    },
    Albums {
        title: String,
        albums: Vec<AlbumItem>,
        /// Ожидаемые релизы (в «Новинках» — пресейвы пользователя).
        upcoming: Vec<UpcomingItem>,
    },
    /// Список исполнителей (любимые).
    Artists {
        title: String,
        artists: Vec<ArtistItem>,
    },
    Artist {
        artist: ArtistItem,
        /// Исполнитель в коллекции.
        liked: bool,
        /// Слушателей за месяц.
        listeners: Option<u64>,
        genres: Vec<String>,
        about: Option<String>,
        /// Популярные треки (их и играет `play_from`).
        tracks: Vec<TrackItem>,
        /// Треки исполнителя из коллекции и услышанные в волне. В источнике страницы идут за
        /// популярными: сначала коллекция (с индекса `tracks.len()`), потом волна.
        collection: Vec<TrackItem>,
        wave: Vec<TrackItem>,
        albums: Vec<AlbumItem>,
        also: Vec<AlbumItem>,
        similar: Vec<ArtistItem>,
        /// Анонсированный релиз исполнителя.
        upcoming: Option<UpcomingItem>,
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
    /// Альбом; `true` — с «Похожим».
    Album(String, bool),
    Artist(String),
    Similar { track_id: String, title: String },
    Search(String),
    Releases,
    Chart,
    /// Всё скачанное, недавно игранное сверху (без сети тоже).
    Downloaded,
    /// Любимые исполнители.
    LikedArtists,
    /// Альбомы в коллекции.
    LikedAlbums,
    /// «Вы недавно слушали».
    History,
    /// «Об исполнителе» целиком.
    About(String),
    /// Чужие плейлисты в коллекции.
    LikedPlaylists,
    /// Скрытые треки («не рекомендовать»).
    Disliked,
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
        extra: Extra,
    },
    Search(String, SearchResult),
    About(api::AboutArtist),
    Playlists(String, Vec<Playlist>),
    Albums(String, Vec<Album>, Vec<api::Upcoming>),
    Artists(String, Vec<Artist>),
    Artist(String, Box<ArtistBrief>),
}

pub type Reply = (u64, Result<Loaded, String>);

/// Дополнения к списку треков: «Похожее» у альбома, пометка списка скрытых.
#[derive(Default)]
pub struct Extra {
    pub similar: Option<api::Similar>,
    /// Скрытые треки: в меню трека — «Вернуть в рекомендации».
    pub hidden: bool,
}

/// Треки открытых страниц: id → (название очереди, треки); сиды волны «по этому списку».
#[derive(Default)]
pub struct Pages {
    pub tracks: HashMap<u64, (String, Vec<Track>)>,
    pub seeds: HashMap<u64, String>,
}

impl Pages {
    /// Ответ сети → страница для Kotlin; треки запоминаются под id.
    pub fn accept(&mut self, e: &Engine, id: u64, r: Result<Loaded, String>) -> Page {
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
                extra,
            } => {
                let items = tracks.iter().map(TrackItem::from).collect();
                let sim = extra.similar.unwrap_or_default();
                self.tracks.insert(id, (title.clone(), tracks));
                let playlist = seed.as_deref().is_some_and(|s| s.starts_with("playlist:"));
                if let Some(seed) = seed {
                    self.seeds.insert(id, seed);
                }
                Page::Tracks {
                    playlist,
                    liked: album.as_ref().is_some_and(|a| e.liked_albums.contains(&a.id)),
                    title,
                    subtitle,
                    cover,
                    album: album.as_ref().map(AlbumItem::from),
                    tracks: items,
                    hidden: extra.hidden,
                    similar_albums: sim.albums.iter().map(AlbumItem::from).collect(),
                    similar_artists: sim.artists.iter().filter(|a| !a.id.is_empty()).map(ArtistItem::from).collect(),
                    similar_waves: sim.waves.iter().map(wave_preset).collect(),
                }
            }
            Loaded::About(a) => Page::About {
                description: a.description.filter(|d| !d.is_empty()),
                links: a
                    .links
                    .into_iter()
                    .map(|l| LinkItem { title: l.title, subtitle: l.subtitle, url: l.url })
                    .collect(),
                photos: a.covers.iter().filter_map(|c| c.uri.as_deref()).map(|u| api::cover_url(u, 1000)).collect(),
                listeners: a.stats.and_then(|s| s.last_month_listeners),
                delta: a.stats.and_then(|s| s.last_month_listeners_delta),
            },
            Loaded::Playlists(title, v) => Page::Playlists {
                title,
                playlists: v.iter().map(PlaylistItem::from).collect(),
            },
            Loaded::Search(query, r) => {
                let tracks = r.tracks.map(|b| b.results).unwrap_or_default();
                let items = tracks.iter().map(TrackItem::from).collect();
                self.tracks.insert(id, (format!("Поиск: {query}"), tracks));
                Page::Search {
                    query,
                    waves: r.waves.iter().map(wave_preset).collect(),
                    tracks: items,
                    albums: r.albums.map(|b| b.results.iter().map(AlbumItem::from).collect()).unwrap_or_default(),
                    artists: r.artists.map(|b| b.results.iter().map(ArtistItem::from).collect()).unwrap_or_default(),
                    playlists: r
                        .playlists
                        .map(|b| b.results.iter().map(PlaylistItem::from).collect())
                        .unwrap_or_default(),
                }
            }
            Loaded::Albums(title, v, up) => Page::Albums {
                title,
                albums: v.iter().map(AlbumItem::from).collect(),
                upcoming: up.iter().map(UpcomingItem::from).collect(),
            },
            Loaded::Artists(title, v) => Page::Artists {
                title,
                artists: v.iter().map(ArtistItem::from).collect(),
            },
            Loaded::Artist(name, b) => {
                let info = b.artist.as_ref();
                let (popular, coll) = (b.popular_tracks.len(), b.collection.len());
                let mut tracks = b.popular_tracks.clone();
                tracks.extend(b.collection.iter().cloned());
                tracks.extend(b.wave.iter().cloned());
                let items: Vec<TrackItem> = tracks.iter().map(TrackItem::from).collect();
                let items_p = items[..popular].to_vec();
                let collection = items[popular..popular + coll].to_vec();
                let wave = items[popular + coll..].to_vec();
                self.tracks.insert(id, (name.clone(), tracks));
                if let Some(a) = info.filter(|a| !a.id.is_empty()) {
                    self.seeds.insert(id, format!("artist:{}", a.id));
                }
                let albums = if b.discography.is_empty() { &b.albums } else { &b.discography };
                Page::Artist {
                    liked: info.is_some_and(|a| e.liked_artists.contains(&a.id)),
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
                    tracks: items_p,
                    collection,
                    wave,
                    albums: albums.iter().map(AlbumItem::from).collect(),
                    also: b.also_albums.iter().map(AlbumItem::from).collect(),
                    similar: b.similar_artists.iter().map(ArtistItem::from).collect(),
                    upcoming: b.upcoming_album.as_ref().map(UpcomingItem::from),
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
        extra: Extra::default(),
    }
}

/// Исполнитель без сети: только его скачанные треки (в блоке «Из коллекции»), остального нет.
fn offline_artist(e: &Engine, aid: &str) -> Result<Loaded, String> {
    let mut entries = e.library.entries();
    entries.sort_by_key(|(_, ts, _)| std::cmp::Reverse(*ts));
    let ts: Vec<Track> = entries
        .iter()
        .filter_map(|(id, ..)| e.snapshot.find(id))
        .filter(|t| t.artists.iter().any(|a| a.id == aid))
        .cloned()
        .collect();
    let name = ts
        .iter()
        .flat_map(|t| &t.artists)
        .find(|a| a.id == aid)
        .map(|a| a.name.clone())
        .ok_or("Скачанных треков этого исполнителя нет")?;
    let brief = ArtistBrief {
        artist: None,
        albums: Vec::new(),
        also_albums: Vec::new(),
        popular_tracks: Vec::new(),
        similar_artists: Vec::new(),
        stats: None,
        discography: Vec::new(),
        familiar: Vec::new(),
        collection: e.offline_tracks(ts),
        wave: Vec::new(),
        concerts: Vec::new(),
        upcoming_album: None,
        about: None,
    };
    Ok(Loaded::Artist(name, Box::new(brief)))
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
                extra: Extra::default(),
            }),
            None => Err("Этот плейлист не успел сохраниться для офлайна".into()),
        };
        let _ = tx.send((id, r));
        return;
    }
    if let Req::Artist(aid) = &req
        && e.client().is_none()
    {
        let _ = tx.send((id, offline_artist(e, aid)));
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
                extra: Extra::default(),
            })
        }
        Req::Album(id, with_similar) => {
            // «Похожее» — дополнение: его ошибка альбом не роняет; выключено в настройках — не спрашиваем.
            let sim = async {
                if with_similar { c.album_similar(&id).await.map_err(|e| tracing::warn!("похожее на альбом {id}: {e}")).ok() } else { None }
            };
            let (a, similar) = tokio::join!(c.album(&id), sim);
            let a = a.map_err(err_str)?;
            let tracks = a.volume_tracks();
            Ok(Loaded::Tracks {
                title: a.full_title(),
                subtitle: a.artist_names(),
                cover: a.cover_uri().map(|u| api::cover_url(u, 400)),
                album: Some(Album { volumes: Vec::new(), ..a }),
                tracks,
                seed: Some(format!("album:{id}")),
                extra: Extra { similar, hidden: false },
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
                extra: Extra::default(),
            })
        }
        Req::Search(q) => Ok(Loaded::Search(q.clone(), c.search(&q).await.map_err(err_str)?)),
        Req::History => {
            let tracks = c.history().await.map_err(err_str)?;
            if tracks.is_empty() {
                return Err("История пуста: включите что-нибудь — и оно появится здесь".into());
            }
            Ok(Loaded::Tracks {
                title: "Вы недавно слушали".into(),
                subtitle: tracks_word(tracks.len()),
                cover: None,
                album: None,
                tracks,
                seed: None,
                extra: Extra::default(),
            })
        }
        Req::About(id) => Ok(Loaded::About(c.about_artist(&id).await.map_err(err_str)?)),
        Req::LikedPlaylists => {
            Ok(Loaded::Playlists("Сохранённые плейлисты".into(), c.liked_playlists().await.map_err(err_str)?))
        }
        Req::Disliked => {
            let tracks = c.disliked_tracks().await.map_err(err_str)?;
            if tracks.is_empty() {
                return Err("Скрытых треков нет: то, что вы попросите не рекомендовать, появится здесь".into());
            }
            Ok(Loaded::Tracks {
                title: "Скрытые треки".into(),
                subtitle: format!("{}: волна их не ставит", tracks_word(tracks.len())),
                cover: None,
                album: None,
                tracks,
                seed: None,
                extra: Extra { similar: None, hidden: true },
            })
        }
        Req::LikedAlbums => Ok(Loaded::Albums("Альбомы".into(), c.liked_albums().await.map_err(err_str)?, Vec::new())),
        Req::LikedArtists => Ok(Loaded::Artists("Исполнители".into(), c.liked_artists().await.map_err(err_str)?)),
        Req::Releases => {
            // Пресейвы — дополнение: их ошибка новинки не роняет.
            let (albums, pre) = tokio::join!(c.new_releases(), c.presaves());
            let pre = pre.unwrap_or_else(|e| {
                tracing::warn!("пресейвы: {e}");
                Vec::new()
            });
            Ok(Loaded::Albums("Новые релизы".into(), albums.map_err(err_str)?, pre))
        }
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
                extra: Extra::default(),
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
    let item = |p: &Playlist| PlaylistItem { mine: e.is_own(p), ..PlaylistItem::from(p) };
    let own = match &e.playlists {
        Loadable::Ready(v) => v.iter().map(item).collect(),
        _ => e.snapshot.playlists.iter().map(item).collect(),
    };
    let personal = e.personal.iter().map(|p| PlaylistItem::from(&p.playlist)).collect();
    (own, personal)
}

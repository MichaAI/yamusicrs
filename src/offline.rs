//! Офлайн-режим: снимок библиотеки (Коллекция, плейлисты и их треки) на диске.
//! Без сети приложение открывается по нему и играет то, что уже лежит в локальной
//! библиотеке. Треки плейлиста перечитываются, только когда сменилась его ревизия.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::{Playlist, Track};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub likes: Vec<Track>,
    pub playlists: Vec<Playlist>,
    /// `uid:kind` → (ревизия, треки).
    pub playlist_tracks: HashMap<String, (u32, Vec<Track>)>,
    /// Метаданные скачанных треков, которых нет ни в лайках, ни в плейлистах (для офлайн-волны).
    pub library_tracks: HashMap<String, Track>,
}

pub fn key(pl: &Playlist) -> String {
    format!("{}:{}", pl.uid, pl.kind)
}

impl Snapshot {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("offline.json")
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| {
                serde_json::from_slice(&b)
                    .map_err(|e| tracing::warn!("offline.json не читается: {e}"))
                    .ok()
            })
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.likes.is_empty() && self.playlists.is_empty()
    }

    /// Нужно ли (пере)читать треки плейлиста: их нет или ревизия сменилась.
    pub fn stale(&self, pl: &Playlist) -> bool {
        self.playlist_tracks.get(&key(pl)).is_none_or(|(rev, _)| *rev != pl.revision)
    }

    pub fn tracks_of(&self, pl: &Playlist) -> Option<&Vec<Track>> {
        self.playlist_tracks.get(&key(pl)).map(|(_, t)| t)
    }

    /// Трек по id среди всего сохранённого — для восстановления очереди без сети.
    pub fn find(&self, id: &str) -> Option<&Track> {
        let id = id.split(':').next().unwrap_or(id);
        self.library_tracks.get(id).or_else(|| {
            self.likes
                .iter()
                .chain(self.playlist_tracks.values().flat_map(|(_, t)| t))
                .find(|t| t.id == id)
        })
    }

    /// Запись в фоне и атомарно: снимок бывает в несколько мегабайт.
    pub fn save(&self, path: PathBuf) {
        let snap = self.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(b) = serde_json::to_vec(&snap) else { return };
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        });
    }
}

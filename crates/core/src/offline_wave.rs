//! Офлайн-волна: программа TinyMl с сервера (`rotor/get-offline-recommender`), её состояние
//! между запусками и сессия волны поверх VM. Программа собрана под конкретный набор скачанных
//! треков, поэтому перезапрашивается, когда библиотека заметно изменилась.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tinyml::{Event, EventKind, Vm, VmError};

/// Старше — перезапрашиваем, даже если библиотека не менялась (сервер мог переобучиться).
const MAX_AGE_SECS: i64 = 3 * 24 * 3600;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub program: Vec<i32>,
    /// Под какие треки собрана программа.
    pub ids: Vec<i32>,
    pub fetched_at: i64,
    /// `offlineRecommenderData` из последнего ответа ротора — подсказка для startWave.
    pub server_data: Vec<i32>,
}

fn store_path(dir: &Path) -> PathBuf {
    dir.join("tinyml.json")
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("tinyml_state.bin")
}

impl Store {
    pub fn load(dir: &Path) -> Option<Self> {
        let b = std::fs::read(store_path(dir)).ok()?;
        serde_json::from_slice(&b)
            .map_err(|e| tracing::warn!("tinyml.json не читается: {e}"))
            .ok()
            .filter(|s: &Store| !s.program.is_empty())
    }

    pub fn save(&self, dir: &Path) {
        let (s, path) = (self.clone(), store_path(dir));
        tokio::task::spawn_blocking(move || {
            if let Ok(b) = serde_json::to_vec(&s) {
                write_atomic(&path, &b);
            }
        });
    }

    /// Пора ли перезапросить программу под текущую библиотеку.
    pub fn stale(&self, library: &HashSet<i32>) -> bool {
        let have: HashSet<i32> = self.ids.iter().copied().collect();
        let diff = have.symmetric_difference(library).count();
        let age = chrono::Utc::now().timestamp() - self.fetched_at;
        age > MAX_AGE_SECS || diff > 20.max(library.len() / 20)
    }
}

fn write_atomic(path: &Path, data: &[u8]) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, data).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn load_state(dir: &Path) -> Option<Vec<i32>> {
    let b = std::fs::read(state_path(dir)).ok()?;
    Some(b.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

pub fn save_state(dir: &Path, ram: Vec<i32>) {
    let path = state_path(dir);
    tokio::task::spawn_blocking(move || {
        let bytes: Vec<u8> = ram.iter().flat_map(|v| v.to_le_bytes()).collect();
        write_atomic(&path, &bytes);
    });
}

/// Идущая офлайн-волна.
pub struct Session {
    vm: Vm,
    events: Vec<Event>,
    dir: PathBuf,
}

impl Session {
    /// Запуск: состояние прошлых сессий подхватывается, если программа та же.
    /// Возвращает сессию и первые треки.
    pub fn start(
        store: &Store,
        dir: &Path,
        available: &[i32],
        context: &[(i32, Option<i32>)],
        seeds: &[String],
    ) -> Result<(Self, Vec<i32>), VmError> {
        let mut vm = Vm::new(store.program.clone(), load_state(dir), 0)?;
        // Как в официальном клиенте: одна программа на библиотеку, вид волны — только её сиды.
        let s = vm.start_wave(&store.server_data, context, seeds, available)?;
        let mut first: Vec<i32> = Vec::new();
        for t in s.tracks {
            if t > 0 && available.contains(&t) && !first.contains(&t) {
                first.push(t);
            }
        }
        tracing::info!("офлайн-волна: старт {first:?}, batch {}", s.batch);
        Ok((
            Self {
                vm,
                events: Vec::new(),
                dir: dir.to_path_buf(),
            },
            first,
        ))
    }

    pub fn event(&mut self, track: i32, kind: EventKind, secs: f32) {
        self.events.push(Event::now(track, kind, secs));
        // Программа смотрит на последние 256.
        if self.events.len() > 512 {
            self.events.drain(..256);
        }
    }

    /// Трек после `queued`; `last` — тот, что сейчас начал играть.
    pub fn next(&mut self, last: i32, queued: i32) -> Result<i32, VmError> {
        let t = std::time::Instant::now();
        let n = self.vm.next_track(&self.events, last, queued)?;
        tracing::debug!("офлайн-волна: {} за {:?} ({} шагов)", n.track, t.elapsed(), self.vm.steps);
        // Состояние (статистика прослушиваний) — на диск: переживёт перезапуск.
        save_state(&self.dir, self.vm.state().to_vec());
        Ok(n.track)
    }
}

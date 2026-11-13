//! Умное скачивание, как в мобильном клиенте (`rotor/get-cache-recommendation`): сервер смотрит,
//! что уже скачано, и отвечает, какие треки докачать для офлайн-волны, а какие из подобранных им
//! раньше удалить. Сыгранное и скачанное самим пользователем не удаляется никогда.
//! Когда качать, решает пользователь (окно по часам и «только не мобильная сеть»);
//! интервалы из ответа сервера не используются.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Effect, Engine, Msg, err_str};
use crate::api::{Client, Quality};
use crate::fetch::Fetcher;
use crate::library::Library;

/// Итог прохода.
pub struct SmartReport {
    /// Когда спросить сервер снова, unix-секунды.
    pub next: i64,
    pub added: usize,
    pub deleted: usize,
}

/// Состояние между запусками: когда следующий запрос.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SmartState {
    pub next_request: i64,
}

fn state_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("smart_download.json")
}

impl SmartState {
    pub fn load(dir: &std::path::Path) -> Self {
        std::fs::read(state_path(dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, dir: &std::path::Path) {
        if let Ok(b) = serde_json::to_vec(self) {
            let _ = std::fs::write(state_path(dir), b);
        }
    }
}

/// Когда и где можно докачивать.
#[derive(Clone, Copy)]
struct Allowed {
    from: u8,
    to: u8,
    wifi_only: bool,
}

impl Allowed {
    fn of(cfg: &crate::config::Config) -> Self {
        Self { from: cfg.smart_from_hour % 24, to: cfg.smart_to_hour % 24, wifi_only: cfg.smart_wifi_only }
    }

    /// `None` — можно; иначе — почему нельзя (для лога).
    fn blocked(self) -> Option<&'static str> {
        use chrono::Timelike;
        if self.wifi_only && crate::api::NET.network.load(std::sync::atomic::Ordering::Relaxed) == 2 {
            return Some("мобильная сеть");
        }
        if self.from != self.to {
            let h = chrono::Local::now().hour() as u8;
            let inside = if self.from < self.to { (self.from..self.to).contains(&h) } else { h >= self.from || h < self.to };
            if !inside {
                return Some("не то время");
            }
        }
        None
    }
}

impl Engine {
    /// Раз в минуту: не пора ли спросить сервер.
    pub(super) fn smart_tick(&mut self) {
        if !self.cfg.smart_download || self.offline || self.smart_running || self.fetcher.is_none() {
            return;
        }
        if self.smart_checked.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        self.smart_checked = Some(Instant::now());
        // Лайки нужны, чтобы отделить «Мне нравится» от прочего скачанного.
        if !matches!(self.likes, super::Loadable::Ready(_)) || chrono::Utc::now().timestamp() < self.smart.next_request {
            return;
        }
        if Allowed::of(&self.cfg).blocked().is_some() {
            return;
        }
        self.smart_start();
    }

    /// Включить или выключить умное скачивание; включённое спрашивает сервер сразу.
    pub fn set_smart_download(&mut self, on: bool) {
        self.cfg.smart_download = on;
        self.cfg.save(&self.paths);
        if on {
            self.smart.next_request = 0;
            self.smart_checked = None;
        }
    }

    /// Скачать треки (плейлист целиком) в библиотеку. Идёт в фоне по одному; прогресс видно по росту
    /// числа скачанных, итог — тостом. Недоступные и уже скачанные пропускаются.
    pub fn download_tracks(&mut self, tracks: Vec<crate::api::Track>) {
        let Some(fetcher) = self.fetcher.clone().filter(|_| !self.offline) else {
            self.toast_err("Без сети скачивать нельзя".into());
            return;
        };
        if self.bulk_running {
            self.toast("Скачивание уже идёт".into());
            return;
        }
        let want: Vec<_> = tracks.into_iter().filter(|t| t.available && !self.library.contains(&t.id)).collect();
        if want.is_empty() {
            self.toast("Всё уже скачано".into());
            return;
        }
        self.bulk_running = true;
        self.toast(format!("Скачиваю {} тр.", want.len()));
        let (q, tx) = (self.quality(), self.tx.clone());
        tokio::spawn(async move {
            let (mut ok, mut failed) = (0, 0);
            for t in &want {
                match fetcher.save(t, q).await {
                    Ok(true) => ok += 1,
                    Ok(false) => {}
                    Err(e) => {
                        failed += 1;
                        tracing::warn!("скачивание {}: {e:#}", t.id);
                    }
                }
            }
            let _ = tx.send(Msg::BulkDone { ok, failed });
        });
    }

    pub(super) fn on_bulk_done(&mut self, ok: usize, failed: usize) {
        self.bulk_running = false;
        if failed > 0 {
            self.toast_err(format!("Скачано {ok}, не вышло {failed}"));
        } else {
            self.toast(format!("Скачано: {ok} тр."));
        }
        if ok > 0 {
            // Как после умного скачивания: метаданные в офлайн-снимок и программа офлайн-волны.
            self.backfill_library_meta();
            self.refresh_offline_program();
            self.emit(Effect::LibraryMeta);
        }
    }

    fn smart_start(&mut self) {
        let Some(fetcher) = self.fetcher.clone() else { return };
        let dto = |id: &str, ts: i64| json!({ "id": id, "timestamp": ts });
        let (mut user, mut liked, mut smart) = (Vec::new(), Vec::new(), Vec::new());
        for (id, ts, s) in self.library.entries() {
            if s {
                smart.push(dto(&id, ts));
            } else if self.liked.contains(&id) {
                liked.push(dto(&id, ts));
            } else {
                user.push(dto(&id, ts));
            }
        }
        let mb = |b: u64| b / (1024 * 1024);
        let limit = (self.cfg.smart_limit_gb.max(0.0) * 1024.0 * 1024.0 * 1024.0) as u64;
        let (_, total) = self.library.stats();
        let body = json!({
            "userCachedTracks": user,
            "autoCachedLikedTracks": liked,
            "smartCachedTracks": smart,
            "smartCacheStorageTotalSizeMb": mb(limit),
            "smartCacheStorageOccupiedSizeMb": mb(self.library.smart_bytes()),
            "totalOccupiedSizeMb": mb(total),
            "currentQuality": self.cfg.smart_quality.as_str(),
        });
        let (q, lib, allowed) = (self.cfg.smart_quality, self.library.clone(), Allowed::of(&self.cfg));
        self.smart_running = true;
        self.spawn(move |c, tx| async move {
            let r = run(c, fetcher, lib, body, q, limit, allowed).await.map_err(err_str);
            let _ = tx.send(Msg::SmartDone(r));
        });
    }

    pub(super) fn on_smart_done(&mut self, r: Result<SmartReport, String>) {
        self.smart_running = false;
        match r {
            Ok(rep) => {
                tracing::info!("умное скачивание: +{} −{}, следующий запрос в {}", rep.added, rep.deleted, rep.next);
                self.smart.next_request = rep.next;
                if rep.added + rep.deleted > 0 {
                    // Новые треки — в снимок (офлайн-волне нужны их метаданные) и в программу TinyMl.
                    self.backfill_library_meta();
                    self.refresh_offline_program();
                    self.emit(Effect::LibraryMeta);
                }
            }
            Err(e) => {
                tracing::warn!("умное скачивание: {e}");
                // Как официальный клиент: повтор через полчаса.
                self.smart.next_request = chrono::Utc::now().timestamp() + 30 * 60;
            }
        }
        self.smart.save(&self.data_dir);
    }
}

async fn run(
    c: Client,
    fetcher: Fetcher,
    lib: Arc<Library>,
    body: serde_json::Value,
    q: Quality,
    limit: u64,
    allowed: Allowed,
) -> color_eyre::Result<SmartReport> {
    let rec = c.cache_recommendation(&body).await?;
    let next = rec.next_request_time.unwrap_or_else(|| chrono::Utc::now().timestamp() + 24 * 3600);
    let del = rec.tracks_to_delete.clone();
    let l = lib.clone();
    let deleted = tokio::task::spawn_blocking(move || l.remove_smart(&del)).await.unwrap_or(0);
    let want: Vec<String> = rec.tracks_to_add.into_iter().filter(|id| !lib.contains(id)).collect();
    tracing::info!("умное скачивание: сервер просит {} докачать, {} удалить", want.len(), rec.tracks_to_delete.len());
    let mut added = 0;
    'all: for chunk in want.chunks(50) {
        for t in c.tracks(chunk).await? {
            if let Some(why) = allowed.blocked() {
                // Остальное докачается в следующем окне: сервер спросим снова через полчаса.
                tracing::info!("умное скачивание: пауза ({why})");
                return Ok(SmartReport { next: chrono::Utc::now().timestamp() + 30 * 60, added, deleted });
            }
            if lib.smart_bytes() >= limit {
                tracing::info!("умное скачивание: место под него кончилось");
                break 'all;
            }
            if !t.available {
                continue;
            }
            match fetcher.save(&t, q).await {
                Ok(true) => {
                    lib.mark_smart(&t.id);
                    added += 1;
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("умное скачивание {}: {e}", t.id),
            }
        }
    }
    Ok(SmartReport { next, added, deleted })
}

//! Ynison в движке: что делать со звуком по командам других устройств и что им сообщать.
//!
//! Правила (как у официальных устройств, проверено Music Assistant вживую):
//! - пока активно другое устройство, мы только видны в списке и ничего не публикуем;
//! - нас выбрали на телефоне — берём его очередь и статус и играем с той же секунды;
//! - активны мы — команды телефона (пауза, перемотка, другой трек) применяются здесь,
//!   а свои изменения уходят на сервер;
//! - включили что-то здесь, пока активен другой, — забираем активность себе;
//! - активность ушла (выбрали другое устройство или сервер сам поставил паузу) — пауза здесь.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{Effect, Engine, Msg, QueueKind, err_str};
use crate::api::Track;
use crate::ynison::{self, Event, Out, Playable, QueueState, Status};

/// Окно очереди, которое уходит на сервер (официальные клиенты тоже не шлют больше 200).
const WINDOW: usize = 200;
/// Пока играем и активны — позиция на сервер раз в столько (между событиями).
const HEARTBEAT: Duration = Duration::from_secs(15);
/// Расхождение позиции с командой, после которого перематываем.
const SEEK_SLACK_MS: u64 = 2500;

impl Engine {
    /// Подключиться, если включено в настройках, есть токен и это не тестовый экземпляр.
    pub fn ynison_start(&mut self) {
        if self.ynison.is_some() || !self.cfg.ynison || self.offline {
            return;
        }
        // Тестовый экземпляр (YAMUSICRS_MPRIS_NAME) не должен перехватывать музыку с телефона.
        let forced = std::env::var("YAMUSICRS_YNISON").ok();
        if forced.as_deref() == Some("0")
            || (std::env::var_os("YAMUSICRS_MPRIS_NAME").is_some() && forced.as_deref() != Some("1"))
        {
            return;
        }
        let Some(token) = self.stored_token.as_ref().map(|t| t.access_token.clone()) else {
            return;
        };
        let id = self.ynison_device_id();
        let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|h| h.trim().to_string())
            .unwrap_or_default();
        let title = self.yn_title.clone().unwrap_or_else(|| {
            if host.is_empty() {
                "yamusicrs".to_string()
            } else {
                format!("yamusicrs · {host}")
            }
        });
        let tx = self.tx.clone();
        let delay = 0;
        self.ynison = Some(ynison::spawn(token, id, title, self.yn_kind, delay, move |e| {
            let _ = tx.send(Msg::Ynison(e));
        }));
    }

    pub fn ynison_stop(&mut self) {
        self.ynison = None;
        self.yn = None;
        self.yn_active = false;
        self.yn_connected = false;
    }

    /// Постоянный id устройства: телефон видит одно и то же устройство между запусками.
    fn ynison_device_id(&self) -> String {
        let path = self
            .paths
            .token_file
            .parent()
            .map(|d| d.join("ynison-device-id"))
            .unwrap_or_else(|| "ynison-device-id".into());
        if let Ok(s) = std::fs::read_to_string(&path)
            && !s.trim().is_empty()
        {
            return s.trim().to_string();
        }
        let id = format!("yamusicrs-{}", hex::encode(rand::random::<[u8; 8]>()));
        let _ = std::fs::write(&path, &id);
        id
    }

    pub fn ynison_label(&self) -> String {
        if !self.cfg.ynison {
            return "выкл".into();
        }
        if self.ynison.is_none() {
            return "вкл (без сети)".into();
        }
        if !self.yn_connected {
            if let Some(left) = self.yn_wait_until.and_then(|t| t.checked_duration_since(Instant::now()))
                && left.as_secs() >= 60
            {
                let m = left.as_secs().div_ceil(60);
                return if m >= 60 {
                    format!("вкл · Яндекс не пускает, повтор через {} ч", m.div_ceil(60))
                } else {
                    format!("вкл · Яндекс не пускает, повтор через {m} мин")
                };
            }
            return "вкл · подключаюсь".into();
        }
        if self.yn_active {
            return "вкл · играет здесь".into();
        }
        match self.yn_other_device() {
            Some(t) => format!("вкл · активно «{t}»"),
            None => "вкл · на связи".into(),
        }
    }

    /// Название активного устройства, если это не мы.
    pub fn yn_other_device(&self) -> Option<String> {
        let r = self.yn.as_ref()?;
        let me = self.ynison.as_ref()?.device_id.as_str();
        let a = r.active.as_deref().filter(|a| *a != me)?;
        Some(r.device_title(a).filter(|t| !t.is_empty()).unwrap_or("другое устройство").to_string())
    }

    // ── входящее ─────────────────────────────────────────────────────────────

    pub fn on_ynison(&mut self, ev: Event) {
        match ev {
            Event::Connected => {
                self.yn_connected = true;
                self.yn_wait_until = None;
                tracing::info!("ynison: подключено");
                // Соединение моргнуло, а музыка здесь играет — снова заявляем, что играет здесь.
                if self.now.is_some() && !self.player.paused() {
                    self.yn_claim();
                }
            }
            Event::Lost { error, retry_in } => {
                let was = self.yn_connected;
                self.yn_connected = false;
                self.yn_active = false;
                if was || retry_in >= 600 {
                    tracing::warn!("ynison: {error}; повтор через {retry_in} с");
                }
                self.yn_wait_until = Some(Instant::now() + Duration::from_secs(retry_in));
                if retry_in >= 600 && was {
                    self.toast_err(format!("Ynison недоступен ({error}); следующая попытка через {} мин", retry_in / 60));
                }
            }
            Event::State(r) => self.on_ynison_state(r),
        }
    }

    fn on_ynison_state(&mut self, r: ynison::Remote) {
        let Some(me) = self.ynison.as_ref().map(|h| h.device_id.clone()) else {
            return;
        };
        let now_active = r.active.as_deref() == Some(me.as_str());
        let became = now_active && !self.yn_active;
        if !now_active && self.yn_active {
            // Активность ушла: выбрали другое устройство или сервер поставил паузу.
            self.yn_active = false;
            if self.now.is_some() && !self.player.paused() {
                self.player.set_paused(true);
                let pos = self.now.as_ref().map_or(0.0, |n| n.pos);
                self.emit(Effect::Playback { playing: false, pos });
                let to = r
                    .active
                    .as_deref()
                    .and_then(|a| r.device_title(a))
                    .filter(|t| !t.is_empty())
                    .map(|t| format!("«{t}»"))
                    .unwrap_or_else(|| "другое устройство".into());
                self.toast(format!("⇄ Музыка переехала на {to}"));
            }
        }
        self.yn_active = now_active;
        if now_active {
            let (queue_cmd, status_cmd) = (r.queue_cmd || became, r.status_cmd || became);
            if let (true, Some(q), Some(s)) = (queue_cmd, r.queue.clone(), r.status) {
                if became {
                    self.toast("⇄ Ynison: играет здесь".into());
                }
                self.yn_apply_queue(q, s);
            } else if let (true, Some(s)) = (status_cmd, r.status) {
                self.yn_apply_status(s);
            }
        }
        if r.status != self.yn.as_ref().and_then(|o| o.status) {
            self.yn_state_at = Instant::now();
        }
        self.yn = Some(r);
    }

    /// Очередь другого устройства: тот же список — только перейти; другой — загрузить треки.
    fn yn_apply_queue(&mut self, q: QueueState, s: Status) {
        let Ok(idx) = usize::try_from(q.index) else { return };
        let Some(target) = q.items.get(idx) else { return };
        let same_list = q.items.len() == self.queue.items.len()
            && q.items.iter().zip(&self.queue.items).all(|(p, t)| p.id == t.id);
        if same_list {
            if self.queue.pos == Some(idx) && self.now.as_ref().is_some_and(|n| n.track.id == target.id) {
                self.yn_apply_status(s);
            } else {
                self.yn_remote_start = true;
                self.play_index_at(idx, s.progress_ms as f64 / 1000.0, s.paused);
            }
            return;
        }
        // Нужные треки уже есть в нашей очереди (например, телефон обрезал её окном) — без запроса.
        let have: HashMap<&str, &Track> = self.queue.items.iter().map(|t| (t.id.as_str(), t)).collect();
        if q.items.iter().all(|p| have.contains_key(p.id.as_str())) {
            let tracks: Vec<Track> = q.items.iter().map(|p| have[p.id.as_str()].clone()).collect();
            return self.yn_load(q, s, tracks);
        }
        self.yn_fetch_gen += 1;
        let generation = self.yn_fetch_gen;
        let ids: Vec<String> = q
            .items
            .iter()
            .map(|p| match &p.album_id {
                Some(a) => format!("{}:{a}", p.id),
                None => p.id.clone(),
            })
            .collect();
        self.spawn(move |c, tx| async move {
            let result = c.tracks(&ids).await.map_err(err_str);
            let _ = tx.send(Msg::YnisonQueue {
                generation,
                queue: q,
                status: s,
                result,
            });
        });
    }

    pub fn on_ynison_queue(
        &mut self,
        generation: u64,
        q: QueueState,
        s: Status,
        result: Result<Vec<Track>, String>,
    ) {
        if generation != self.yn_fetch_gen || !self.yn_active {
            return;
        }
        match result {
            Ok(tracks) => self.yn_load(q, s, tracks),
            Err(e) => self.toast_err(format!("Ynison: очередь не загрузилась: {e}")),
        }
    }

    /// Заменить очередь присланной и включить нужный трек с нужной секунды.
    fn yn_load(&mut self, q: QueueState, s: Status, tracks: Vec<Track>) {
        let target = usize::try_from(q.index).ok().and_then(|i| q.items.get(i)).map(|p| p.id.clone());
        let Some(idx) = target.and_then(|id| tracks.iter().position(|t| t.id == id)) else {
            return self.toast_err("Ynison: текущий трек очереди недоступен".into());
        };
        let (kind, title) = match (&q.entity_type[..], &q.wave_session) {
            ("RADIO", Some(session)) => {
                let seeds: Vec<String> = q.entity_id.split(',').filter(|x| !x.is_empty()).map(String::from).collect();
                let name = if seeds.iter().any(|x| x == super::MY_WAVE) || seeds.is_empty() {
                    "Моя волна".to_string()
                } else {
                    "Волна".to_string()
                };
                self.wave.active_seeds = Some(seeds);
                self.wave.active_name = Some(name.clone());
                (
                    QueueKind::Wave {
                        session: session.clone(),
                        batch: None,
                    },
                    name,
                )
            }
            _ => (QueueKind::List, "С другого устройства".to_string()),
        };
        self.repeat_one = q.repeat == "ONE";
        self.replace_queue(tracks, kind, title);
        self.yn_remote_start = true;
        self.play_index_at(idx, s.progress_ms as f64 / 1000.0, s.paused);
    }

    /// Пауза/игра и позиция по команде другого устройства.
    fn yn_apply_status(&mut self, s: Status) {
        if self.now.is_none() {
            if !s.paused
                && let Some(p) = self.queue.pos
            {
                self.yn_remote_start = true;
                self.play_index_at(p, s.progress_ms as f64 / 1000.0, false);
            }
            return;
        }
        if self.player.paused() != s.paused {
            self.player.set_paused(s.paused);
            let pos = self.now.as_ref().map_or(0.0, |n| n.pos);
            self.emit(Effect::Playback { playing: !s.paused, pos });
        }
        let local = self.now.as_ref().map_or(0, |n| (n.pos * 1000.0) as u64);
        if local.abs_diff(s.progress_ms) > SEEK_SLACK_MS
            && let Some(now) = &mut self.now
        {
            let secs = s.progress_ms as f64 / 1000.0;
            self.player.seek(now.token, secs);
            now.pos = secs;
            self.emit(Effect::Seeked(secs));
        }
    }

    // ── исходящее ────────────────────────────────────────────────────────────

    fn yn_status(&self) -> Status {
        let (progress_ms, duration_ms) = match (&self.now, &self.loading_track) {
            (Some(n), _) => ((n.pos * 1000.0) as u64, n.track.duration_ms.unwrap_or(0)),
            (None, Some(t)) => (0, t.duration_ms.unwrap_or(0)),
            _ => (0, 0),
        };
        let paused = match (&self.now, &self.loading_track) {
            (None, None) => true,
            _ => self.player.paused(),
        };
        Status {
            paused,
            progress_ms: if duration_ms > 0 { progress_ms.min(duration_ms) } else { progress_ms },
            duration_ms,
        }
    }

    fn yn_queue(&self) -> Option<QueueState> {
        let pos = self.queue.pos?;
        let n = self.queue.items.len();
        // Окно не больше WINDOW треков вокруг текущего.
        let start = pos.saturating_sub(WINDOW / 2).min(n.saturating_sub(WINDOW));
        let end = (start + WINDOW).min(n);
        let (entity_type, entity_id, wave_session) = match &self.queue.kind {
            QueueKind::Wave { session, .. } => (
                "RADIO",
                self.wave.active_seeds.as_ref().map(|s| s.join(",")).unwrap_or_default(),
                Some(session.clone()),
            ),
            _ => ("VARIOUS", String::new(), None),
        };
        let from = "yamusicrs".to_string();
        Some(QueueState {
            entity_type: entity_type.into(),
            entity_id,
            from: from.clone(),
            index: (pos - start) as i64,
            items: self.queue.items[start..end]
                .iter()
                .map(|t| Playable {
                    id: t.id.clone(),
                    album_id: t.album().map(|a| a.id.clone()).filter(|a| !a.is_empty()),
                    title: t.title.clone(),
                    cover: t.cover_uri().map(str::to_owned),
                    from: from.clone(),
                })
                .collect(),
            repeat: if self.repeat_one { "ONE" } else { "NONE" }.into(),
            wave_session,
        })
    }

    /// Позиция/пауза — на сервер, если мы активны.
    pub fn yn_publish_status(&mut self) {
        if !self.yn_active {
            return;
        }
        if let Some(h) = &self.ynison {
            h.send(Out::Status(self.yn_status()));
            self.yn_last_sent = Instant::now();
        }
    }

    fn yn_publish_player(&mut self) {
        if !self.yn_active {
            return;
        }
        if let (Some(h), Some(q)) = (&self.ynison, self.yn_queue()) {
            h.send(Out::Player(q, self.yn_status()));
            self.yn_last_sent = Instant::now();
        }
    }

    /// Забрать воспроизведение себе (включили что-то здесь).
    fn yn_claim(&mut self) {
        if self.yn_active || !self.yn_connected {
            return;
        }
        if let Some(h) = &self.ynison {
            h.send(Out::Activate);
            self.yn_active = true;
        }
        self.yn_publish_player();
    }

    /// Трек начал играть (или встал на паузу в начале).
    pub fn yn_track_started(&mut self) {
        if std::mem::take(&mut self.yn_remote_start) {
            // Запущено по команде телефона: подтверждаем трек и позицию, активность не трогаем.
            self.yn_publish_player();
            return;
        }
        if !self.yn_active && !self.player.paused() {
            return self.yn_claim();
        }
        self.yn_publish_player();
    }

    /// Пауза/игра нажата здесь.
    pub fn yn_local_play_state(&mut self) {
        if !self.yn_active && !self.player.paused() {
            return self.yn_claim();
        }
        self.yn_publish_status();
    }

    // ── пульт: управление другим активным устройством ───────────────────────

    /// Устройства аккаунта, кроме нас: (id, название, тип, активно ли, можно ли включить на нём).
    pub fn yn_devices(&self) -> Vec<(String, String, String, bool, bool)> {
        let (Some(r), Some(h)) = (&self.yn, &self.ynison) else { return Vec::new() };
        r.devices
            .iter()
            .filter(|d| d.id != h.device_id)
            .map(|d| (d.id.clone(), d.title.clone(), d.kind.clone(), r.active.as_deref() == Some(d.id.as_str()), d.can_play))
            .collect()
    }

    /// Играет другое устройство: его очередь и статус (для пульта).
    pub fn yn_remote(&self) -> Option<(String, &QueueState, Status)> {
        if self.yn_active {
            return None;
        }
        let title = self.yn_other_device()?;
        let r = self.yn.as_ref()?;
        Some((title, r.queue.as_ref()?, r.status?))
    }

    /// Перевести воспроизведение на устройство (или сюда — своим id).
    pub fn yn_transfer(&mut self, device_id: &str) {
        let Some(h) = &self.ynison else { return };
        if device_id == h.device_id {
            // Сюда: забираем активность и, если очередь там есть, продолжаем с той же секунды.
            if let Some(r) = self.yn.clone()
                && let (Some(q), Some(s)) = (r.queue, r.status)
                && !self.yn_active
            {
                h.send(Out::Activate);
                self.yn_active = true;
                self.yn_apply_queue(q, s);
                return;
            }
            return self.yn_claim();
        }
        // Отсюда: пауза здесь, активность — туда.
        if self.yn_active && self.now.is_some() && !self.player.paused() {
            self.player.set_paused(true);
            let pos = self.now.as_ref().map_or(0.0, |n| n.pos);
            self.emit(Effect::Playback { playing: false, pos });
        }
        if self.yn_active {
            // Сначала актуальная позиция, чтобы там продолжили с нужного места.
            self.yn_publish_player();
        }
        if let Some(h) = &self.ynison {
            h.send(Out::Transfer(device_id.to_string()));
        }
        self.yn_active = false;
    }

    /// Пульт: пауза/игра на активном устройстве.
    pub fn yn_remote_toggle(&mut self) {
        let Some((_, _, s)) = self.yn_remote() else { return };
        let progress_ms = self.yn_remote_pos(s);
        if let Some(h) = &self.ynison {
            h.send(Out::RemoteStatus(Status { paused: !s.paused, progress_ms, ..s }));
        }
    }

    /// Пульт: перемотка на активном устройстве.
    pub fn yn_remote_seek(&mut self, ms: u64) {
        let Some((_, _, s)) = self.yn_remote() else { return };
        if let Some(h) = &self.ynison {
            h.send(Out::RemoteStatus(Status { progress_ms: ms.min(s.duration_ms.max(ms)), ..s }));
        }
    }

    /// Пульт: соседний трек очереди активного устройства.
    pub fn yn_remote_skip(&mut self, delta: i64) {
        let Some((_, q, s)) = self.yn_remote() else { return };
        // Назад в первые секунды трека — на предыдущий, позже — в начало этого (как в приложениях).
        let delta = if delta < 0 && self.yn_remote_pos(s) > 3000 { 0 } else { delta };
        let idx = q.index + delta;
        if idx < 0 || idx as usize >= q.items.len() {
            return;
        }
        let q = QueueState { index: idx, ..q.clone() };
        if let Some(h) = &self.ynison {
            h.send(Out::RemotePlayer(q, Status { progress_ms: 0, paused: false, duration_ms: 0 }));
        }
    }

    /// Позиция активного устройства сейчас: последняя присланная плюс прошедшее время, если играет.
    pub fn yn_remote_pos(&self, s: Status) -> u64 {
        if s.paused {
            return s.progress_ms;
        }
        let since = self.yn_state_at.elapsed().as_millis() as u64;
        (s.progress_ms + since).min(if s.duration_ms > 0 { s.duration_ms } else { u64::MAX })
    }

    pub fn yn_tick(&mut self) {
        if self.yn_active && !self.player.paused() && self.now.is_some() && self.yn_last_sent.elapsed() > HEARTBEAT {
            self.yn_publish_status();
        }
    }
}

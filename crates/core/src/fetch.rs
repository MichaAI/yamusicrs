//! Получение трека для воспроизведения: из библиотеки или потоковой загрузкой,
//! с расшифровкой `encraw` на лету и сохранением в библиотеку по завершении.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use aes::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use color_eyre::eyre::{Result, eyre};
use futures_util::StreamExt;
use parking_lot::Mutex;

use crate::api::{Client, DownloadInfo, Quality, Track, cover_url};
use crate::audio::{Source, StreamBuffer};
use crate::library::{Download, Library, codec_rank};

type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

#[derive(Clone)]
pub struct Fetcher {
    client: Client,
    library: Arc<Library>,
    /// Текущее качество стрима; меняется на лету из UI.
    quality: Arc<AtomicU8>,
    /// Активные загрузки: повторный запрос того же трека (prefetch → play) берёт тот же буфер.
    active: Arc<Mutex<HashMap<String, Arc<StreamBuffer>>>>,
    /// Быстрый старт: при lossless сначала AAC (играет сразу), FLAC подменяет его на ходу.
    fast: Arc<AtomicBool>,
    /// Прогретые треки: ссылка и начало файла уже есть — переход на них мгновенный.
    warm: Arc<Mutex<HashMap<String, Warm>>>,
}

/// Начало файла, скачанное заранее; остаток докачивается при открытии.
struct Warm {
    at: Instant,
    rank: u8,
    task: tokio::task::JoinHandle<Result<(DownloadInfo, Arc<StreamBuffer>)>>,
}

/// Сколько прогревать: ~10 с AAC 256.
const WARM_HEAD: u64 = 320 * 1024;
/// Ссылки на файлы подписаны и со временем протухают.
const WARM_TTL: Duration = Duration::from_secs(15 * 60);
const WARM_MAX: usize = 8;

/// Откуда взялся трек — для индикатора в статусной строке.
#[derive(Debug, Clone)]
pub enum Origin {
    Library,
    Network { buf: Arc<StreamBuffer> },
}

pub fn quality_rank(q: Quality) -> u8 {
    match q {
        Quality::Lossless => 3,
        Quality::Hq => 2,
        Quality::Nq => 1,
    }
}

fn quality_from_rank(r: u8) -> Quality {
    match r {
        3 => Quality::Lossless,
        2 => Quality::Hq,
        _ => Quality::Nq,
    }
}

impl Fetcher {
    pub fn new(client: Client, library: Arc<Library>, quality: Quality) -> Self {
        Self {
            client,
            library,
            quality: Arc::new(AtomicU8::new(quality_rank(quality))),
            active: Arc::default(),
            fast: Arc::new(AtomicBool::new(true)),
            warm: Arc::default(),
        }
    }

    pub fn set_fast_start(&self, on: bool) {
        self.fast.store(on, Ordering::Relaxed);
    }

    pub fn fast_start(&self) -> bool {
        self.fast.load(Ordering::Relaxed)
    }

    /// В каком качестве начинать: при быстром старте вместо lossless — AAC.
    fn start_quality(&self) -> Quality {
        match self.quality() {
            Quality::Lossless if self.fast_start() => Quality::Hq,
            q => q,
        }
    }

    /// Прогреть следующие треки: ссылка и первые ~10 с, чтобы переход был мгновенным.
    pub fn warm(&self, tracks: &[Track]) {
        let q = self.start_quality();
        let rank = quality_rank(q);
        let mut warm = self.warm.lock();
        warm.retain(|_, w| {
            let keep = w.at.elapsed() < WARM_TTL && w.rank == rank;
            if !keep {
                w.task.abort();
            }
            keep
        });
        for t in tracks {
            if warm.contains_key(&t.id) || self.active.lock().contains_key(&t.id) || self.library.contains(&t.id) {
                continue;
            }
            let (this, id) = (self.clone(), t.id.clone());
            let task = tokio::spawn(async move {
                let info = this.client.file_info(&id, q).await?;
                let key = cipher_key(&info)?;
                let buf = StreamBuffer::new(info.size)?;
                this.download(&info.urls(), key, &buf, 0, Some(WARM_HEAD)).await?;
                tracing::debug!("прогрет {id}: {} КБ", WARM_HEAD / 1024);
                Ok((info, buf))
            });
            warm.insert(t.id.clone(), Warm { at: Instant::now(), rank, task });
        }
        // Лишние — самые старые.
        while warm.len() > WARM_MAX {
            let Some(old) = warm.iter().min_by_key(|(_, w)| w.at).map(|(k, _)| k.clone()) else { break };
            if let Some(w) = warm.remove(&old) {
                w.task.abort();
            }
        }
    }

    /// Прогретое начало трека, если оно ещё годно.
    async fn take_warm(&self, id: &str) -> Option<(DownloadInfo, Arc<StreamBuffer>, u8)> {
        let w = self.warm.lock().remove(id)?;
        if w.at.elapsed() >= WARM_TTL || w.rank != quality_rank(self.start_quality()) {
            w.task.abort();
            return None;
        }
        match w.task.await {
            Ok(Ok((info, buf))) => Some((info, buf, w.rank)),
            Ok(Err(e)) => {
                tracing::debug!("прогрев {id} не удался: {e}");
                None
            }
            Err(_) => None,
        }
    }

    pub fn quality(&self) -> Quality {
        quality_from_rank(self.quality.load(Ordering::Relaxed))
    }

    pub fn set_quality(&self, q: Quality) {
        self.quality.store(quality_rank(q), Ordering::Relaxed);
    }

    pub async fn open(&self, track: &Track) -> Result<(Source, Origin)> {
        // Уже качается (предзагрузка или апгрейд) — тот же буфер: он не хуже файла.
        if let Some(buf) = self.active.lock().get(&track.id).cloned() {
            return Ok((Source::stream(buf.clone()), Origin::Network { buf }));
        }
        // Скачанный — играем сразу; если он хуже желаемого, приложение после старта
        // попросит lossless и подменит источник на ходу, без паузы.
        if let Some((path, _)) = self.library.lookup(&track.id) {
            return Ok((Source::File(path), Origin::Library));
        }
        if let Some((info, buf, rank)) = self.take_warm(&track.id).await {
            self.spawn_download(track, &info, buf.clone(), rank)?;
            return Ok((Source::stream(buf.clone()), Origin::Network { buf }));
        }
        let q = self.start_quality();
        let info = self.client.file_info(&track.id, q).await?;
        let buf = self.start_download(track, &info, quality_rank(q))?;
        Ok((Source::stream(buf.clone()), Origin::Network { buf }))
    }

    /// Поток трека в заданном качестве (отладка); загрузка, как обычно, ляжет в библиотеку.
    pub async fn debug_source(&self, track: &Track, q: Quality) -> Result<(Source, String)> {
        let info = self.client.file_info(&track.id, q).await?;
        let buf = self.start_download(track, &info, quality_rank(q))?;
        Ok((Source::stream(buf), info.codec))
    }

    /// Скачанный трек хуже lossless и пора спросить сервер о лучшем (подмена на ходу).
    pub fn should_upgrade(&self, track: &Track) -> bool {
        self.quality() == Quality::Lossless && self.library.needs_upgrade(&track.id, quality_rank(Quality::Lossless))
    }

    /// Источник получше для играющего трека (горячая подмена на lossless).
    /// `None` — lossless у трека нет.
    pub async fn upgrade_source(&self, track: &Track) -> Result<Option<(Source, Origin)>> {
        let info = self.client.file_info(&track.id, Quality::Lossless).await?;
        if codec_rank(&info.codec) < 3 {
            self.library.mark_checked(&track.id, 3);
            return Ok(None);
        }
        tracing::info!("подмена {} на ходу → {}", track.id, info.codec);
        let buf = self.start_download(track, &info, quality_rank(Quality::Lossless))?;
        Ok(Some((Source::stream(buf.clone()), Origin::Network { buf })))
    }

    /// Скачивает трек целиком в библиотеку, не играя (умное скачивание).
    /// `false` — уже был скачан или сохранить не вышло.
    pub async fn save(&self, track: &Track, q: Quality) -> Result<bool> {
        if self.library.contains(&track.id) || self.active.lock().contains_key(&track.id) {
            return Ok(false);
        }
        let info = self.client.file_info(&track.id, q).await?;
        let urls = info.urls();
        if urls.is_empty() {
            return Err(eyre!("сервер не дал ссылку на файл"));
        }
        let key = cipher_key(&info)?;
        let buf = StreamBuffer::new(info.size)?;
        self.download(&urls, key, &buf, 0, None).await?;
        buf.finish();
        self.persist(track.clone(), info.codec.clone(), quality_rank(q), buf).await;
        Ok(self.library.contains(&track.id))
    }

    /// Запускает загрузку в фоне; по завершении трек ложится в библиотеку.
    /// `rank` — какое качество просили (по нему библиотека решает, искать ли лучше).
    fn start_download(&self, track: &Track, info: &DownloadInfo, rank: u8) -> Result<Arc<StreamBuffer>> {
        let buf = StreamBuffer::new(info.size)?;
        self.spawn_download(track, info, buf.clone(), rank)?;
        Ok(buf)
    }

    /// Качает в `buf` с того места, где он кончается (после прогрева — не с нуля).
    fn spawn_download(&self, track: &Track, info: &DownloadInfo, buf: Arc<StreamBuffer>, rank: u8) -> Result<()> {
        let urls = info.urls();
        if urls.is_empty() {
            return Err(eyre!("сервер не дал ссылку на файл"));
        }
        let key = cipher_key(info)?;
        let (codec, size) = (info.codec.clone(), info.size);
        self.active.lock().insert(track.id.clone(), buf.clone());

        let this = self.clone();
        let track = track.clone();
        let b = buf;
        tokio::spawn(async move {
            let from = b.len();
            // Файл короче прогретого начала — он уже весь здесь.
            let r = if size.is_some_and(|n| from >= n) {
                Ok(())
            } else {
                this.download(&urls, key, &b, from, None).await
            };
            match r {
                Ok(()) => {
                    b.finish();
                    this.persist(track.clone(), codec, rank, b.clone()).await;
                }
                Err(e) => {
                    tracing::warn!("загрузка {} не удалась: {e}", track.id);
                    b.fail(e.to_string());
                }
            }
            // Свою запись: тот же трек могли начать качать заново в другом качестве.
            let mut act = this.active.lock();
            if act.get(&track.id).is_some_and(|x| Arc::ptr_eq(x, &b)) {
                act.remove(&track.id);
            }
        });
        Ok(())
    }

    /// Байты `[from, to)` файла в `buf` (дописывает в конец). `to == None` — до конца.
    async fn download(
        &self,
        urls: &[String],
        key: Option<[u8; 16]>,
        buf: &StreamBuffer,
        from: u64,
        to: Option<u64>,
    ) -> Result<()> {
        if to.is_some_and(|t| t <= from) {
            return Ok(());
        }
        let mut last_err = eyre!("нет ссылок");
        for url in urls {
            let mut req = self.client.http().get(url);
            if from > 0 || to.is_some() {
                let end = to.map(|t| (t - 1).to_string()).unwrap_or_default();
                req = req.header(reqwest::header::RANGE, format!("bytes={from}-{end}"));
            }
            let resp = match req.send().await.and_then(|r| r.error_for_status()) {
                Ok(r) => r,
                Err(e) => {
                    last_err = e.into();
                    continue;
                }
            };
            // Сервер мог проигнорировать Range и отдать файл целиком — тогда пропускаем начало сами.
            let mut skip = if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT { 0 } else { from };
            let mut left = to.map(|t| t - from);
            let mut cipher = key.map(|k| {
                let mut c = Aes128Ctr::new(&k.into(), &[0u8; 16].into());
                c.seek(from);
                c
            });
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                let mut chunk = &chunk[..];
                if skip > 0 {
                    let n = (skip as usize).min(chunk.len());
                    chunk = &chunk[n..];
                    skip -= n as u64;
                }
                if let Some(l) = left {
                    chunk = &chunk[..(l as usize).min(chunk.len())];
                }
                let mut chunk = chunk.to_vec();
                if let Some(c) = cipher.as_mut() {
                    c.apply_keystream(&mut chunk);
                }
                buf.push(&chunk)?;
                if let Some(l) = left.as_mut() {
                    *l -= chunk.len() as u64;
                    if *l == 0 {
                        break;
                    }
                }
            }
            return Ok(());
        }
        Err(last_err)
    }

    async fn persist(&self, track: Track, codec: String, requested_rank: u8, buf: Arc<StreamBuffer>) {
        let cover = match track.cover_uri() {
            Some(u) => fetch_bytes(&self.client, &cover_url(u, 1000)).await.ok(),
            None => None,
        };
        // В теги — синхронный LRC, если есть (его понимают многие плееры), иначе просто текст.
        let lyrics = if track.has_sync_lyrics() {
            self.client.lyrics(&track.id, true).await.ok()
        } else if track.has_text_lyrics() {
            self.client.lyrics(&track.id, false).await.ok()
        } else {
            None
        };
        let id = track.id.clone();
        // Пока качали, трек уже лёг в библиотеку в качестве получше (подмена на ходу) — не затираем.
        if self.library.codec_of(&id).is_some_and(|c| codec_rank(&c) > codec_rank(&codec)) {
            return tracing::info!("{id}: в библиотеке уже лучше {codec}, не сохраняю");
        }
        let data = match buf.snapshot() {
            Ok(d) => d,
            Err(e) => return tracing::warn!("не удалось прочитать загрузку {id}: {e}"),
        };
        let lib = self.library.clone();
        let res = tokio::task::spawn_blocking(move || {
            lib.store(Download {
                track,
                requested_rank,
                codec,
                data,
                cover,
                lyrics,
            })
        })
        .await;
        match res {
            Ok(Ok(p)) => tracing::info!("сохранён {id} → {}", p.display()),
            Ok(Err(e)) => tracing::warn!("не удалось сохранить {id}: {e:#}"),
            Err(e) => tracing::warn!("persist panic: {e}"),
        }
        // Копии трека (снимок, remux, теги) освобождены, но glibc держит их в аренах потоков.
        // SAFETY: malloc_trim безопасна в любой момент.
        #[cfg(target_env = "gnu")]
        unsafe { libc::malloc_trim(0) };
    }
}

fn cipher_key(info: &DownloadInfo) -> Result<Option<[u8; 16]>> {
    Ok(match (info.transport.as_str(), info.key.as_deref()) {
        ("encraw", Some(k)) if !k.is_empty() => {
            let k = hex::decode(k)?;
            Some(<[u8; 16]>::try_from(k.as_slice()).map_err(|_| eyre!("ключ не 128 бит"))?)
        }
        _ => None,
    })
}

pub async fn fetch_bytes(client: &Client, url: &str) -> Result<Vec<u8>> {
    Ok(client
        .http()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}

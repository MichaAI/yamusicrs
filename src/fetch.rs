//! Получение трека для воспроизведения: из библиотеки или потоковой загрузкой,
//! с расшифровкой `encraw` на лету и сохранением в библиотеку по завершении.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use aes::cipher::{KeyIvInit, StreamCipher};
use color_eyre::eyre::{Result, eyre};
use futures_util::StreamExt;
use parking_lot::Mutex;

use crate::api::{Client, DownloadInfo, Quality, Track, cover_url};
use crate::audio::{Source, StreamBuffer};
use crate::library::{Download, Library, codec_rank};

type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

/// Сколько ждать ответа о качестве получше, прежде чем играть файл из библиотеки.
const UPGRADE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Clone)]
pub struct Fetcher {
    client: Client,
    library: Arc<Library>,
    /// Текущее качество стрима; меняется на лету из UI.
    quality: Arc<AtomicU8>,
    /// Активные загрузки: повторный запрос того же трека (prefetch → play) берёт тот же буфер.
    active: Arc<Mutex<HashMap<String, Arc<StreamBuffer>>>>,
}

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
        if let Some((path, entry)) = self.library.lookup(&track.id) {
            // Файл хуже, чем просим, и давно не проверяли — сперва спрашиваем сервер:
            // дадут лучше — сразу играем его из сети (файл в библиотеке заменится),
            // нет или сеть не ответила — играем файл.
            let want = self.quality();
            if entry.wants_upgrade(quality_rank(want)) {
                let ask = self.client.file_info(&track.id, want);
                match tokio::time::timeout(UPGRADE_TIMEOUT, ask).await {
                    Ok(Ok(info)) if codec_rank(&info.codec) > codec_rank(&entry.codec) => {
                        tracing::info!("апгрейд {}: {} → {}", track.id, entry.codec, info.codec);
                        match self.start_download(track, &info) {
                            Ok(buf) => return Ok((Source::stream(buf.clone()), Origin::Network { buf })),
                            Err(e) => tracing::warn!("апгрейд {}: {e}", track.id),
                        }
                    }
                    Ok(Ok(_)) => self.library.mark_checked(&track.id, quality_rank(want)),
                    Ok(Err(e)) => tracing::warn!("апгрейд {}: {e}", track.id),
                    Err(_) => tracing::warn!("апгрейд {}: сервер не ответил, играю файл", track.id),
                }
            }
            return Ok((Source::File(path), Origin::Library));
        }
        let info = self.client.file_info(&track.id, self.quality()).await?;
        let buf = self.start_download(track, &info)?;
        Ok((Source::stream(buf.clone()), Origin::Network { buf }))
    }

    /// Поток трека в заданном качестве (отладка); загрузка, как обычно, ляжет в библиотеку.
    pub async fn debug_source(&self, track: &Track, q: Quality) -> Result<(Source, String)> {
        let info = self.client.file_info(&track.id, q).await?;
        let buf = self.start_download(track, &info)?;
        Ok((Source::stream(buf), info.codec))
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
        let buf = self.start_download(track, &info)?;
        Ok(Some((Source::stream(buf.clone()), Origin::Network { buf })))
    }

    /// Запускает загрузку в фоне; по завершении трек ложится в библиотеку.
    fn start_download(&self, track: &Track, info: &DownloadInfo) -> Result<Arc<StreamBuffer>> {
        let urls = info.urls();
        if urls.is_empty() {
            return Err(eyre!("сервер не дал ссылку на файл"));
        }
        let key = match (info.transport.as_str(), info.key.as_deref()) {
            ("encraw", Some(k)) if !k.is_empty() => {
                let k = hex::decode(k)?;
                Some(<[u8; 16]>::try_from(k.as_slice()).map_err(|_| eyre!("ключ не 128 бит"))?)
            }
            _ => None,
        };
        let buf = StreamBuffer::new(info.size)?;
        let codec = info.codec.clone();
        let rank = quality_rank(self.quality());
        self.active.lock().insert(track.id.clone(), buf.clone());

        let this = self.clone();
        let track = track.clone();
        let b = buf.clone();
        tokio::spawn(async move {
            match this.download(&urls, key, &b).await {
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
        Ok(buf)
    }

    async fn download(&self, urls: &[String], key: Option<[u8; 16]>, buf: &StreamBuffer) -> Result<()> {
        let mut last_err = eyre!("нет ссылок");
        for url in urls {
            let resp = match self
                .client
                .http()
                .get(url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = e.into();
                    continue;
                }
            };
            let mut cipher = key.map(|k| Aes128Ctr::new(&k.into(), &[0u8; 16].into()));
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let mut chunk = chunk?.to_vec();
                if let Some(c) = cipher.as_mut() {
                    c.apply_keystream(&mut chunk);
                }
                buf.push(&chunk)?;
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
        unsafe { libc::malloc_trim(0) };
    }
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

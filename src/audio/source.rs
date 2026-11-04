//! Источник байтов для декодера, который наполняется по мере скачивания.
//!
//! Чтение блокируется, пока нужные байты не докачаны, — так трек начинает играть
//! через долю секунды после старта загрузки, а не после её окончания.
//!
//! Байты лежат не в памяти процесса, а в удалённом с диска файле в кеше: lossless-трек
//! весит 30–60 МБ, а держим сразу два (текущий и следующий). Читается он из page cache.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;

struct Inner {
    /// Сколько байтов уже записано в файл.
    len: u64,
    total: Option<u64>,
    done: bool,
    error: Option<String>,
}

/// Общий буфер загрузки: пишет загрузчик, читает декодер.
pub struct StreamBuffer {
    file: File,
    inner: Mutex<Inner>,
    cv: Condvar,
}

/// Файл без имени: создаём в кеше и сразу удаляем, место освободится с последней ручкой.
fn spill_file() -> io::Result<File> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = directories::ProjectDirs::from("", "", "yamusicrs")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "stream-{}-{}.part",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let file = File::options().read(true).write(true).create_new(true).open(&path)?;
    std::fs::remove_file(&path)?;
    Ok(file)
}

impl std::fmt::Debug for StreamBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.lock();
        f.debug_struct("StreamBuffer")
            .field("len", &g.len)
            .field("total", &g.total)
            .field("done", &g.done)
            .finish()
    }
}

impl StreamBuffer {
    pub fn new(total: Option<u64>) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            file: spill_file()?,
            inner: Mutex::new(Inner {
                len: 0,
                total,
                done: false,
                error: None,
            }),
            cv: Condvar::new(),
        }))
    }

    pub fn push(&self, bytes: &[u8]) -> io::Result<()> {
        // Пишет только загрузчик, поэтому смещение можно взять без удержания замка на запись.
        let at = self.inner.lock().len;
        self.file.write_all_at(bytes, at)?;
        self.inner.lock().len = at + bytes.len() as u64;
        self.cv.notify_all();
        Ok(())
    }

    pub fn finish(&self) {
        let mut g = self.inner.lock();
        g.done = true;
        g.total = Some(g.len);
        drop(g);
        self.cv.notify_all();
    }

    pub fn fail(&self, err: String) {
        let mut g = self.inner.lock();
        g.error = Some(err);
        g.done = true;
        drop(g);
        self.cv.notify_all();
    }

    fn wake(&self) {
        self.cv.notify_all();
    }

    /// Доля скачанного, 0..=1.
    pub fn progress(&self) -> f32 {
        let g = self.inner.lock();
        match (g.done, g.total) {
            (true, _) => 1.0,
            (false, Some(t)) if t > 0 => g.len as f32 / t as f32,
            _ => 0.0,
        }
    }

    /// Все данные после завершения загрузки.
    pub fn snapshot(&self) -> io::Result<Vec<u8>> {
        let len = self.inner.lock().len;
        let mut data = vec![0; len as usize];
        self.file.read_exact_at(&mut data, 0)?;
        Ok(data)
    }
}

/// Читатель поверх [`StreamBuffer`] со своей позицией.
pub struct StreamReader {
    buf: Arc<StreamBuffer>,
    cancel: Arc<AtomicBool>,
    pos: u64,
}

impl StreamReader {
    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl Read for StreamReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut g = self.buf.inner.lock();
        loop {
            if self.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "cancelled"));
            }
            let len = g.len;
            if self.pos < len {
                drop(g);
                let n = ((len - self.pos) as usize).min(out.len());
                self.buf.file.read_exact_at(&mut out[..n], self.pos)?;
                self.pos += n as u64;
                return Ok(n);
            }
            if let Some(e) = &g.error {
                return Err(io::Error::other(e.clone()));
            }
            if g.done {
                return Ok(0);
            }
            self.buf.cv.wait_for(&mut g, Duration::from_millis(200));
        }
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => {
                // Для seek от конца нужен размер: ждём, пока он станет известен.
                let mut g = self.buf.inner.lock();
                loop {
                    if let Some(t) = g.total {
                        break t as i64 + d;
                    }
                    if self.is_cancelled() || g.error.is_some() {
                        return Err(io::Error::other("размер потока неизвестен"));
                    }
                    self.buf.cv.wait_for(&mut g, Duration::from_millis(200));
                }
            }
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek до начала"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

impl MediaSource for StreamReader {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.buf.inner.lock().total
    }
}

/// То, откуда декодер читает трек.
pub enum Source {
    File(std::path::PathBuf),
    /// Буфер загрузки + флаг отмены именно этого чтения (буфер может переиспользоваться).
    Stream(Arc<StreamBuffer>, Arc<AtomicBool>),
}

impl Source {
    pub fn stream(buf: Arc<StreamBuffer>) -> Self {
        Source::Stream(buf, Arc::new(AtomicBool::new(false)))
    }

    pub fn open(&self) -> io::Result<Box<dyn MediaSource>> {
        Ok(match self {
            Source::File(p) => Box::new(File::open(p)?),
            Source::Stream(b, c) => Box::new(StreamReader {
                buf: b.clone(),
                cancel: c.clone(),
                pos: 0,
            }),
        })
    }

    pub fn cancel(&self) {
        if let Source::Stream(b, c) = self {
            c.store(true, Ordering::Relaxed);
            b.wake();
        }
    }

    /// Чтение отменили (переключили трек): ошибки такого потока — не ошибки трека.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Source::Stream(_, c) if c.load(Ordering::Relaxed))
    }

    /// Ручка, которой можно прервать чтение из другого потока (например, если декодер
    /// завис в ожидании недокачанных байтов, а пользователь уже переключил трек).
    pub fn canceller(&self) -> Option<Canceller> {
        match self {
            Source::Stream(b, c) => Some(Canceller(b.clone(), c.clone())),
            Source::File(_) => None,
        }
    }

    pub fn extension_hint(&self) -> Option<String> {
        match self {
            Source::File(p) => p.extension().and_then(|e| e.to_str()).map(str::to_owned),
            Source::Stream(..) => None,
        }
    }
}

pub struct Canceller(Arc<StreamBuffer>, Arc<AtomicBool>);

impl Canceller {
    pub fn cancel(&self) {
        self.1.store(true, Ordering::Relaxed);
        self.0.wake();
    }

    pub fn is_cancelled(&self) -> bool {
        self.1.load(Ordering::Relaxed)
    }
}

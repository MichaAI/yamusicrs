//! Источник байтов для декодера, который наполняется по мере скачивания.
//!
//! Чтение блокируется, пока нужные байты не докачаны, — так трек начинает играть
//! через долю секунды после старта загрузки, а не после её окончания.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};
use symphonia::core::io::MediaSource;

#[derive(Default)]
struct Inner {
    data: Vec<u8>,
    total: Option<u64>,
    done: bool,
    error: Option<String>,
}

/// Общий буфер загрузки: пишет загрузчик, читает декодер.
#[derive(Default)]
pub struct StreamBuffer {
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl std::fmt::Debug for StreamBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.lock();
        f.debug_struct("StreamBuffer")
            .field("len", &g.data.len())
            .field("total", &g.total)
            .field("done", &g.done)
            .finish()
    }
}

impl StreamBuffer {
    pub fn new(total: Option<u64>) -> Arc<Self> {
        let sb = Self::default();
        sb.inner.lock().total = total;
        if let Some(t) = total {
            sb.inner.lock().data.reserve_exact(t as usize);
        }
        Arc::new(sb)
    }

    pub fn push(&self, bytes: &[u8]) {
        self.inner.lock().data.extend_from_slice(bytes);
        self.cv.notify_all();
    }

    pub fn finish(&self) {
        let mut g = self.inner.lock();
        g.done = true;
        g.total = Some(g.data.len() as u64);
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
            (false, Some(t)) if t > 0 => g.data.len() as f32 / t as f32,
            _ => 0.0,
        }
    }

    /// Снимок всех данных после завершения загрузки.
    pub fn snapshot(&self) -> Vec<u8> {
        self.inner.lock().data.clone()
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
            let len = g.data.len() as u64;
            if self.pos < len {
                let n = ((len - self.pos) as usize).min(out.len());
                let start = self.pos as usize;
                out[..n].copy_from_slice(&g.data[start..start + n]);
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
}

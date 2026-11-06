//! Вывод звука через AAudio: поток float с частотой исходника, данные берёт [`Filler`].
//!
//! Устройство отключилось (наушники, Bluetooth) — AAudio закрывает поток; открываем
//! заново из отдельного потока (в колбэке ошибки этого делать нельзя).

use std::sync::{Arc, Weak};

use ndk::audio::{
    AudioCallbackResult, AudioContentType, AudioDirection, AudioError, AudioFormat, AudioPerformanceMode,
    AudioStream, AudioStreamBuilder, AudioUsage,
};
use parking_lot::Mutex;
use yamusic_core::audio::{Format, Output, Shared};

/// Поток AAudio: ручки потокобезопасны, вызовы из разных потоков — под мьютексом.
struct Stream(AudioStream);
// SAFETY: AAudioStream можно запускать/останавливать/закрывать из любого потока,
// доступ к нему сериализован мьютексом `Inner`.
unsafe impl Send for Stream {}

struct Inner {
    stream: Option<Stream>,
    format: Option<Format>,
    active: bool,
}

pub struct AAudioOut {
    shared: Arc<Shared>,
    inner: Mutex<Inner>,
    me: Weak<AAudioOut>,
}

/// Фабрика для [`yamusic_core::audio::Player::with_output`].
pub fn spawn(shared: Arc<Shared>) -> color_eyre::Result<Arc<dyn Output>> {
    let active = !shared.paused();
    Ok(Arc::new_cyclic(|me| AAudioOut {
        shared,
        inner: Mutex::new(Inner {
            stream: None,
            format: None,
            active,
        }),
        me: me.clone(),
    }))
}

impl AAudioOut {
    fn open(&self, inner: &mut Inner, f: Format) {
        inner.stream = None;
        let channels = f.channels.max(1) as usize;
        let mut filler = self.shared.filler(f.rate, f.channels);
        let me = self.me.clone();
        let built = AudioStreamBuilder::new().map(|b| {
            b.direction(AudioDirection::Output)
                .format(AudioFormat::PCM_Float)
                .sample_rate(f.rate as i32)
                .channel_count(f.channels as i32)
                .performance_mode(AudioPerformanceMode::None)
                .usage(AudioUsage::Media)
                .content_type(AudioContentType::Music)
                .data_callback(Box::new(move |_s, data, frames| {
                    // SAFETY: AAudio даёт буфер на `frames` кадров формата PCM_Float с `channels` каналами.
                    let out = unsafe { std::slice::from_raw_parts_mut(data.cast::<f32>(), frames as usize * channels) };
                    filler.fill(out);
                    AudioCallbackResult::Continue
                }))
                .error_callback(Box::new(move |_s, e: AudioError| {
                    tracing::warn!("AAudio: {e:?} — открываю поток заново");
                    let me = me.clone();
                    std::thread::spawn(move || {
                        if let Some(me) = me.upgrade() {
                            me.reconnect();
                        }
                    });
                }))
        });
        match built.and_then(|b| b.open_stream()) {
            Ok(s) => {
                tracing::info!(
                    "AAudio: {} Гц, {} кан., буфер {} кадров",
                    s.sample_rate(),
                    s.channel_count(),
                    s.buffer_size_in_frames()
                );
                if inner.active
                    && let Err(e) = s.request_start()
                {
                    tracing::error!("AAudio start: {e:?}");
                }
                inner.stream = Some(Stream(s));
            }
            Err(e) => tracing::error!("AAudio: поток {} Гц × {} не открылся: {e:?}", f.rate, f.channels),
        }
    }
}

impl Output for AAudioOut {
    fn configure(&self, f: Format) {
        let mut inner = self.inner.lock();
        if inner.format != Some(f) {
            inner.format = Some(f);
            self.open(&mut inner, f);
        }
    }

    fn current(&self) -> Option<Format> {
        self.inner.lock().format
    }

    fn reconnect(&self) {
        let mut inner = self.inner.lock();
        if let Some(f) = inner.format {
            self.open(&mut inner, f);
        }
    }

    fn set_active(&self, active: bool) {
        let mut inner = self.inner.lock();
        inner.active = active;
        if let Some(Stream(s)) = &inner.stream {
            let r = if active { s.request_start() } else { s.request_pause() };
            if let Err(e) = r {
                tracing::warn!("AAudio {}: {e:?}", if active { "start" } else { "pause" });
            }
        }
    }

    fn quit(&self) {
        let mut inner = self.inner.lock();
        if let Some(Stream(s)) = inner.stream.take() {
            let _ = s.request_stop();
        }
    }
}

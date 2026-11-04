//! Поток декодирования: читает [`Source`], декодирует symphonia и пишет f32 в кольцо.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError};
use symphonia::core::codecs::audio::well_known::{CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP3};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};
use tokio::sync::mpsc::UnboundedSender;

use super::output::Handle;
use super::{Cmd, Event, Format, Segment, Shared, Source, SourceInfo, Token};

struct Active {
    token: Token,
    source: Source,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    format: Format,
    info: SourceInfo,
    gain: f32,
    /// Конец трека, сек (если контейнер знает длительность) — для кроссфейда.
    end_secs: Option<f64>,
    /// Решение о кроссфейде в следующий трек уже принято.
    fade_checked: bool,
}

/// Уходящий трек во время кроссфейда: доигрывает поверх начала следующего.
struct Fade {
    from: Active,
    buf: Vec<f32>,
    pos: usize,
    /// Сколько кадров уже смешано и сколько всего длится переход.
    done: u64,
    len: u64,
    ended: bool,
}

fn secs(tb: Option<TimeBase>, ts: Timestamp) -> Option<f64> {
    tb.map(|tb| tb.calc_time_saturating(ts).as_secs_f64())
}

struct Dec {
    shared: Arc<Shared>,
    prod: rtrb::Producer<f32>,
    events: UnboundedSender<Event>,
    out: Handle,
    cur: Option<Active>,
    next: Option<(Token, Source, f32, bool)>,
    /// Следующий трек, открытый ради кроссфейда, но другого формата: начнётся встык.
    next_open: Option<Active>,
    fade: Option<Fade>,
    /// Открытый следующий трек другого формата: ждёт, пока доиграет кольцо.
    pending: Option<Active>,
    /// Последний трек дочитан, ждём `Next` от приложения.
    awaiting_next: bool,
    drained_sent: bool,
    leftover: Vec<f32>,
    leftover_pos: usize,
    scratch: Vec<f32>,
}

pub(super) fn spawn(
    shared: Arc<Shared>,
    prod: rtrb::Producer<f32>,
    rx: Receiver<Cmd>,
    events: UnboundedSender<Event>,
    out: Handle,
) {
    std::thread::Builder::new()
        .name("decoder".into())
        .spawn(move || {
            let mut d = Dec {
                shared,
                prod,
                events,
                out,
                cur: None,
                next: None,
                next_open: None,
                fade: None,
                pending: None,
                awaiting_next: false,
                drained_sent: true,
                leftover: Vec::new(),
                leftover_pos: 0,
                scratch: Vec::new(),
            };
            d.run(rx);
        })
        .expect("decoder thread");
}

/// Открывает трек; `Err(None)` — чтение отменили на полпути (трек переключили), это не ошибка.
fn open(token: Token, source: Source, gain: f32) -> Result<Active, Option<String>> {
    let cancelled = source.canceller();
    open_inner(token, source, gain).map_err(|e| (!cancelled.is_some_and(|c| c.is_cancelled())).then_some(e))
}

fn open_inner(token: Token, source: Source, gain: f32) -> Result<Active, String> {
    let ms = source.open().map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(ms, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = source.extension_hint() {
        hint.with_extension(&ext);
    }
    let reader = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("формат: {e}"))?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or("в файле нет аудиодорожки")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("нет параметров кодека")?
        .clone();
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("декодер: {e}"))?;
    let end_secs = track
        .duration
        .and_then(|d| secs(track.time_base, track.start_ts.saturating_add(d)));
    let rate = params.sample_rate.ok_or("неизвестна частота")?;
    let channels = params.channels.as_ref().map(|c| c.count() as u16).unwrap_or(2);
    let codec = match params.codec {
        c if c == CODEC_ID_FLAC => "FLAC",
        c if c == CODEC_ID_ALAC => "ALAC",
        c if c == CODEC_ID_AAC => "AAC",
        c if c == CODEC_ID_MP3 => "MP3",
        _ => "?",
    };
    let bits = if codec == "FLAC" || codec == "ALAC" {
        params.bits_per_sample
    } else {
        None
    };
    Ok(Active {
        token,
        track_id: track.id,
        time_base: track.time_base,
        source,
        reader,
        decoder,
        format: Format { rate, channels },
        info: SourceInfo {
            codec: codec.into(),
            rate,
            bits,
        },
        gain,
        end_secs,
        fade_checked: false,
    })
}

impl Dec {
    fn run(&mut self, rx: Receiver<Cmd>) {
        loop {
            let idle = self.cur.is_none() && self.pending.is_none() && self.leftover_pos >= self.leftover.len();
            let cmd = if idle {
                self.check_drained();
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => return,
                }
            };
            if let Some(cmd) = cmd {
                self.handle(cmd);
                continue;
            }

            if let Some(p) = &self.pending {
                if self.shared.played.load(Ordering::Acquire) >= self.shared.written.load(Ordering::Acquire) {
                    self.out.configure(p.format);
                    let p = self.pending.take().expect("pending");
                    self.begin(p, 0.0);
                } else {
                    std::thread::sleep(Duration::from_millis(5));
                }
                continue;
            }

            if self.leftover_pos < self.leftover.len() {
                if !self.push_leftover() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                continue;
            }

            if self.cur.is_some() {
                self.decode_step();
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Load {
                token,
                source,
                gain,
                start_secs,
            } => {
                self.reset();
                match open(token, source, gain) {
                    Ok(a) => {
                        self.out.configure(a.format);
                        self.begin(a, 0.0);
                        if start_secs > 0.0 {
                            self.seek(start_secs);
                        }
                    }
                    Err(Some(e)) => {
                        let _ = self.events.send(Event::Error(token, e));
                    }
                    Err(None) => {}
                }
            }
            Cmd::Next {
                token,
                source,
                gain,
                crossfade,
            } => {
                self.drained_sent = false;
                if self.cur.is_none() && self.pending.is_none() && self.awaiting_next {
                    self.awaiting_next = false;
                    self.start_following(token, source, gain);
                } else {
                    self.drop_next();
                    self.next = Some((token, source, gain, crossfade));
                }
            }
            Cmd::ClearNext => self.drop_next(),
            Cmd::SetGain { token, gain } => {
                if let Some(a) = self.cur.as_mut().filter(|a| a.token == token) {
                    a.gain = gain;
                }
            }
            Cmd::Seek { token, secs } => {
                if self.cur.as_ref().is_some_and(|a| a.token == token) {
                    self.seek(secs);
                } else {
                    let _ = self.events.send(Event::SeekMiss(token, secs));
                }
            }
            Cmd::Stop => {
                self.reset();
                self.drained_sent = true;
                self.shared.segments.lock().clear();
            }
        }
    }

    fn reset(&mut self) {
        if let Some(a) = self.cur.take() {
            a.source.cancel();
        }
        if let Some(a) = self.pending.take() {
            a.source.cancel();
        }
        self.drop_next();
        self.fade = None;
        self.leftover.clear();
        self.leftover_pos = 0;
        self.awaiting_next = false;
        self.drained_sent = false;
        self.shared.flush();
    }

    /// Новый трек начинает звучать с текущей позиции записи в кольце.
    fn begin(&mut self, a: Active, base_secs: f64) {
        let start = self.shared.written.load(Ordering::Acquire);
        let played = self.shared.played.load(Ordering::Acquire);
        let mut segs = self.shared.segments.lock();
        // Сегменты, которые уже целиком позади, больше не нужны.
        while segs.len() > 1 && segs[1].start <= played {
            segs.pop_front();
        }
        segs.push_back(Segment {
            start,
            token: a.token,
            base_secs,
            format: a.format,
            info: a.info.clone(),
        });
        drop(segs);
        self.drained_sent = false;
        self.cur = Some(a);
    }

    fn drop_next(&mut self) {
        if let Some((_, s, _, _)) = self.next.take() {
            s.cancel();
        }
        if let Some(a) = self.next_open.take() {
            a.source.cancel();
        }
    }

    fn start_following(&mut self, token: Token, source: Source, gain: f32) {
        match open(token, source, gain) {
            Ok(a) => self.follow(a),
            Err(Some(e)) => {
                let _ = self.events.send(Event::Error(token, e));
            }
            Err(None) => {}
        }
    }

    fn follow(&mut self, a: Active) {
        if Some(a.format) == self.out.current() {
            self.begin(a, 0.0);
        } else {
            self.pending = Some(a);
        }
    }

    /// До конца текущего трека осталось не больше длины кроссфейда — начинаем переход:
    /// следующий трек становится текущим (с него идёт отсчёт позиции), а уходящий
    /// доигрывает поверх, затухая.
    fn maybe_crossfade(&mut self, pts_end: Timestamp) {
        let fade_secs = self.shared.crossfade_secs();
        let Some(a) = self.cur.as_mut() else { return };
        if a.fade_checked || fade_secs <= 0.0 || self.fade.is_some() {
            return;
        }
        let (Some(end), Some(now)) = (a.end_secs, secs(a.time_base, pts_end)) else {
            return;
        };
        let remaining = end - now;
        if remaining > fade_secs as f64 || !self.next.as_ref().is_some_and(|n| n.3) {
            return;
        }
        a.fade_checked = true;
        // Хвост короче полсекунды — переход не успеет прозвучать, обычный gapless.
        if remaining < 0.5 {
            return;
        }
        let (token, source, gain, _) = self.next.take().expect("next");
        let next = match open(token, source, gain) {
            Ok(n) => n,
            Err(e) => {
                if let Some(e) = e {
                    let _ = self.events.send(Event::Error(token, e));
                }
                return;
            }
        };
        if next.format != a.format {
            self.next_open = Some(next);
            return;
        }
        let len = (remaining * a.format.rate as f64) as u64;
        let from = self.cur.take().expect("cur");
        self.fade = Some(Fade {
            from,
            buf: Vec::new(),
            pos: 0,
            done: 0,
            len,
            ended: false,
        });
        self.begin(next, 0.0);
    }

    /// Следующий сэмпл уходящего трека; декодирует пакеты по мере надобности.
    fn fade_sample(f: &mut Fade, scratch: &mut Vec<f32>) -> f32 {
        while f.pos >= f.buf.len() && !f.ended {
            let a = &mut f.from;
            let packet = match a.reader.next_packet() {
                Ok(Some(p)) => p,
                _ => {
                    f.ended = true;
                    break;
                }
            };
            if packet.track_id != a.track_id {
                continue;
            }
            match a.decoder.decode(&packet) {
                Ok(buf) => {
                    let n = buf.samples_interleaved();
                    scratch.resize(n, 0.0);
                    buf.copy_to_slice_interleaved(&mut scratch[..]);
                    let g = a.gain;
                    f.buf.clear();
                    f.buf.extend(scratch.iter().map(|s| s * g));
                    f.pos = 0;
                }
                Err(SymError::DecodeError(_)) => {}
                Err(_) => f.ended = true,
            }
        }
        if f.pos < f.buf.len() {
            f.pos += 1;
            f.buf[f.pos - 1]
        } else {
            0.0
        }
    }

    /// Подмешивает уходящий трек к только что декодированному началу нового.
    fn mix_fade(&mut self) {
        let Some(f) = self.fade.as_mut() else { return };
        let ch = f.from.format.channels.max(1) as usize;
        let mut tmp = Vec::new();
        for (i, frame) in self.scratch.chunks_mut(ch).enumerate() {
            let t = ((f.done + i as u64) as f32 / f.len.max(1) as f32).min(1.0);
            let (gin, gout) = (
                (t * std::f32::consts::FRAC_PI_2).sin(),
                (t * std::f32::consts::FRAC_PI_2).cos(),
            );
            for s in frame {
                let o = if t < 1.0 { Self::fade_sample(f, &mut tmp) } else { 0.0 };
                *s = *s * gin + o * gout;
            }
        }
        f.done += (self.scratch.len() / ch) as u64;
        if f.done >= f.len || (f.ended && f.pos >= f.buf.len()) {
            self.fade = None;
        }
    }

    fn seek(&mut self, secs: f64) {
        let Some(a) = self.cur.as_mut() else { return };
        let Some(time) = Time::try_from_secs_f64(secs) else {
            return;
        };
        match a.reader.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(a.track_id),
            },
        ) {
            Ok(to) => {
                a.decoder.reset();
                let actual = a
                    .time_base
                    .map(|tb| tb.calc_time_saturating(to.actual_ts).as_secs_f64())
                    .unwrap_or(secs);
                let mut a = self.cur.take().expect("cur");
                a.fade_checked = false;
                // Перемотка во время перехода: уходящий трек больше не нужен.
                self.fade = None;
                self.leftover.clear();
                self.leftover_pos = 0;
                self.shared.flush();
                if let Some(seg) = self.shared.segments.lock().back_mut() {
                    // Текущий сегмент переезжает: начинается с текущей записи и с новой позиции.
                    seg.start = self.shared.written.load(Ordering::Acquire);
                    seg.base_secs = actual;
                }
                self.cur = Some(a);
            }
            Err(e) => tracing::warn!("seek: {e}"),
        }
    }

    fn decode_step(&mut self) {
        let a = self.cur.as_mut().expect("cur");
        let packet = match a.reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return self.track_ended(),
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::ConnectionAborted => {
                self.cur = None;
                return;
            }
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return self.track_ended();
            }
            // Чтение отменили (трек переключают) — ошибка любого вида, трек не «кончился».
            Err(_) if a.source.is_cancelled() => {
                self.cur = None;
                return;
            }
            Err(e) => {
                let token = a.token;
                tracing::warn!("чтение пакета: {e}");
                let _ = self.events.send(Event::Error(token, e.to_string()));
                return self.track_ended();
            }
        };
        if packet.track_id != a.track_id {
            return;
        }
        let pts_end = packet.pts.saturating_add(packet.dur);
        match a.decoder.decode(&packet) {
            Ok(buf) => {
                let n = buf.samples_interleaved();
                self.scratch.resize(n, 0.0);
                buf.copy_to_slice_interleaved(&mut self.scratch[..]);
                if a.gain != 1.0 {
                    let g = a.gain;
                    self.scratch.iter_mut().for_each(|s| *s *= g);
                }
                self.mix_fade();
                std::mem::swap(&mut self.leftover, &mut self.scratch);
                self.leftover_pos = 0;
                self.push_leftover();
                self.maybe_crossfade(pts_end);
            }
            Err(SymError::DecodeError(e)) => tracing::debug!("битый пакет: {e}"),
            Err(e) => {
                tracing::warn!("декодер: {e}");
                self.track_ended();
            }
        }
    }

    /// Возвращает true, если хоть что-то удалось записать.
    fn push_leftover(&mut self) -> bool {
        let rest = &self.leftover[self.leftover_pos..];
        let n = self.prod.slots().min(rest.len());
        if n == 0 {
            return false;
        }
        if let Ok(mut chunk) = self.prod.write_chunk_uninit(n) {
            let (a, b) = chunk.as_mut_slices();
            for (dst, src) in a.iter_mut().chain(b.iter_mut()).zip(rest) {
                dst.write(*src);
            }
            // SAFETY: все n слотов только что инициализированы.
            unsafe { chunk.commit_all() };
        }
        self.leftover_pos += n;
        self.shared.written.fetch_add(n as u64, Ordering::AcqRel);
        true
    }

    fn track_ended(&mut self) {
        if self.cur.take().is_none() {
            return;
        }
        if let Some(a) = self.next_open.take() {
            return self.follow(a);
        }
        match self.next.take() {
            Some((token, source, gain, _)) => self.start_following(token, source, gain),
            None => {
                self.awaiting_next = true;
                let _ = self.events.send(Event::NeedNext);
            }
        }
    }

    fn check_drained(&mut self) {
        if self.drained_sent || !self.awaiting_next {
            return;
        }
        if self.shared.played.load(Ordering::Acquire) >= self.shared.written.load(Ordering::Acquire) {
            self.drained_sent = true;
            let _ = self.events.send(Event::Drained);
        }
    }
}

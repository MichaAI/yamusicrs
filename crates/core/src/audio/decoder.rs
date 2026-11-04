//! Поток декодирования: читает [`Source`], декодирует symphonia и пишет f32 в кольцо.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use symphonia::core::codecs::audio::well_known::{CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP3};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::packet::Packet;
use symphonia::core::units::{Time, TimeBase, Timestamp};
use tokio::sync::mpsc::UnboundedSender;

use super::resample::Resampler;
use super::source::Canceller;
use super::{Cmd, Event, Format, Output, Segment, Shared, Source, SourceInfo, Token};

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
    /// Источник подменили на ходу, а его частота другая: приводим к частоте вывода.
    resampler: Option<Resampler>,
}

impl Active {
    /// Декодирует пакет в `out` (с ресемплингом, если нужен, и усилением).
    fn decode_into(&mut self, packet: &Packet, out: &mut Vec<f32>, tmp: &mut Vec<f32>) -> Result<(), SymError> {
        let buf = self.decoder.decode(packet)?;
        let n = buf.samples_interleaved();
        match &mut self.resampler {
            None => {
                out.resize(n, 0.0);
                buf.copy_to_slice_interleaved(&mut out[..]);
            }
            Some(r) => {
                tmp.resize(n, 0.0);
                buf.copy_to_slice_interleaved(&mut tmp[..]);
                out.clear();
                r.process(tmp, out);
            }
        }
        if self.gain != 1.0 {
            let g = self.gain;
            out.iter_mut().for_each(|s| *s *= g);
        }
        Ok(())
    }

    fn secs(&self, ts: Timestamp) -> Option<f64> {
        secs(self.time_base, ts)
    }
}

// ── горячая замена источника (например, AAC → FLAC посреди трека) ────────────
//
// Вспомогательный поток открывает новый источник, перематывает его чуть впереди
// декодера и декодирует кусок `pre`. Декодер, дойдя до этого места, придерживает
// свой вывод (`hold`), находит точный сдвиг между источниками взаимной корреляцией
// и сводит их коротким равномощным наплывом — без паузы и щелчка.

/// Насколько декодер может опережать воспроизведение, сек.
const LOOKAHEAD_SECS: f64 = 3.0;

/// Окно корреляции, кадров.
const CORR_N: usize = 4096;
/// Наплыв при хорошем совпадении и при плохом (другой мастер, тишина), кадров.
const SPLICE_SHORT: usize = 1024;
const SPLICE_LONG: usize = 8192;
/// Поиск сдвига ±0.1 с; начинаем придерживать вывод за 0.15 с до точки.
const SEARCH_SECS: f64 = 0.1;
const HOLD_PRE_SECS: f64 = 0.15;

enum Swap {
    Preparing { token: Token, cancel: Option<Canceller> },
    Ready(Box<SwapReady>),
}

struct SwapReady {
    token: Token,
    active: Active,
    /// Время (по треку) первого кадра `pre` и сам кусок нового источника на частоте вывода.
    start: f64,
    pre: Vec<f32>,
    /// Придержанный вывод текущего источника и время его первого кадра.
    hold: Vec<f32>,
    hold_start: Option<f64>,
}

enum Prepared {
    Ready { token: Token, active: Active, start: f64, pre: Vec<f32> },
    /// `None` — отменили (не ошибка).
    Failed { token: Token, err: Option<String> },
}

fn load_secs(a: &AtomicU64) -> f64 {
    f64::from_bits(a.load(Ordering::Relaxed))
}

/// Готовит новый источник к подмене: открыть, перемотать впереди декодера, декодировать кусок.
fn prepare(token: Token, source: Source, out: Format, decoded: Arc<AtomicU64>) -> Prepared {
    let failed = |err: String| Prepared::Failed { token, err: Some(err) };
    let mut a = match open(token, source, 1.0) {
        Ok(a) => a,
        Err(err) => return Prepared::Failed { token, err },
    };
    if a.format.channels != out.channels {
        a.source.cancel();
        return failed(format!("другое число каналов ({} вместо {})", a.format.channels, out.channels));
    }
    if a.format.rate != out.rate {
        a.resampler = Some(Resampler::new(a.format.rate, out.rate, out.channels));
        a.info.resampled_to = Some(out.rate);
        a.format = out;
    }
    // Перематываем с запасом: пока новый источник докачивается до нужного места,
    // декодер уходит вперёд. Не успели — запас вдвое больше.
    let mut lead = 2.0;
    let mut ok = false;
    for _ in 0..7 {
        let target = load_secs(&decoded) + lead;
        if a.end_secs.is_some_and(|e| target > e - 3.0) {
            break;
        }
        let Some(time) = Time::try_from_secs_f64(target) else { break };
        let to = match a.reader.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(a.track_id),
            },
        ) {
            Ok(to) => to,
            Err(_) if a.source.is_cancelled() => return Prepared::Failed { token, err: None },
            Err(e) => return failed(format!("перемотка: {e}")),
        };
        a.decoder.reset();
        if let Some(r) = &mut a.resampler {
            r.reset();
        }
        let actual = a.secs(to.actual_ts).unwrap_or(target);
        if actual > load_secs(&decoded) + 0.8 {
            ok = true;
            break;
        }
        lead *= 2.0;
    }
    if !ok {
        a.source.cancel();
        return failed("не догнал текущую позицию до конца трека".into());
    }
    let ch = out.channels.max(1) as usize;
    let need = (CORR_N + SPLICE_LONG) * ch + (out.rate as usize / 10) * ch;
    let (mut pre, mut chunk, mut tmp) = (Vec::new(), Vec::new(), Vec::new());
    let mut start = None;
    while pre.len() < need {
        let packet = match a.reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return failed("источник кончился раньше времени".into()),
            Err(_) if a.source.is_cancelled() => return Prepared::Failed { token, err: None },
            Err(e) => return failed(format!("чтение: {e}")),
        };
        if packet.track_id != a.track_id {
            continue;
        }
        match a.decode_into(&packet, &mut chunk, &mut tmp) {
            Ok(()) => {
                if start.is_none() {
                    start = a.secs(packet.pts);
                }
                pre.extend_from_slice(&chunk);
            }
            Err(SymError::DecodeError(_)) => {}
            Err(e) => return failed(format!("декодер: {e}")),
        }
    }
    // Первые кадры после сброса ресемплера считаются с нулевой историей — их не берём.
    let skip = if a.resampler.is_some() { 256 } else { 0 };
    pre.drain(..skip * ch);
    let start = start.unwrap_or(0.0) + skip as f64 / out.rate as f64;
    Prepared::Ready { token, active: a, start, pre }
}

/// Сводит придержанный вывод `hold` (с `hold_start`) с новым источником `pre` (с `pre_start`).
/// Возвращает итоговые сэмплы, кадр `hold`, с которого звучит новый источник, и качество совпадения.
fn splice(
    hold: &[f32],
    hold_start: f64,
    pre: &[f32],
    pre_start: f64,
    rate: u32,
    ch: usize,
    gain: f32,
) -> Option<(Vec<f32>, usize, f32)> {
    let mono = |v: &[f32]| -> Vec<f32> { v.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect() };
    let am = mono(hold);
    let fm: Vec<f32> = mono(pre).into_iter().map(|s| s * gain).collect();
    let m = (rate as f64 * SEARCH_SECS) as isize;
    let hs = ((pre_start - hold_start) * rate as f64).round() as isize;
    if hs - m < 0 || (hs + m) as usize + CORR_N + SPLICE_LONG > am.len() || fm.len() < CORR_N + SPLICE_LONG {
        return None;
    }
    let f = &fm[..CORR_N];
    let ef: f32 = f.iter().map(|x| x * x).sum();
    // Скользящая энергия окна hold.
    let lo = (hs - m) as usize;
    let mut ea: f32 = am[lo..lo + CORR_N].iter().map(|x| x * x).sum();
    let (mut best_k, mut best_c) = (0isize, f32::MIN);
    for k in -m..=m {
        let st = (hs + k) as usize;
        if k > -m {
            let out = am[st - 1];
            let inn = am[st + CORR_N - 1];
            ea += inn * inn - out * out;
        }
        let dot: f32 = am[st..st + CORR_N].iter().zip(f).map(|(x, y)| x * y).sum();
        let c = dot / (ea.max(0.0) * ef).sqrt().max(1e-12);
        if c > best_c {
            best_c = c;
            best_k = k;
        }
    }
    // Тишина: сдвиг не определить, но и щёлкать нечему.
    let silent = ef < 1e-6 * CORR_N as f32;
    if silent {
        best_k = 0;
        best_c = 1.0;
    }
    let fl = if best_c >= 0.7 { SPLICE_SHORT } else { SPLICE_LONG };
    let h0 = (hs + best_k) as usize;
    let mut out = Vec::with_capacity(h0 * ch + pre.len());
    out.extend_from_slice(&hold[..h0 * ch]);
    for i in 0..fl {
        let t = (i as f32 + 0.5) / fl as f32;
        let (gin, gout) = (
            (t * std::f32::consts::FRAC_PI_2).sin(),
            (t * std::f32::consts::FRAC_PI_2).cos(),
        );
        for c in 0..ch {
            out.push(hold[(h0 + i) * ch + c] * gout + pre[i * ch + c] * gain * gin);
        }
    }
    out.extend(pre[fl * ch..].iter().map(|s| s * gain));
    Some((out, h0, best_c))
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
    out: Arc<dyn Output>,
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
    tmp: Vec<f32>,
    /// До какой секунды трека дошёл декодер (f64 в битах) — для подготовки подмены.
    decoded: Arc<AtomicU64>,
    swap: Option<Swap>,
    prep_tx: Sender<Prepared>,
    prep_rx: Receiver<Prepared>,
}

pub(super) fn spawn(
    shared: Arc<Shared>,
    prod: rtrb::Producer<f32>,
    rx: Receiver<Cmd>,
    events: UnboundedSender<Event>,
    out: Arc<dyn Output>,
) {
    std::thread::Builder::new()
        .name("decoder".into())
        .spawn(move || {
            let (prep_tx, prep_rx) = crossbeam_channel::unbounded();
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
                tmp: Vec::new(),
                decoded: Arc::new(AtomicU64::new(0f64.to_bits())),
                swap: None,
                prep_tx,
                prep_rx,
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
            resampled_to: None,
        },
        gain,
        end_secs,
        fade_checked: false,
        resampler: None,
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
            if let Ok(p) = self.prep_rx.try_recv() {
                self.on_prepared(p);
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
                // Не уходим вперёд дальше LOOKAHEAD_SECS: файл и так качается целиком в свой
                // буфер, а большой запас в кольце только откладывает подмену источника и прочее.
                if self.ahead_secs() > LOOKAHEAD_SECS {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                self.decode_step();
            }
        }
    }

    /// Сколько секунд звука уже записано в кольцо, но ещё не сыграно.
    fn ahead_secs(&self) -> f64 {
        let Some(a) = self.cur.as_ref() else { return 0.0 };
        let written = self.shared.written.load(Ordering::Acquire);
        let played = self.shared.played.load(Ordering::Acquire);
        written.saturating_sub(played) as f64 / (a.format.rate as f64 * a.format.channels.max(1) as f64)
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
            Cmd::Swap { token, source } => self.start_swap(token, source),
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
        self.abort_swap(false);
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
        // Недописанный в кольцо хвост (придержанный при подмене) прозвучит раньше нового трека.
        let start = self.shared.written.load(Ordering::Acquire) + (self.leftover.len() - self.leftover_pos) as u64;
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
            // Следующий не открылся — ждём другой, иначе очередь молча встанет.
            Err(e) => {
                if let Some(e) = e {
                    let _ = self.events.send(Event::Error(token, e));
                }
                self.awaiting_next = true;
                let _ = self.events.send(Event::NeedNext);
            }
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
        // Идёт сведение подмены — решим на следующем пакете; подмена ещё не началась — отменяем.
        match &self.swap {
            Some(Swap::Ready(r)) if r.hold_start.is_some() => return,
            Some(_) => {
                self.abort_swap(false);
                let _ = self.events.send(Event::Swapped(token_of(&self.cur), Err("трек уже заканчивается".into())));
            }
            None => {}
        }
        let Some(a) = self.cur.as_mut() else { return };
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
            let mut tmp = Vec::new();
            match a.decode_into(&packet, scratch, &mut tmp) {
                Ok(()) => {
                    f.buf.clear();
                    f.buf.extend_from_slice(scratch);
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
                if let Some(r) = &mut a.resampler {
                    r.reset();
                }
                let actual = a
                    .time_base
                    .map(|tb| tb.calc_time_saturating(to.actual_ts).as_secs_f64())
                    .unwrap_or(secs);
                self.decoded.store(actual.to_bits(), Ordering::Relaxed);
                let token = a.token;
                if self.swap.is_some() {
                    self.abort_swap(false);
                    let _ = self.events.send(Event::Swapped(token, Err("перемотка во время перехода".into())));
                }
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
        let (t0, t1) = (a.secs(packet.pts), a.secs(pts_end));
        match a.decode_into(&packet, &mut self.scratch, &mut self.tmp) {
            Ok(()) => {
                if let Some(t1) = t1 {
                    self.decoded.store(t1.to_bits(), Ordering::Relaxed);
                }
                if self.hold_for_swap(t0, t1) {
                    return;
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
        if self.swap.is_some() {
            let token = token_of(&self.cur);
            self.abort_swap(true);
            let _ = self.events.send(Event::Swapped(token, Err("трек закончился раньше перехода".into())));
        }
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

    fn start_swap(&mut self, token: Token, source: Source) {
        let ok = self.cur.as_ref().is_some_and(|a| a.token == token) && self.fade.is_none() && self.pending.is_none();
        if !ok {
            source.cancel();
            let _ = self.events.send(Event::Swapped(token, Err("трек уже сменился".into())));
            return;
        }
        self.abort_swap(false);
        let out = self.cur.as_ref().expect("cur").format;
        let cancel = source.canceller();
        let (tx, decoded) = (self.prep_tx.clone(), self.decoded.clone());
        let spawned = std::thread::Builder::new()
            .name("swap-prep".into())
            .spawn(move || {
                let _ = tx.send(prepare(token, source, out, decoded));
            });
        if spawned.is_ok() {
            self.swap = Some(Swap::Preparing { token, cancel });
        }
    }

    fn on_prepared(&mut self, p: Prepared) {
        let waiting = matches!(&self.swap, Some(Swap::Preparing { token, .. }) if Some(*token) == self.cur.as_ref().map(|a| a.token));
        match p {
            Prepared::Failed { token, err } => {
                tracing::info!("подмена {token}: подготовка не удалась: {}", err.as_deref().unwrap_or("отменена"));
                if matches!(&self.swap, Some(Swap::Preparing { token: t, .. }) if *t == token) {
                    self.swap = None;
                }
                if let Some(e) = err {
                    let _ = self.events.send(Event::Swapped(token, Err(e)));
                }
            }
            Prepared::Ready { token, active, start, pre } => {
                let mine = waiting && matches!(&self.swap, Some(Swap::Preparing { token: t, .. }) if *t == token);
                let late = load_secs(&self.decoded) > start - HOLD_PRE_SECS;
                tracing::info!(
                    "подмена {token}: источник готов с {start:.2} с, декодер на {:.2} с{}",
                    load_secs(&self.decoded),
                    if !mine { " — уже не нужен" } else if late { " — опоздал" } else { "" }
                );
                if !mine || late {
                    active.source.cancel();
                    if mine {
                        self.swap = None;
                        let _ = self.events.send(Event::Swapped(token, Err("не успел к нужному месту".into())));
                    }
                    return;
                }
                self.swap = Some(Swap::Ready(Box::new(SwapReady {
                    token,
                    active,
                    start,
                    pre,
                    hold: Vec::new(),
                    hold_start: None,
                })));
            }
        }
    }

    /// Только что декодированный кусок (`scratch`, время `t0..t1`) придержан ради подмены —
    /// тогда `true`; если придержано достаточно, источники сводятся и подмена завершается.
    fn hold_for_swap(&mut self, t0: Option<f64>, t1: Option<f64>) -> bool {
        let Some(Swap::Ready(r)) = &mut self.swap else { return false };
        let Some(a) = self.cur.as_ref().filter(|a| a.token == r.token) else {
            return false;
        };
        let (Some(t0), Some(t1)) = (t0, t1) else { return false };
        if r.hold_start.is_none() {
            if t1 < r.start - HOLD_PRE_SECS {
                return false;
            }
            if t0 > r.start - SEARCH_SECS {
                // Проскочили начало окна поиска — свести точно уже не выйдет.
                let token = r.token;
                self.abort_swap(false);
                let _ = self.events.send(Event::Swapped(token, Err("не успел к нужному месту".into())));
                return false;
            }
            r.hold_start = Some(t0);
        }
        r.hold.extend_from_slice(&self.scratch);
        let (rate, ch) = (a.format.rate, a.format.channels.max(1) as usize);
        let hold_start = r.hold_start.expect("hold_start");
        let need_end = r.start + SEARCH_SECS + (CORR_N + SPLICE_LONG) as f64 / rate as f64 + 0.01;
        if hold_start + (r.hold.len() / ch) as f64 / rate as f64 <= need_end {
            return true;
        }
        let Some(Swap::Ready(r)) = self.swap.take() else { unreachable!() };
        let r = *r;
        let gain = a.gain;
        let Some((out, h0, corr)) = splice(&r.hold, hold_start, &r.pre, r.start, rate, ch, gain) else {
            r.active.source.cancel();
            // Придержанное (включая только что декодированное) просто отдаём как есть.
            self.leftover = r.hold;
            self.leftover_pos = 0;
            self.push_leftover();
            let _ = self.events.send(Event::Swapped(r.token, Err("не удалось совместить".into())));
            return true;
        };
        tracing::info!(
            "подмена источника: {} → {} · сдвиг {:+.1} мс · совпадение {corr:.2}",
            a.info.codec,
            r.active.info.codec,
            ((h0 as f64 / rate as f64 + hold_start) - r.start) * 1000.0
        );
        let mut new = r.active;
        new.gain = gain;
        new.fade_checked = false;
        let info = new.info.clone();
        let base = hold_start + h0 as f64 / rate as f64;
        let start = self.shared.written.load(Ordering::Acquire) + (h0 * ch) as u64;
        self.shared.segments.lock().push_back(Segment {
            start,
            token: new.token,
            base_secs: base,
            format: new.format,
            info: info.clone(),
        });
        let token = new.token;
        if let Some(old) = self.cur.replace(new) {
            old.source.cancel();
        }
        self.leftover = out;
        self.leftover_pos = 0;
        self.push_leftover();
        let _ = self.events.send(Event::Swapped(token, Ok(info)));
        true
    }

    /// Отменяет подмену. `keep_hold` — придержанный звук всё же отдать.
    fn abort_swap(&mut self, keep_hold: bool) {
        match self.swap.take() {
            Some(Swap::Preparing { cancel, .. }) => {
                if let Some(c) = cancel {
                    c.cancel();
                }
            }
            Some(Swap::Ready(r)) => {
                r.active.source.cancel();
                if keep_hold && !r.hold.is_empty() {
                    let rest = self.leftover.split_off(self.leftover_pos);
                    self.leftover = r.hold;
                    self.leftover.extend(rest);
                    self.leftover_pos = 0;
                }
            }
            None => {}
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

fn token_of(a: &Option<Active>) -> Token {
    a.as_ref().map_or(0, |a| a.token)
}

/// Отладка подмены без звука: `a` играет, на секунде `at` его подменяют на `b` тем же
/// путём, что и в плеере. `rate` — принудительная частота вывода (проверка ресемплера).
/// Возвращает отчёт; `wav` — куда записать склеенный кусок для прослушивания.
pub fn debug_splice(
    a: Source,
    b: Source,
    at: f64,
    rate: Option<u32>,
    wav: Option<&std::path::Path>,
) -> Result<String, String> {
    let mut aa = open(1, a, 1.0).map_err(|e| e.unwrap_or_else(|| "отменено".into()))?;
    if let Some(r) = rate.filter(|r| *r != aa.format.rate) {
        aa.resampler = Some(Resampler::new(aa.format.rate, r, aa.format.channels));
        aa.format.rate = r;
    }
    let out = aa.format;
    let decoded = Arc::new(AtomicU64::new((at - 2.0).max(0.0).to_bits()));
    let (bb, start, pre) = match prepare(2, b, out, decoded) {
        Prepared::Ready { active, start, pre, .. } => (active, start, pre),
        Prepared::Failed { err, .. } => return Err(err.unwrap_or_else(|| "отменено".into())),
    };
    let time = Time::try_from_secs_f64((start - 1.0).max(0.0)).ok_or("время")?;
    aa.reader
        .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(aa.track_id) })
        .map_err(|e| format!("перемотка a: {e}"))?;
    aa.decoder.reset();
    if let Some(r) = &mut aa.resampler {
        r.reset();
    }
    let ch = out.channels.max(1) as usize;
    let need_end = start + SEARCH_SECS + (CORR_N + SPLICE_LONG) as f64 / out.rate as f64 + 0.01;
    let (mut hold, mut chunk, mut tmp) = (Vec::new(), Vec::new(), Vec::new());
    let mut hold_start = None;
    loop {
        let p = aa.reader.next_packet().map_err(|e| format!("a: {e}"))?.ok_or("a кончился")?;
        if p.track_id != aa.track_id {
            continue;
        }
        let (t0, t1) = (aa.secs(p.pts).unwrap_or(0.0), aa.secs(p.pts.saturating_add(p.dur)).unwrap_or(0.0));
        if aa.decode_into(&p, &mut chunk, &mut tmp).is_err() {
            continue;
        }
        if hold_start.is_none() {
            if t1 < start - HOLD_PRE_SECS {
                continue;
            }
            hold_start = Some(t0);
        }
        hold.extend_from_slice(&chunk);
        if hold_start.expect("hs") + (hold.len() / ch) as f64 / out.rate as f64 > need_end {
            break;
        }
    }
    let hs = hold_start.expect("hs");
    let (spliced, h0, corr) = splice(&hold, hs, &pre, start, out.rate, ch, 1.0).ok_or("splice: не хватило данных")?;
    let fl = if corr >= 0.7 { SPLICE_SHORT } else { SPLICE_LONG };
    // Самый большой скачок между соседними сэмплами: до склейки и в зоне склейки.
    let jump = |v: &[f32], from: usize, to: usize| -> f32 {
        let (from, to) = (from.max(1), to.min(v.len() / ch));
        (from..to)
            .flat_map(|i| (0..ch).map(move |c| (i, c)))
            .map(|(i, c)| (v[i * ch + c] - v[(i - 1) * ch + c]).abs())
            .fold(0.0, f32::max)
    };
    let before = jump(&hold, 1, h0);
    let around = jump(&spliced, h0.saturating_sub(64), h0 + fl + 64);
    // Насколько источники совпадают после выравнивания (разница относительно сигнала).
    let (mut diff, mut sig) = (0f64, 0f64);
    for i in 0..CORR_N.min(pre.len() / ch).min(hold.len() / ch - h0) {
        for c in 0..ch {
            let x = hold[(h0 + i) * ch + c] as f64;
            let y = pre[i * ch + c] as f64;
            diff += (x - y) * (x - y);
            sig += x * x;
        }
    }
    let resid_db = 10.0 * (diff / sig.max(1e-12)).log10();
    if let Some(path) = wav {
        write_wav(path, &spliced, out.rate, out.channels).map_err(|e| format!("wav: {e}"))?;
    }
    Ok(format!(
        "a: {} {} Гц → вывод {} Гц\nb: {} {} Гц{}\nточка: {start:.3} с · сдвиг {:+.2} мс · совпадение {corr:.3} · наплыв {fl} кадров\n\
         разница после выравнивания: {resid_db:.1} дБ\nмакс. скачок сэмпла: до склейки {before:.4}, в склейке {around:.4}",
        aa.info.codec,
        aa.info.rate,
        out.rate,
        bb.info.codec,
        bb.info.rate,
        bb.info.resampled_to.map(|r| format!(" (ресемплинг в {r})")).unwrap_or_default(),
        ((hs + h0 as f64 / out.rate as f64) - start) * 1000.0,
    ))
}

fn write_wav(path: &std::path::Path, s: &[f32], rate: u32, ch: u16) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(44 + s.len() * 2);
    let data_len = (s.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&ch.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * ch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for x in s {
        b.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b)
}

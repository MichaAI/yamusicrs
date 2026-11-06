//! Обработка звука в потоке вывода: параметрический эквалайзер (каскад peaking-биквадов)
//! и анализатор спектра для визуализации.
//!
//! Эквалайзер с нулевыми усилениями и выключенный пропускает звук без единой операции
//! (bit-perfect сохраняется). Спектр считается по уже обработанному и отданному звуку.

use std::f32::consts::PI;
use std::sync::atomic::{AtomicU32, Ordering};

/// Сколько полос у спектра, который отдаётся интерфейсу.
pub const SPECTRUM_BANDS: usize = 64;
const FFT: usize = 2048;

/// Настройки эквалайзера: центры полос (Гц) и усиления (дБ).
#[derive(Debug, Clone, PartialEq)]
pub struct EqParams {
    pub enabled: bool,
    pub freqs: Vec<f32>,
    pub gains: Vec<f32>,
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            enabled: false,
            freqs: eq_freqs(10),
            gains: vec![0.0; 10],
        }
    }
}

impl EqParams {
    fn active(&self) -> bool {
        self.enabled && self.gains.iter().any(|g| g.abs() > 0.05)
    }
}

/// `n` центров полос, логарифмически от 31 Гц до 16 кГц (для 10 — классические октавы).
pub fn eq_freqs(n: usize) -> Vec<f32> {
    if n == 10 {
        return vec![31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
    }
    let n = n.max(1);
    (0..n)
        .map(|i| {
            let t = if n == 1 { 0.5 } else { i as f32 / (n - 1) as f32 };
            31.0 * (16000.0f32 / 31.0).powf(t)
        })
        .collect()
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    fn peaking(rate: f32, freq: f32, gain_db: f32, q: f32) -> Option<Self> {
        if gain_db.abs() < 0.05 || freq >= rate * 0.45 {
            return None;
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * freq / rate;
        let alpha = w.sin() / (2.0 * q);
        let cw = w.cos();
        let a0 = 1.0 + alpha / a;
        Some(Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cw) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cw) / a0,
            a2: (1.0 - alpha / a) / a0,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct State {
    z1: f32,
    z2: f32,
}

/// Эквалайзер одного потока вывода (состояние фильтров по каналам).
pub struct Equalizer {
    rate: f32,
    channels: usize,
    filters: Vec<Biquad>,
    state: Vec<State>,
    version: u32,
    on: bool,
}

impl Equalizer {
    pub fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate: rate as f32,
            channels: channels.max(1),
            filters: Vec::new(),
            state: Vec::new(),
            version: u32::MAX,
            on: false,
        }
    }

    pub fn configure(&mut self, p: &EqParams, version: u32) {
        self.version = version;
        self.on = p.active();
        let n = p.freqs.len().max(1) as f32;
        // Ширина полосы ~ расстояние между соседними центрами: Q из числа полос на 9 октав.
        let q = (n / 9.0 * 1.2).clamp(0.7, 4.0).max(0.9) * 1.0;
        self.filters = p
            .freqs
            .iter()
            .zip(&p.gains)
            .filter_map(|(f, g)| Biquad::peaking(self.rate, *f, *g, q))
            .collect();
        self.state.resize(self.filters.len() * self.channels, State::default());
        self.state.iter_mut().for_each(|s| *s = State::default());
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        if !self.on || self.filters.is_empty() {
            return;
        }
        let ch = self.channels;
        for frame in buf.chunks_exact_mut(ch) {
            for (c, s) in frame.iter_mut().enumerate() {
                let mut x = *s;
                for (i, f) in self.filters.iter().enumerate() {
                    let st = &mut self.state[i * ch + c];
                    let y = f.b0 * x + st.z1;
                    st.z1 = f.b1 * x - f.a1 * y + st.z2;
                    st.z2 = f.b2 * x - f.a2 * y;
                    x = y;
                }
                *s = x.clamp(-1.0, 1.0);
            }
        }
    }
}

/// Спектр для интерфейса: уровни 0..1 по логарифмическим полосам, f32 в атомиках.
pub struct Spectrum {
    bands: [AtomicU32; SPECTRUM_BANDS],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            bands: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}

impl Spectrum {
    pub fn store(&self, i: usize, v: f32) {
        self.bands[i].store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn read(&self) -> Vec<f32> {
        self.bands.iter().map(|b| f32::from_bits(b.load(Ordering::Relaxed))).collect()
    }
}

/// Накапливает моно-сэмплы и раз в окно считает спектр.
pub struct Analyzer {
    rate: f32,
    channels: usize,
    buf: Vec<f32>,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Границы полос в бинах FFT.
    edges: Vec<usize>,
    smooth: [f32; SPECTRUM_BANDS],
    /// Включён ли анализ (выключенный не тратит CPU).
    pub enabled: bool,
}

impl Analyzer {
    pub fn new(rate: u32, channels: usize) -> Self {
        let rate = rate as f32;
        let window = (0..FFT).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT as f32).cos()).collect();
        let bin = |hz: f32| ((hz * FFT as f32 / rate) as usize).clamp(1, FFT / 2);
        let (lo, hi) = (40.0f32, (rate * 0.45).min(16000.0));
        let mut edges: Vec<usize> = (0..=SPECTRUM_BANDS)
            .map(|i| bin(lo * (hi / lo).powf(i as f32 / SPECTRUM_BANDS as f32)))
            .collect();
        // Низкие полосы уже, чем бин: растягиваем, чтобы каждая имела хотя бы один бин.
        for i in 1..edges.len() {
            if edges[i] <= edges[i - 1] {
                edges[i] = (edges[i - 1] + 1).min(FFT / 2);
            }
        }
        Self {
            rate,
            channels: channels.max(1),
            buf: Vec::with_capacity(FFT * 2),
            window,
            re: vec![0.0; FFT],
            im: vec![0.0; FFT],
            edges,
            smooth: [0.0; SPECTRUM_BANDS],
            enabled: false,
        }
    }

    pub fn push(&mut self, data: &[f32], out: &Spectrum) {
        if !self.enabled {
            return;
        }
        for frame in data.chunks_exact(self.channels) {
            self.buf.push(frame.iter().sum::<f32>() / self.channels as f32);
        }
        // Окно скользит по ~86 мс/кадр экрана; лишнее старьё отбрасываем.
        if self.buf.len() >= FFT {
            let start = self.buf.len() - FFT;
            self.analyze(start, out);
            self.buf.clear();
        }
    }

    fn analyze(&mut self, start: usize, out: &Spectrum) {
        for i in 0..FFT {
            self.re[i] = self.buf[start + i] * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im);
        let _ = self.rate;
        for b in 0..SPECTRUM_BANDS {
            let (a, z) = (self.edges[b], self.edges[b + 1].max(self.edges[b] + 1));
            let mut peak = 0.0f32;
            for k in a..z.min(FFT / 2) {
                peak = peak.max((self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt());
            }
            // Амплитуда → дБ в окне −70..0, плюс небольшой наклон: у музыки спад к верхам.
            let amp = peak / (FFT as f32 / 4.0);
            let tilt = 1.5 * (b as f32 / SPECTRUM_BANDS as f32) * 6.0;
            let db = 20.0 * amp.max(1e-7).log10() + tilt;
            let v = ((db + 70.0) / 70.0).clamp(0.0, 1.0);
            // Быстро вверх, медленно вниз.
            let s = &mut self.smooth[b];
            *s = if v > *s { *s + (v - *s) * 0.7 } else { *s + (v - *s) * 0.25 };
            out.store(b, *s);
        }
    }
}

/// Радикс-2 БПФ на месте.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

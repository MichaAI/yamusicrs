//! Потоковый ресемплер windowed-sinc (окно Блэкмана) с таблицей фаз.
//!
//! Нужен только для горячей замены источника посреди трека, когда у нового другая
//! частота, а поток PipeWire пересоздавать нельзя (будет пауза). Это уже не
//! bit-perfect — интерфейс об этом сообщает и умеет перезапустить трек в родном формате.

/// Фаз в таблице ядра; между соседними — линейная интерполяция.
const PHASES: usize = 256;
/// Нулей sinc по каждую сторону при частоте среза = 1 (при понижении частоты ядро шире).
const ZEROS: f64 = 32.0;

pub struct Resampler {
    /// Входных сэмплов на один выходной.
    step: f64,
    ch: usize,
    /// Полуширина ядра во входных сэмплах.
    half: usize,
    /// `(PHASES + 1) × 2·half` коэффициентов, строка — дробная часть позиции.
    table: Vec<f32>,
    /// История входа (чередующиеся каналы); начинается с `half` нулевых кадров.
    buf: Vec<f32>,
    /// Позиция следующего выходного кадра во входных кадрах от начала `buf`.
    pos: f64,
}

impl Resampler {
    pub fn new(from: u32, to: u32, channels: u16) -> Self {
        let step = from as f64 / to as f64;
        // Срез — чуть ниже Найквиста меньшей из частот (в долях входного Найквиста).
        let cutoff = (to as f64 / from as f64).min(1.0) * 0.97;
        let half = (ZEROS / cutoff).ceil() as usize;
        let taps = 2 * half;
        let mut table = vec![0f32; (PHASES + 1) * taps];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let row = &mut table[p * taps..(p + 1) * taps];
            let mut sum = 0.0;
            for (j, c) in row.iter_mut().enumerate() {
                // Отвод j — входной кадр i0 - half + 1 + j; расстояние до позиции вывода.
                let x = (j as f64 - half as f64 + 1.0) - frac;
                let t = x / half as f64;
                let w = if t.abs() >= 1.0 {
                    0.0
                } else {
                    0.42 + 0.5 * (std::f64::consts::PI * t).cos() + 0.08 * (2.0 * std::f64::consts::PI * t).cos()
                };
                let v = cutoff * sinc(cutoff * x) * w;
                *c = v as f32;
                sum += v;
            }
            // Единичное усиление на постоянном токе для каждой фазы — без «ряби» громкости.
            if sum.abs() > 1e-9 {
                row.iter_mut().for_each(|c| *c = (*c as f64 / sum) as f32);
            }
        }
        let ch = channels.max(1) as usize;
        Self {
            step,
            ch,
            half,
            table,
            buf: vec![0.0; half * ch],
            pos: half as f64,
        }
    }

    /// Добавляет вход и дописывает в `out` всё, что уже можно посчитать.
    /// Выходной кадр k соответствует входному моменту k·from/to — без сдвига во времени.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        let ch = self.ch;
        let taps = 2 * self.half;
        let frames = self.buf.len() / ch;
        loop {
            let i0 = self.pos.floor() as usize;
            if i0 + self.half >= frames {
                break;
            }
            let ph = (self.pos - i0 as f64) * PHASES as f64;
            let p = (ph as usize).min(PHASES - 1);
            let w = (ph - p as f64) as f32;
            let r0 = &self.table[p * taps..(p + 1) * taps];
            let r1 = &self.table[(p + 1) * taps..(p + 2) * taps];
            let first = i0 + 1 - self.half;
            for c in 0..ch {
                let mut acc = 0f32;
                for j in 0..taps {
                    let coef = r0[j] + (r1[j] - r0[j]) * w;
                    acc += self.buf[(first + j) * ch + c] * coef;
                }
                out.push(acc);
            }
            self.pos += self.step;
        }
        // Выкидываем вход, который больше не попадёт ни в одно ядро.
        let keep_from = (self.pos.floor() as usize + 1).saturating_sub(self.half);
        if keep_from > 0 {
            self.buf.drain(..keep_from * ch);
            self.pos -= keep_from as f64;
        }
    }

    /// Новая позиция (после перемотки): история не должна протечь в новый кусок.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.buf.resize(self.half * self.ch, 0.0);
        self.pos = self.half as f64;
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

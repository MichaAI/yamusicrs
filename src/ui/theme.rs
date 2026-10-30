//! Палитра интерфейса, которая подстраивается под обложку текущего трека.

use std::time::Instant;

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    pub const fn hex(v: u32) -> Self {
        Rgb(((v >> 16) & 0xff) as f32, ((v >> 8) & 0xff) as f32, (v & 0xff) as f32)
    }

    pub fn color(self) -> Color {
        Color::Rgb(self.0.round() as u8, self.1.round() as u8, self.2.round() as u8)
    }

    pub fn lerp(self, o: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb(
            self.0 + (o.0 - self.0) * t,
            self.1 + (o.1 - self.1) * t,
            self.2 + (o.2 - self.2) * t,
        )
    }

    pub fn scale(self, k: f32) -> Rgb {
        Rgb(
            (self.0 * k).min(255.0),
            (self.1 * k).min(255.0),
            (self.2 * k).min(255.0),
        )
    }

    fn to_hsl(self) -> (f32, f32, f32) {
        let (r, g, b) = (self.0 / 255.0, self.1 / 255.0, self.2 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        if (max - min).abs() < 1e-6 {
            return (0.0, 0.0, l);
        }
        let d = max - min;
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s, l)
    }

    fn from_hsl(h: f32, s: f32, l: f32) -> Rgb {
        if s == 0.0 {
            return Rgb(l * 255.0, l * 255.0, l * 255.0);
        }
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        let f = |mut t: f32| {
            t = t.rem_euclid(1.0);
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        Rgb(f(h + 1.0 / 3.0) * 255.0, f(h) * 255.0, f(h - 1.0 / 3.0) * 255.0)
    }

    /// Тот же оттенок, но с заданными насыщенностью и светлотой (в пределах).
    fn tuned(self, s_min: f32, l_min: f32, l_max: f32) -> Rgb {
        let (h, s, l) = self.to_hsl();
        Rgb::from_hsl(h, s.max(s_min), l.clamp(l_min, l_max))
    }

    pub fn rotate_hue(self, by: f32) -> Rgb {
        let (h, s, l) = self.to_hsl();
        Rgb::from_hsl(h + by, s, l)
    }

    fn saturation(self) -> f32 {
        self.to_hsl().1
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Главный акцент: прогресс, активные элементы.
    pub accent: Rgb,
    /// Второй цвет градиентов.
    pub accent2: Rgb,
    /// Обычный текст.
    pub fg: Rgb,
    /// Второстепенный текст.
    pub dim: Rgb,
    /// Тонированный фон выделения/панелей.
    pub surface: Rgb,
}

impl Default for Palette {
    fn default() -> Self {
        // Фирменный жёлтый Музыки + тёплый оранжевый.
        Palette {
            accent: Rgb::hex(0xFFCC00),
            accent2: Rgb::hex(0xFF6B3D),
            fg: Rgb::hex(0xE6E6E6),
            dim: Rgb::hex(0x8A8A8A),
            surface: Rgb::hex(0x2A2620),
        }
    }
}

impl Palette {
    pub fn lerp(&self, o: &Palette, t: f32) -> Palette {
        Palette {
            accent: self.accent.lerp(o.accent, t),
            accent2: self.accent2.lerp(o.accent2, t),
            fg: self.fg.lerp(o.fg, t),
            dim: self.dim.lerp(o.dim, t),
            surface: self.surface.lerp(o.surface, t),
        }
    }

    /// k-means по уменьшенной обложке: два самых «характерных» цвета → акценты.
    pub fn from_image(img: &image::RgbaImage) -> Palette {
        let small = image::imageops::thumbnail(img, 40, 40);
        let px: Vec<[f32; 3]> = small
            .pixels()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect();
        if px.is_empty() {
            return Palette::default();
        }
        let k = 6;
        let mut centers: Vec<[f32; 3]> = (0..k).map(|i| px[i * px.len() / k]).collect();
        let mut assign = vec![0usize; px.len()];
        for _ in 0..12 {
            for (i, p) in px.iter().enumerate() {
                assign[i] = (0..k)
                    .min_by(|&a, &b| dist(p, &centers[a]).total_cmp(&dist(p, &centers[b])))
                    .unwrap_or(0);
            }
            let mut sum = vec![[0f32; 4]; k];
            for (i, p) in px.iter().enumerate() {
                let s = &mut sum[assign[i]];
                s[0] += p[0];
                s[1] += p[1];
                s[2] += p[2];
                s[3] += 1.0;
            }
            for (c, s) in centers.iter_mut().zip(&sum) {
                if s[3] > 0.0 {
                    *c = [s[0] / s[3], s[1] / s[3], s[2] / s[3]];
                }
            }
        }
        let mut counts = vec![0usize; k];
        for a in &assign {
            counts[*a] += 1;
        }
        // Оценка кластера: населённость × насыщенность — серые фоны обложек не побеждают.
        let mut ranked: Vec<(Rgb, f32)> = centers
            .iter()
            .zip(&counts)
            .filter(|(_, n)| **n > 0)
            .map(|(c, n)| {
                let rgb = Rgb(c[0], c[1], c[2]);
                (rgb, (*n as f32).sqrt() * (0.15 + rgb.saturation()))
            })
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let dominant = ranked.first().map(|r| r.0).unwrap_or(Rgb::hex(0xFFCC00));
        let accent = dominant.tuned(0.45, 0.58, 0.72);
        // Второй акцент — заметно отличный по цвету кластер, либо сдвиг оттенка.
        let accent2 = ranked
            .iter()
            .skip(1)
            .map(|r| r.0)
            .find(|c| dist(&[c.0, c.1, c.2], &[dominant.0, dominant.1, dominant.2]) > 70.0 * 70.0)
            .map(|c| c.tuned(0.4, 0.5, 0.68))
            .unwrap_or_else(|| accent.rotate_hue(0.08));
        Palette {
            accent,
            accent2,
            fg: Rgb::hex(0xE8E8E8).lerp(accent, 0.08),
            dim: Rgb::hex(0x8C8C8C).lerp(accent, 0.15),
            surface: dominant.tuned(0.25, 0.12, 0.16),
        }
    }
}

fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Плавный переход палитры при смене трека.
pub struct ThemeAnim {
    from: Palette,
    to: Palette,
    start: Instant,
}

const THEME_FADE_SECS: f32 = 0.9;

impl ThemeAnim {
    pub fn new() -> Self {
        Self {
            from: Palette::default(),
            to: Palette::default(),
            start: Instant::now(),
        }
    }

    pub fn set(&mut self, p: Palette) {
        self.from = self.current();
        self.to = p;
        self.start = Instant::now();
    }

    pub fn current(&self) -> Palette {
        let t = (self.start.elapsed().as_secs_f32() / THEME_FADE_SECS).min(1.0);
        // smoothstep
        let t = t * t * (3.0 - 2.0 * t);
        self.from.lerp(&self.to, t)
    }

    pub fn animating(&self) -> bool {
        self.start.elapsed().as_secs_f32() < THEME_FADE_SECS
    }
}

/// Цвет градиента в точке `t` с бегущим бликом (`phase` — время, сек).
pub fn shimmer(a: Rgb, b: Rgb, t: f32, phase: f32) -> Rgb {
    let base = a.lerp(b, t);
    let x = (t - (phase * 0.35).fract() * 1.6 + 0.3).abs();
    let glow = (1.0 - x / 0.12).max(0.0);
    base.lerp(Rgb(255.0, 255.0, 255.0), glow * 0.35)
}

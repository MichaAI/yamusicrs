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
    /// Четыре разных цвета обложки для блоба.
    pub blob: [Rgb; 4],
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
            blob: [
                Rgb::hex(0xFFCC00),
                Rgb::hex(0xFF6B3D),
                Rgb::hex(0xF02A4F),
                Rgb::hex(0x7A5ADC),
            ],
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
            blob: [0, 1, 2, 3].map(|i| self.blob[i].lerp(o.blob[i], t)),
        }
    }

    /// Median cut как в Quickshell `ColorQuantizer`: обложка 64×64, 2⁴ корзин равной
    /// населённости, похожие корзины сливаются. Акцент — самый «характерный» цвет.
    pub fn from_image(img: &image::RgbaImage) -> Palette {
        let small = image::imageops::thumbnail(img, 64, 64);
        let mut px: Vec<[f32; 3]> = small
            .pixels()
            .filter(|p| p[3] > 0)
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect();
        if px.is_empty() {
            return Palette::default();
        }
        let mut buckets = Vec::new();
        median_cut(&mut px, 0, 4, &mut buckets);
        // Слить почти одинаковые корзины: их число = доля цвета в обложке.
        let mut clusters: Vec<([f32; 3], f32)> = Vec::new();
        for c in buckets {
            match clusters.iter_mut().find(|(k, _)| dist(k, &c) < 28.0 * 28.0) {
                Some((k, n)) => {
                    for i in 0..3 {
                        k[i] = (k[i] * *n + c[i]) / (*n + 1.0);
                    }
                    *n += 1.0;
                }
                None => clusters.push((c, 1.0)),
            }
        }
        // Оценка: доля × насыщенность; почти белое/чёрное не годится в акцент.
        let mut ranked: Vec<(Rgb, f32)> = clusters
            .iter()
            .map(|(c, n)| {
                let rgb = Rgb(c[0], c[1], c[2]);
                let (_, sat, l) = rgb.to_hsl();
                let usable = if !(0.08..=0.94).contains(&l) { 0.1 } else { 1.0 };
                (rgb, n.sqrt() * (0.12 + sat) * usable)
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
            .find(|c| {
                // Другой оттенок и не серый — иначе градиент получается плоским.
                let (h1, s1, _) = c.to_hsl();
                let (h0, _, _) = dominant.to_hsl();
                let dh = (h1 - h0).abs().min(1.0 - (h1 - h0).abs());
                dh > 0.06 && s1 > 0.25
            })
            .map(|c| c.tuned(0.4, 0.5, 0.68))
            .unwrap_or_else(|| accent.rotate_hue(0.08));
        // Для блоба — до четырёх заметно разных цветов обложки (по оценке «характерности»),
        // недостающие достраиваются соседними оттенками акцента.
        let mut blob: Vec<Rgb> = Vec::new();
        for (c, _) in &ranked {
            let c = c.tuned(0.30, 0.42, 0.70);
            if blob
                .iter()
                .all(|b| dist(&[b.0, b.1, b.2], &[c.0, c.1, c.2]) > 45.0 * 45.0)
            {
                blob.push(c);
            }
            if blob.len() == 4 {
                break;
            }
        }
        for shift in [0.09, -0.09, 0.18, -0.18] {
            if blob.len() < 4 {
                blob.push(accent.rotate_hue(shift));
            }
        }
        Palette {
            accent,
            accent2,
            fg: Rgb::hex(0xE8E8E8).lerp(accent, 0.08),
            dim: Rgb::hex(0x8C8C8C).lerp(accent, 0.15),
            surface: dominant.tuned(0.25, 0.12, 0.16),
            blob: [blob[0], blob[1], blob[2], blob[3]],
        }
    }
}

fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Рекурсивное деление пополам по каналу с наибольшим разбросом; на глубине `max` — среднее.
fn median_cut(px: &mut [[f32; 3]], depth: u32, max: u32, out: &mut Vec<[f32; 3]>) {
    if px.is_empty() {
        return;
    }
    if depth >= max || px.len() < 2 {
        let n = px.len() as f32;
        let sum = px
            .iter()
            .fold([0f32; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]);
        out.push([sum[0] / n, sum[1] / n, sum[2] / n]);
        return;
    }
    let range = |i: usize| {
        let (lo, hi) = px
            .iter()
            .fold((255f32, 0f32), |(lo, hi), p| (lo.min(p[i]), hi.max(p[i])));
        hi - lo
    };
    let ch = (0..3).max_by(|&a, &b| range(a).total_cmp(&range(b))).unwrap_or(0);
    px.sort_by(|a, b| a[ch].total_cmp(&b[ch]));
    let mid = px.len() / 2;
    let (l, r) = px.split_at_mut(mid);
    median_cut(l, depth + 1, max, out);
    median_cut(r, depth + 1, max, out);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yellow_smiley_is_yellow() {
        // Белый фон, жёлтый круг, чёрные детали — как обложка со смайлом.
        let img = image::RgbaImage::from_fn(200, 200, |x, y| {
            let (dx, dy) = (x as f32 - 100.0, y as f32 - 100.0);
            let r = (dx * dx + dy * dy).sqrt();
            if r < 15.0 {
                image::Rgba([10, 10, 10, 255])
            } else if r < 80.0 {
                image::Rgba([250, 230, 20, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            }
        });
        let p = Palette::from_image(&img);
        let Rgb(r, g, b) = p.accent;
        eprintln!("accent = {r:.0} {g:.0} {b:.0}");
        assert!(
            r > 150.0 && g > 150.0 && b < 120.0,
            "ожидали жёлтый, получили {r:.0} {g:.0} {b:.0}"
        );
    }

    /// `YAMUSICRS_COVER=cover.jpg cargo test palette_of_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn palette_of_file() {
        let img = image::open(std::env::var("YAMUSICRS_COVER").unwrap())
            .unwrap()
            .to_rgba8();
        let p = Palette::from_image(&img);
        eprintln!(
            "accent {:?}\naccent2 {:?}\nsurface {:?}",
            p.accent, p.accent2, p.surface
        );
    }
}

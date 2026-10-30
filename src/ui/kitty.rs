//! Обложки через kitty graphics protocol с юникод-плейсхолдерами.
//!
//! Картинка передаётся терминалу один раз (`a=t`) и живёт там как объект с id.
//! Затем создаётся виртуальное размещение (`U=1`), а в буфер ratatui пишутся
//! ячейки `U+10EEEE` + диакритики строки/столбца, цвет текста = id картинки.
//! Это обычный текст: диффинг ratatui его не перерисовывает, а значит
//! анимации вокруг не трогают картинку и не роняют FPS.
//!
//! Без kitty рисуем полублоками `▀` (fg = верхний пиксель, bg = нижний).

use std::collections::HashMap;
use std::io::Write;

use base64::Engine;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::diacritics::DIACRITICS;

const PLACEHOLDER: char = '\u{10EEEE}';
/// Сторона картинки, которую отдаём терминалу.
const UPLOAD_PX: u32 = 640;
/// Радиус скругления углов обложки, доля стороны.
const CORNER_RADIUS: f32 = 0.14;
/// Сколько обложек держим загруженными в терминале.
const KEEP: usize = 48;

/// Полублочный рендер: размер в ячейках и (верх, низ) для каждой ячейки.
type HalfBlocks = ((u16, u16), Vec<(Color, Color)>);

pub struct Image {
    rgba: image::RgbaImage,
    /// Копия картинки в терминале на каждый размер в ячейках: одна обложка может
    /// одновременно стоять в шапке плейлиста и в боковой панели, и у каждого места
    /// должно быть своё размещение, иначе они перетягивают его друг у друга.
    variants: HashMap<(u16, u16), u32>,
    last_used: u64,
    /// Кеш полублочного рендера под конкретный размер.
    halfblock: Option<HalfBlocks>,
}

pub struct Images {
    kitty: bool,
    next_id: u32,
    tick: u64,
    by_key: HashMap<String, Image>,
    /// Escape-последовательности, которые надо вывести до следующего кадра.
    out: Vec<u8>,
    blob: Option<BlobState>,
}

/// Анимированный блоб волны: одна картинка в терминале, кадры правятся на месте.
struct BlobState {
    id: u32,
    placed: Option<Rect>,
    last: std::time::Instant,
}

/// Сторона кадра блоба в пикселях: терминал сам растянет и размоет.
const BLOB_PX: u32 = 72;
/// Не чаще ~15 кадров в секунду — блобу больше не нужно.
const BLOB_FRAME: std::time::Duration = std::time::Duration::from_millis(66);

impl Images {
    pub fn new(kitty: bool) -> Self {
        // id берём случайный в 24 битах, чтобы не пересечься с другой программой в том же окне.
        let seed = (std::process::id() & 0xff) << 16;
        Self {
            kitty,
            next_id: seed | 0x0101,
            tick: 0,
            by_key: HashMap::new(),
            out: Vec::new(),
            blob: None,
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.by_key.contains_key(key)
    }

    pub fn insert(&mut self, key: String, img: image::RgbaImage) {
        if self.by_key.contains_key(&key) {
            return;
        }
        let mut rgba = if img.width() > UPLOAD_PX || img.height() > UPLOAD_PX {
            image::imageops::resize(&img, UPLOAD_PX, UPLOAD_PX, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        round_corners(&mut rgba, CORNER_RADIUS);
        self.by_key.insert(
            key,
            Image {
                rgba,
                variants: HashMap::new(),
                last_used: self.tick,
                halfblock: None,
            },
        );
        self.evict();
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = (self.next_id + 1) & 0xFF_FFFF;
        // Байты id не должны быть нулевыми целиком — иначе цвет совпадёт с «чёрным» по умолчанию.
        if self.next_id & 0xFF == 0 {
            self.next_id += 1;
        }
        id
    }

    fn evict(&mut self) {
        while self.by_key.len() > KEEP {
            let Some(oldest) = self
                .by_key
                .iter()
                .min_by_key(|(_, v)| v.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(img) = self.by_key.remove(&oldest) {
                for id in img.variants.values() {
                    delete(&mut self.out, *id);
                }
            }
        }
    }

    /// Рисует картинку в `area` (квадрат по центру с учётом пропорций ячейки).
    pub fn render(&mut self, key: &str, area: Rect, buf: &mut Buffer, cell_aspect: f32) {
        self.tick += 1;
        let tick = self.tick;
        let kitty = self.kitty;
        let rect = fit_square(area, cell_aspect);
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        // id выделяем только под новый размер, а не каждый кадр.
        let size = (rect.width, rect.height);
        let id = match self.by_key.get(key) {
            Some(i) if kitty && !i.variants.contains_key(&size) => Some(self.alloc_id()),
            Some(_) => None,
            None => return,
        };
        let Some(img) = self.by_key.get_mut(key) else {
            return;
        };
        img.last_used = tick;
        if kitty {
            let id = match img.variants.get(&size) {
                Some(id) => *id,
                None => {
                    // Старые размеры (после ресайза окна) больше не нужны.
                    if img.variants.len() >= 3 {
                        for (_, old) in img.variants.drain() {
                            delete(&mut self.out, old);
                        }
                    }
                    let id = id.expect("id");
                    transmit(&mut self.out, id, &img.rgba);
                    let _ = write!(self.out, "\x1b_Ga=p,U=1,i={id},c={},r={},C=1,q=2\x1b\\", size.0, size.1);
                    img.variants.insert(size, id);
                    id
                }
            };
            let fg = Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8);
            for row in 0..rect.height.min(DIACRITICS.len() as u16) {
                for col in 0..rect.width.min(DIACRITICS.len() as u16) {
                    let mut s = String::with_capacity(12);
                    s.push(PLACEHOLDER);
                    s.push(DIACRITICS[row as usize]);
                    s.push(DIACRITICS[col as usize]);
                    if let Some(cell) = buf.cell_mut((rect.x + col, rect.y + row)) {
                        cell.set_symbol(&s).set_fg(fg).set_bg(Color::Reset);
                        cell.modifier = ratatui::style::Modifier::empty();
                    }
                }
            }
        } else {
            let size = (rect.width, rect.height);
            if img.halfblock.as_ref().is_none_or(|(s, _)| *s != size) {
                let small = image::imageops::resize(
                    &img.rgba,
                    size.0 as u32,
                    size.1 as u32 * 2,
                    image::imageops::FilterType::Triangle,
                );
                let mut cells = Vec::with_capacity(size.0 as usize * size.1 as usize);
                for y in 0..size.1 as u32 {
                    for x in 0..size.0 as u32 {
                        // Прозрачные (срезанные углом) пиксели — цвет терминала по умолчанию.
                        let px = |p: &image::Rgba<u8>| {
                            if p[3] < 128 {
                                Color::Reset
                            } else {
                                Color::Rgb(p[0], p[1], p[2])
                            }
                        };
                        cells.push((px(small.get_pixel(x, y * 2)), px(small.get_pixel(x, y * 2 + 1))));
                    }
                }
                img.halfblock = Some((size, cells));
            }
            let (_, cells) = img.halfblock.as_ref().expect("halfblock");
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let (fg, bg) = cells[y as usize * size.0 as usize + x as usize];
                    if let Some(cell) = buf.cell_mut((rect.x + x, rect.y + y)) {
                        cell.set_symbol("▀").set_fg(fg).set_bg(bg);
                    }
                }
            }
        }
    }

    /// Отдаёт накопленные команды терминалу. Вызывать перед `terminal.draw`.
    pub fn flush(&mut self, w: &mut impl Write) -> std::io::Result<()> {
        if !self.out.is_empty() {
            w.write_all(&self.out)?;
            w.flush()?;
            self.out.clear();
        }
        Ok(())
    }

    /// Удаляет все наши картинки из терминала (при выходе).
    /// Рисует блоб под текстом в `rect` (только kitty; иначе false — рисуйте фолбэк).
    pub fn blob(&mut self, rect: Rect, colors: [[f32; 3]; 2], phase: f32) -> bool {
        if !self.kitty || rect.width == 0 || rect.height == 0 {
            return false;
        }
        let fresh = self.blob.is_none();
        if fresh {
            let id = self.alloc_id();
            self.blob = Some(BlobState {
                id,
                placed: None,
                last: std::time::Instant::now(),
            });
        }
        let b = self.blob.as_mut().expect("blob");
        let id = b.id;
        if fresh {
            let px = blob_frame(BLOB_PX, colors, phase);
            send_rgba(&mut self.out, &format!("a=t,f=32,s={BLOB_PX},v={BLOB_PX},i={id}"), &px);
        } else if b.last.elapsed() >= BLOB_FRAME {
            b.last = std::time::Instant::now();
            let px = blob_frame(BLOB_PX, colors, phase);
            // r=1 — правим корневой кадр, X=1 — заменяем пиксели, а не смешиваем.
            send_rgba(
                &mut self.out,
                &format!("a=f,r=1,X=1,f=32,s={BLOB_PX},v={BLOB_PX},i={id}"),
                &px,
            );
        }
        let b = self.blob.as_mut().expect("blob");
        if b.placed != Some(rect) {
            if b.placed.is_some() {
                let _ = write!(self.out, "\x1b_Ga=d,d=i,i={id},p=1,q=2\x1b\\");
            }
            // Обычное размещение под текстом (z<0): надпись поверх блоба остаётся видна.
            let _ = write!(
                self.out,
                "\x1b7\x1b[{};{}H\x1b_Ga=p,i={id},p=1,c={},r={},z=-1,C=1,q=2\x1b\\\x1b8",
                rect.y + 1,
                rect.x + 1,
                rect.width,
                rect.height
            );
            b.placed = Some(rect);
        }
        true
    }

    /// Убирает блоб с экрана (данные остаются в терминале до выхода).
    pub fn hide_blob(&mut self) {
        if let Some(b) = self.blob.as_mut()
            && b.placed.take().is_some()
        {
            let _ = write!(self.out, "\x1b_Ga=d,d=i,i={},p=1,q=2\x1b\\", b.id);
        }
    }

    pub fn clear_all(&mut self, w: &mut impl Write) -> std::io::Result<()> {
        if self.kitty {
            for id in self
                .by_key
                .values()
                .flat_map(|i| i.variants.values())
                .chain(self.blob.as_ref().map(|b| &b.id))
            {
                write!(w, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\")?;
            }
            w.flush()?;
        }
        Ok(())
    }
}

/// Квадрат максимального размера внутри `area`, по центру.
/// `cell_aspect` = высота ячейки / ширина ячейки в пикселях (обычно ~2).
pub fn fit_square(area: Rect, cell_aspect: f32) -> Rect {
    let w_by_h = (area.height as f32 * cell_aspect).floor() as u16;
    let (w, h) = if w_by_h <= area.width {
        (w_by_h, area.height)
    } else {
        (area.width, (area.width as f32 / cell_aspect).floor() as u16)
    };
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// d=I — удалить картинку вместе с данными.
fn delete(out: &mut Vec<u8>, id: u32) {
    let _ = write!(out, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
}

fn transmit(out: &mut Vec<u8>, id: u32, rgba: &image::RgbaImage) {
    let mut png = Vec::new();
    if image::DynamicImage::ImageRgba8(rgba.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_err()
    {
        return;
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            let _ = write!(out, "\x1b_Ga=t,f=100,t=d,i={id},q=2,m={more};");
        } else {
            let _ = write!(out, "\x1b_Gm={more};");
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// Сырые RGBA-данные кусками по 4096 символов base64.
fn send_rgba(out: &mut Vec<u8>, head: &str, px: &[u8]) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(px);
    let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            let _ = write!(out, "\x1b_G{head},q=2,m={more};");
        } else {
            let _ = write!(out, "\x1b_Gm={more};");
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// Пиксель блоба в точке (u, v) ∈ [-1, 1]²: RGBA, альфа мягко уходит в ноль к краям.
/// Четыре «метабола» гуляют по кругу; цвет — смесь двух акцентов по тому, какой шар ближе.
pub fn blob_pixel(u: f32, v: f32, colors: [[f32; 3]; 2], t: f32) -> [f32; 4] {
    const BALLS: [(f32, f32, f32, f32, f32); 4] = [
        (0.31, 0.53, 0.0, 0.48, 0.0),
        (0.47, 0.29, 2.1, 0.40, 1.0),
        (0.23, 0.41, 4.2, 0.36, 0.0),
        (0.39, 0.61, 1.3, 0.30, 1.0),
    ];
    let (mut field, mut w2) = (0.0f32, 0.0f32);
    for (fx, fy, ph, r, which) in BALLS {
        let cx = 0.38 * (t * fx + ph).sin();
        let cy = 0.38 * (t * fy + ph * 1.7).cos();
        let d2 = (u - cx).powi(2) + (v - cy).powi(2);
        let f = (-d2 / (r * r)).exp();
        field += f;
        w2 += f * which;
    }
    let mix = (w2 / field.max(1e-4)).clamp(0.0, 1.0);
    let c = [0, 1, 2].map(|i| colors[0][i] + (colors[1][i] - colors[0][i]) * mix);
    // Ядро яркое, края — мягкое свечение.
    let a = ((field - 0.25) / 0.9).clamp(0.0, 1.0);
    let a = a * a * (3.0 - 2.0 * a);
    let glow = 1.0 + 0.25 * (field - 1.0).max(0.0);
    [
        (c[0] * glow).min(255.0),
        (c[1] * glow).min(255.0),
        (c[2] * glow).min(255.0),
        a * 255.0,
    ]
}

fn blob_frame(n: u32, colors: [[f32; 3]; 2], t: f32) -> Vec<u8> {
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let u = x as f32 / (n - 1) as f32 * 2.0 - 1.0;
            let v = y as f32 / (n - 1) as f32 * 2.0 - 1.0;
            let p = blob_pixel(u, v, colors, t);
            px.extend(p.map(|c| c as u8));
        }
    }
    px
}

/// Отношение высоты ячейки к ширине по данным терминала.
pub fn cell_aspect() -> f32 {
    match crossterm::terminal::window_size() {
        Ok(ws) if ws.width > 0 && ws.columns > 0 && ws.rows > 0 => {
            let cw = ws.width as f32 / ws.columns as f32;
            let ch = ws.height as f32 / ws.rows as f32;
            (ch / cw).clamp(1.0, 3.0)
        }
        _ => 2.0,
    }
}

/// Степень супереллипса углов: 2 — обычная дуга, 4–5 — сквиркл.
const SQUIRCLE_N: f32 = 5.0;

/// Скругляет углы сквирклом (`|x|ⁿ + |y|ⁿ = rⁿ`) через альфу со сглаживанием по краю.
fn round_corners(img: &mut image::RgbaImage, radius_frac: f32) {
    let (w, h) = img.dimensions();
    let r = radius_frac * w.min(h) as f32;
    if r < 1.0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            // Центр пикселя относительно центра ближайшей дуги угла.
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let cx = px.clamp(r, w as f32 - r);
            let cy = py.clamp(r, h as f32 - r);
            let (dx, dy) = ((px - cx).abs(), (py - cy).abs());
            let d = (dx.powf(SQUIRCLE_N) + dy.powf(SQUIRCLE_N)).powf(1.0 / SQUIRCLE_N);
            let coverage = (r - d + 0.5).clamp(0.0, 1.0);
            if coverage < 1.0 {
                let a = &mut img.get_pixel_mut(x, y)[3];
                *a = (*a as f32 * coverage).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn corners_are_transparent() {
        let mut img = image::RgbaImage::from_pixel(100, 100, image::Rgba([255, 0, 0, 255]));
        super::round_corners(&mut img, 0.1);
        assert_eq!(img.get_pixel(0, 0)[3], 0);
        assert_eq!(img.get_pixel(99, 99)[3], 0);
        assert_eq!(img.get_pixel(50, 0)[3], 255);
        assert_eq!(img.get_pixel(50, 50)[3], 255);
    }
}

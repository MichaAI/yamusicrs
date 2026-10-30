//! Возможности kitty поверх обычного текста: крупный текст (OSC 66), ссылки (OSC 8),
//! форма курсора мыши (OSC 22), буфер обмена (OSC 52) и уведомления (OSC 99).
//!
//! Крупный текст и ссылки выводятся оверлеем после кадра ratatui: в символ клетки
//! escape-последовательность не вшить (ratatui считает её ширину и ломает дифф).

use std::io::Write;

use base64::Engine;
use ratatui::style::{Color, Modifier, Style};

/// Кусок текста оверлея со своим стилем.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: Style,
    /// Волнистое подчёркивание этого цвета (караоке).
    pub curly: Option<Color>,
}

/// Текст, который дописывается поверх кадра: крупный (scale > 1) и/или со ссылкой.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub x: u16,
    pub y: u16,
    pub scale: u8,
    pub runs: Vec<Run>,
    pub link: Option<String>,
}

fn sgr(out: &mut Vec<u8>, style: Style, curly: Option<Color>) {
    let _ = out.write_all(b"\x1b[0");
    let col = |out: &mut Vec<u8>, c: Color, base: u8| match c {
        Color::Rgb(r, g, b) => {
            let _ = write!(out, ";{base}8;2;{r};{g};{b}");
        }
        Color::Black => {
            let _ = write!(out, ";{base}0");
        }
        // Яркий белый: 97 (текст) / 107 (фон).
        Color::White => {
            let _ = write!(out, ";{}", if base == 3 { 97 } else { 107 });
        }
        _ => {}
    };
    if let Some(fg) = style.fg {
        col(out, fg, 3);
    }
    if let Some(bg) = style.bg {
        col(out, bg, 4);
    }
    if style.add_modifier.contains(Modifier::BOLD) {
        let _ = out.write_all(b";1");
    }
    if let Some(Color::Rgb(r, g, b)) = curly {
        // 4:3 — волнистое подчёркивание, 58 — его цвет.
        let _ = write!(out, ";4:3;58:2::{r}:{g}:{b}");
    }
    let _ = out.write_all(b"m");
}

impl Overlay {
    /// Ширина в клетках экрана.
    pub fn cols(&self) -> usize {
        self.runs
            .iter()
            .map(|r| unicode_width::UnicodeWidthStr::width(r.text.as_str()))
            .sum::<usize>()
            * self.scale as usize
    }

    /// Стирает все строки блока через ECH: kitty удаляет любой крупный символ, задетый
    /// стёртой областью. Пробелы так не работают: запись в нижнюю строку крупного символа
    /// kitty не стирает, а перескакивает его и пишет правее.
    pub fn erase(&self, out: &mut Vec<u8>, screen_w: u16) {
        let n = self.cols().min(screen_w.saturating_sub(self.x) as usize);
        if n == 0 {
            return;
        }
        for k in 0..self.scale as u16 {
            let _ = write!(out, "\x1b[0m\x1b[{};{}H\x1b[{n}X", self.y + k + 1, self.x + 1);
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        let _ = write!(out, "\x1b[{};{}H", self.y + 1, self.x + 1);
        if let Some(url) = &self.link {
            let _ = write!(out, "\x1b]8;;{url}\x1b\\");
        }
        for run in &self.runs {
            sgr(out, run.style, run.curly);
            if self.scale > 1 {
                // OSC 66 ограничен 4096 байтами — режем с запасом по символам.
                let chars: Vec<char> = run.text.chars().collect();
                for chunk in chars.chunks(512) {
                    let s: String = chunk.iter().collect();
                    let _ = write!(out, "\x1b]66;s={};{s}\x07", self.scale);
                }
            } else {
                let _ = out.write_all(run.text.as_bytes());
            }
        }
        let _ = out.write_all(b"\x1b[0m");
        if self.link.is_some() {
            let _ = out.write_all(b"\x1b]8;;\x1b\\");
        }
    }
}

/// Форма курсора мыши (OSC 22): `pointer` над кликабельным, иначе по умолчанию.
pub fn pointer_shape(out: &mut Vec<u8>, pointer: bool) {
    let _ = write!(out, "\x1b]22;{}\x1b\\", if pointer { "pointer" } else { "" });
}

/// Скопировать в системный буфер обмена (OSC 52).
pub fn copy_to_clipboard(out: &mut Vec<u8>, text: &str) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text);
    let _ = write!(out, "\x1b]52;c;{b64}\x1b\\");
}

/// Уведомление «сейчас играет» с обложкой и кнопками; показывается, только когда окно не в фокусе.
pub fn notify(out: &mut Vec<u8>, id: &str, title: &str, body: &str, icon: Option<&[u8]>, buttons: &[&str]) {
    let b64 = |s: &[u8]| base64::engine::general_purpose::STANDARD.encode(s);
    let head = format!("i={id}:o=unfocused:a=report,focus:f={}", b64(b"YaMusicRS"));
    if let Some(icon) = icon {
        // Данные иконки — кусками до 2048 байт до кодирования, все с d=0.
        for chunk in icon.chunks(2048) {
            let _ = write!(out, "\x1b]99;{head}:d=0:p=icon:e=1;{}\x1b\\", b64(chunk));
        }
    }
    let _ = write!(out, "\x1b]99;{head}:d=0:p=title:e=1;{}\x1b\\", b64(title.as_bytes()));
    let _ = write!(out, "\x1b]99;{head}:d=0:p=body:e=1;{}\x1b\\", b64(body.as_bytes()));
    if !buttons.is_empty() {
        let joined = buttons.join("\u{2028}");
        let _ = write!(out, "\x1b]99;{head}:d=0:p=buttons:e=1;{}\x1b\\", b64(joined.as_bytes()));
    }
    let _ = write!(out, "\x1b]99;{head}:d=1;\x1b\\");
}

/// Ответ терминала на уведомление: id и номер кнопки (None — клик по телу).
#[derive(Debug, PartialEq)]
pub struct NotifyReply {
    pub id: String,
    pub button: Option<u32>,
}

/// crossterm не знает OSC-ответов: `ESC ]` он отдаёт как Alt+`]`, остальное — как
/// обычные клавиши до `ESC \` (Alt+`\`). Фильтр копит такие «клавиши» и разбирает ответ.
#[derive(Default)]
pub struct OscFilter {
    buf: Option<String>,
}

pub enum Filtered {
    /// Обычная клавиша — дальше в приложение.
    Pass,
    /// Проглочено (часть OSC-ответа).
    Swallowed,
    Notify(NotifyReply),
}

impl OscFilter {
    pub fn feed(&mut self, k: &crossterm::event::KeyEvent) -> Filtered {
        use crossterm::event::{KeyCode, KeyModifiers};
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match (&mut self.buf, k.code) {
            (None, KeyCode::Char(']')) if alt => {
                self.buf = Some(String::new());
                Filtered::Swallowed
            }
            (Some(_), KeyCode::Char('\\')) if alt => {
                let s = self.buf.take().unwrap_or_default();
                parse_notify(&s).map_or(Filtered::Swallowed, Filtered::Notify)
            }
            (Some(b), KeyCode::Char(c)) => {
                b.push(c);
                // Защита от зависания в режиме глотания.
                if b.len() > 512 {
                    self.buf = None;
                }
                Filtered::Swallowed
            }
            (Some(_), _) => {
                self.buf = None;
                Filtered::Pass
            }
            (None, _) => Filtered::Pass,
        }
    }
}

/// `99;i=<id>;<кнопка>` или `99;i=<id>;` (клик по уведомлению).
fn parse_notify(s: &str) -> Option<NotifyReply> {
    let rest = s.strip_prefix("99;")?;
    let (meta, payload) = rest.split_once(';')?;
    let id = meta.split(':').find_map(|kv| kv.strip_prefix("i="))?.to_string();
    if meta.contains("p=") {
        return None; // close/alive и прочие служебные ответы
    }
    Some(NotifyReply {
        id,
        button: payload.trim().parse().ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn keys(s: &str) -> Vec<KeyEvent> {
        let mut v = vec![KeyEvent::new(KeyCode::Char(']'), KeyModifiers::ALT)];
        v.extend(s.chars().map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        v.push(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::ALT));
        v
    }

    #[test]
    fn swallows_notification_reply() {
        let mut f = OscFilter::default();
        let mut got = None;
        for k in keys("99;i=yam-5;2") {
            if let Filtered::Notify(r) = f.feed(&k) {
                got = Some(r);
            }
        }
        assert_eq!(
            got,
            Some(NotifyReply {
                id: "yam-5".into(),
                button: Some(2)
            })
        );
        // Обычная клавиша после ответа проходит.
        assert!(matches!(
            f.feed(&KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
            Filtered::Pass
        ));
    }
}

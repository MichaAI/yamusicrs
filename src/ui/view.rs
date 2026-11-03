//! Отрисовка всех экранов.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::term::{Overlay, Run};
use super::theme::{Palette, Rgb, shimmer};
use crate::api::Track;
use crate::app::{
    App, ArtistRow, Loadable, LoginState, QueueKind, Screen, SearchItem, Tab, View, artist_rows, quality_label,
};
use crate::fetch::Origin;
use crate::lyrics::{self, Lyrics};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub struct Ctx {
    pub p: Palette,
    /// Время с запуска, сек — фаза всех анимаций.
    pub t: f32,
    /// Время с прошлого кадра, сек.
    pub dt: f32,
    pub cell_aspect: f32,
    /// Доступен крупный текст kitty (OSC 66).
    pub big_text: bool,
}

impl Ctx {
    fn fg(&self) -> Style {
        Style::new().fg(self.p.fg.color())
    }
    fn dim(&self) -> Style {
        Style::new().fg(self.p.dim.color())
    }
    fn accent(&self) -> Style {
        Style::new().fg(self.p.accent.color())
    }
    fn spinner(&self) -> &'static str {
        SPINNER[(self.t * 12.0) as usize % SPINNER.len()]
    }
}

pub fn draw(f: &mut Frame, app: &mut App, ctx: &Ctx) {
    let area = f.area();
    app.overlays.clear();
    app.occluders.clear();
    // Блоб живёт только на экране волны; под справкой и полноэкранным текстом его не видно.
    let on_wave = matches!(app.page().view, View::Wave) && !app.lyrics_mode && app.wave.items.ready().is_some();
    let blob_visible =
        matches!(app.screen, Screen::Main) && !app.show_help && (app.stage_mode && app.now.is_some() || on_wave);
    if !blob_visible {
        app.images.hide_blob();
    }
    match &app.screen {
        Screen::Connecting(err) => connecting(f, area, err.as_deref(), ctx),
        Screen::Login(l) => login(f, area, l, ctx),
        Screen::Main => main_screen(f, app, ctx),
    }
    // OSC-текст пишется поверх готового кадра, поэтому сам перекрыл бы справку и всплывашки.
    let occ = std::mem::take(&mut app.occluders);
    // Крупный символ, не влезший в экран, kitty переносит на следующую строку — и он остаётся мусором.
    for o in &mut app.overlays {
        let mut room = area.right().saturating_sub(o.x) as usize;
        if o.y + o.scale as u16 > area.bottom() {
            room = 0;
        }
        o.runs.retain_mut(|r| {
            let cut: String = r
                .text
                .chars()
                .scan(0usize, |used, ch| {
                    *used += ch.width().unwrap_or(0) * o.scale as usize;
                    (*used <= room).then_some(ch)
                })
                .collect();
            room = room.saturating_sub(cut.width() * o.scale as usize);
            r.text = cut;
            !r.text.is_empty()
        });
    }
    app.overlays.retain(|o| !o.runs.is_empty());
    app.overlays.retain(|o| {
        let w: usize = o.runs.iter().map(|r| r.text.width()).sum();
        let r = Rect {
            x: o.x,
            y: o.y,
            width: (w * o.scale as usize) as u16,
            height: o.scale as u16,
        };
        !occ.iter().any(|c| c.intersects(r))
    });
}

// ── утилиты текста ───────────────────────────────────────────────────────────

/// Обрезает по ширине терминала с «…» и добивает пробелами до `w`.
fn fit(s: &str, w: usize) -> String {
    let sw = s.width();
    if sw <= w {
        return format!("{s}{}", " ".repeat(w - sw));
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut cur = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if cur + cw + 1 > w {
            break;
        }
        out.push(c);
        cur += cw;
    }
    out.push('…');
    cur += 1;
    out.push_str(&" ".repeat(w.saturating_sub(cur)));
    out
}

/// Колонка таблицы: текст + минимум один пробел до следующей колонки.
fn col(s: &str, w: usize) -> String {
    if w < 2 {
        return fit(s, w);
    }
    format!("{} ", fit(s, w - 1))
}

/// Бегущая строка: если не влезает — плавно прокручивается с паузами по краям.
fn marquee(s: &str, w: usize, t: f32) -> String {
    let chars: Vec<char> = s.chars().collect();
    let sw = s.width();
    if sw <= w {
        return fit(s, w);
    }
    let overflow = chars.len() - w.min(chars.len());
    let pause = 2.0;
    let speed = 4.0;
    let cycle = pause * 2.0 + overflow as f32 / speed * 2.0;
    let ph = t % cycle;
    let off = if ph < pause {
        0.0
    } else if ph < pause + overflow as f32 / speed {
        (ph - pause) * speed
    } else if ph < pause * 2.0 + overflow as f32 / speed {
        overflow as f32
    } else {
        overflow as f32 - (ph - pause * 2.0 - overflow as f32 / speed) * speed
    };
    let start = (off as usize).min(overflow);
    let window: String = chars[start..].iter().collect();
    fit(&window, w)
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Текст с горизонтальным градиентом и бегущим бликом.
fn gradient_line<'a>(s: &str, a: Rgb, b: Rgb, t: f32, bold: bool) -> Vec<Span<'a>> {
    let n = s.chars().count().max(1);
    s.chars()
        .enumerate()
        .map(|(i, c)| {
            let col = shimmer(a, b, i as f32 / n as f32, t);
            let mut st = Style::new().fg(col.color());
            if bold {
                st = st.add_modifier(Modifier::BOLD);
            }
            Span::styled(c.to_string(), st)
        })
        .collect()
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn card<'a>(ctx: &Ctx, title: impl Into<Line<'a>>) -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ctx.p.dim.lerp(ctx.p.accent, 0.25).scale(0.8).color()))
        .title(title)
}

// ── экраны входа ─────────────────────────────────────────────────────────────

fn connecting(f: &mut Frame, area: Rect, err: Option<&str>, ctx: &Ctx) {
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{} ", ctx.spinner()), ctx.accent()),
        Span::styled("Подключаюсь к Яндекс Музыке…", ctx.fg()),
    ])];
    if let Some(e) = err {
        lines.push(Line::raw(""));
        lines.push(Line::styled(e.to_string(), ctx.dim()));
        lines.push(Line::styled("l — войти заново · q — выход", ctx.dim()));
    }
    let r = centered(area, area.width.min(90), lines.len() as u16);
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        r,
    );
}

fn qr_lines(url: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::new(url.as_bytes()) else {
        return Vec::new();
    };
    let w = code.width();
    let colors = code.to_colors();
    let dark = |x: isize, y: isize| -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < w
            && colors[y as usize * w + x as usize] == qrcode::Color::Dark
    };
    // Тихая зона 1 модуль, две строки модулей на строку терминала.
    let mut out = Vec::new();
    let mut y = -1isize;
    while y < w as isize + 1 {
        let mut s = String::new();
        for x in -1..=w as isize {
            s.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => ' ',
                (true, false) => '▄',
                (false, true) => '▀',
                (false, false) => '█',
            });
        }
        out.push(s);
        y += 2;
    }
    out
}

fn login(f: &mut Frame, area: Rect, l: &LoginState, ctx: &Ctx) {
    let qr = l
        .code
        .as_ref()
        .map(|c| qr_lines(&c.verification_url))
        .unwrap_or_default();
    let qr_w = qr.first().map_or(0, |s| s.chars().count()) as u16;
    let w = (60 + qr_w + 4).max(100).min(area.width);
    let h = (qr.len() as u16 + 4).max(18).min(area.height);
    let r = centered(area, w, h);
    f.render_widget(Clear, r);
    let title = Line::from(gradient_line(
        " yamusicrs · вход ",
        ctx.p.accent,
        ctx.p.accent2,
        ctx.t,
        true,
    ));
    let block = card(ctx, title);
    let inner = block.inner(r);
    f.render_widget(block, r);

    let [top, bottom] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(inner);
    let [left, right] = Layout::horizontal([Constraint::Min(40), Constraint::Length(qr_w + 2)]).areas(top);
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                " Токен вручную (после входа скопируй access_token из адресной строки):",
                ctx.dim(),
            ),
            Line::styled(
                format!(" {}", crate::api::auth::manual_token_url()),
                ctx.dim().add_modifier(Modifier::UNDERLINED),
            ),
        ]),
        bottom,
    );
    let mut lines = vec![Line::raw("")];
    match &l.code {
        Some(c) => {
            lines.push(Line::from(vec![
                Span::styled(" 1. ", ctx.accent()),
                Span::styled("Открой ", ctx.fg()),
                Span::styled(
                    c.verification_url.clone(),
                    ctx.accent().add_modifier(Modifier::UNDERLINED),
                ),
            ]));
            lines.push(Line::styled("    (или отсканируй QR) и введи код:", ctx.fg()));
            lines.push(Line::raw(""));
            let spaced: String = c.user_code.chars().flat_map(|ch| [ch, ' ']).collect();
            let mut code = vec![Span::raw("      ")];
            code.extend(gradient_line(
                spaced.trim_end(),
                ctx.p.accent,
                ctx.p.accent2,
                ctx.t,
                true,
            ));
            lines.push(Line::from(code));
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled(format!("    {} ", ctx.spinner()), ctx.accent()),
                Span::styled(l.status.clone(), ctx.dim()),
            ]));
        }
        None => lines.push(Line::from(vec![
            Span::styled(format!(" {} ", ctx.spinner()), ctx.accent()),
            Span::styled(l.status.clone(), ctx.dim()),
        ])),
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" 2. ", ctx.accent()),
        Span::styled("Или вставь OAuth-токен / ссылку с access_token:", ctx.fg()),
    ]));
    let masked = if l.input.len() > 8 {
        format!("{}{}", &l.input[..6], "•".repeat((l.input.len() - 6).min(30)))
    } else {
        l.input.clone()
    };
    let cursor = if ((ctx.t * 2.0) as u32).is_multiple_of(2) {
        "▏"
    } else {
        " "
    };
    lines.push(Line::from(vec![
        Span::styled("    › ", ctx.accent()),
        Span::styled(masked, ctx.fg()),
        Span::styled(cursor, ctx.accent()),
    ]));
    if let Some(e) = &l.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            format!(" ✗ {e}"),
            Style::new().fg(Color::Rgb(255, 110, 100)),
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " Enter — войти по токену · r — новый код · Esc — выход",
        ctx.dim(),
    ));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), left);

    if !qr.is_empty() {
        let qr_area = Rect {
            y: right.y + 1,
            height: right.height.saturating_sub(1),
            ..right
        };
        let ql: Vec<Line> = qr
            .into_iter()
            .map(|s| Line::styled(s, Style::new().fg(Color::White).bg(Color::Black)))
            .collect();
        f.render_widget(Paragraph::new(ql), qr_area);
    }
}

// ── главный экран ────────────────────────────────────────────────────────────

fn main_screen(f: &mut Frame, app: &mut App, ctx: &Ctx) {
    let area = f.area();
    let [header, body, player, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(4),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, header, app, ctx);
    let show_side = body.width >= 90;
    let (main, side) = if show_side {
        let side_w = (body.width as f32 * 0.36).clamp(34.0, 64.0) as u16;
        let [m, s] = Layout::horizontal([Constraint::Min(40), Constraint::Length(side_w)]).areas(body);
        (m, Some(s))
    } else {
        (body, None)
    };
    app.areas.header = header;
    app.areas.main = main;
    app.areas.list = Rect::default();
    app.areas.lyrics = Rect::default();

    app.areas.cover = Rect::default();
    app.controls.clear();
    if app.stage_mode && app.now.is_some() {
        draw_stage(f, body, app, ctx);
        draw_player(f, player, app, ctx);
        draw_footer(f, footer, app, ctx);
        apply_scramble(f, app, area, body);
        draw_toasts(f, area, app, ctx);
        draw_popup(f, area, app, ctx);
        return;
    }
    if app.lyrics_mode {
        let b = card(ctx, Line::styled(" Текст ", ctx.accent()));
        let inner = b.inner(main);
        f.render_widget(b, main);
        lyrics_panel(f, inner, app, ctx, true);
    } else {
        draw_page(f, main, app, ctx);
    }
    if let Some(side) = side {
        draw_side(f, side, app, ctx);
    } else {
        app.areas.info = player;
    }
    draw_player(f, player, app, ctx);
    draw_footer(f, footer, app, ctx);
    apply_scramble(f, app, area, main);
    draw_toasts(f, area, app, ctx);
    draw_popup(f, area, app, ctx);
    if app.show_help {
        app.occluders.push(area);
        draw_help(f, area, ctx);
    }
}

/// Всплывающее окно: метаданные трека или выбор исполнителя.
fn draw_popup(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    app.popup_hits.clear();
    let Some(popup) = &app.popup else {
        app.popup_area = Rect::default();
        return;
    };
    let label = |s: &str| Span::styled(format!("{s:>14}  "), ctx.dim());
    let mut lines: Vec<Line> = vec![Line::raw("")];
    // Строки, по которым можно кликнуть: (номер строки в `lines`, действие).
    let mut clickable: Vec<(usize, usize)> = Vec::new();
    let (title, width) = match popup {
        crate::app::Popup::Artists { list, sel } => {
            for (i, a) in list.iter().enumerate() {
                clickable.push((lines.len(), i));
                let on = i == *sel;
                lines.push(Line::from(vec![
                    Span::styled(if on { "  ▶ " } else { "    " }, ctx.accent()),
                    Span::styled(
                        a.name.clone(),
                        if on { ctx.accent().add_modifier(Modifier::BOLD) } else { ctx.fg() },
                    ),
                ]));
            }
            lines.push(Line::raw(""));
            lines.push(Line::styled("  Enter — открыть · Esc — закрыть", ctx.dim()));
            let w = list.iter().map(|a| a.name.width()).max().unwrap_or(0).max(32) as u16 + 8;
            (" Исполнители ", w)
        }
        crate::app::Popup::TrackInfo { track, file } => {
            let t = track;
            let row = |lines: &mut Vec<Line<'static>>, k: &str, v: String, hl: bool| {
                lines.push(Line::from(vec![
                    label(k),
                    Span::styled(v, if hl { ctx.accent() } else { ctx.fg() }),
                ]));
            };
            row(&mut lines, "Название", t.full_title(), false);
            clickable.push((lines.len(), 0));
            row(&mut lines, "Исполнители", t.artist_names(), true);
            if let Some(a) = t.album() {
                clickable.push((lines.len(), 1));
                let kind = match a.kind.as_deref() {
                    Some("single") => " · сингл",
                    Some("compilation") => " · сборник",
                    _ => "",
                };
                row(&mut lines, "Альбом", format!("{}{kind}", a.full_title()), true);
                if let Some(d) = a.release_date.as_deref().map(|d| d.get(..10).unwrap_or(d).to_string()) {
                    row(&mut lines, "Релиз", d, false);
                } else if let Some(y) = a.year {
                    row(&mut lines, "Год", y.to_string(), false);
                }
                if let Some(p) = a.track_position.filter(|p| p.index > 0) {
                    let of = a.track_count.map(|n| format!(" из {n}")).unwrap_or_default();
                    let disc = if p.volume > 1 { format!(", диск {}", p.volume) } else { String::new() };
                    row(&mut lines, "Трек", format!("{}{of}{disc}", p.index), false);
                }
                if let Some(g) = &a.genre {
                    row(&mut lines, "Жанр", g.clone(), false);
                }
            }
            row(&mut lines, "Длительность", fmt_time(t.duration_secs()), false);
            if t.explicit() {
                row(&mut lines, "Содержание", "🅴 explicit".into(), false);
            }
            if let Some(n) = app.now.as_ref().filter(|n| n.track.id == t.id) {
                let bits = n.info.bits.map(|b| format!(" · {b} бит")).unwrap_or_default();
                row(
                    &mut lines,
                    "Поток",
                    format!("{} · {:.1} кГц{bits}", n.info.codec, n.info.rate as f32 / 1000.0),
                    false,
                );
            }
            if let Some(r) = &t.r128 {
                row(&mut lines, "Громкость", format!("{:.1} LUFS · пик {:.1} dBTP", r.i, r.tp), false);
            }
            let lyr = if t.has_sync_lyrics() {
                "синхронный"
            } else if t.has_text_lyrics() {
                "обычный"
            } else {
                "нет"
            };
            row(&mut lines, "Текст", lyr.into(), false);
            if let Some((p, e)) = file {
                let mb = e.size as f64 / 1024.0 / 1024.0;
                row(&mut lines, "Файл", format!("{} · {mb:.1} МБ", e.codec), false);
                row(&mut lines, "", p.display().to_string(), false);
            } else {
                row(&mut lines, "Файл", "ещё не сохранён".into(), false);
            }
            row(&mut lines, "ID", t.full_id(), false);
            clickable.push((lines.len(), 2));
            row(&mut lines, "Ссылка", App::track_url(t), true);
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "  i — исполнитель · o — альбом · y — копировать ссылку · Esc — закрыть",
                ctx.dim(),
            ));
            let w = lines.iter().map(Line::width).max().unwrap_or(40) as u16 + 4;
            (" О треке ", w)
        }
    };
    let h = lines.len() as u16 + 3;
    let r = centered(area, width.min(area.width.saturating_sub(4)), h.min(area.height));
    app.popup_area = r;
    app.occluders.push(r);
    f.render_widget(Clear, r);
    let b = card(ctx, Line::from(gradient_line(title, ctx.p.accent, ctx.p.accent2, ctx.t, true)));
    let inner = b.inner(r);
    f.render_widget(b, r);
    for (li, act) in clickable {
        let y = inner.y + li as u16;
        if y < inner.bottom() {
            app.popup_hits.push((Rect { y, height: 1, ..inner }, act));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_header(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(gradient_line("♫ yamusicrs", ctx.p.accent, ctx.p.accent2, ctx.t, true));
    spans.push(Span::raw("  "));
    for (i, t) in Tab::ALL.iter().enumerate() {
        let active = *t == app.tab;
        let label = format!(" {} {} ", i + 1, t.title());
        let x0 = area.x + spans.iter().map(|s| s.content.width()).sum::<usize>() as u16;
        app.areas.tabs[i] = (x0, x0 + label.width() as u16);
        if active {
            spans.push(Span::styled(
                label,
                Style::new()
                    .fg(Color::Black)
                    .bg(ctx.p.accent.color())
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label, ctx.dim()));
        }
        spans.push(Span::raw(" "));
    }
    let left_w: usize = spans.iter().map(|s| s.content.width()).sum();

    let mut right = Vec::new();
    if let Some(acc) = &app.account {
        let name = acc
            .account
            .display_name
            .clone()
            .or(acc.account.login.clone())
            .unwrap_or_default();
        right.push(Span::styled(name, ctx.fg()));
        if acc.plus.as_ref().is_some_and(|p| p.has_plus) {
            right.push(Span::raw(" "));
            right.extend(gradient_line(
                "Плюс",
                Rgb::hex(0xFF5C4D),
                Rgb::hex(0xEB469F),
                ctx.t * 0.7,
                true,
            ));
        }
    }
    right.push(Span::raw(" "));
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let gap = (area.width as usize).saturating_sub(left_w + right_w);
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx) {
    let hints: &[(&str, &str)] = if app.search_editing {
        &[("Enter", "искать"), ("Esc", "выйти из поля"), ("^U", "очистить")]
    } else if app.stage_mode && app.now.is_some() {
        &[
            ("Space", "пауза"),
            ("n/p", "трек"),
            ("←→", "перемотка"),
            ("↑↓", "громкость"),
            ("l", "♥"),
            ("v/Esc", "выйти"),
            ("?", "все клавиши"),
        ]
    } else {
        match app.page().view {
            View::Wave if matches!(app.queue.kind, crate::app::QueueKind::Wave { .. }) => &[
                ("n", "дальше"),
                ("d", "не нравится"),
                ("l", "♥"),
                ("Space", "пауза"),
                ("←→", "настройка"),
                ("?", "все клавиши"),
            ],
            View::Releases => &[
                ("←↑↓→", "выбор"),
                ("Enter", "открыть"),
                ("Shift+←→", "перемотка"),
                ("Space", "пауза"),
                ("?", "все клавиши"),
            ],
            View::Search if app.search_shows_history() => &[
                ("Enter", "искать снова"),
                ("Del", "убрать из истории"),
                ("/", "новый поиск"),
                ("?", "все клавиши"),
            ],
            View::Wave => &[
                ("↑↓", "настройка"),
                ("←→", "значение"),
                ("Enter", "запустить"),
                ("?", "все клавиши"),
            ],
            _ => &[
                ("Enter", "играть"),
                ("Space", "пауза"),
                ("n/p", "трек"),
                ("l", "♥"),
                ("a", "в очередь"),
                ("o/i", "альбом/артист"),
                ("t", "текст"),
                ("v", "сцена"),
                ("?", "все клавиши"),
            ],
        }
    };
    let mut spans = vec![Span::raw(" ")];
    for (k, d) in hints {
        spans.push(Span::styled(*k, ctx.accent()));
        spans.push(Span::styled(format!(" {d}  "), ctx.dim()));
    }
    let (n, bytes) = app.library.stats();
    let lib = format!(
        "{} · в библиотеке {n} · {:.1} ГБ ",
        app.gfx.backend(),
        bytes as f64 / 1e9
    );
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    if used + lib.width() < area.width as usize {
        spans.push(Span::raw(" ".repeat(area.width as usize - used - lib.width())));
        spans.push(Span::styled(lib, ctx.dim()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── страницы ─────────────────────────────────────────────────────────────────

fn draw_page(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    let title = match &app.page().view {
        View::Wave => "Моя волна".to_string(),
        View::Likes => format!("Мне нравится · {}", app.likes.ready().map_or(0, Vec::len)),
        View::Playlists => "Плейлисты".into(),
        View::Search => "Поиск".into(),
        View::Queue => format!("Очередь · {}", app.queue.title),
        View::Releases => "Новые релизы".into(),
        View::Tracks { title, .. } => title.clone(),
        View::Artist { name, .. } => name.clone(),
    };
    let crumbs = if app.stack.len() > 1 { "‹ " } else { "" };
    let block = card(
        ctx,
        Line::from(vec![
            Span::styled(format!(" {crumbs}"), ctx.dim()),
            Span::styled(format!("{title} "), ctx.accent()),
        ]),
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    match &app.page().view {
        View::Wave => draw_wave(f, inner, app, ctx),
        View::Likes => {
            let tracks = match &app.likes {
                Loadable::Ready(t) => t.clone(),
                other => return status(f, inner, other, ctx, "Загружаю коллекцию"),
            };
            track_list(f, inner, app, &tracks, ctx);
        }
        View::Playlists => draw_playlists(f, inner, app, ctx),
        View::Releases => draw_releases(f, inner, app, ctx),
        View::Search => draw_search(f, inner, app, ctx),
        View::Queue => {
            let tracks = app.queue.items.clone();
            if tracks.is_empty() {
                let p = Paragraph::new("Очередь пуста — включи волну (w) или выбери что-нибудь в коллекции")
                    .style(ctx.dim())
                    .alignment(Alignment::Center);
                f.render_widget(p, centered(inner, inner.width, 1));
            } else {
                track_list(f, inner, app, &tracks, ctx);
            }
        }
        View::Tracks {
            title,
            subtitle,
            cover,
            tracks,
            ..
        } => {
            let (title, subtitle, cover) = (title.clone(), subtitle.clone(), cover.clone());
            let head_h = 7.min(inner.height / 3);
            let [head, list] = Layout::vertical([Constraint::Length(head_h), Constraint::Min(1)]).areas(inner);
            let tracks = match tracks {
                Loadable::Ready(t) => Some(t.clone()),
                other => {
                    status(f, list, other, ctx, "Загружаю треки");
                    None
                }
            };
            draw_collection_head(
                f,
                head,
                app,
                ctx,
                &title,
                &subtitle,
                cover.as_deref(),
                tracks.as_deref(),
            );
            if let Some(t) = tracks {
                track_list(f, list, app, &t, ctx);
            }
        }
        View::Artist { brief, name, .. } => {
            let Loadable::Ready(b) = brief else {
                return status(f, inner, brief, ctx, "Загружаю исполнителя");
            };
            let b = b.clone();
            let name = name.clone();
            let me = b.artist.as_ref().map(|a| a.id.clone()).unwrap_or_default();
            let rows: Vec<RowKind> = artist_rows(&b)
                .into_iter()
                .map(|r| match r {
                    ArtistRow::Header(h) => RowKind::Header(h.to_string()),
                    ArtistRow::Wave(_) => RowKind::Other("≋ Моя волна по исполнителю".into(), String::new()),
                    ArtistRow::Track(t) => RowKind::Track(t.clone()),
                    ArtistRow::Album(a) => {
                        // У чужих релизов (сборники, фиты) показываем, чьи они.
                        let whose = if a.artists.iter().any(|x| x.id == me) {
                            String::new()
                        } else {
                            format!(" — {}", a.artist_names())
                        };
                        let kind = match a.kind.as_deref() {
                            Some("single") => "сингл",
                            Some("compilation") => "сборник",
                            _ => "альбом",
                        };
                        let year = a.year.map(|y| format!("{y} · ")).unwrap_or_default();
                        let n = a.track_count.filter(|n| *n > 1).map(|n| format!(" · {n} тр.")).unwrap_or_default();
                        RowKind::Other(format!("◉ {}{whose}", a.full_title()), format!("{year}{kind}{n}"))
                    }
                    ArtistRow::Artist(a) => RowKind::Other(format!("☺ {}", a.name), String::new()),
                })
                .collect();
            let head_h = 8.min(inner.height / 3);
            let [head, list] = Layout::vertical([Constraint::Length(head_h), Constraint::Min(1)]).areas(inner);
            draw_artist_head(f, head, app, ctx, &name, &b);
            mixed_list(f, list, app, &rows, ctx);
        }
    }
}

fn status<T>(f: &mut Frame, area: Rect, l: &Loadable<T>, ctx: &Ctx, what: &str) {
    let line = match l {
        Loadable::Loading => Line::from(vec![
            Span::styled(format!("{} ", ctx.spinner()), ctx.accent()),
            Span::styled(format!("{what}…"), ctx.dim()),
        ]),
        Loadable::Failed(e) => Line::styled(format!("✗ {e}"), Style::new().fg(Color::Rgb(255, 110, 100))),
        Loadable::Ready(_) => Line::raw(""),
    };
    f.render_widget(
        Paragraph::new(line).alignment(Alignment::Center),
        centered(area, area.width, 1),
    );
}

/// Заглушка вместо обложки: тонированный квадрат с нотой (или спиннером, пока грузится).
fn cover_placeholder(f: &mut Frame, area: Rect, ctx: &Ctx, loading: bool) {
    let r = super::kitty::fit_square(area, ctx.cell_aspect);
    if r.width < 2 || r.height < 1 {
        return;
    }
    let buf = f.buffer_mut();
    for y in 0..r.height {
        for x in 0..r.width {
            // Диагональный градиент акцентов, сильно притушенный.
            let t = (x as f32 / r.width as f32 + y as f32 / r.height as f32) / 2.0;
            let c = ctx.p.surface.lerp(ctx.p.accent.lerp(ctx.p.accent2, t), 0.18);
            if let Some(cell) = buf.cell_mut((r.x + x, r.y + y)) {
                cell.set_symbol(" ").set_bg(c.color());
            }
        }
    }
    let glyph = if loading { ctx.spinner() } else { "♫" };
    f.render_widget(
        Paragraph::new(Line::styled(glyph, ctx.accent())).alignment(Alignment::Center),
        Rect {
            y: r.y + r.height / 2,
            height: 1,
            ..r
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_collection_head(
    f: &mut Frame,
    area: Rect,
    app: &mut App,
    ctx: &Ctx,
    title: &str,
    subtitle: &str,
    cover: Option<&str>,
    tracks: Option<&[Track]>,
) {
    if area.height < 3 {
        return;
    }
    let cover_w = ((area.height as f32) * ctx.cell_aspect) as u16;
    let [c, text] = Layout::horizontal([Constraint::Length(cover_w + 2), Constraint::Min(10)]).areas(area);
    let cover_area = Rect {
        x: c.x + 1,
        width: cover_w,
        ..c
    };
    match cover {
        Some(key) if app.images.contains(key) => app.images.render(key, cover_area, f.buffer_mut(), ctx.cell_aspect),
        _ => cover_placeholder(f, cover_area, ctx, cover.is_some()),
    }
    let total: f64 = tracks.map_or(0.0, |t| t.iter().map(Track::duration_secs).sum());
    let mut lines = vec![Line::raw("")];
    let mut t = vec![Span::raw(" ")];
    t.extend(gradient_line(title, ctx.p.accent, ctx.p.accent2, ctx.t * 0.5, true));
    lines.push(Line::from(t));
    lines.push(Line::styled(format!(" {subtitle}"), ctx.fg()));
    if total > 0.0 {
        lines.push(Line::styled(
            format!(" {} · {}", tracks.map_or(0, <[Track]>::len), fmt_time(total)),
            ctx.dim(),
        ));
    }
    lines.push(Line::styled(
        " Enter — играть с выбранного · s — перемешивание",
        ctx.dim(),
    ));
    f.render_widget(Paragraph::new(lines), text);
}

/// Шапка страницы исполнителя: фото, имя, жанры и страны, слушатели и лайки, описание.
fn draw_artist_head(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx, name: &str, b: &crate::api::ArtistBrief) {
    if area.height < 3 {
        return;
    }
    let info = b.artist.as_ref();
    let photo = info.and_then(|a| a.cover_uri());
    let cover_w = ((area.height as f32) * ctx.cell_aspect) as u16;
    let [c, text] = Layout::horizontal([Constraint::Length(cover_w + 2), Constraint::Min(10)]).areas(area);
    let cover_area = Rect {
        x: c.x + 1,
        width: cover_w,
        ..c
    };
    match photo {
        Some(key) if app.images.contains(key) => app.images.render(key, cover_area, f.buffer_mut(), ctx.cell_aspect),
        _ => cover_placeholder(f, cover_area, ctx, photo.is_some()),
    }
    let mut lines = vec![Line::raw("")];
    let mut t = vec![Span::raw(" ")];
    t.extend(gradient_line(name, ctx.p.accent, ctx.p.accent2, ctx.t * 0.5, true));
    lines.push(Line::from(t));
    if let Some(a) = info {
        let mut tags: Vec<String> = a.genres.iter().map(|g| genre_name(g)).collect();
        tags.extend(a.countries.iter().cloned());
        if !tags.is_empty() {
            lines.push(Line::styled(format!(" {}", tags.join(" · ")), ctx.fg()));
        }
    }
    let mut stats = Vec::new();
    if let Some(n) = b.stats.and_then(|s| s.last_month_listeners) {
        stats.push(format!("{} слушателей за месяц", fmt_count(n)));
    }
    if let Some(n) = info.and_then(|a| a.likes_count) {
        stats.push(format!("♥ {}", fmt_count(n)));
    }
    if let Some(c) = info.and_then(|a| a.counts) {
        stats.push(format!("{} треков · {} релизов", c.tracks, c.direct_albums));
    }
    if !stats.is_empty() {
        lines.push(Line::styled(format!(" {}", stats.join(" · ")), ctx.dim()));
    }
    if let Some(d) = info.and_then(|a| a.description.as_ref()).filter(|d| !d.text.is_empty()) {
        let room = (area.height as usize).saturating_sub(lines.len()).min(3);
        let w = (text.width as usize).saturating_sub(2).max(10);
        let wrapped = textwrap::wrap(&d.text, w);
        let n = wrapped.len();
        for (i, l) in wrapped.into_iter().take(room).enumerate() {
            let cut = i + 1 == room && n > room;
            lines.push(Line::styled(
                format!(" {}{}", l, if cut { "…" } else { "" }),
                ctx.dim().add_modifier(Modifier::ITALIC),
            ));
        }
    }
    f.render_widget(Paragraph::new(lines), text);
}

/// 6555818 → «6,6 млн».
fn fmt_count(n: u64) -> String {
    let f = |v: f64, s: &str| format!("{}{s}", format!("{v:.1}").trim_end_matches(".0").replace('.', ","));
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => f(n as f64 / 1e3, " тыс."),
        _ => f(n as f64 / 1e6, " млн"),
    }
}

/// Коды жанров Яндекса → по-человечески (самые частые; остальное как есть).
fn genre_name(g: &str) -> String {
    match g {
        "pop" => "поп",
        "ruspop" => "русская поп-музыка",
        "rusrap" => "русский рэп",
        "foreignrap" | "rap" => "рэп",
        "rock" => "рок",
        "rusrock" => "русский рок",
        "alternative" => "альтернатива",
        "indie" => "инди",
        "electronics" => "электроника",
        "dance" => "танцевальная",
        "house" => "хаус",
        "techno" => "техно",
        "metal" => "метал",
        "rnb" => "R&B",
        "hiphop" => "хип-хоп",
        "jazz" => "джаз",
        "classical" | "classicalmusic" => "классика",
        "soundtrack" => "саундтрек",
        "folk" => "фолк",
        "phonkgenre" | "phonk" => "фонк",
        "estrada" => "эстрада",
        "shanson" => "шансон",
        "lounge" => "лаунж",
        "ambient" => "эмбиент",
        "dnb" => "драм-н-бейс",
        "dubstep" => "дабстеп",
        "kpop" => "K-pop",
        "anime" => "аниме",
        "videogame" => "музыка из игр",
        other => other,
    }
    .to_string()
}

/// Окно прокрутки так, чтобы курсор был виден с запасом в 2 строки.
fn scroll_window(sel: usize, offset: &mut usize, len: usize, h: usize) -> std::ops::Range<usize> {
    if h == 0 {
        return 0..0;
    }
    let margin = 2.min(h / 4);
    if sel < *offset + margin {
        *offset = sel.saturating_sub(margin);
    }
    if sel + margin >= *offset + h {
        *offset = (sel + margin + 1).saturating_sub(h);
    }
    *offset = (*offset).min(len.saturating_sub(h));
    *offset..(*offset + h).min(len)
}

/// Окно прокрутки с анимацией: целевой сдвиг считается как обычно, а на экран
/// попадает плавно догоняющий его (за пару кадров на строку, с замедлением).
fn animated_window(app: &mut App, sel: usize, len: usize, h: usize) -> std::ops::Range<usize> {
    let p = app.page_mut();
    let mut target = p.offset;
    scroll_window(sel, &mut target, len, h);
    p.offset = target;
    let t = target as f32;
    p.anim += (t - p.anim) * 0.35;
    if (t - p.anim).abs() < 0.5 {
        p.anim = t;
    }
    let shown = (p.anim.round().max(0.0) as usize).min(len.saturating_sub(h.min(len)));
    p.view_offset = shown;
    shown..(shown + h).min(len)
}

/// Строка под мышью считается из позиции мыши и того, что сейчас на экране:
/// после прокрутки колесом мышь стоит, а строки под ней — уже другие.
fn sync_hover(app: &mut App, area: Rect, start: usize, len: usize) {
    let (mx, my) = app.mouse;
    let inside = mx >= area.x && mx < area.x + area.width && my >= area.y && my < area.y + area.height;
    if app.hover.is_some() || inside {
        app.hover = inside.then(|| start + (my - area.y) as usize).filter(|r| *r < len);
    }
}

fn playing_glyph(ctx: &Ctx, paused: bool) -> String {
    if paused {
        return "⏸".into();
    }
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let a = ((ctx.t * 7.0).sin() * 3.5 + 3.5) as usize;
    let b = ((ctx.t * 9.3 + 1.0).sin() * 3.5 + 3.5) as usize;
    format!("{}{}", BARS[a.min(7)], BARS[b.min(7)])
}

fn row_style(ctx: &Ctx, selected: bool) -> Style {
    if selected {
        Style::new().bg(ctx.p.surface.lerp(ctx.p.accent, 0.12).color())
    } else {
        Style::new()
    }
}

/// Ширины колонок «название / артист / альбом» строки трека.
fn track_columns(w: usize) -> (usize, usize, usize) {
    let flex = w.saturating_sub(2 + 1 + 5 + 1 + 6 + 4);
    let (tw, aw) = if flex > 60 {
        (flex * 45 / 100, flex * 30 / 100)
    } else {
        (flex * 60 / 100, flex * 40 / 100)
    };
    (tw, aw, flex.saturating_sub(tw + aw))
}

fn track_row<'a>(app: &App, ctx: &Ctx, t: &Track, n: usize, w: usize, selected: bool) -> Line<'a> {
    let playing = app.now.as_ref().is_some_and(|x| x.track.id == t.id);
    let loading = app.loading_track.as_ref().is_some_and(|x| x.id == t.id) && !playing;
    let marker = if playing {
        playing_glyph(ctx, app.player.paused())
    } else if !t.available {
        "⊘ ".into()
    } else if loading {
        format!("{} ", ctx.spinner())
    } else if selected {
        "▌ ".into()
    } else {
        "  ".into()
    };
    let dur = if t.available && t.duration_ms.is_some_and(|d| d > 0) {
        fmt_time(t.duration_secs())
    } else {
        "—".into()
    };
    let liked = app.is_liked(t);
    let cached = app.library.contains(&t.id);
    let (tw, aw, albw) = track_columns(w);
    let title_style = if playing {
        ctx.accent().add_modifier(Modifier::BOLD)
    } else if !t.available {
        Style::new().fg(ctx.p.dim.scale(0.6).color())
    } else {
        ctx.fg()
    };
    // Недоступный трек — вся строка приглушена.
    let artist_style = if t.available {
        Style::new().fg(ctx.p.fg.lerp(ctx.p.dim, 0.4).color())
    } else {
        title_style
    };
    let album_style = if t.available { ctx.dim() } else { title_style };
    let mut title = t.full_title();
    if t.explicit() {
        title.push_str(" ᴱ");
    }
    let mut spans = vec![
        Span::styled(fit(&marker, 2), ctx.accent()),
        Span::raw(" "),
        Span::styled(fit(&format!("{n:>4}"), 5), ctx.dim()),
        Span::styled(col(&title, tw), title_style),
        Span::styled(col(&t.artist_names(), aw), artist_style),
    ];
    if albw > 0 {
        spans.push(Span::styled(
            col(&t.album().map(|a| a.full_title()).unwrap_or_default(), albw),
            album_style,
        ));
    }
    spans.push(Span::styled(format!("{dur:>6} "), ctx.dim()));
    spans.push(Span::styled(
        if liked { "♥" } else { " " },
        Style::new().fg(ctx.p.accent2.color()),
    ));
    spans.push(Span::styled(
        if cached { "●" } else { " " },
        Style::new().fg(ctx.p.dim.scale(0.8).color()),
    ));
    Line::from(spans).style(row_style(ctx, selected))
}

fn track_list(f: &mut Frame, area: Rect, app: &mut App, tracks: &[Track], ctx: &Ctx) {
    // Строка фильтра снизу, если он активен.
    let (filter, editing) = (app.page().filter.clone(), app.page().filter_editing);
    let area = if editing || !filter.is_empty() {
        let [list, bar] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let shown = app.page().filtered.as_ref().map_or(tracks.len(), Vec::len);
        let cursor = if editing && ((ctx.t * 2.0) as u32).is_multiple_of(2) {
            "▏"
        } else {
            " "
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ⌕ ", ctx.accent()),
                Span::styled(filter, ctx.fg()),
                Span::styled(cursor, ctx.accent()),
                Span::styled(format!("  {shown} из {}", tracks.len()), ctx.dim()),
                Span::styled(
                    if editing {
                        "  · Enter — готово, Esc — сбросить"
                    } else {
                        "  · Esc — сбросить"
                    },
                    ctx.dim(),
                ),
            ]))
            .style(Style::new().bg(ctx.p.surface.color())),
            bar,
        );
        list
    } else {
        area
    };
    app.areas.list = area;
    let idx: Vec<usize> = match &app.page().filtered {
        Some(v) => v.iter().copied().filter(|i| *i < tracks.len()).collect(),
        None => (0..tracks.len()).collect(),
    };
    let h = area.height as usize;
    let sel = app.page().sel.min(idx.len().saturating_sub(1));
    let range = animated_window(app, sel, idx.len(), h);
    sync_hover(app, area, range.start, idx.len());
    let lines: Vec<Line> = range
        .clone()
        .map(|i| track_row(app, ctx, &tracks[idx[i]], idx[i] + 1, area.width as usize, i == sel))
        .collect();
    f.render_widget(Paragraph::new(lines), area);
    let covers: Vec<RowCover> = range
        .clone()
        .map(|i| {
            (
                i,
                tracks[idx[i]].cover_uri().map(str::to_owned),
                tracks[idx[i]].full_title(),
            )
        })
        .collect();
    cover_popup(f, area, app, range.start, sel, &covers, ctx);
    scrollbar(f, area, range.start, h, idx.len(), ctx);
}

/// Строка списка для всплывающей обложки: (видимый индекс, обложка, название).
type RowCover = (usize, Option<String>, String);

/// Всплывающая обложка трека под мышью (или под курсором, если он постоял на месте):
/// сразу после названия, под строкой — чтобы саму строку не закрывать.
fn cover_popup(f: &mut Frame, area: Rect, app: &mut App, start: usize, sel: usize, covers: &[RowCover], ctx: &Ctx) {
    let target = app.hover.and_then(|h| covers.iter().find(|c| c.0 == h)).or_else(|| {
        app.selection_settled()
            .then(|| covers.iter().find(|c| c.0 == sel))
            .flatten()
    });
    let Some((i, Some(uri), title)) = target else { return };
    let key = crate::app::thumb_key(uri);
    if !app.images.contains(&key) {
        app.want_thumb(uri);
        return;
    }
    let h = 8.min(area.height.saturating_sub(2) / 2).max(3);
    let w = ((h as f32) * ctx.cell_aspect).round() as u16;
    let row_y = area.y + (*i - start) as u16;
    // Колонки как в track_row: маркер 2 + пробел + номер 5, дальше название.
    let title_x = area.x + 8;
    let (tw, _, _) = track_columns(area.width as usize);
    let text_w = title.width().min(tw) as u16;
    // Под мышью — рядом с курсором, иначе после названия под строкой.
    let hovered = app.hover == Some(*i);
    let (ax, ay) = if hovered {
        (app.mouse.0 + 3, app.mouse.1)
    } else {
        (title_x + text_w + 2, row_y)
    };
    let x = ax.min(area.x + area.width.saturating_sub(w + 2));
    let y = if ay + 1 + h <= area.y + area.height {
        ay + 1
    } else {
        ay.saturating_sub(h)
    };
    let r = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    app.occluders.push(r);
    app.images.render(&key, r, f.buffer_mut(), ctx.cell_aspect);
}

fn scrollbar(f: &mut Frame, area: Rect, offset: usize, h: usize, len: usize, ctx: &Ctx) {
    if len <= h || h < 3 {
        return;
    }
    let x = area.x + area.width - 1;
    let thumb = ((h * h) as f32 / len as f32).max(1.0);
    let pos = offset as f32 / (len - h) as f32 * (h as f32 - thumb);
    let buf = f.buffer_mut();
    for i in 0..h {
        let on = (i as f32) >= pos.floor() && (i as f32) < pos + thumb;
        if let Some(c) = buf.cell_mut((x, area.y + i as u16)) {
            c.set_symbol(if on { "┃" } else { "│" });
            c.set_fg(if on {
                ctx.p.accent.color()
            } else {
                ctx.p.dim.scale(0.4).color()
            });
        }
    }
}

enum RowKind {
    Header(String),
    Track(Track),
    Other(String, String),
}

fn mixed_list(f: &mut Frame, area: Rect, app: &mut App, rows: &[RowKind], ctx: &Ctx) {
    app.areas.list = area;
    let h = area.height as usize;
    let sel = app.page().sel.min(rows.len().saturating_sub(1));
    let range = animated_window(app, sel, rows.len(), h);
    sync_hover(app, area, range.start, rows.len());
    let w = area.width as usize;
    let mut n = 0;
    let mut lines = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if matches!(r, RowKind::Track(_)) {
            n += 1;
        }
        if !range.contains(&i) {
            continue;
        }
        let selected = i == sel;
        lines.push(match r {
            RowKind::Header(s) => {
                let mut sp = vec![Span::raw(" ")];
                sp.extend(gradient_line(s, ctx.p.accent, ctx.p.accent2, ctx.t * 0.3, true));
                Line::from(sp)
            }
            RowKind::Track(t) => track_row(app, ctx, t, n, w, selected),
            RowKind::Other(a, b) => Line::from(vec![
                Span::styled(if selected { "▌ " } else { "  " }, ctx.accent()),
                Span::styled(fit(a, w.saturating_sub(12)), ctx.fg()),
                Span::styled(format!("{b:>8}"), ctx.dim()),
            ])
            .style(row_style(ctx, selected)),
        });
    }
    f.render_widget(Paragraph::new(lines), area);
    let covers: Vec<RowCover> = range
        .clone()
        .filter_map(|i| match &rows[i] {
            RowKind::Track(t) => Some((i, t.cover_uri().map(str::to_owned), t.full_title())),
            _ => None,
        })
        .collect();
    cover_popup(f, area, app, range.start, sel, &covers, ctx);
    scrollbar(f, area, range.start, h, rows.len(), ctx);
}

fn draw_playlists(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    let pls = match &app.playlists {
        Loadable::Ready(p) => p.clone(),
        other => return status(f, area, other, ctx, "Загружаю плейлисты"),
    };
    let rows: Vec<RowKind> = pls
        .iter()
        .map(|p| RowKind::Other(format!("▤ {}", p.title), format!("{} тр.", p.track_count.unwrap_or(0))))
        .collect();
    mixed_list(f, area, app, &rows, ctx);
}

/// Сетка «Новые релизы», как на сайте: обложка, под ней название, артисты и тип.
fn draw_releases(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    app.grid_hits.clear();
    let albums = match &app.releases {
        Some(Loadable::Ready(a)) => a.clone(),
        Some(other) => return status(f, area, other, ctx, "Загружаю новые релизы"),
        None => return,
    };
    if albums.is_empty() {
        let p = Paragraph::new("Новых релизов нет")
            .style(ctx.dim())
            .alignment(Alignment::Center);
        return f.render_widget(p, centered(area, area.width, 1));
    }
    // Поля по 1 колонке, между карточками 2; карточка не уже 16 и не шире 30 колонок.
    const GAP: u16 = 2;
    let area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(3),
        ..area
    };
    let cols = ((area.width + GAP) / (22 + GAP))
        .max(1)
        .min(((area.width + GAP) / (16 + GAP)).max(1));
    let cw = ((area.width + GAP) / cols).saturating_sub(GAP).min(30);
    // floor — как в fit_square, иначе под квадратом остаётся пустая строка.
    let cover_h = ((cw as f32 / ctx.cell_aspect).floor() as u16).max(1);
    let card_h = cover_h + 4;
    app.grid_cols = cols as usize;
    let rows_total = albums.len().div_ceil(cols as usize);
    let visible = ((area.height + 1) / card_h).max(1) as usize;
    let sel = app.page().sel.min(albums.len() - 1);
    let range = animated_window(app, sel / cols as usize, rows_total, visible);

    for row in range.clone() {
        let y = area.y + ((row - range.start) as u16) * card_h;
        if y + card_h > area.y + area.height + 1 {
            break;
        }
        for c in 0..cols as usize {
            let i = row * cols as usize + c;
            let Some(a) = albums.get(i) else { break };
            let x = area.x + c as u16 * (cw + GAP);
            let card = Rect {
                x,
                y,
                width: cw,
                height: card_h - 1,
            };
            app.grid_hits.push((card, i));
            let cover = Rect {
                height: cover_h,
                ..card
            };
            match a.cover_uri() {
                Some(uri) => {
                    let key = crate::app::thumb_key(uri);
                    if app.images.contains(&key) {
                        app.images.render(&key, cover, f.buffer_mut(), ctx.cell_aspect);
                    } else {
                        app.want_thumb(uri);
                        cover_placeholder(f, cover, ctx, true);
                    }
                }
                None => cover_placeholder(f, cover, ctx, false),
            }
            let selected = i == sel;
            let text = Rect {
                y: y + cover_h,
                height: 3,
                ..card
            };
            let badge = if a.explicit() { " Ⓔ" } else { "" };
            let tw = (cw as usize).saturating_sub(badge.width());
            let title_style = if selected {
                ctx.accent().add_modifier(Modifier::BOLD)
            } else {
                ctx.fg().add_modifier(Modifier::BOLD)
            };
            let lines = vec![
                Line::from(vec![
                    Span::styled(fit(&a.full_title(), tw), title_style),
                    Span::styled(badge, ctx.dim()),
                ]),
                Line::styled(fit(&a.artist_names(), cw as usize), ctx.dim()),
                Line::styled(a.kind_label(), ctx.dim().add_modifier(Modifier::BOLD)),
            ];
            f.render_widget(Paragraph::new(lines).style(row_style(ctx, selected)), text);
        }
    }
    // Обложки следующего ряда — заранее, чтобы при прокрутке не мигали заглушки.
    let next = range.end * cols as usize;
    for a in albums.iter().skip(next).take(cols as usize) {
        if let Some(uri) = a.cover_uri() {
            app.want_thumb(uri);
        }
    }
    let full = Rect {
        width: area.width + 2,
        ..area
    };
    let ch = card_h as usize;
    scrollbar(f, full, range.start * ch, area.height as usize, rows_total * ch, ctx);
}

fn draw_search(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    let [input, results] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
    let border = if app.search_editing {
        ctx.p.accent
    } else {
        ctx.p.dim.scale(0.7)
    };
    let b = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border.color()));
    let cursor = if app.search_editing && ((ctx.t * 2.0) as u32).is_multiple_of(2) {
        "▏"
    } else {
        " "
    };
    let text = if app.search_input.is_empty() && !app.search_editing {
        Line::styled(" / — искать треки, альбомы, исполнителей", ctx.dim())
    } else {
        Line::from(vec![
            Span::styled(" ⌕ ", ctx.accent()),
            Span::styled(app.search_input.clone(), ctx.fg()),
            Span::styled(cursor, ctx.accent()),
        ])
    };
    f.render_widget(Paragraph::new(text).block(b), input);

    let items = match &app.search {
        None => return,
        Some(Loadable::Ready(items)) => items.clone(),
        Some(other) => return status(f, results, other, ctx, "Ищу"),
    };
    if items.is_empty() {
        f.render_widget(
            Paragraph::new("Ничего не нашлось")
                .style(ctx.dim())
                .alignment(Alignment::Center),
            results,
        );
        return;
    }
    let rows: Vec<RowKind> = items
        .into_iter()
        .map(|i| match i {
            SearchItem::Header(h) => RowKind::Header(h.into()),
            SearchItem::Track(t) => RowKind::Track(t),
            SearchItem::Album(a) => RowKind::Other(
                format!("◉ {} — {}", a.full_title(), a.artist_names()),
                a.year.map(|y| y.to_string()).unwrap_or_default(),
            ),
            SearchItem::Artist(a) => RowKind::Other(format!("☺ {}", a.name), String::new()),
            SearchItem::Playlist(p) => {
                RowKind::Other(format!("▤ {}", p.title), format!("{} тр.", p.track_count.unwrap_or(0)))
            }
            SearchItem::Query(q) => RowKind::Other(format!("⟲ {q}"), String::new()),
        })
        .collect();
    let sel = app.page().sel;
    if app.search_editing {
        // Пока печатаем, курсор списка не подсвечиваем.
        app.page_mut().sel = usize::MAX / 2;
        mixed_list(f, results, app, &rows, ctx);
        app.page_mut().sel = sel;
    } else {
        mixed_list(f, results, app, &rows, ctx);
    }
}

fn draw_wave(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    app.wave.hits.clear();
    let items = match &app.wave.items {
        Loadable::Ready(v) => v.clone(),
        other => {
            app.images.hide_blob();
            return status(f, area, other, ctx, "Загружаю Мою волну");
        }
    };
    let [left, right] = Layout::horizontal([Constraint::Percentage(46), Constraint::Min(20)]).areas(area);
    let sel = app.page().sel.min(items.len().saturating_sub(1));
    draw_wheel(f, left, app, &items, sel, ctx);
    draw_wave_stage(f, right, app, items.get(sel), ctx);
}

/// Дуга пресетов: выбранный по центру и ближе всех к краю, остальные уходят по кривой и гаснут.
fn draw_wheel(f: &mut Frame, area: Rect, app: &mut App, items: &[crate::api::WheelItem], sel: usize, ctx: &Ctx) {
    const SLOT: f32 = 4.0; // строк на пункт: иконка в 3 строки + зазор
    let icon_h = 3u16;
    let icon_w = ((icon_h as f32) * ctx.cell_aspect).round() as u16;
    let mid = area.y as f32 + area.height as f32 / 2.0 - 1.0;
    let radius = (area.width as f32 * 0.55).min(40.0);
    for (i, item) in items.iter().enumerate() {
        let d = i as f32 - app.wave.arc;
        let y = (mid + d * SLOT).round() as i32 - 1;
        if y < area.y as i32 || y + icon_h as i32 > (area.y + area.height) as i32 {
            continue;
        }
        let y = y as u16;
        let theta = (d * 0.33).clamp(-1.4, 1.4);
        let x = area.x + 2 + (radius * (1.0 - theta.cos())) as u16;
        if x + icon_w + 4 >= area.x + area.width {
            continue;
        }
        let icon_rect = Rect {
            x,
            y,
            width: icon_w,
            height: icon_h,
        };
        match item.icon().map(crate::app::agent_key) {
            Some(k) if app.images.contains(&k) => app.images.render(&k, icon_rect, f.buffer_mut(), ctx.cell_aspect),
            other => {
                if other.is_some()
                    && let Some(u) = item.icon()
                {
                    app.want_agent(u);
                }
                let c = item
                    .color()
                    .map_or(ctx.p.accent, |(r, g, b)| Rgb(r as f32, g as f32, b as f32));
                f.render_widget(
                    Paragraph::new(Line::styled("●", Style::new().fg(c.color()))).alignment(Alignment::Center),
                    Rect {
                        y: y + 1,
                        height: 1,
                        ..icon_rect
                    },
                );
            }
        }
        // Яркость по расстоянию от центра дуги.
        let fade = (1.0 - d.abs() / 4.5).clamp(0.15, 1.0);
        let is_sel = i == sel;
        let active = app.is_active_preset(item);
        let base = if item.accent() { Rgb::hex(0xFFDB4D) } else { ctx.p.fg };
        let col = base.lerp(ctx.p.dim.scale(0.4), 1.0 - fade);
        let mut name_style = Style::new().fg(col.color());
        if is_sel {
            name_style = name_style.add_modifier(Modifier::BOLD);
        }
        let tx = x + icon_w + 2;
        let tw = (area.x + area.width).saturating_sub(tx + 1);
        let mut spans = Vec::new();
        if active {
            spans.push(Span::styled(
                format!("{} ", playing_glyph(ctx, app.player.paused())),
                ctx.accent(),
            ));
        }
        spans.push(Span::styled(
            fit(item.name(), tw as usize).trim_end().to_string(),
            name_style,
        ));
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: tx,
                y: y + 1,
                width: tw,
                height: 1,
            },
        );
        if is_sel && let Some(desc) = item.data.wave.as_ref().and_then(|w| w.description.clone()) {
            f.render_widget(
                Paragraph::new(Line::styled(fit(&desc, tw as usize), ctx.dim())),
                Rect {
                    x: tx,
                    y: y + 2,
                    width: tw,
                    height: 1,
                },
            );
        }
        app.wave.hits.push((
            Rect {
                x,
                y,
                width: area.x + area.width - x,
                height: icon_h,
            },
            i,
        ));
    }
}

/// Правая часть: блоб в цвете пресета, название поверх, кнопки управления.
fn draw_wave_stage(f: &mut Frame, area: Rect, app: &mut App, _sel: Option<&crate::api::WheelItem>, ctx: &Ctx) {
    let wave_on = matches!(app.queue.kind, QueueKind::Wave { .. });
    let playing = wave_on && app.now.is_some() && !app.player.paused();
    let [stage, hint] = Layout::vertical([Constraint::Min(6), Constraint::Length(2)]).areas(area);

    // Палитра блоба — только из обложки (до четырёх разных цветов).
    let colors = to_cols(ctx.p.blob);
    // Блоб «дышит» быстрее, когда играет.
    let speed = if playing { 1.0 } else { 0.3 };
    // Скорость меняется плавно — без рывка при паузе/запуске.
    app.blob_speed += (speed - app.blob_speed) * (ctx.dt * 2.0).min(1.0);
    app.wave_phase += ctx.dt * app.blob_speed;
    // Блоб — живой фон всей правой части.
    let blob_rect = stage;
    let colors = draw_aura(f, app, ctx, blob_rect, colors, if playing { 1.0 } else { 0.0 }, None);

    let title = if wave_on {
        app.wave.active_name.clone().unwrap_or_else(|| "Моя волна".into())
    } else {
        "Моя волна".into()
    };
    let ty = stage.y + stage.height / 2;
    // Крупно через OSC 66, если влезает; буквам — цвет по блобу под ними.
    let cols = title.width();
    let sc = fit_scale(ctx, cols, stage.width.saturating_sub(4), 3, 3);
    let big = sc > 1;
    if big {
        let x0 = stage.x + (stage.width - (cols * sc as usize) as u16) / 2;
        let y0 = ty.saturating_sub(sc as u16 / 2);
        let mut x = x0 as f32;
        let runs = title
            .chars()
            .map(|ch| {
                let cw = (ch.width().unwrap_or(1) * sc as usize) as f32;
                let col = blob_contrast(
                    &app.images,
                    blob_rect,
                    x + cw / 2.0,
                    y0 as f32 + sc as f32 / 2.0,
                    colors,
                    app.wave_phase,
                )
                .unwrap_or(Color::White);
                x += cw;
                Run {
                    text: ch.to_string(),
                    style: Style::new().fg(col).add_modifier(Modifier::BOLD),
                    curly: None,
                }
            })
            .collect();
        put_big(f, app, x0, y0, sc, runs, None);
    }
    let spaced: String = title.to_uppercase().chars().flat_map(|c| [c, ' ']).collect();
    let ty = if big { ty + 1 } else { ty };
    if !big {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                spaced.trim_end().to_string(),
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            )))
            .alignment(Alignment::Center),
            Rect {
                y: ty,
                height: 1,
                ..stage
            },
        );
    }
    if let Some(n) = app.now.as_ref().filter(|_| wave_on) {
        f.render_widget(
            Paragraph::new(Line::styled(
                format!("{} — {}", n.track.full_title(), n.track.artist_names()),
                Style::new().fg(Color::White),
            ))
            .alignment(Alignment::Center),
            Rect {
                y: ty + 2,
                height: 1,
                ..stage
            },
        );
    }

    contrast_over_blob(
        f,
        &app.images,
        blob_rect,
        Rect {
            y: ty,
            height: 3,
            ..stage
        },
        colors,
        app.wave_phase,
    );

    // Кнопки ⏮ ▶ ⏭ — кликабельные.
    let hint_line = if app.wave.loading {
        Line::from(vec![
            Span::styled(format!("{} ", ctx.spinner()), ctx.accent()),
            Span::styled("запускаю…", ctx.dim()),
        ])
    } else {
        Line::styled("Enter — включить · n — дальше · d — мимо", ctx.dim())
    };
    f.render_widget(
        Paragraph::new(hint_line).alignment(Alignment::Center),
        Rect {
            y: hint.y + 1,
            height: 1,
            ..hint
        },
    );
}

/// Цвет текста, читаемый на блобе в точке (x, y) в клетках; None — блоба там почти нет.
fn blob_contrast(images: &super::kitty::Images, blob: Rect, x: f32, y: f32, colors: Cols, t: f32) -> Option<Color> {
    // Примерный фон терминала там, где блоб прозрачный.
    const BG: [f32; 3] = [24.0, 22.0, 30.0];
    let u = (x - blob.x as f32) / blob.width.max(1) as f32 * 2.0 - 1.0;
    let v = (y - blob.y as f32) / blob.height.max(1) as f32 * 2.0 - 1.0;
    if !(-1.0..=1.0).contains(&u) || !(-1.0..=1.0).contains(&v) {
        return None;
    }
    // Реальный кадр блоба в kitty, иначе — формула фолбэка.
    let p = images.blob_sample((u + 1.0) / 2.0, (v + 1.0) / 2.0).unwrap_or_else(|| {
        crate::render::blob_pixel(
            u,
            v,
            &crate::render::Blob {
                colors,
                phase: t,
                energy: 0.5,
                dither: 0.0,
            },
        )
    });
    let a = p[3] / 255.0;
    if a < 0.15 {
        return None;
    }
    let px = [0, 1, 2].map(|i| p[i] * a + BG[i] * (1.0 - a));
    // Относительная яркость по WCAG (линейный sRGB): так её и видит глаз.
    let lin = |c: f32| {
        let c = c / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let l = 0.2126 * lin(px[0]) + 0.7152 * lin(px[1]) + 0.0722 * lin(px[2]);
    // Белый или почти чёрный — что контрастнее (граница ≈0.18), без серых середин.
    // Гистерезис: буква меняет цвет, только если фон ушёл за границу заметно.
    let key = (x as u16, y as u16);
    let mut memo = images.contrast_memo.borrow_mut();
    let prev = memo.get(&key).copied();
    let dark = match prev {
        Some(true) => l > 0.14,
        Some(false) => l > 0.23,
        None => l > 0.18,
    };
    memo.insert(key, dark);
    Some(if dark {
        Color::Rgb(16, 14, 20)
    } else {
        Color::Rgb(255, 255, 255)
    })
}

/// Текст поверх блоба: каждой букве — тёмный или белый цвет по яркости того, что под ней.
fn contrast_over_blob(f: &mut Frame, images: &super::kitty::Images, blob: Rect, text: Rect, colors: Cols, t: f32) {
    let buf = f.buffer_mut();
    for y in text.y..text.y + text.height {
        for x in text.x..text.x + text.width {
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            if cell.symbol().trim().is_empty() {
                continue;
            }
            if let Some(c) = blob_contrast(images, blob, x as f32 + 0.5, y as f32 + 0.5, colors, t) {
                cell.set_fg(c);
            }
        }
    }
}

/// Блоб/фон: вектором (GPU или CPU) в kitty, иначе полублоками.
/// `hole` — область обложки поверх ореола: там кадр блоба прозрачный.
fn draw_aura(
    f: &mut Frame,
    app: &mut App,
    ctx: &Ctx,
    rect: Rect,
    target: Cols,
    energy: f32,
    hole: Option<Rect>,
) -> Cols {
    // Цвета перетекают к новым плавно (смена пресета, наведение, новый трек).
    let k = (ctx.dt * 2.5).min(1.0);
    for (c, t) in app.blob_colors.iter_mut().zip(target) {
        for i in 0..3 {
            c[i] += (t[i] - c[i]) * k;
        }
    }
    let colors = app.blob_colors;
    // «Энергия» (размах движения шаров) тоже догоняет цель плавно.
    app.blob_energy += (energy - app.blob_energy) * (ctx.dt * 2.0).min(1.0);
    let energy = app.blob_energy;
    // Пиксели кадра в пропорциях области: ~256 по ширине.
    let pw = 256u32;
    let ph =
        ((pw as f32 * rect.height as f32 * ctx.cell_aspect / rect.width.max(1) as f32).round() as u32).clamp(16, 512);
    let params = crate::render::Blob {
        colors,
        phase: app.wave_phase,
        energy,
        dither: app.cfg.blob_dither as f32,
    };
    // Kitty рисует блоб поверх фона ячеек, так что под справкой он просвечивал бы сквозь неё.
    if app.show_help {
        app.images.hide_blob();
        return colors;
    }
    let (images, gfx) = (&mut app.images, &mut app.gfx);
    // Дыра в пикселях кадра; на 1 пиксель меньше обложки, чтобы по краю не было щели.
    let hole = hole.map(|c| {
        let sx = pw as f32 / rect.width.max(1) as f32;
        let sy = ph as f32 / rect.height.max(1) as f32;
        (
            (c.x.saturating_sub(rect.x)) as f32 * sx + 1.0,
            (c.y.saturating_sub(rect.y)) as f32 * sy + 1.0,
            (c.right().saturating_sub(rect.x)) as f32 * sx - 1.0,
            (c.bottom().saturating_sub(rect.y)) as f32 * sy - 1.0,
        )
    });
    let frame = || {
        let mut px = gfx.blob(pw, ph, &params);
        if let Some(h) = hole {
            crate::ui::kitty::cut_hole(&mut px, pw, ph, h);
        }
        px
    };
    if !images.blob(rect, pw, ph, frame) {
        blob_halfblocks(f, rect, colors, app.wave_phase);
    }
    colors
}

/// Четыре цвета палитры блоба (0..255).
type Cols = [[f32; 3]; 4];

fn to_cols(c: [Rgb; 4]) -> Cols {
    c.map(|k| [k.0, k.1, k.2])
}

/// Фолбэк без kitty: тот же блоб полублоками.
fn blob_halfblocks(f: &mut Frame, r: Rect, colors: Cols, t: f32) {
    let buf = f.buffer_mut();
    for y in 0..r.height {
        for x in 0..r.width {
            let u = x as f32 / r.width.max(1) as f32 * 2.0 - 1.0;
            let px = |row: f32| {
                let v = row / (r.height as f32 * 2.0) * 2.0 - 1.0;
                crate::render::blob_pixel(
                    u,
                    v,
                    &crate::render::Blob {
                        colors,
                        phase: t,
                        energy: 0.5,
                        dither: 0.0,
                    },
                )
            };
            let (top, bot) = (px(y as f32 * 2.0), px(y as f32 * 2.0 + 1.0));
            // Прозрачные места не трогаем — там остаётся фон терминала и текст.
            if top[3] < 12.0 && bot[3] < 12.0 {
                continue;
            }
            let col = |p: [f32; 4]| {
                let a = p[3] / 255.0;
                Color::Rgb((p[0] * a) as u8, (p[1] * a) as u8, (p[2] * a) as u8)
            };
            if let Some(c) = buf.cell_mut((r.x + x, r.y + y)) {
                c.set_symbol("▀").set_fg(col(top)).set_bg(col(bot));
            }
        }
    }
}

// ── сцена ────────────────────────────────────────────────────────────────────

/// Полноэкранный «сейчас играет»: крупная обложка со свечением в её цветах, текст справа.
fn draw_stage(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    let Some(track) = app.now.as_ref().map(|n| n.track.clone()) else {
        return;
    };
    let [left, right] = Layout::horizontal([Constraint::Percentage(52), Constraint::Min(30)]).areas(area);
    let [cover_zone, info, ctrl] =
        Layout::vertical([Constraint::Min(8), Constraint::Length(6), Constraint::Length(4)]).areas(left);

    // Обложка ~70% зоны, ореол-блоб в 1.6 раза больше и под ней.
    let max_h = (cover_zone.height as f32 * 0.72) as u16;
    let max_w = (cover_zone.width as f32 * 0.72) as u16;
    let h = max_h.min((max_w as f32 / ctx.cell_aspect) as u16).max(4);
    let w = ((h as f32) * ctx.cell_aspect).round() as u16;
    let cover = Rect {
        x: cover_zone.x + (cover_zone.width.saturating_sub(w)) / 2,
        y: cover_zone.y + (cover_zone.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let gh = ((h as f32) * 1.6) as u16;
    let gw = ((gh as f32) * ctx.cell_aspect).round() as u16;
    let glow = Rect {
        x: cover.x.saturating_sub((gw.saturating_sub(w)) / 2).max(area.x),
        y: cover.y.saturating_sub((gh.saturating_sub(h)) / 2).max(area.y),
        width: gw.min(area.width),
        height: gh.min(area.height),
    };
    let colors = to_cols(ctx.p.blob);
    let speed = if app.player.paused() { 0.15 } else { 0.6 };
    // Скорость меняется плавно — без рывка при паузе/запуске.
    app.blob_speed += (speed - app.blob_speed) * (ctx.dt * 2.0).min(1.0);
    app.wave_phase += ctx.dt * app.blob_speed;
    // Дыра — только когда обложка уже есть (иначе под заглушкой пусть светит).
    let hole = track
        .cover_uri()
        .filter(|k| app.images.contains(k))
        .map(|_| crate::ui::kitty::fit_square(cover, ctx.cell_aspect));
    draw_aura(f, app, ctx, glow, colors, if app.player.paused() { 0.0 } else { 1.0 }, hole);
    match track.cover_uri() {
        Some(k) if app.images.contains(k) => {
            app.images.render(k, cover, f.buffer_mut(), ctx.cell_aspect);
            app.areas.cover = cover;
        }
        _ => cover_placeholder(f, cover, ctx, true),
    }

    // Подпись под обложкой: название до ×3, артист до ×2 (OSC 66), если влезают.
    // Клик: название — метаданные, исполнитель — к нему (несколько — выбор), альбом — к альбому.
    let iw = info.width.saturating_sub(4);
    let row_rect = |y: u16, cols: usize, scale: u8| {
        let w = ((cols * scale as usize) as u16).min(info.width);
        Rect {
            x: info.x + (info.width - w) / 2,
            y,
            width: w,
            height: scale as u16,
        }
    };
    app.areas.stage_title = Rect::default();
    app.areas.stage_artist = Rect::default();
    app.areas.stage_album = Rect::default();
    let title = track.full_title();
    let artist = track.artist_names();
    let album = track.album().map(|a| match a.year {
        Some(y) => format!("{} · {y}", a.full_title()),
        None => a.full_title(),
    });
    let mut y = info.y;
    let bottom = info.y + info.height;
    let s1 = fit_scale(ctx, title.width(), iw, bottom.saturating_sub(y).saturating_sub(2), 3);
    if s1 > 1 {
        let n = title.chars().count().max(1);
        let runs = title
            .chars()
            .enumerate()
            .map(|(i, ch)| {
                let c = shimmer(ctx.p.fg, ctx.p.accent, i as f32 / n as f32, ctx.t * 0.25);
                Run {
                    text: ch.to_string(),
                    style: Style::new().fg(c.color()).add_modifier(Modifier::BOLD),
                    curly: None,
                }
            })
            .collect();
        let x = info.x + (info.width - (title.width() * s1 as usize) as u16) / 2;
        put_big(f, app, x, y, s1, runs, None);
        app.areas.stage_title = row_rect(y, title.width(), s1);
        y += s1 as u16;
    } else {
        y += 1;
        let mut t = vec![];
        t.extend(gradient_line(
            marquee(&title, iw as usize, ctx.t).trim_end(),
            ctx.p.fg,
            ctx.p.accent,
            ctx.t * 0.25,
            true,
        ));
        f.render_widget(
            Paragraph::new(Line::from(t)).alignment(Alignment::Center),
            Rect { y, height: 1, ..info },
        );
        app.areas.stage_title = row_rect(y, title.width().min(iw as usize), 1);
        y += 1;
    }
    let s2 = fit_scale(ctx, artist.width(), iw, bottom.saturating_sub(y).saturating_sub(1), 2);
    if s2 > 1 {
        let runs = vec![Run {
            text: artist.clone(),
            style: ctx.accent().add_modifier(Modifier::BOLD),
            curly: None,
        }];
        let x = info.x + (info.width - (artist.width() * s2 as usize) as u16) / 2;
        put_big(f, app, x, y, s2, runs, None);
        app.areas.stage_artist = row_rect(y, artist.width(), s2);
        y += s2 as u16;
    } else if y < bottom {
        f.render_widget(
            Paragraph::new(Line::styled(
                marquee(&artist, iw as usize, ctx.t + 1.3).trim_end().to_string(),
                ctx.accent(),
            ))
            .alignment(Alignment::Center),
            Rect { y, height: 1, ..info },
        );
        app.areas.stage_artist = row_rect(y, artist.width().min(iw as usize), 1);
        y += 1;
    }
    if y < bottom {
        let album = album.unwrap_or_default();
        app.areas.stage_album = row_rect(y, album.width(), 1);
        f.render_widget(
            Paragraph::new(Line::styled(album, ctx.dim())).alignment(Alignment::Center),
            Rect { y, height: 1, ..info },
        );
    }

    if ctrl.height >= 3 && ctrl.width >= CONTROLS_W {
        let r = Rect {
            x: ctrl.x + (ctrl.width - CONTROLS_W) / 2,
            y: ctrl.y,
            width: CONTROLS_W,
            height: 3,
        };
        control_box(f, app, ctx, r);
    }

    let lyr = Rect {
        x: right.x + 1,
        width: right.width.saturating_sub(2),
        ..right
    };
    lyrics_panel(f, lyr, app, ctx, true);
}

/// Рамка с кнопками «назад · play/pause · вперёд»; ширина — `CONTROLS_W`.
const CONTROLS_W: u16 = 2 + 2 * 2 + 3 + 3 * 2;

fn control_box(f: &mut Frame, app: &mut App, ctx: &Ctx, r: Rect) {
    let (pad, gap, icon) = (2u16, 3u16, 1u16);
    let b = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ctx.p.dim.lerp(ctx.p.accent, 0.3).scale(0.8).color()));
    f.render_widget(b, r);
    // Nerd Font (Font Awesome, BMP): назад, play/pause, вперёд — escape-кодами, чтобы не терялись.
    let play = if app.player.paused() { "\u{f04b}" } else { "\u{f04c}" };
    let buttons = [
        ("\u{f048}", 0u8, ctx.fg()),
        (play, 1, ctx.accent()),
        ("\u{f051}", 2, ctx.fg()),
    ];
    for (i, (label, action, style)) in buttons.into_iter().enumerate() {
        let x = r.x + 1 + pad + i as u16 * (icon + gap);
        f.render_widget(
            Paragraph::new(Line::styled(label, style.add_modifier(Modifier::BOLD))),
            Rect {
                x,
                y: r.y + 1,
                width: icon,
                height: 1,
            },
        );
        // Кликабельна вся колонка кнопки вместе с половинами промежутков.
        let zx = x.saturating_sub(gap / 2 + 1).max(r.x);
        app.controls.push((
            Rect {
                x: zx,
                y: r.y,
                width: icon + gap,
                height: 3,
            },
            action,
        ));
    }
}

// ── боковая панель ───────────────────────────────────────────────────────────

fn draw_side(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    let block = card(ctx, Line::styled(" Сейчас играет ", ctx.accent()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let track = app
        .now
        .as_ref()
        .map(|n| n.track.clone())
        .or_else(|| app.loading_track.clone());
    let Some(track) = track else {
        let p = Paragraph::new(vec![
            Line::raw(""),
            Line::styled("Тишина.", ctx.fg()),
            Line::styled("w — включить Мою волну", ctx.dim()),
        ])
        .alignment(Alignment::Center);
        f.render_widget(p, inner);
        app.areas.info = inner;
        return;
    };

    let cover_h = ((inner.width as f32 - 2.0) / ctx.cell_aspect).floor() as u16;
    // +1 строка к обложке: по ширине место есть (запас в 2 колонки под пропорции ячейки).
    let cover_h = cover_h.min(inner.height.saturating_sub(7) * 2 / 3).max(4) + 1;
    let [cover, gap, info, lyr] = Layout::vertical([
        Constraint::Length(cover_h),
        Constraint::Length(1),
        Constraint::Length(6),
        Constraint::Min(0),
    ])
    .areas(inner);
    let _ = gap;
    match track.cover_uri() {
        Some(k) if app.images.contains(k) => {
            app.images.render(k, cover, f.buffer_mut(), ctx.cell_aspect);
            app.areas.cover = super::kitty::fit_square(cover, ctx.cell_aspect);
        }
        _ => {
            let r = super::kitty::fit_square(cover, ctx.cell_aspect);
            f.render_widget(
                Paragraph::new(Line::styled(format!("{} ♪", ctx.spinner()), ctx.dim())).alignment(Alignment::Center),
                centered(r, r.width, 1),
            );
        }
    }
    app.areas.info = info;
    // Кнопки управления — справа от названия и артиста, в рамке на три строки.
    // Рамка ровно по содержимому: значки шириной в 1 клетку (BMP), равные отступы.
    let box_w = CONTROLS_W;
    let with_box = info.width >= box_w + 16 && info.height >= 3;
    let text = if with_box {
        Rect {
            width: info.width - box_w - 1,
            ..info
        }
    } else {
        info
    };
    // Название вдвое крупнее (OSC 66), если влезает по ширине.
    let title = track.full_title();
    let big = ctx.big_text && title.width() * 2 + 2 <= text.width as usize;
    draw_info(f, text, app, ctx, &track, big);
    if big {
        let n = title.chars().count().max(1);
        let runs = title
            .chars()
            .enumerate()
            .map(|(i, ch)| {
                let c = shimmer(ctx.p.fg, ctx.p.accent, i as f32 / n as f32, ctx.t * 0.25);
                Run {
                    text: ch.to_string(),
                    style: Style::new().fg(c.color()).add_modifier(Modifier::BOLD),
                    curly: None,
                }
            })
            .collect();
        put_big(f, app, text.x + 1, text.y, 2, runs, Some(App::track_url(&track)));
    }
    if with_box {
        let r = Rect {
            x: info.x + info.width - box_w,
            y: info.y,
            width: box_w,
            height: 3,
        };
        control_box(f, app, ctx, r);
    }
    if app.kitty_text {
        let w = text.width.saturating_sub(1);
        // При крупном названии оно уже со ссылкой и занимает две строки.
        let shift = u16::from(big);
        let links = [
            (!big).then(|| App::track_url(&track)),
            artist_url(&track),
            album_url(&track),
        ];
        for (row, url) in links.into_iter().enumerate() {
            if let Some(o) = url.and_then(|u| link_cells(f, text.x + 1, text.y + row as u16 + shift, w, u)) {
                app.overlays.push(o);
            }
        }
    }
    if lyr.height > 2 && !app.lyrics_mode {
        let sep = Rect { height: 1, ..lyr };
        f.render_widget(
            Paragraph::new(Line::styled(
                "─".repeat(lyr.width as usize),
                Style::new().fg(ctx.p.dim.scale(0.35).color()),
            )),
            sep,
        );
        let rest = Rect {
            y: lyr.y + 1,
            height: lyr.height - 1,
            ..lyr
        };
        lyrics_panel(f, rest, app, ctx, false);
    }
}

fn draw_info(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx, t: &Track, big_title: bool) {
    let w = area.width as usize;
    let mut lines = Vec::new();
    if big_title {
        // Две строки под крупное название — его дорисует оверлей.
        lines.push(Line::raw(""));
    }
    let title = marquee(&t.full_title(), w.saturating_sub(2), ctx.t);
    let mut tl = vec![Span::raw(" ")];
    tl.extend(gradient_line(
        title.trim_end(),
        ctx.p.fg,
        ctx.p.accent,
        ctx.t * 0.25,
        true,
    ));
    lines.push(if big_title { Line::raw("") } else { Line::from(tl) });
    lines.push(Line::styled(
        format!(" {}", marquee(&t.artist_names(), w.saturating_sub(2), ctx.t + 1.3)),
        ctx.accent(),
    ));
    let album = t.album().map(|a| match a.year {
        Some(y) => format!("{} · {y}", a.full_title()),
        None => a.full_title(),
    });
    lines.push(Line::styled(
        format!(" {}", fit(&album.unwrap_or_default(), w.saturating_sub(2))),
        ctx.dim(),
    ));

    let mut badges = vec![Span::raw(" ")];
    if let Some(n) = &app.now
        && n.track.id == t.id
    {
        let i = &n.info;
        let fmt = match (i.bits, i.codec.as_str()) {
            (Some(b), c) => format!(" {c} {b}/{} ", khz(i.rate)),
            (None, c) => format!(" {c} {} ", khz(i.rate)),
        };
        let lossless = i.codec == "FLAC" || i.codec == "ALAC";
        let bg = if lossless { ctx.p.accent } else { ctx.p.dim };
        badges.push(Span::styled(
            fmt,
            Style::new()
                .fg(Color::Black)
                .bg(bg.color())
                .add_modifier(Modifier::BOLD),
        ));
        badges.push(Span::raw(" "));
        match &n.origin {
            Some(Origin::Library) => badges.push(Span::styled("● библиотека", ctx.dim())),
            Some(Origin::Network { buf, .. }) => {
                let p = buf.progress();
                if p < 1.0 {
                    badges.push(Span::styled(format!("⇣ {:.0}%", p * 100.0), ctx.dim()));
                } else {
                    badges.push(Span::styled("⇣ скачан", ctx.dim()));
                }
            }
            None => {}
        }
    } else {
        badges.push(Span::styled(format!("{} загрузка…", ctx.spinner()), ctx.dim()));
    }
    if app.is_liked(t) {
        badges.push(Span::styled("  ♥", Style::new().fg(ctx.p.accent2.color())));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(badges));
    f.render_widget(Paragraph::new(lines), area);
}

fn khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{}kHz", rate / 1000)
    } else {
        format!("{:.1}kHz", rate as f32 / 1000.0)
    }
}

/// Наибольший масштаб крупного текста ≤ `max`, при котором `cols` колонок влезают в `width`
/// и блок помещается в `rows` строк. 1 — крупный текст недоступен или не влезает.
fn fit_scale(ctx: &Ctx, cols: usize, width: u16, rows: u16, max: u8) -> u8 {
    if !ctx.big_text {
        return 1;
    }
    (2..=max)
        .rev()
        .find(|s| cols * *s as usize <= width as usize && *s as u16 <= rows)
        .unwrap_or(1)
}

/// Крупный текст: клетки блока очищаются, сам текст рисует терминал (OSC 66) оверлеем.
fn put_big(f: &mut Frame, app: &mut App, x: u16, y: u16, scale: u8, runs: Vec<Run>, link: Option<String>) {
    let cols: usize = runs.iter().map(|r| r.text.width()).sum();
    let buf = f.buffer_mut();
    for yy in y..y + scale as u16 {
        for xx in x..x + (cols * scale as usize) as u16 {
            if let Some(c) = buf.cell_mut((xx, yy)) {
                c.set_symbol(" ");
            }
        }
    }
    app.overlays.push(Overlay {
        x,
        y,
        scale,
        runs,
        link,
    });
}

/// Ссылка (OSC 8) поверх уже нарисованного текста: клетки перечитываются из кадра,
/// поэтому цвета и стиль остаются ровно такими же. Хвостовые пробелы не входят.
fn link_cells(f: &mut Frame, x: u16, y: u16, width: u16, url: String) -> Option<Overlay> {
    let buf = f.buffer_mut();
    let mut cells: Vec<(String, Style)> = (x..x + width)
        .filter_map(|xx| buf.cell((xx, y)).map(|c| (c.symbol().to_string(), c.style())))
        .collect();
    while cells.last().is_some_and(|c| c.0.trim().is_empty()) {
        cells.pop();
    }
    let lead = cells.iter().take_while(|c| c.0.trim().is_empty()).count();
    if lead == cells.len() {
        return None;
    }
    let runs = cells[lead..]
        .iter()
        .map(|(t, st)| Run {
            text: t.clone(),
            style: *st,
            curly: None,
        })
        .collect();
    Some(Overlay {
        x: x + lead as u16,
        y,
        scale: 1,
        runs,
        link: Some(url),
    })
}

fn artist_url(t: &Track) -> Option<String> {
    t.artists
        .first()
        .filter(|a| !a.id.is_empty())
        .map(|a| format!("https://music.yandex.ru/artist/{}", a.id))
}

fn album_url(t: &Track) -> Option<String> {
    t.album()
        .filter(|a| !a.id.is_empty())
        .map(|a| format!("https://music.yandex.ru/album/{}", a.id))
}

/// Примерный «вес» символа во времени пения: гласные тянутся, согласные коротки,
/// пробелы и знаки — почти мгновенны (запятая/точка дают короткую паузу).
fn sung_weight(c: char) -> f32 {
    const VOWELS: &str = "aeiouyаеёиоуыэюяäöüéèêàáíóúåæø";
    let l = c.to_lowercase().next().unwrap_or(c);
    if VOWELS.contains(l) {
        1.0
    } else if c.is_alphanumeric() {
        0.35
    } else if c.is_whitespace() {
        0.12
    } else if matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | '…' | '—') {
        0.3
    } else {
        0.03
    }
}

/// Веса символов реплики; гласные последнего слова тянутся дольше (обычно его допевают).
fn sung_weights(chars: &[char]) -> Vec<f32> {
    let mut w: Vec<f32> = chars.iter().map(|&c| sung_weight(c)).collect();
    let last = chars.iter().rposition(|c| c.is_alphanumeric());
    if let Some(e) = last {
        let mut s = e;
        while s > 0 && !chars[s - 1].is_whitespace() {
            s -= 1;
        }
        for i in s..=e {
            if w[i] >= 1.0 {
                w[i] = 1.8;
            }
        }
    }
    w
}

/// Темп пения вокруг реплики `at` в секундах на единицу веса. У LRC (и в оф. клиенте) есть только
/// начала строк, а промежуток до следующей включает вдох/проигрыш, поэтому берём нижний квантиль
/// отношений «промежуток / вес» по соседним строкам: спетые встык показывают настоящий темп.
/// Окно локальное — медленный припев и быстрый куплет в одной песне поются в разном темпе.
fn sung_rate(lines: &[lyrics::Line], at: usize) -> f64 {
    let lo = at.saturating_sub(4);
    let hi = (at + 6).min(lines.len());
    let mut r: Vec<f64> = lines[lo..hi]
        .windows(2)
        .filter_map(|w| {
            let chars: Vec<char> = w[0].text.chars().collect();
            let wt: f32 = sung_weights(&chars).iter().sum();
            let gap = w[1].at - w[0].at;
            (wt >= 2.0 && gap > 0.2).then(|| gap / wt as f64)
        })
        .collect();
    if r.is_empty() {
        return 0.2;
    }
    r.sort_by(f64::total_cmp);
    r[r.len() * 3 / 10].clamp(0.08, 0.6)
}

/// Доля времени `frac` → позиция заливки в символах (дробная) по весам.
fn karaoke_fill(weights: &[f32], frac: f32) -> f32 {
    let total: f32 = weights.iter().sum();
    if total <= 0.0 {
        return frac * weights.len() as f32;
    }
    let mut left = frac.clamp(0.0, 1.0) * total;
    for (i, &w) in weights.iter().enumerate() {
        if left < w {
            return i as f32 + left / w.max(1e-6);
        }
        left -= w;
    }
    weights.len() as f32
}

/// Слово вокруг позиции `at` в строке: [начало, конец) в индексах символов.
fn current_word(chars: &[char], at: usize) -> Option<(usize, usize)> {
    let at = at.min(chars.len().checked_sub(1)?);
    let mut a = at;
    // Если попали в пробел — берём предыдущее слово.
    while a > 0 && chars[a].is_whitespace() {
        a -= 1;
    }
    if chars[a].is_whitespace() {
        return None;
    }
    let mut start = a;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = a + 1;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    Some((start, end))
}

/// Текст + геометрия для мыши (колесо прокручивает, клик по строке перематывает).
fn lyrics_panel(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx, big: bool) {
    let (mut hits, mut ov) = (Vec::new(), Vec::new());
    draw_lyrics(f, area, app, ctx, big, &mut hits, &mut ov);
    app.lyrics_hits = hits;
    app.overlays.extend(ov);
    app.areas.lyrics = area;
}

fn draw_lyrics(
    f: &mut Frame,
    area: Rect,
    app: &App,
    ctx: &Ctx,
    big: bool,
    hits: &mut Vec<(u16, usize)>,
    overlays: &mut Vec<Overlay>,
) {
    if area.height == 0 {
        return;
    }
    let pos = app.now.as_ref().map_or(0.0, |n| n.pos);
    let Some((_, lyrics)) = &app.lyrics else {
        let msg = match &app.now {
            Some(n) if n.track.has_sync_lyrics() || n.track.has_text_lyrics() => {
                format!("{} текст загружается", ctx.spinner())
            }
            Some(_) => "♪ без текста".into(),
            None => String::new(),
        };
        f.render_widget(
            Paragraph::new(Line::styled(msg, ctx.dim())).alignment(Alignment::Center),
            centered(area, area.width, 1),
        );
        return;
    };
    let w = area.width as usize;
    match lyrics {
        Lyrics::Synced(lines) => {
            // Каждая реплика переносится по словам и занимает свои строки; между репликами — пустая строка.
            // В крупном режиме текущая реплика — двойного размера (OSC 66), её строки вдвое выше.
            let gap = 1usize;
            let wrap_w = w.saturating_sub(4).max(8);
            let cur = lyrics::current(lines, pos + 0.25);
            let scale: usize = if big && ctx.big_text { 2 } else { 1 };
            let wrapped: Vec<Vec<String>> = lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let t = if l.text.trim().is_empty() {
                        "♪"
                    } else {
                        l.text.as_str()
                    };
                    let ww = if Some(i) == cur {
                        (wrap_w / scale).max(4)
                    } else {
                        wrap_w
                    };
                    textwrap::wrap(t, ww).into_iter().map(|c| c.into_owned()).collect()
                })
                .collect();
            let height = |i: usize| wrapped[i].len() * if Some(i) == cur { scale } else { 1 };
            // Верх каждой реплики в строках от начала текста.
            let mut tops = Vec::with_capacity(wrapped.len() + 1);
            let mut acc = 0usize;
            for i in 0..wrapped.len() {
                tops.push(acc as f32);
                acc += height(i) + gap;
            }
            tops.push(acc as f32);
            // Дробный индекс прокрутки → дробная строка: плавно между верхами соседних реплик.
            let sc = app.lyrics_scroll.max(-1.0);
            let scroll_row = if sc < 0.0 {
                sc * (1 + gap) as f32
            } else {
                let i = (sc.floor() as usize).min(wrapped.len().saturating_sub(1));
                let frac = sc - i as f32;
                tops[i] + (tops[i + 1] - tops[i]) * frac
            };
            let center = area.y as f32 + area.height as f32 * if big { 0.42 } else { 0.35 };
            let bottom = (area.y + area.height) as i32;
            for (i, (l, rows)) in lines.iter().zip(&wrapped).enumerate() {
                let is_cur = Some(i) == cur;
                let d = (i as f32 - app.lyrics_scroll).abs();
                let all: Vec<char> = rows.iter().flat_map(|r| r.chars()).collect();
                let total = all.len().max(1);
                let weights = if is_cur { sung_weights(&all) } else { Vec::new() };
                // Караоке-заливка текущей реплики: конец — по темпу песни (с запасом),
                // но не позже начала следующей; паузу после реплики не заливаем.
                let frac = if is_cur {
                    let sung: f64 = weights.iter().map(|&w| w as f64).sum();
                    let next = lines.get(i + 1);
                    let end = match next {
                        // Пустая метка в LRC — явный конец вокала (так её читает и оф. клиент).
                        Some(n) if n.text.trim().is_empty() => n.at,
                        _ => {
                            let rate = sung_rate(lines, i);
                            let next = next.map_or(f64::INFINITY, |n| n.at);
                            next.min(l.at + (sung * rate * 1.2 + 0.3).max(0.8))
                        }
                    };
                    ((pos + 0.25 - l.at) / (end - l.at).max(0.1)).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                // Позиция заливки в символах: слова с длинными гласными получают больше времени.
                let fill = if is_cur { karaoke_fill(&weights, frac) } else { 0.0 };
                let base = if is_cur {
                    ctx.p.fg
                } else {
                    let k = (d / if big { 6.0 } else { 4.0 }).min(1.0);
                    ctx.p.fg.lerp(ctx.p.dim.scale(0.6), 0.3 + 0.7 * k)
                };
                // Текущее слово (по времени внутри строки) — для подчёркивания.
                let word = if is_cur && frac > 0.0 {
                    current_word(&all, fill as usize)
                } else {
                    None
                };
                let s = if is_cur { scale } else { 1 };
                let mut done = 0usize;
                for (ri, row) in rows.iter().enumerate() {
                    let yi = (center + tops[i] + (ri * s) as f32 - scroll_row).round() as i32;
                    let n = row.chars().count();
                    if yi < area.y as i32 || yi + s as i32 > bottom {
                        done += n;
                        continue;
                    }
                    for k in 0..s {
                        hits.push((yi as u16 + k as u16, i));
                    }
                    let tw = row.width() * s;
                    let x0 = area.x as usize + w.saturating_sub(tw) / 2;
                    let color_at = |gi: usize| {
                        let p = gi as f32 / total as f32;
                        if is_cur && (gi as f32) < fill {
                            shimmer(ctx.p.accent, ctx.p.accent2, p, ctx.t * 0.3)
                        } else {
                            base
                        }
                    };
                    let in_word = |gi: usize| word.is_some_and(|(a, b)| gi >= a && gi < b);
                    if s > 1 {
                        // Крупная строка: клетки блока чистим, текст — оверлеем OSC 66 с волнистым подчёркиванием.
                        let buf = f.buffer_mut();
                        for yy in yi as u16..yi as u16 + s as u16 {
                            for xx in x0 as u16..(x0 + tw).min((area.x + area.width) as usize) as u16 {
                                if let Some(c) = buf.cell_mut((xx, yy)) {
                                    c.set_symbol(" ");
                                }
                            }
                        }
                        let runs = row
                            .chars()
                            .enumerate()
                            .map(|(ci, ch)| {
                                let gi = done + ci;
                                Run {
                                    text: ch.to_string(),
                                    style: Style::new().fg(color_at(gi).color()).add_modifier(Modifier::BOLD),
                                    curly: in_word(gi).then(|| ctx.p.accent2.color()),
                                }
                            })
                            .collect();
                        overlays.push(Overlay {
                            x: x0 as u16,
                            y: yi as u16,
                            scale: s as u8,
                            runs,
                            link: None,
                        });
                    } else {
                        let buf = f.buffer_mut();
                        let mut x = x0;
                        for (ci, ch) in row.chars().enumerate() {
                            let cw = ch.width().unwrap_or(1);
                            if x + cw > (area.x + area.width) as usize {
                                break;
                            }
                            let gi = done + ci;
                            if let Some(c) = buf.cell_mut((x as u16, yi as u16)) {
                                c.set_char(ch).set_fg(color_at(gi).color());
                                if is_cur {
                                    c.modifier.insert(Modifier::BOLD);
                                }
                                if in_word(gi) {
                                    c.modifier.insert(Modifier::UNDERLINED);
                                    c.underline_color = ctx.p.accent2.color();
                                }
                            }
                            x += cw;
                        }
                    }
                    done += n;
                }
            }
            let spacing = (1 + gap) as f32;
            // Вступление/проигрыш: пульсирующие точки.
            if cur.is_none() || lines.get(cur.unwrap_or(0)).is_some_and(|l| l.text.trim().is_empty()) {
                let dots: String = (0..3)
                    .map(|i| {
                        if ((ctx.t * 2.0) as usize + i).is_multiple_of(3) {
                            '●'
                        } else {
                            '•'
                        }
                    })
                    .collect();
                let y = (center - spacing).round().max(area.y as f32) as u16;
                if cur.is_none() && y < area.y + area.height {
                    f.render_widget(
                        Paragraph::new(Line::styled(dots, ctx.accent())).alignment(Alignment::Center),
                        Rect { y, height: 1, ..area },
                    );
                }
            }
        }
        Lyrics::Plain(text) => {
            // Текст без таймингов стоит на месте и листается колесом.
            let max = (text.len() as u16).saturating_sub(area.height / 2);
            let scroll = app.plain_scroll.min(max);
            let lines: Vec<Line> = text.iter().map(|l| Line::styled(l.clone(), ctx.fg())).collect();
            f.render_widget(
                Paragraph::new(lines)
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true })
                    .scroll((scroll, 0)),
                area,
            );
        }
    }
}

// ── эффект «перебор символов» ────────────────────────────────────────────────

/// Буквы сначала перебираются случайными знаками и проявляются слева направо.
/// Картинки (плейсхолдеры kitty) и кнопки управления не трогаются.
fn scramble(buf: &mut ratatui::buffer::Buffer, rect: Rect, since: std::time::Instant, skip: &[Rect], tint: Color) {
    const GLYPHS: [char; 16] = [
        '░', '▒', '/', '\\', '<', '>', '=', '+', '*', '#', '%', '_', '~', ':', '·', '¦',
    ];
    let dur = crate::app::SCRAMBLE.as_secs_f32();
    let t = since.elapsed().as_secs_f32();
    if t >= dur {
        return;
    }
    let tick = (t * 20.0) as u32;
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            if skip
                .iter()
                .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            {
                continue;
            }
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            let sym = cell.symbol();
            if sym.trim().is_empty() || sym.starts_with('\u{10EEEE}') {
                continue;
            }
            // Порог проявления: слева направо + немного случайности на клетку.
            let h = (x as u32).wrapping_mul(2654435761) ^ (y as u32).wrapping_mul(40503);
            let jitter = (h % 1000) as f32 / 1000.0;
            let reveal = ((x - rect.x) as f32 / rect.width.max(1) as f32) * 0.65 + jitter * 0.35;
            if t / dur < reveal {
                let g = GLYPHS[((h >> 7).wrapping_add(tick.wrapping_mul(7919)) % GLYPHS.len() as u32) as usize];
                cell.set_char(g).set_fg(tint);
            }
        }
    }
}

fn apply_scramble(f: &mut Frame, app: &App, screen: Rect, main: Rect) {
    let tint = Color::Rgb(140, 130, 150);
    let skip = app.controls.iter().map(|c| c.0).collect::<Vec<_>>();
    let buf = f.buffer_mut();
    if let Some(t) = app.fx_start {
        scramble(buf, screen, t, &skip, tint);
    }
    if let Some(t) = app.fx_view {
        scramble(buf, main, t, &skip, tint);
    }
    if let Some(t) = app.fx_track {
        scramble(buf, app.areas.info, t, &skip, tint);
    }
}

// ── плеер ────────────────────────────────────────────────────────────────────

fn draw_player(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    app.areas.player = area;
    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(ctx.p.dim.scale(0.4).color()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [l1, l2, l3] = Layout::vertical([Constraint::Length(1); 3]).areas(inner);

    let paused = app.player.paused();
    let (title, pos, dur) = match (&app.now, &app.loading_track) {
        (_, Some(t)) => (
            format!("{} {} — {}", ctx.spinner(), t.full_title(), t.artist_names()),
            0.0,
            t.duration_secs(),
        ),
        (Some(n), None) => (
            format!(
                "{} {} — {}",
                if paused { "⏸" } else { "▶" },
                n.track.full_title(),
                n.track.artist_names()
            ),
            n.pos,
            n.track.duration_secs(),
        ),
        (None, None) => ("■ ничего не играет".into(), 0.0, 0.0),
    };

    // Строка 1: трек слева, состояние справа.
    let vol = app.player.volume();
    let vol_bar: String = (0..8).map(|i| if (i as f32) < vol * 8.0 { '▮' } else { '▯' }).collect();
    let mut right = vec![];
    if app.repeat_one {
        right.push(Span::styled("⟲1 ", ctx.accent()));
    }
    if app.shuffle {
        right.push(Span::styled("⤨ ", ctx.accent()));
    }
    right.push(Span::styled(format!("{} ", quality_label(app.quality())), ctx.dim()));
    right.push(Span::styled("vol ", ctx.dim()));
    right.push(Span::styled(vol_bar, ctx.accent()));
    right.push(Span::styled(format!(" {:>3.0}% ", vol * 100.0), ctx.dim()));
    let rw: usize = right.iter().map(|s| s.content.width()).sum();
    let tw = (l1.width as usize).saturating_sub(rw + 2);
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(marquee(&title, tw, ctx.t), ctx.fg().add_modifier(Modifier::BOLD)),
    ];
    spans.push(Span::raw(" "));
    spans.extend(right);
    f.render_widget(Paragraph::new(Line::from(spans)), l1);
    if app.kitty_text
        && let Some(n) = &app.now
        && let Some(o) = link_cells(f, l1.x + 3, l1.y, tw.saturating_sub(2) as u16, App::track_url(&n.track))
    {
        app.overlays.push(o);
    }

    // Строка 2: прогресс с субсимвольной точностью и бегущим бликом.
    let left = format!(" {} ", fmt_time(pos));
    let rt = format!(" {} ", fmt_time(dur));
    let bar_w = (l2.width as usize).saturating_sub(left.width() + rt.width());
    app.areas.progress = Rect {
        x: l2.x + left.width() as u16,
        width: bar_w as u16,
        ..l2
    };
    let frac = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let buffered = match app.now.as_ref().and_then(|n| n.origin.as_ref()) {
        Some(Origin::Network { buf, .. }) => buf.progress(),
        _ => 1.0,
    };
    let mut sp = vec![Span::styled(left, ctx.dim())];
    // Пройденное — средняя линия с градиентом, впереди — тонкая, позиция — толстая точка.
    let head = ((frac * bar_w as f32) as usize).min(bar_w.saturating_sub(1));
    let phase = if paused { 0.0 } else { ctx.t };
    for i in 0..bar_w {
        let tcol = i as f32 / bar_w.max(1) as f32;
        if i == head && dur > 0.0 {
            let c = ctx
                .p
                .accent
                .lerp(ctx.p.accent2, tcol)
                .lerp(Rgb(255.0, 255.0, 255.0), 0.25);
            sp.push(Span::styled(
                "●",
                Style::new().fg(c.color()).add_modifier(Modifier::BOLD),
            ));
        } else if i < head {
            let c = if paused {
                ctx.p.accent.lerp(ctx.p.accent2, tcol).scale(0.7)
            } else {
                shimmer(ctx.p.accent, ctx.p.accent2, tcol, phase)
            };
            sp.push(Span::styled("━", Style::new().fg(c.color())));
        } else {
            let col = if tcol <= buffered {
                ctx.p.dim.scale(0.55)
            } else {
                ctx.p.dim.scale(0.25)
            };
            sp.push(Span::styled("─", Style::new().fg(col.color())));
        }
    }
    sp.push(Span::styled(rt, ctx.dim()));
    f.render_widget(Paragraph::new(Line::from(sp)), l2);

    // Строка 3: контекст очереди.
    let ctx_line = if app.queue.items.is_empty() {
        String::new()
    } else {
        format!(
            " {} · {}/{}",
            app.queue.title,
            app.queue.pos.map_or(0, |p| p + 1),
            app.queue.items.len()
        )
    };
    f.render_widget(Paragraph::new(Line::styled(ctx_line, ctx.dim())), l3);
}

fn draw_toasts(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx) {
    let mut y = area.y + 1;
    let mut occ = Vec::new();
    for t in app.toasts.iter().rev().take(4) {
        let age = t.at.elapsed().as_secs_f32();
        let w = (t.text.width() as u16 + 4).min(area.width.saturating_sub(4));
        // Въезд справа в первые 200 мс, затухание в последние 600.
        let slide = ((0.2 - age).max(0.0) / 0.2 * w as f32) as u16;
        let fade = ((4.0 - age) / 0.6).clamp(0.0, 1.0);
        let r = Rect {
            x: area.x + area.width - w - 2 + slide.min(w),
            y,
            width: w.saturating_sub(slide),
            height: 1,
        };
        if r.width == 0 {
            continue;
        }
        let base = if t.error {
            Rgb(200.0, 70.0, 60.0)
        } else {
            ctx.p.surface.lerp(ctx.p.accent, 0.25)
        };
        let bg = Rgb(20.0, 20.0, 20.0).lerp(base, fade);
        let fg = ctx.p.dim.lerp(Rgb(250.0, 250.0, 250.0), fade);
        f.render_widget(Clear, r);
        occ.push(r);
        f.render_widget(
            Paragraph::new(Line::styled(
                fit(&format!(" {} ", t.text), r.width as usize),
                Style::new().fg(fg.color()).bg(bg.color()),
            )),
            r,
        );
        y += 1;
    }
    app.occluders.extend(occ);
}

fn draw_help(f: &mut Frame, area: Rect, ctx: &Ctx) {
    let keys: &[(&str, &str)] = &[
        ("1-6 / Tab", "вкладки"),
        ("↑↓ j k, PgUp PgDn, g G", "навигация"),
        ("Enter", "играть / открыть"),
        ("Esc, Backspace", "назад"),
        ("Space", "пауза"),
        ("n / p", "следующий / предыдущий"),
        ("← → (Shift)", "перемотка 5 с (30 с)"),
        ("+ / -", "громкость"),
        ("l / L", "♥ играющий / выделенный"),
        ("d", "не рекомендовать (волна)"),
        ("a / A", "в конец очереди / следующим"),
        ("o / i", "альбом / исполнитель (несколько — выбор)"),
        ("клик по подписи на сцене", "название — о треке, исполнитель, альбом"),
        ("w", "запустить Мою волну"),
        ("/", "поиск"),
        ("Del", "убрать запрос из истории поиска"),
        ("t", "текст на весь экран"),
        ("v / клик по обложке", "сцена: обложка крупно + текст"),
        ("r / s", "повтор трека / перемешивание"),
        ("Q", "качество: Lossless → Высокое → Экономия"),
        ("N", "выравнивание громкости вкл/выкл"),
        ("x", "плавный переход: выкл → 3 → 6 → 10 с"),
        ("R", "обновить библиотеку"),
        ("q", "выход"),
    ];
    let h = keys.len() as u16 + 4;
    let r = centered(area, 64, h);
    f.render_widget(Clear, r);
    let b = card(
        ctx,
        Line::from(gradient_line(" Клавиши ", ctx.p.accent, ctx.p.accent2, ctx.t, true)),
    );
    let inner = b.inner(r);
    f.render_widget(b, r);
    let lines: Vec<Line> = std::iter::once(Line::raw(""))
        .chain(keys.iter().map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!("  {:>24}  ", k), ctx.accent()),
                Span::styled(*d, ctx.fg()),
            ])
        }))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

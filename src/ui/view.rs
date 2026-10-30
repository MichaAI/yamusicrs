//! Отрисовка всех экранов.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    // Блоб живёт только на экране волны; под справкой и полноэкранным текстом его не видно.
    let blob_visible = matches!(app.screen, Screen::Main)
        && matches!(app.page().view, View::Wave)
        && !app.show_help
        && !app.lyrics_mode
        && app.wave.items.ready().is_some();
    if !blob_visible {
        app.images.hide_blob();
    }
    match &app.screen {
        Screen::Connecting => connecting(f, area, ctx),
        Screen::Login(l) => login(f, area, l, ctx),
        Screen::Main => main_screen(f, app, ctx),
    }
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

fn connecting(f: &mut Frame, area: Rect, ctx: &Ctx) {
    let r = centered(area, 44, 3);
    let line = Line::from(vec![
        Span::styled(format!("{} ", ctx.spinner()), ctx.accent()),
        Span::styled("Подключаюсь к Яндекс Музыке…", ctx.fg()),
    ]);
    f.render_widget(Paragraph::new(line).alignment(Alignment::Center), r);
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
    draw_toasts(f, area, app, ctx);
    if app.show_help {
        draw_help(f, area, ctx);
    }
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
    let lib = format!("в библиотеке {n} · {:.1} ГБ ", bytes as f64 / 1e9);
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
        View::Artist { brief, .. } => {
            let Loadable::Ready(b) = brief else {
                return status(f, inner, brief, ctx, "Загружаю исполнителя");
            };
            let rows: Vec<RowKind> = artist_rows(b)
                .into_iter()
                .map(|r| match r {
                    ArtistRow::Header(h) => RowKind::Header(h.to_string()),
                    ArtistRow::Track(t) => RowKind::Track(t.clone()),
                    ArtistRow::Album(a) => RowKind::Other(
                        format!("◉ {}", a.full_title()),
                        a.year.map(|y| y.to_string()).unwrap_or_default(),
                    ),
                })
                .collect();
            mixed_list(f, inner, app, &rows, ctx);
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
    let mut offset = app.page().offset;
    let range = scroll_window(sel, &mut offset, idx.len(), h);
    app.page_mut().offset = offset;
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
    let x = (title_x + text_w + 2).min(area.x + area.width.saturating_sub(w + 2));
    let y = if row_y + 1 + h <= area.y + area.height {
        row_y + 1
    } else {
        row_y.saturating_sub(h)
    };
    let r = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
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
    let mut offset = app.page().offset;
    let range = scroll_window(sel, &mut offset, rows.len(), h);
    app.page_mut().offset = offset;
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
            _ => {
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
fn draw_wave_stage(f: &mut Frame, area: Rect, app: &mut App, sel: Option<&crate::api::WheelItem>, ctx: &Ctx) {
    let wave_on = matches!(app.queue.kind, QueueKind::Wave { .. });
    let playing = wave_on && app.now.is_some() && !app.player.paused();
    let [stage, controls, hint] =
        Layout::vertical([Constraint::Min(6), Constraint::Length(3), Constraint::Length(2)]).areas(area);

    // Цвет блоба — агент выбранного пресета, второй — палитра обложки играющего трека.
    let agent = sel
        .and_then(|i| i.color())
        .map_or(ctx.p.accent, |(r, g, b)| Rgb(r as f32, g as f32, b as f32));
    let second = if wave_on { ctx.p.accent } else { agent.rotate_hue(0.12) };
    let colors = [[agent.0, agent.1, agent.2], [second.0, second.1, second.2]];
    // Блоб «дышит» быстрее, когда играет.
    let speed = if playing { 1.0 } else { 0.3 };
    app.wave_phase += ctx.dt * speed;
    let blob_rect = super::kitty::fit_square(stage, ctx.cell_aspect);
    if !app.images.blob(blob_rect, colors, app.wave_phase) {
        blob_halfblocks(f, blob_rect, colors, app.wave_phase);
    }

    let title = if wave_on {
        app.wave.active_name.clone().unwrap_or_else(|| "Моя волна".into())
    } else {
        "Моя волна".into()
    };
    let spaced: String = title.to_uppercase().chars().flat_map(|c| [c, ' ']).collect();
    let ty = stage.y + stage.height / 2;
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
    let play = if wave_on && !app.player.paused() { "⏸" } else { "▶" };
    let labels = [("⏮", 0u8), (play, 1), ("⏭", 2)];
    let total = 3 * 5 + 2 * 4;
    let mut x = controls.x + controls.width.saturating_sub(total) / 2;
    app.wave.controls.clear();
    for (label, action) in labels {
        let w = 5u16;
        let r = Rect {
            x,
            y: controls.y,
            width: w,
            height: 3,
        };
        let (fg, bg) = if action == 1 {
            (Color::Black, agent.color())
        } else {
            (ctx.p.fg.color(), ctx.p.surface.color())
        };
        let b = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(if action == 1 {
                agent.color()
            } else {
                ctx.p.dim.scale(0.6).color()
            }));
        f.render_widget(
            Paragraph::new(Line::styled(
                label,
                Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Center)
            .block(b),
            r,
        );
        app.wave.controls.push((r, action));
        x += w + 4;
    }

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

/// Текст поверх блоба: каждой букве — тёмный или белый цвет по яркости того, что под ней.
fn contrast_over_blob(f: &mut Frame, blob: Rect, text: Rect, colors: [[f32; 3]; 2], t: f32) {
    // Примерный фон терминала там, где блоб прозрачный.
    const BG: [f32; 3] = [24.0, 22.0, 30.0];
    let buf = f.buffer_mut();
    for y in text.y..text.y + text.height {
        for x in text.x..text.x + text.width {
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            if cell.symbol().trim().is_empty() {
                continue;
            }
            let u = ((x.saturating_sub(blob.x)) as f32 + 0.5) / blob.width.max(1) as f32 * 2.0 - 1.0;
            let v = ((y.saturating_sub(blob.y)) as f32 + 0.5) / blob.height.max(1) as f32 * 2.0 - 1.0;
            let p = super::kitty::blob_pixel(u, v, colors, t);
            let a = p[3] / 255.0;
            let px = [0, 1, 2].map(|i| p[i] * a + BG[i] * (1.0 - a));
            let lum = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
            cell.set_fg(if lum > 135.0 {
                Color::Rgb(18, 16, 22)
            } else {
                Color::Rgb(250, 250, 250)
            });
        }
    }
}

/// Фолбэк без kitty: тот же блоб полублоками.
fn blob_halfblocks(f: &mut Frame, r: Rect, colors: [[f32; 3]; 2], t: f32) {
    let buf = f.buffer_mut();
    for y in 0..r.height {
        for x in 0..r.width {
            let u = x as f32 / r.width.max(1) as f32 * 2.0 - 1.0;
            let px = |row: f32| {
                let v = row / (r.height as f32 * 2.0) * 2.0 - 1.0;
                let p = super::kitty::blob_pixel(u, v, colors, t);
                let a = p[3] / 255.0;
                Color::Rgb((p[0] * a) as u8, (p[1] * a) as u8, (p[2] * a) as u8)
            };
            if let Some(c) = buf.cell_mut((r.x + x, r.y + y)) {
                c.set_symbol("▀")
                    .set_fg(px(y as f32 * 2.0))
                    .set_bg(px(y as f32 * 2.0 + 1.0));
            }
        }
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
    let cover_h = cover_h.min(inner.height.saturating_sub(7) * 2 / 3).max(4);
    let [cover, gap, info, lyr] = Layout::vertical([
        Constraint::Length(cover_h),
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Min(0),
    ])
    .areas(inner);
    let _ = gap;
    match track.cover_uri() {
        Some(k) if app.images.contains(k) => app.images.render(k, cover, f.buffer_mut(), ctx.cell_aspect),
        _ => {
            let r = super::kitty::fit_square(cover, ctx.cell_aspect);
            f.render_widget(
                Paragraph::new(Line::styled(format!("{} ♪", ctx.spinner()), ctx.dim())).alignment(Alignment::Center),
                centered(r, r.width, 1),
            );
        }
    }
    app.areas.info = info;
    draw_info(f, info, app, ctx, &track);
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

fn draw_info(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx, t: &Track) {
    let w = area.width as usize;
    let mut lines = Vec::new();
    let title = marquee(&t.full_title(), w.saturating_sub(2), ctx.t);
    let mut tl = vec![Span::raw(" ")];
    tl.extend(gradient_line(
        title.trim_end(),
        ctx.p.fg,
        ctx.p.accent,
        ctx.t * 0.25,
        true,
    ));
    lines.push(Line::from(tl));
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

/// Текст + геометрия для мыши (колесо прокручивает, клик по строке перематывает).
fn lyrics_panel(f: &mut Frame, area: Rect, app: &mut App, ctx: &Ctx, big: bool) {
    let mut hits = Vec::new();
    draw_lyrics(f, area, app, ctx, big, &mut hits);
    app.lyrics_hits = hits;
    app.areas.lyrics = area;
}

fn draw_lyrics(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx, big: bool, hits: &mut Vec<(u16, usize)>) {
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
            let gap = 1usize;
            let wrap_w = w.saturating_sub(4).max(8);
            let wrapped: Vec<Vec<String>> = lines
                .iter()
                .map(|l| {
                    let t = if l.text.trim().is_empty() {
                        "♪"
                    } else {
                        l.text.as_str()
                    };
                    textwrap::wrap(t, wrap_w).into_iter().map(|c| c.into_owned()).collect()
                })
                .collect();
            // Верх каждой реплики в строках от начала текста.
            let mut tops = Vec::with_capacity(wrapped.len() + 1);
            let mut acc = 0usize;
            for wl in &wrapped {
                tops.push(acc as f32);
                acc += wl.len() + gap;
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
            let cur = lyrics::current(lines, pos + 0.25);
            let buf = f.buffer_mut();
            for (i, (l, rows)) in lines.iter().zip(&wrapped).enumerate() {
                let is_cur = Some(i) == cur;
                let d = (i as f32 - app.lyrics_scroll).abs();
                // Караоке-заливка текущей реплики по времени до следующей.
                let frac = if is_cur {
                    let end = lines.get(i + 1).map_or(l.at + 4.0, |n| n.at);
                    ((pos + 0.25 - l.at) / (end - l.at).max(0.1)).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let base = if is_cur {
                    ctx.p.fg
                } else {
                    let k = (d / if big { 6.0 } else { 4.0 }).min(1.0);
                    ctx.p.fg.lerp(ctx.p.dim.scale(0.6), 0.3 + 0.7 * k)
                };
                let total: usize = rows.iter().map(|r| r.chars().count()).sum::<usize>().max(1);
                let mut done = 0usize;
                for (ri, row) in rows.iter().enumerate() {
                    let yi = (center + tops[i] + ri as f32 - scroll_row).round() as i32;
                    let n = row.chars().count();
                    if yi < area.y as i32 || yi >= (area.y + area.height) as i32 {
                        done += n;
                        continue;
                    }
                    hits.push((yi as u16, i));
                    let tw = row.width();
                    let mut x = area.x as usize + w.saturating_sub(tw) / 2;
                    for (ci, ch) in row.chars().enumerate() {
                        let cw = ch.width().unwrap_or(1);
                        if x + cw > (area.x + area.width) as usize {
                            break;
                        }
                        let p = (done + ci) as f32 / total as f32;
                        let col = if is_cur && p < frac {
                            shimmer(ctx.p.accent, ctx.p.accent2, p, ctx.t * 0.3)
                        } else {
                            base
                        };
                        if let Some(c) = buf.cell_mut((x as u16, yi as u16)) {
                            c.set_char(ch).set_fg(col.color());
                            if is_cur {
                                c.modifier.insert(Modifier::BOLD);
                            }
                        }
                        x += cw;
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
            // Обычный текст прокручивается пропорционально прогрессу трека.
            let dur = app.now.as_ref().map_or(1.0, |n| n.track.duration_secs().max(1.0));
            let total = text.len() as f32;
            let scroll = ((pos / dur) as f32 * (total - area.height as f32 * 0.5)).max(0.0) as u16;
            let lines: Vec<Line> = text.iter().map(|l| Line::styled(l.clone(), ctx.fg())).collect();
            f.render_widget(
                Paragraph::new(lines).alignment(Alignment::Center).scroll((scroll, 0)),
                area,
            );
        }
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

fn draw_toasts(f: &mut Frame, area: Rect, app: &App, ctx: &Ctx) {
    let mut y = area.y + 1;
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
        f.render_widget(
            Paragraph::new(Line::styled(
                fit(&format!(" {} ", t.text), r.width as usize),
                Style::new().fg(fg.color()).bg(bg.color()),
            )),
            r,
        );
        y += 1;
    }
}

fn draw_help(f: &mut Frame, area: Rect, ctx: &Ctx) {
    let keys: &[(&str, &str)] = &[
        ("1-5 / Tab", "вкладки"),
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
        ("o / i", "альбом / исполнитель"),
        ("w", "запустить Мою волну"),
        ("/", "поиск"),
        ("t", "текст на весь экран"),
        ("r / s", "повтор трека / перемешивание"),
        ("Q", "качество: Lossless → Высокое → Экономия"),
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

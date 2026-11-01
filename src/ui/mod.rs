//! Терминал, цикл событий и кадров.

pub mod diacritics;
pub mod kitty;
pub mod term;
pub mod theme;
mod view;

use std::time::{Duration, Instant};

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange,
    EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::SetTitle;

pub const TITLE: &str = "YaMusicRS";
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::app::{App, Msg};

pub async fn run(mut app: App, mut rx: UnboundedReceiver<Msg>) -> color_eyre::Result<()> {
    let mut terminal = ratatui::init();
    // \e[22;0t — запомнить заголовок окна, чтобы вернуть его при выходе.
    print!("\x1b[22;0t");
    execute!(
        std::io::stdout(),
        EnableBracketedPaste,
        EnableMouseCapture,
        EnableFocusChange,
        SetTitle(TITLE)
    )?;
    let res = event_loop(&mut terminal, &mut app, &mut rx).await;
    app.shutdown();
    let _ = app.images.clear_all(terminal.backend_mut());
    let _ = execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        DisableFocusChange
    );
    // \e[23;0t — вернуть заголовок, который был до запуска.
    print!("\x1b[23;0t");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    ratatui::restore();
    res
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: &mut UnboundedReceiver<Msg>,
) -> color_eyre::Result<()> {
    let mut events = EventStream::new();
    let fps = app.cfg.fps.clamp(5, 120);
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / fps as f64));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_frame = Instant::now();
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    let mut cell_aspect = kitty::cell_aspect();
    let mut dirty = true;
    let mut refresh_checked = false;
    let mut last_title = String::new();
    let mut osc = term::OscFilter::default();
    let mut prev_overlays: Vec<term::Overlay> = Vec::new();
    // Отпечаток клеток буфера под каждым оверлеем прошлого кадра.
    let mut prev_sigs: Vec<u64> = Vec::new();

    loop {
        tokio::select! {
            ev = events.next() => {
                match ev {
                    // Ctrl+L — перерисовать экран с нуля (если терминал что-то оставил).
                    Some(Ok(Event::Key(k)))
                        if k.kind != KeyEventKind::Release
                            && k.modifiers.contains(KeyModifiers::CONTROL)
                            && matches!(k.code, KeyCode::Char('l' | 'д')) =>
                    {
                        terminal.clear()?;
                        prev_overlays.clear();
                        app.images.on_resize();
                    }
                    Some(Ok(Event::Key(k))) if k.kind != KeyEventKind::Release => match osc.feed(&k) {
                        term::Filtered::Pass => app.on_key(k),
                        term::Filtered::Swallowed => {}
                        term::Filtered::Notify(r) => app.on_notify_reply(r),
                    },
                    Some(Ok(Event::Paste(s))) => app.on_paste(s),
                    Some(Ok(Event::Mouse(m))) => app.on_mouse(m),
                    Some(Ok(Event::FocusGained)) => {
                        app.focused = true;
                        app.images.blob_every = std::time::Duration::from_millis(66);
                    }
                    Some(Ok(Event::FocusLost)) => {
                        app.focused = false;
                        app.images.blob_every = std::time::Duration::from_millis(125);
                    }
                    Some(Ok(Event::Resize(..))) => {
                        cell_aspect = kitty::cell_aspect();
                        app.images.on_resize();
                        // Kitty переразбивает строки при ресайзе, и крупный текст разъезжается: стираем всё.
                        terminal.clear()?;
                        prev_overlays.clear();
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()),
                    _ => {}
                }
                dirty = true;
            }
            Some(msg) = rx.recv() => {
                app.on_msg(msg);
                dirty = true;
            }
            _ = tick.tick() => {}
        }
        if app.quit {
            return Ok(());
        }
        if !refresh_checked && let Some(tok) = app.token_needs_refresh() {
            refresh_checked = true;
            tracing::info!(
                "токен скоро истечёт: refresh при следующем запуске (exp {:?})",
                tok.expires_at
            );
        }

        app.tick();
        // В покое (пауза, нет анимаций) перерисовываемся редко — экономим CPU.
        // Без ввода рисуем с частотой, которую диктует то, что сейчас на экране.
        if !dirty && last_draw.elapsed() < app.frame_interval(fps) {
            continue;
        }
        if last_draw.elapsed() < Duration::from_secs_f64(0.8 / fps as f64) && !dirty {
            continue;
        }
        dirty = false;
        let elapsed = last_frame.elapsed();
        last_frame = Instant::now();
        last_draw = Instant::now();

        let ctx = view::Ctx {
            p: app.theme.current(),
            t: app.started.elapsed().as_secs_f32(),
            dt: elapsed.as_secs_f32().min(0.1),
            cell_aspect,
            big_text: app.kitty_text,
        };
        // Весь кадр — одним синхронным обновлением (mode 2026), без промежуточных состояний.
        let mut out = b"\x1b[?2026h".to_vec();
        let mut sigs = Vec::new();
        let mut keep = Vec::new();
        terminal.draw(|f| {
            view::draw(f, app, &ctx);
            // Неизменный оверлей над неизменными клетками не трогаем: перерисовка каждый
            // кадр сбрасывает в kitty подсветку ссылки под мышью — обводка мигает.
            sigs = app.overlays.iter().map(|o| overlay_sig(f.buffer_mut(), o)).collect();
            keep = app
                .overlays
                .iter()
                .zip(&sigs)
                .map(|(o, sig)| {
                    prev_overlays
                        .iter()
                        .zip(&prev_sigs)
                        .any(|(p, ps)| p == o && ps == sig)
                })
                .collect();
            // Прошлый крупный текст стираем ДО текста кадра: если писать в нижнюю строку
            // крупного символа, kitty не стирает его, а сдвигает текст вправо за него.
            let width = f.area().width;
            for o in prev_overlays.iter().filter(|o| o.scale > 1) {
                let kept = app.overlays.iter().zip(&keep).any(|(n, k)| *k && n == o);
                if !kept {
                    o.erase(&mut out, width);
                }
            }
            // Через тот же stdout, что и бэкенд ratatui: уходит в терминал раньше диффа кадра.
            let _ = std::io::Write::write_all(&mut std::io::stdout(), &std::mem::take(&mut out));
        })?;
        // Картинки и команды размещения уходят в терминал отдельно от текстового диффа.
        app.images.flush(terminal.backend_mut())?;
        // Оверлеи (крупный текст, ссылки) и разовые OSC-команды — поверх готового кадра.
        let mut out = std::mem::take(&mut app.osc_out);
        prev_overlays = app.overlays.clone();
        prev_sigs = sigs;
        let width = terminal.size()?.width;
        for (o, _) in app.overlays.iter().zip(&keep).filter(|(_, k)| !**k) {
            // Место под крупный текст чистим заново: там мог остаться чужой крупный символ.
            if o.scale > 1 {
                o.erase(&mut out, width);
            }
            o.write(&mut out);
        }
        out.extend_from_slice(b"\x1b[?2026l");
        let w = terminal.backend_mut();
        std::io::Write::write_all(w, &out)?;
        std::io::Write::flush(w)?;
        let title = app.window_title();
        if title != last_title {
            execute!(terminal.backend_mut(), SetTitle(&title))?;
            last_title = title;
        }
    }
}

/// Отпечаток клеток буфера, которые накрывает оверлей: если ratatui их перерисует,
/// оверлей надо написать заново.
fn overlay_sig(buf: &mut ratatui::buffer::Buffer, o: &term::Overlay) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let area = buf.area;
    for y in o.y..(o.y + o.scale as u16).min(area.bottom()) {
        for x in o.x..(o.x + o.cols() as u16).min(area.right()) {
            if let Some(c) = buf.cell((x, y)) {
                c.symbol().hash(&mut h);
                (c.fg, c.bg, c.modifier).hash(&mut h);
            }
        }
    }
    h.finish()
}

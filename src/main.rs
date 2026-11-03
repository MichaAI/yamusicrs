mod api;
mod app;
mod audio;
mod config;
mod fetch;
mod library;
mod lyrics;
mod mpris;
mod offline;
mod offline_wave;
mod tinyml;
mod render;
mod ui;

use clap::Parser;

/// TUI-клиент Яндекс Музыки: lossless через PipeWire, Моя волна, синхронные тексты.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Забыть сохранённый токен и войти заново.
    #[arg(long)]
    logout: bool,
    /// Войти по готовому OAuth-токену (или ссылке с access_token).
    #[arg(long, value_name = "TOKEN")]
    token: Option<String>,
    /// Диагностика аудио: проиграть локальные файлы подряд (gapless) через PipeWire и выйти.
    #[arg(long, value_name = "FILE", num_args = 1.., hide = true)]
    play: Vec<std::path::PathBuf>,
    /// Отладка офлайн-волны: прогнать программу TinyMl (json ответа) на треках из json запроса.
    #[arg(long, value_names = ["PROGRAM", "REQUEST"], num_args = 2, hide = true)]
    tinyml_bench: Vec<std::path::PathBuf>,
    /// Для --play: кроссфейд между файлами, сек.
    #[arg(long, value_name = "SECS", default_value_t = 0.0, hide = true)]
    crossfade: f32,
}

fn main() -> color_eyre::Result<()> {
    // Потоков ещё нет, менять окружение и настройки malloc безопасно.
    // SAFETY: вызывается до запуска любых потоков.
    unsafe {
        // Vulkan-загрузчик открывает драйверы всех вендоров (radeon и lavapipe тянут LLVM),
        // это ~20 МБ памяти ради блоба. Оставляем только драйвер своей видеокарты.
        if std::env::var_os("VK_LOADER_DRIVERS_SELECT").is_none()
            && std::env::var_os("VK_LOADER_DRIVERS_DISABLE").is_none()
        {
            match vulkan_drivers() {
                Some(sel) => std::env::set_var("VK_LOADER_DRIVERS_SELECT", sel),
                None => std::env::set_var("VK_LOADER_DRIVERS_DISABLE", "*lvp*"),
            }
        }
        // По арене malloc на поток — это десятки мегабайт фрагментов; плееру хватит двух.
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
    // Плееру хватает двух рабочих потоков; блокирующих (декод картинок) — не больше четырёх.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .thread_keep_alive(std::time::Duration::from_secs(2))
        .enable_all()
        .build()?
        .block_on(run())
}

/// Маски манифестов Vulkan-драйверов под видеокарты из `/sys/class/drm`;
/// `None`, если встретился незнакомый вендор (тогда пусть загрузчик решает сам).
fn vulkan_drivers() -> Option<String> {
    let mut sel: Vec<&str> = Vec::new();
    for e in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let Ok(v) = std::fs::read_to_string(e.path().join("device/vendor")) else {
            continue;
        };
        let masks: &[&str] = match v.trim() {
            "0x8086" => &["*intel*"],
            "0x1002" => &["*radeon*", "*amd*"],
            "0x10de" => &["*nvidia*", "*nouveau*"],
            _ => return None,
        };
        for m in masks {
            if !sel.contains(m) {
                sel.push(m);
            }
        }
    }
    (!sel.is_empty()).then(|| sel.join(","))
}

async fn run() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let paths = config::Paths::new();

    std::fs::create_dir_all(&paths.log_dir)?;
    let appender = tracing_appender::rolling::never(&paths.log_dir, "yamusicrs.log");
    let (writer, _guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("YAMUSICRS_LOG").unwrap_or_else(|_| "yamusicrs=info".into()),
        )
        .init();

    if cli.logout {
        let _ = std::fs::remove_file(&paths.token_file);
    }
    if let Some(t) = cli.token {
        let tok = api::auth::parse_pasted_token(&t).ok_or_else(|| color_eyre::eyre::eyre!("это не похоже на токен"))?;
        api::auth::StoredToken {
            access_token: tok,
            refresh_token: None,
            expires_at: None,
        }
        .save(&paths.token_file)?;
    }

    if let [program, request] = cli.tinyml_bench.as_slice() {
        return tinyml_bench(program, request);
    }
    if !cli.play.is_empty() {
        return play_files(cli.play, cli.crossfade).await;
    }

    let cfg = config::Config::load(&paths);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = app::App::new(tx, cfg, paths)?;
    ui::run(app, rx).await
}

async fn play_files(files: Vec<std::path::PathBuf>, crossfade: f32) -> color_eyre::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let player = audio::Player::new(tx, 1.0)?;
    player.set_crossfade(crossfade);
    let mut files = files.into_iter().enumerate();
    let (t, first) = files.next().expect("file");
    player.load(t as u64, audio::Source::File(first), 1.0, 0.0);
    // Следующий файл ставится заранее (как в приложении), иначе кроссфейду не с чем сводить.
    let mut queued = t as u64;
    let mut enqueue = |player: &audio::Player, queued: &mut u64| {
        if let Some((t, f)) = files.next() {
            player.enqueue_next(t as u64, audio::Source::File(f), 1.0, true);
            *queued = t as u64;
        }
    };
    enqueue(&player, &mut queued);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(audio::Event::Drained) | None => break,
                Some(audio::Event::NeedNext) => enqueue(&player, &mut queued),
                Some(audio::Event::SeekMiss(..)) => {}
                Some(audio::Event::Error(_, e)) => color_eyre::eyre::bail!(e),
            },
            _ = tick.tick() => {
                if let Some(p) = player.position() {
                    if p.token == queued {
                        enqueue(&player, &mut queued);
                    }
                    let i = p.info;
                    println!("#{} {:6.2}s  {} {:?}bit {}Hz", p.token, p.secs, i.codec, i.bits, i.rate);
                }
            }
        }
    }
    Ok(())
}

fn tinyml_bench(program: &std::path::Path, request: &std::path::Path) -> color_eyre::Result<()> {
    use tinyml::{Event, EventKind, Vm};
    let p: serde_json::Value = serde_json::from_slice(&std::fs::read(program)?)?;
    let prog: Vec<i32> = serde_json::from_value(p["result"]["offlineRecommenderSource"].clone())?;
    let r: serde_json::Value = serde_json::from_slice(&std::fs::read(request)?)?;
    let mut avail: Vec<i32> = Vec::new();
    for k in ["autoCachedLikedTracks", "autoCachedRecommenderTracks", "userCachedTracks"] {
        for v in r[k].as_array().into_iter().flatten() {
            if let Some(id) = v.as_str().and_then(|s| s.parse().ok()) {
                avail.push(id);
            }
        }
    }
    let level = std::env::var("TINYML_LOG").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let t = std::time::Instant::now();
    let mut vm = Vm::new(prog.clone(), None, level)?;
    println!("init: {:?}, {} шагов, программа {} слов, доступно {}", t.elapsed(), vm.steps, prog.len(), avail.len());
    let t = std::time::Instant::now();
    let s = vm.start_wave(&[], &[], &["user:onyourwave".into()], &avail)?;
    println!("startWave: {:?}, {} шагов → {:?}", t.elapsed(), vm.steps, s);
    let mut events = Vec::new();
    let (mut last, mut queued) = (s.tracks[0], s.tracks[1]);
    events.push(Event::now(last, EventKind::Started, 0.0));
    let mut seen = std::collections::HashSet::new();
    for i in 0..30 {
        let kind = if i % 3 == 2 { EventKind::Skip } else { EventKind::Finished };
        events.push(Event::now(last, kind, if kind == EventKind::Skip { 12.0 } else { 180.0 }));
        let t = std::time::Instant::now();
        let n = vm.next_track(&events, last, queued)?;
        println!(
            "nextTrack #{i}: {:?}, {} шагов → {} ({}){}",
            t.elapsed(),
            vm.steps,
            n.track,
            n.batch,
            if seen.insert(n.track) { "" } else { " ПОВТОР" }
        );
        last = queued;
        queued = n.track;
        events.push(Event::now(last, EventKind::Started, 0.0));
    }
    let t = std::time::Instant::now();
    let state = vm.state().to_vec();
    let vm2 = Vm::new(prog, Some(state), level)?;
    println!("восстановление из состояния: {:?}, init {} шагов", t.elapsed(), vm2.steps);
    Ok(())
}

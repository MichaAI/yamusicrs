mod api;
mod app;
mod audio;
mod config;
mod fetch;
mod library;
mod lyrics;
mod mpris;
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
}

fn main() -> color_eyre::Result<()> {
    // Плееру хватает двух рабочих потоков; блокирующих (декод картинок) — не больше четырёх.
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .thread_keep_alive(std::time::Duration::from_secs(2))
        .enable_all()
        .build()?
        .block_on(run())
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

    if !cli.play.is_empty() {
        return play_files(cli.play).await;
    }

    let cfg = config::Config::load(&paths);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = app::App::new(tx, cfg, paths)?;
    ui::run(app, rx).await
}

async fn play_files(files: Vec<std::path::PathBuf>) -> color_eyre::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let player = audio::Player::new(tx, 1.0)?;
    let mut files = files.into_iter().enumerate();
    let (t, first) = files.next().expect("file");
    player.load(t as u64, audio::Source::File(first), 1.0, 0.0);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(audio::Event::Drained) | None => break,
                Some(audio::Event::NeedNext) => {
                    if let Some((t, f)) = files.next() {
                        player.enqueue_next(t as u64, audio::Source::File(f), 1.0);
                    }
                }
                Some(audio::Event::SeekMiss(..)) => {}
                Some(audio::Event::Error(_, e)) => color_eyre::eyre::bail!(e),
            },
            _ = tick.tick() => {
                if let Some(p) = player.position() {
                    let i = p.info;
                    println!("#{} {:6.2}s  {} {:?}bit {}Hz", p.token, p.secs, i.codec, i.bits, i.rate);
                }
            }
        }
    }
    Ok(())
}

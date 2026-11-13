//! Замеры ядра: из чего складывается запуск и сколько стоят частые действия с треком.
//! Ничего не играет и не пишет в настоящую библиотеку (загрузки идут во временную папку).
//!
//! `cargo run --release -p yamusic-core --example bench`

use std::time::{Duration, Instant};

use yamusic_core::api::{Client, Quality, auth::StoredToken};
use yamusic_core::config::{Paths, State};
use yamusic_core::fetch::{Fetcher, Origin};
use yamusic_core::library::Library;
use yamusic_core::offline::Snapshot;

fn ms(d: Duration) -> String {
    format!("{:>7.1} мс", d.as_secs_f64() * 1000.0)
}

fn timed<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let v = f();
    println!("  {name:<44}{}", ms(t.elapsed()));
    v
}

/// Разбор контейнера и первый декодированный пакет — то, что декодер делает при смене трека.
fn open_and_decode(data: Vec<u8>, ext: &str) -> Option<(Duration, Duration)> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    let t = Instant::now();
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(data)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let mut reader = symphonia::default::get_probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default()).ok()?;
    let track = reader.default_track(TrackType::Audio)?;
    let tid = track.id;
    let params = track.codec_params.as_ref()?.audio()?.clone();
    let mut dec = symphonia::default::get_codecs().make_audio_decoder(&params, &AudioDecoderOptions::default()).ok()?;
    let opened = t.elapsed();
    loop {
        let p = reader.next_packet().ok()??;
        if p.track_id == tid && dec.decode(&p).is_ok() {
            break;
        }
    }
    Some((opened, t.elapsed()))
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let paths = Paths::new();
    let data_dir = paths.index_file.parent().unwrap().to_path_buf();
    println!("Запуск (чтение состояния с диска):");
    let t0 = Instant::now();
    let state = timed("state.json", || State::load(&paths));
    let snap = timed("снимок коллекции (лайки, плейлисты)", || Snapshot::load(&Snapshot::path(&data_dir)));
    let cfg = yamusic_core::config::Config::load(&paths);
    let lib = timed("индекс библиотеки", || Library::open(cfg.library_dir.clone(), paths.index_file.clone(), 0.0));
    let queue = timed("очередь прошлого запуска", || yamusic_core::offline::load_queue(&data_dir));
    timed("программа офлайн-волны", || yamusic_core::offline_wave::Store::load(&data_dir).is_some());
    println!("  {:<44}{}", "итого до первого кадра с данными", ms(t0.elapsed()));
    println!("  (лайков в снимке {}, треков в библиотеке {}, в очереди {})", snap.likes.len(), lib.stats().0, queue.len());

    // Локальный трек: открыть и получить первый звук.
    if let Some((path, e)) = lib.ids().first().and_then(|id| lib.lookup(id)) {
        let data = std::fs::read(&path)?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("flac").to_string();
        println!("\nСкачанный трек ({}, {:.1} МБ):", e.codec, data.len() as f64 / 1e6);
        if let Some((open, first)) = open_and_decode(data, &ext) {
            println!("  {:<44}{}", "разбор контейнера + декодер", ms(open));
            println!("  {:<44}{}", "до первого декодированного пакета", ms(first));
        }
    }

    let Some(tok) = StoredToken::load(&paths.token_file) else {
        println!("\nНет токена — сетевые замеры пропущены");
        return Ok(());
    };
    println!("\nСеть:");
    let t = Instant::now();
    let mut c = Client::new(tok.access_token.clone());
    c.account_status().await?;
    println!("  {:<44}{}", "первый запрос (DNS + TLS + account/status)", ms(t.elapsed()));
    for i in 0..3 {
        let t = Instant::now();
        c.account_status().await?;
        println!("  {:<44}{}", format!("account/status по готовому соединению #{}", i + 1), ms(t.elapsed()));
    }
    let ids: Vec<String> = state.resume.map(|r| r.ids).unwrap_or_default();
    let fresh: Vec<&String> = ids.iter().filter(|id| !lib.contains(id)).take(3).collect();
    let tracks = c.tracks(&fresh.iter().map(|s| s.to_string()).collect::<Vec<_>>()).await?;
    // Библиотека для замеров — временная: настоящую не трогаем.
    let tmp = std::env::temp_dir().join(format!("yamusicrs-bench-{}", std::process::id()));
    let scratch = Library::open(tmp.join("lib"), tmp.join("index.json"), 0.0);
    yamusic_core::audio::source::set_spill_dir(tmp.clone());
    for (q, name) in [(Quality::Hq, "AAC (быстрый старт)"), (Quality::Lossless, "lossless сразу")] {
        println!("\nСмена трека из сети, {name}:");
        for t in &tracks {
            let f = Fetcher::new(c.clone(), scratch.clone(), q);
            f.set_fast_start(false);
            let t0 = Instant::now();
            let (_, origin) = match f.open(t).await {
                Ok(v) => v,
                Err(e) => {
                    println!("  {}: {e}", t.id);
                    continue;
                }
            };
            let info = t0.elapsed();
            let Origin::Network { buf } = origin else { continue };
            let mut first = None;
            // 64 КБ хватает декодеру, чтобы разобрать контейнер и начать звук.
            while buf.len() < 256 * 1024 && t0.elapsed() < Duration::from_secs(15) {
                if first.is_none() && buf.len() >= 64 * 1024 {
                    first = Some(t0.elapsed());
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            println!(
                "  get-file-info {} · 64 КБ через {} · 256 КБ через {}",
                ms(info).trim(),
                first.map(|d| ms(d).trim().to_string()).unwrap_or_else(|| "—".into()),
                ms(t0.elapsed()).trim()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

//! Разовая проверка Ynison: подключиться пультом-плеером на 20 с и показать устройства.
//! `cargo run --release -p yamusic-core --example ynison_probe -- <token.json>`

use std::time::Duration;

fn main() {
    tracing_subscriber::fmt().with_env_filter("yamusic_core=debug").init();
    let path = std::env::args().nth(1).expect("путь к token.json");
    let tok: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let token = tok["access_token"].as_str().unwrap().to_string();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let h = yamusic_core::ynison::spawn(token, "yamusicrs-probe".into(), "yamusicrs · проба".into(), "ANDROID", 0, |e| match e {
            yamusic_core::ynison::Event::Connected => println!("ПОДКЛЮЧЕНО"),
            yamusic_core::ynison::Event::Lost { error, retry_in } => {
                println!("ПОТЕРЯНО: {error} (повтор через {retry_in} с)");
                std::process::exit(0);
            }
            yamusic_core::ynison::Event::State(r) => {
                println!("активно: {:?}", r.active);
                for d in &r.devices {
                    println!("  {} [{}] {}", d.title, d.kind, d.id);
                }
                if let (Some(q), Some(s)) = (&r.queue, r.status) {
                    println!("  очередь {} {} — {}/{}; {:?}", q.entity_type, q.items.len(), q.index, s.progress_ms, s.paused);
                }
            }
        });
        tokio::time::sleep(Duration::from_secs(20)).await;
        drop(h);
    });
}

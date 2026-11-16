//! Системный диалог «Сохранить как» через xdg-desktop-portal (FileChooser.SaveFile):
//! окно рисует окружение (GNOME, KDE, wlr/hyprland-портал) поверх терминала.

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use zbus::{OwnedObjectPath, OwnedVariant, Proxy, Variant};

const DEST: &str = "org.freedesktop.portal.Desktop";

/// `Ok(None)` — пользователь закрыл диалог; `Err` — портала нет или он сломан.
pub async fn save_file(title: &str, name: &str, folder: &Path) -> Result<Option<PathBuf>, String> {
    inner(title, name, folder).await.map_err(|e| e.to_string())
}

async fn inner(title: &str, name: &str, folder: &Path) -> zbus::Result<Option<PathBuf>> {
    let conn = zbus::Connection::session().await?;
    // Подписка на ответ — до вызова, по предсказуемому пути запроса, иначе ответ можно пропустить.
    let token = format!("yamusicrs{}", rand::random::<u32>());
    let sender = conn
        .unique_name()
        .map(|n| n.as_str().trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default();
    let handle = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let request = Proxy::new(&conn, DEST, handle.as_str(), "org.freedesktop.portal.Request").await?;
    let mut responses = request.receive_signal("Response").await?;

    let chooser = Proxy::new(&conn, DEST, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.FileChooser").await?;
    let mut folder_bytes = folder.as_os_str().as_bytes().to_vec();
    folder_bytes.push(0);
    let mut opts: HashMap<&str, Variant> = HashMap::new();
    opts.insert("handle_token", Variant::from(token.as_str()));
    opts.insert("current_name", Variant::from(name));
    opts.insert("current_folder", Variant::from(folder_bytes));
    let _: OwnedObjectPath = chooser.call("SaveFile", &("", title, opts)).await?;

    let Some(msg) = responses.next().await else { return Ok(None) };
    let (code, results): (u32, HashMap<String, OwnedVariant>) = msg.body().deserialize()?;
    if code != 0 {
        return Ok(None);
    }
    let uris: Vec<String> = match results.get("uris") {
        Some(v) => Vec::<String>::try_from(v.try_clone()?)?,
        None => Vec::new(),
    };
    Ok(uris.first().and_then(|u| u.strip_prefix("file://")).map(|p| PathBuf::from(percent_decode(p))))
}

/// `file://`-URI портала кодирует пробелы и кириллицу как %XX.
fn percent_decode(s: &str) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    std::ffi::OsString::from_vec(out)
}

/// Переносит готовый файл: rename, а между разными ФС — копия и удаление.
pub fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst)?;
    std::fs::remove_file(src)
}

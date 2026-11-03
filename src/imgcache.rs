//! Дисковый кеш картинок: обложки, миниатюры, фото — всё, что хоть раз пришло из сети.
//! Без сети картинки берутся отсюда (или из тегов скачанных треков), онлайн — не качаются заново.

use std::path::{Path, PathBuf};

/// Больше — старые картинки удаляются до `PRUNE_TO`.
const MAX_BYTES: u64 = 256 << 20;
const PRUNE_TO: u64 = 192 << 20;

fn fnv1a(s: &str) -> u64 {
    s.bytes()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn path(dir: &Path, url: &str) -> PathBuf {
    dir.join("img").join(format!("{:016x}", fnv1a(url)))
}

pub fn read(dir: &Path, url: &str) -> Option<Vec<u8>> {
    std::fs::read(path(dir, url)).ok()
}

pub fn write(dir: &Path, url: &str, data: &[u8]) {
    let p = path(dir, url);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, data).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// Держит кеш в пределах: самые давно изменённые файлы — первыми на выход.
pub fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir.join("img")) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= MAX_BYTES {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= PRUNE_TO {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}

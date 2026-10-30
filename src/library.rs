//! Локальная библиотека: скачанные треки лежат как нормальные файлы
//! `Artist/Album (Year)/NN. Title.flac` с тегами и обложкой, индекс — в JSON.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use color_eyre::eyre::{Result, bail, eyre};
use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{ItemKey, Tag, TagExt, TagType};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::api::Track;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Путь относительно корня библиотеки.
    pub path: PathBuf,
    pub codec: String,
    pub size: u64,
    pub last_played: i64,
    /// Когда последний раз спрашивали сервер, нет ли версии лучше (unix-время).
    #[serde(default)]
    pub checked_at: i64,
    /// Для какого качества (ранг 1..=3) делалась последняя проверка.
    #[serde(default)]
    pub checked_rank: u8,
}

impl Entry {
    /// Стоит ли спросить сервер о версии получше при желаемом ранге `want`.
    pub fn wants_upgrade(&self, want: u8) -> bool {
        codec_rank(&self.codec) < want
            && (self.checked_rank < want || chrono::Utc::now().timestamp() - self.checked_at > RECHECK_SECS)
    }
}

/// Как часто перепроверять, не появилось ли у трека качество получше.
pub const RECHECK_SECS: i64 = 30 * 24 * 3600;

/// Ранг качества кодека: 3 — lossless, 2 — AAC 256/MP3 320, 1 — HE-AAC и прочее сжатое сильнее.
pub fn codec_rank(codec: &str) -> u8 {
    match codec {
        "flac" | "flac-mp4" => 3,
        "aac" | "aac-mp4" | "mp3" => 2,
        _ => 1,
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    tracks: HashMap<String, Entry>,
}

pub struct Library {
    root: PathBuf,
    index_path: PathBuf,
    index: Mutex<Index>,
    max_bytes: u64,
}

/// Готовый к сохранению трек.
pub struct Download {
    pub track: Track,
    /// Ранг качества, которое просили при скачивании.
    pub requested_rank: u8,
    pub codec: String,
    pub data: Vec<u8>,
    pub cover: Option<Vec<u8>>,
    pub lyrics: Option<String>,
}

impl Library {
    pub fn open(root: PathBuf, index_path: PathBuf, max_gb: f64) -> Arc<Self> {
        let index = std::fs::read(&index_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Arc::new(Self {
            root,
            index_path,
            index: Mutex::new(index),
            max_bytes: (max_gb * 1024.0 * 1024.0 * 1024.0) as u64,
        })
    }

    /// Скачанный трек, если файл на месте: абсолютный путь и запись индекса.
    pub fn lookup(&self, track_id: &str) -> Option<(PathBuf, Entry)> {
        let id = track_id.split(':').next().unwrap_or(track_id);
        let mut idx = self.index.lock();
        let e = idx.tracks.get_mut(id)?;
        let p = self.root.join(&e.path);
        if p.is_file() {
            e.last_played = chrono::Utc::now().timestamp();
            Some((p, e.clone()))
        } else {
            idx.tracks.remove(id);
            None
        }
    }

    /// Запоминает, что сервер лучшего качества не предложил.
    pub fn mark_checked(&self, track_id: &str, rank: u8) {
        let id = track_id.split(':').next().unwrap_or(track_id);
        if let Some(e) = self.index.lock().tracks.get_mut(id) {
            e.checked_at = chrono::Utc::now().timestamp();
            e.checked_rank = rank;
        }
        self.save_index();
    }

    pub fn contains(&self, track_id: &str) -> bool {
        let id = track_id.split(':').next().unwrap_or(track_id);
        self.index.lock().tracks.contains_key(id)
    }

    pub fn stats(&self) -> (usize, u64) {
        let idx = self.index.lock();
        (idx.tracks.len(), idx.tracks.values().map(|e| e.size).sum())
    }

    fn save_index(&self) {
        let data = serde_json::to_vec(&*self.index.lock()).unwrap_or_default();
        if let Some(dir) = self.index_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.index_path.with_extension("tmp");
        if std::fs::write(&tmp, data).is_ok() {
            let _ = std::fs::rename(tmp, &self.index_path);
        }
    }

    /// Сохраняет скачанный трек: ремукс в родной контейнер, теги, обложка, индекс.
    pub fn store(&self, dl: Download) -> Result<PathBuf> {
        let (bytes, ext, tag_type) = match dl.codec.as_str() {
            "flac-mp4" => (remux_flac(&dl.data)?, "flac", TagType::VorbisComments),
            "flac" => (dl.data, "flac", TagType::VorbisComments),
            "aac-mp4" | "he-aac-mp4" => (dl.data, "m4a", TagType::Mp4Ilst),
            "mp3" => (dl.data, "mp3", TagType::Id3v2),
            "aac" | "he-aac" => (dl.data, "aac", TagType::Id3v2),
            other => bail!("неизвестный кодек {other}"),
        };
        let rel = relative_path(&dl.track, ext);
        let path = self.root.join(&rel);
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        let tmp = path.with_extension(format!("{ext}.part"));
        std::fs::write(&tmp, &bytes)?;
        if ext != "aac" {
            let tag = build_tag(&dl.track, tag_type, dl.cover, dl.lyrics);
            tag.save_to_path(&tmp, WriteOptions::default())
                .map_err(|e| eyre!("теги: {e}"))?;
        }
        std::fs::rename(&tmp, &path)?;
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(bytes.len() as u64);
        let now = chrono::Utc::now().timestamp();
        let old = self.index.lock().tracks.insert(
            dl.track.id.clone(),
            Entry {
                path: rel.clone(),
                codec: dl.codec,
                size,
                last_played: now,
                checked_at: now,
                checked_rank: dl.requested_rank,
            },
        );
        // Апгрейд качества: старый файл (например, .m4a вместо нового .flac) больше не нужен.
        if let Some(old) = old
            && old.path != rel
        {
            let _ = std::fs::remove_file(self.root.join(&old.path));
        }
        self.evict();
        self.save_index();
        Ok(path)
    }

    /// Удаляет давно не игранное, пока библиотека не влезет в лимит.
    fn evict(&self) {
        if self.max_bytes == 0 {
            return;
        }
        let mut idx = self.index.lock();
        let mut total: u64 = idx.tracks.values().map(|e| e.size).sum();
        if total <= self.max_bytes {
            return;
        }
        let mut by_age: Vec<(String, i64, u64, PathBuf)> = idx
            .tracks
            .iter()
            .map(|(k, e)| (k.clone(), e.last_played, e.size, e.path.clone()))
            .collect();
        by_age.sort_by_key(|e| e.1);
        for (id, _, size, rel) in by_age {
            if total <= self.max_bytes {
                break;
            }
            let p = self.root.join(&rel);
            let _ = std::fs::remove_file(&p);
            // Чистим опустевшие папки альбома и артиста.
            let mut dir = p.parent();
            while let Some(d) = dir {
                if d == self.root || std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
            idx.tracks.remove(&id);
            total -= size;
        }
    }
}

/// Имена файлов: без слэшей и прочего, что ломает ФС или шеллы.
pub fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '/' => '∕',
            '\\' => '⧵',
            ':' => '꞉',
            '*' => '∗',
            '?' => '？',
            '"' => '\'',
            '<' => '‹',
            '>' => '›',
            '|' => '∣',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    out = out.trim().trim_end_matches('.').trim().to_string();
    if out.is_empty() {
        out = "_".into();
    }
    // Длинные имена режем по символам, чтобы не превысить 255 байт.
    while out.len() > 180 {
        out.pop();
    }
    out
}

pub fn relative_path(t: &Track, ext: &str) -> PathBuf {
    let album = t.album();
    let artist = album
        .filter(|a| !a.artists.is_empty())
        .map(|a| a.artist_names())
        .unwrap_or_else(|| {
            t.artists
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown Artist".into())
        });
    let album_dir = match album {
        Some(a) => match a.year {
            Some(y) => format!("{} ({y})", a.full_title()),
            None => a.full_title(),
        },
        None => "Singles".into(),
    };
    let pos = album.and_then(|a| a.track_position).unwrap_or_default();
    let prefix = match (pos.volume, pos.index) {
        (_, 0) => String::new(),
        (v, i) if v > 1 => format!("{v}-{i:02}. "),
        (_, i) => format!("{i:02}. "),
    };
    let file = format!("{prefix}{}.{ext}", sanitize(&t.full_title()));
    PathBuf::from(sanitize(&artist)).join(sanitize(&album_dir)).join(file)
}

fn build_tag(t: &Track, tt: TagType, cover: Option<Vec<u8>>, lyrics: Option<String>) -> Tag {
    let mut tag = Tag::new(tt);
    tag.insert_text(ItemKey::TrackTitle, t.full_title());
    tag.insert_text(ItemKey::TrackArtist, t.artist_names());
    for a in &t.artists {
        tag.push(lofty::tag::TagItem::new(
            ItemKey::TrackArtists,
            lofty::tag::ItemValue::Text(a.name.clone()),
        ));
    }
    if let Some(a) = t.album() {
        tag.insert_text(ItemKey::AlbumTitle, a.full_title());
        if !a.artists.is_empty() {
            tag.insert_text(ItemKey::AlbumArtist, a.artist_names());
        }
        // DATE: полная дата релиза, если есть, иначе год.
        let date = a
            .release_date
            .as_deref()
            .and_then(|d| d.get(..10))
            .map(str::to_owned)
            .or(a.year.map(|y| y.to_string()));
        if let Some(d) = date {
            tag.insert_text(ItemKey::RecordingDate, d);
        }
        if let Some(g) = &a.genre {
            tag.insert_text(ItemKey::Genre, g.clone());
        }
        if let Some(p) = a.track_position {
            if p.index > 0 {
                tag.insert_text(ItemKey::TrackNumber, p.index.to_string());
            }
            if p.volume > 0 {
                tag.insert_text(ItemKey::DiscNumber, p.volume.to_string());
            }
        }
        if let Some(n) = a.track_count {
            tag.insert_text(ItemKey::TrackTotal, n.to_string());
        }
    }
    if let Some(r) = t.r128 {
        // ReplayGain по R128 от Яндекса (референс RG2 = -18 LUFS).
        tag.insert_text(ItemKey::ReplayGainTrackGain, format!("{:.2} dB", -18.0 - r.i));
        tag.insert_text(ItemKey::ReplayGainTrackPeak, format!("{:.6}", 10f32.powf(r.tp / 20.0)));
    }
    tag.insert_text(ItemKey::Comment, format!("https://music.yandex.ru/track/{}", t.id));
    if let Some(l) = lyrics {
        tag.insert_text(ItemKey::Lyrics, l);
    }
    if let Some(c) = cover {
        let mime = if c.starts_with(&[0x89, b'P', b'N', b'G']) {
            MimeType::Png
        } else {
            MimeType::Jpeg
        };
        tag.push_picture(
            Picture::unchecked(c)
                .pic_type(PictureType::CoverFront)
                .mime_type(mime)
                .build(),
        );
    }
    tag
}

/// Интервал точек SEEKTABLE, секунды (так же делает референсный `flac`).
const SEEK_POINT_SECS: u64 = 10;

/// FLAC в MP4 → нативный FLAC: `fLaC` + STREAMINFO из `dfLa` + SEEKTABLE + кадры как есть.
/// Перекодирования нет, аудиоданные побитово те же.
pub fn remux_flac(mp4: &[u8]) -> Result<Vec<u8>> {
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mss = MediaSourceStream::new(Box::new(Cursor::new(mp4.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp4");
    let mut reader =
        symphonia::default::get_probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or_else(|| eyre!("нет аудиодорожки"))?;
    let tid = track.id;
    let si = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .and_then(|p| p.extra_data.clone())
        .ok_or_else(|| eyre!("нет STREAMINFO"))?;
    if si.len() != 34 {
        bail!("STREAMINFO неожиданной длины {}", si.len());
    }
    let rate = streaminfo_rate(&si);
    if rate == 0 {
        bail!("STREAMINFO: нулевая частота");
    }
    let tb = track.time_base;
    // Перевод времени контейнера (тики timescale MP4) в номера сэмплов.
    let to_samples = |ticks: u64| -> u64 {
        match tb {
            Some(tb) => (ticks as u128 * rate as u128 * tb.numer.get() as u128 / tb.denom.get() as u128) as u64,
            None => ticks,
        }
    };

    let mut frames = Vec::with_capacity(mp4.len());
    let mut points: Vec<SeekPoint> = Vec::new();
    let mut next_point = 0u64;
    let mut sample = 0u64;
    while let Some(p) = reader.next_packet()? {
        if p.track_id != tid {
            continue;
        }
        let pts = u64::try_from(p.pts.get()).map(to_samples).unwrap_or(sample);
        let dur = to_samples(p.dur.get()) as u16;
        if pts >= next_point {
            points.push(SeekPoint {
                sample: pts,
                offset: frames.len() as u64,
                frame_samples: dur,
            });
            next_point = (pts / (rate as u64 * SEEK_POINT_SECS) + 1) * rate as u64 * SEEK_POINT_SECS;
        }
        frames.extend_from_slice(&p.data);
        sample = pts + dur as u64;
    }

    let seek_len = points.len() * 18;
    let mut out = Vec::with_capacity(4 + 4 + 34 + 4 + seek_len + frames.len());
    out.extend_from_slice(b"fLaC");
    out.extend_from_slice(&[0x00, 0, 0, 34]); // STREAMINFO, не последний блок
    out.extend_from_slice(&si);
    let len = seek_len as u32;
    out.extend_from_slice(&[0x80 | 3, (len >> 16) as u8, (len >> 8) as u8, len as u8]); // последний | SEEKTABLE
    for sp in &points {
        out.extend_from_slice(&sp.sample.to_be_bytes());
        out.extend_from_slice(&sp.offset.to_be_bytes());
        out.extend_from_slice(&sp.frame_samples.to_be_bytes());
    }
    out.extend_from_slice(&frames);
    Ok(out)
}

struct SeekPoint {
    /// Номер первого сэмпла кадра.
    sample: u64,
    /// Смещение кадра от начала первого кадра (после метаданных).
    offset: u64,
    frame_samples: u16,
}

/// Частота дискретизации из STREAMINFO: 20 бит начиная с 10-го байта.
fn streaminfo_rate(si: &[u8]) -> u32 {
    (u32::from(si[10]) << 12) | (u32::from(si[11]) << 4) | (u32::from(si[12]) >> 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("AC/DC"), "AC∕DC");
        assert_eq!(sanitize("  What?.. "), "What？");
        assert_eq!(sanitize(""), "_");
    }
}

#[cfg(test)]
mod remux_tests {
    /// `YAMUSICRS_REMUX_IN=x.mp4 YAMUSICRS_REMUX_OUT=x.flac cargo test remux_file -- --ignored`
    #[test]
    #[ignore]
    fn remux_file() {
        let input = std::env::var("YAMUSICRS_REMUX_IN").unwrap();
        let out = std::env::var("YAMUSICRS_REMUX_OUT").unwrap();
        let data = std::fs::read(input).unwrap();
        std::fs::write(&out, super::remux_flac(&data).unwrap()).unwrap();
        let track: crate::api::Track = serde_json::from_value(serde_json::json!({
            "id": 1, "title": "Тест/Трек", "artists": [{"id": 2, "name": "Артист"}],
            "albums": [{"id": 3, "title": "Альбом", "year": 2024, "trackPosition": {"volume": 1, "index": 7}}],
            "r128": {"i": -9.5, "tp": 0.5}
        }))
        .unwrap();
        let cover = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0];
        use lofty::tag::TagExt;
        super::build_tag(
            &track,
            lofty::tag::TagType::VorbisComments,
            Some(cover),
            Some("ла-ла".into()),
        )
        .save_to_path(&out, lofty::config::WriteOptions::default())
        .unwrap();
        assert_eq!(
            super::relative_path(&track, "flac"),
            std::path::PathBuf::from("Артист/Альбом (2024)/07. Тест∕Трек.flac")
        );
    }
}

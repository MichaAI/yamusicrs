//! Экспорт трека обычным файлом: в любом формате, который отдаёт сервер (без пережатия),
//! или распакованным в WAV.

use std::io::Write;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail, eyre};

use crate::api::{Quality, Track};
use crate::library::{self, Download};

/// Во что экспортировать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Без потерь, как на сервере.
    Flac,
    /// MP3 320 кбит/с — играет где угодно.
    Mp3,
    /// AAC 256 кбит/с в M4A.
    Aac,
    /// HE-AAC 64 кбит/с — самый маленький файл.
    Small,
    /// Несжатый PCM: распаковка лучшего, что есть у сервера.
    Wav,
}

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "flac" => Self::Flac,
            "mp3" => Self::Mp3,
            "aac" => Self::Aac,
            "small" => Self::Small,
            "wav" => Self::Wav,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Flac => "FLAC",
            Self::Mp3 => "MP3 320",
            Self::Aac => "AAC 256",
            Self::Small => "HE-AAC 64",
            Self::Wav => "WAV",
        }
    }

    /// Что просить у `get-file-info`: качество и допустимые кодеки.
    pub fn request(self) -> (Quality, &'static [&'static str]) {
        match self {
            Self::Flac | Self::Wav => (Quality::Lossless, &["flac", "flac-mp4"]),
            Self::Mp3 => (Quality::Hq, &["mp3"]),
            Self::Aac => (Quality::Hq, &["aac-mp4", "aac"]),
            Self::Small => (Quality::Nq, &["he-aac-mp4", "he-aac"]),
        }
    }

    /// Сервер дал то, что просили (а не запасной вариант похуже).
    pub fn accepts(self, codec: &str) -> bool {
        match self {
            // WAV распаковывается из чего угодно: lossless у трека может и не быть.
            Self::Wav => true,
            f => f.request().1.contains(&codec),
        }
    }
}

/// «Исполнитель — Название» для имени файла.
pub fn file_stem(t: &Track) -> String {
    let artists = t.artist_names();
    library::sanitize(&if artists.is_empty() { t.full_title() } else { format!("{artists} — {}", t.full_title()) })
}

/// Кладёт скачанный трек в `dir` готовым файлом (родной контейнер, теги, обложка) или WAV.
pub fn write(dir: &Path, fmt: Format, dl: Download) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    if fmt != Format::Wav {
        return library::write_tagged(dir, &file_stem(&dl.track), dl);
    }
    let stem = file_stem(&dl.track);
    let ext = match dl.codec.as_str() {
        "flac" | "flac-mp4" | "aac-mp4" | "he-aac-mp4" => "mp4",
        "mp3" => "mp3",
        _ => "aac",
    };
    let ext = if dl.codec == "flac" { "flac" } else { ext };
    let path = dir.join(format!("{stem}.wav"));
    let tmp = dir.join(format!("{stem}.wav.part"));
    decode_to_wav(dl.data, ext, &tmp)?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Распаковывает аудио в WAV: 24 бита для lossless глубже 16 бит, иначе 16.
fn decode_to_wav(data: Vec<u8>, ext: &str, out: &Path) -> Result<()> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SymError;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(data)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let mut reader = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| eyre!("формат: {e}"))?;
    let track = reader.default_track(TrackType::Audio).ok_or_else(|| eyre!("нет аудиодорожки"))?;
    let tid = track.id;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or_else(|| eyre!("нет параметров кодека"))?.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| eyre!("декодер: {e}"))?;
    let rate = params.sample_rate.ok_or_else(|| eyre!("неизвестна частота"))?;
    let ch = params.channels.as_ref().map(|c| c.count() as u16).unwrap_or(2);
    let bytes: u16 = if params.bits_per_sample.is_some_and(|b| b > 16) { 3 } else { 2 };

    let mut f = std::io::BufWriter::new(std::fs::File::create(out)?);
    // Заголовок с нулевыми длинами — впишем в конце.
    f.write_all(&[0u8; 44])?;
    let mut samples: Vec<f32> = Vec::new();
    let mut total: u64 = 0;
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => bail!("чтение: {e}"),
        };
        if packet.track_id != tid {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => bail!("декодер: {e}"),
        };
        samples.resize(buf.samples_interleaved(), 0.0);
        buf.copy_to_slice_interleaved(&mut samples[..]);
        for s in &samples {
            let s = s.clamp(-1.0, 1.0) as f64;
            if bytes == 3 {
                let v = (s * 8_388_607.0).round() as i32;
                f.write_all(&v.to_le_bytes()[..3])?;
            } else {
                f.write_all(&((s * 32_767.0).round() as i16).to_le_bytes())?;
            }
        }
        total += samples.len() as u64 * bytes as u64;
    }
    if total == 0 {
        bail!("в файле нет звука");
    }
    let data_len = u32::try_from(total).map_err(|_| eyre!("трек слишком длинный для WAV"))?;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data_len).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&ch.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * ch as u32 * bytes as u32).to_le_bytes());
    h.extend_from_slice(&(ch * bytes).to_le_bytes());
    h.extend_from_slice(&(bytes * 8).to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_len.to_le_bytes());
    let mut file = f.into_inner().map_err(|e| eyre!("запись: {e}"))?;
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(&h)?;
    Ok(())
}

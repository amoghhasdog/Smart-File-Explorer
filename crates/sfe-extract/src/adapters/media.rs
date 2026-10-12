//! Audio/video container metadata only (duration, resolution). No decoding
//! of frames or samples; no ffmpeg.

use std::io::{Read, Seek, SeekFrom};

use sfe_core::FileMeta;

use crate::{ExtractError, ExtractInput, Extracted, Extractor, FileProbe, Limits, Result, Support};

pub struct Media;

const MP4_EXTS: &[&str] = &["mp4", "m4v", "m4a", "mov", "3gp"];
const MKV_EXTS: &[&str] = &["mkv", "webm", "mka"];
const AUDIO_EXTS: &[&str] = &["mp3", "flac", "wav", "ogg", "oga", "opus", "aac", "aiff"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Mp4,
    Matroska,
    Symphonia,
}

fn container(probe: &FileProbe<'_>) -> Option<(Container, Support)> {
    if let Some(m) = probe.mime {
        let c = match m {
            "video/mp4" | "audio/m4a" | "video/quicktime" | "audio/x-m4a" => Container::Mp4,
            "video/x-matroska" | "video/webm" | "audio/webm" => Container::Matroska,
            "audio/mpeg" | "audio/x-flac" | "audio/x-wav" | "audio/ogg" | "audio/aac"
            | "audio/x-aiff" => Container::Symphonia,
            _ => return None,
        };
        return Some((c, Support::ByMagic));
    }
    let c = if MP4_EXTS.contains(&probe.ext) {
        Container::Mp4
    } else if MKV_EXTS.contains(&probe.ext) {
        Container::Matroska
    } else if AUDIO_EXTS.contains(&probe.ext) {
        Container::Symphonia
    } else {
        return None;
    };
    Some((c, Support::ByExt))
}

impl Extractor for Media {
    fn id(&self) -> &'static str {
        "media"
    }

    fn version(&self) -> u32 {
        1
    }

    fn supports(&self, probe: &FileProbe<'_>) -> Support {
        container(probe).map(|(_, s)| s).unwrap_or(Support::No)
    }

    fn extract(&self, input: &mut dyn ExtractInput, _limits: &Limits) -> Result<Extracted> {
        let mut head = [0u8; 16];
        input.seek(SeekFrom::Start(0))?;
        let n = input.read(&mut head)?;
        let head = &head[..n];
        // Decide by content, falling back through parsers.
        let kind = if head.len() >= 8 && &head[4..8] == b"ftyp" {
            Container::Mp4
        } else if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
            Container::Matroska
        } else {
            Container::Symphonia
        };
        input.seek(SeekFrom::Start(0))?;
        let meta = match kind {
            Container::Mp4 => mp4_meta(input)?,
            Container::Matroska => mkv_meta(input)?,
            Container::Symphonia => audio_meta(input)?,
        };
        Ok(Extracted {
            text: None,
            meta,
            truncated: false,
        })
    }
}

fn mp4_meta(input: &mut dyn ExtractInput) -> Result<FileMeta> {
    let size = input.size();
    let reader = mp4::Mp4Reader::read_header(ReadSeek(input), size)
        .map_err(|e| ExtractError::Parse(format!("mp4: {e}")))?;
    let mut meta = FileMeta {
        duration_ms: Some(reader.duration().as_millis() as u64),
        ..Default::default()
    };
    for track in reader.tracks().values() {
        if track.track_type().ok() == Some(mp4::TrackType::Video) {
            meta.width = Some(track.width() as u32);
            meta.height = Some(track.height() as u32);
            break;
        }
    }
    Ok(meta)
}

fn mkv_meta(input: &mut dyn ExtractInput) -> Result<FileMeta> {
    let m = matroska::Matroska::open(ReadSeek(input))
        .map_err(|e| ExtractError::Parse(format!("matroska: {e}")))?;
    let mut meta = FileMeta {
        duration_ms: m.info.duration.map(|d| d.as_millis() as u64),
        ..Default::default()
    };
    for t in &m.tracks {
        if let matroska::Settings::Video(v) = &t.settings {
            meta.width = Some(v.pixel_width as u32);
            meta.height = Some(v.pixel_height as u32);
            break;
        }
    }
    Ok(meta)
}

fn audio_meta(input: &mut dyn ExtractInput) -> Result<FileMeta> {
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::probe::Hint;
    let bytes = input.read_all()?;
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &Default::default(), &Default::default())
        .map_err(|e| ExtractError::Parse(format!("audio: {e}")))?;
    let track = probed
        .format
        .default_track()
        .ok_or(ExtractError::Unsupported("no audio track"))?;
    let p = &track.codec_params;
    let duration_ms = match (p.n_frames, p.sample_rate) {
        (Some(frames), Some(rate)) if rate > 0 => Some(frames * 1000 / rate as u64),
        _ => None,
    };
    Ok(FileMeta {
        duration_ms,
        ..Default::default()
    })
}

struct ReadSeek<'a>(&'a mut dyn ExtractInput);

impl Read for ReadSeek<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for ReadSeek<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemInput;

    /// 16-bit mono PCM WAV of `seconds` at 8 kHz.
    fn wav(seconds: u32) -> Vec<u8> {
        let rate = 8000u32;
        let samples = rate * seconds;
        let data_len = samples * 2;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.resize(out.len() + data_len as usize, 0);
        out
    }

    #[test]
    fn wav_duration() {
        let e = Media
            .extract(&mut MemInput::new(wav(3)), &Limits::default())
            .unwrap();
        assert_eq!(e.meta.duration_ms, Some(3000));
        assert!(e.meta.width.is_none());
    }

    #[test]
    fn garbage_is_a_parse_error_not_a_panic() {
        let err = Media
            .extract(
                &mut MemInput::new(b"\0\0\0\x18ftypgarbage".to_vec()),
                &Limits::default(),
            )
            .unwrap_err();
        assert!(matches!(err, ExtractError::Parse(_)), "{err}");
        let err = Media
            .extract(
                &mut MemInput::new(b"random bytes here".to_vec()),
                &Limits::default(),
            )
            .unwrap_err();
        assert!(matches!(err, ExtractError::Parse(_)), "{err}");
    }

    #[test]
    fn support_by_mime_and_ext() {
        let p = |ext, mime| FileProbe { ext, mime, size: 1 };
        assert_eq!(
            Media.supports(&p("bin", Some("video/mp4"))),
            Support::ByMagic
        );
        assert_eq!(Media.supports(&p("mkv", None)), Support::ByExt);
        assert_eq!(Media.supports(&p("mp3", None)), Support::ByExt);
        assert_eq!(Media.supports(&p("txt", None)), Support::No);
    }
}

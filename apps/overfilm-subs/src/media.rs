//! Reading a video or audio file's sound as 16 kHz mono with filmcraft's decoders (MP4/MOV,
//! MKV/WebM, MXF, Ogg, MPEG TS/PS, AVI, MP3, AAC, FLAC, WAV…).

use std::path::Path;
use std::sync::Arc;

use filmcraft_media::reader::FileReader;

/// Decoded in pieces of this many seconds.
const BLOCK_SECONDS: i64 = 60;
/// Longest input accepted (6 hours of 16 kHz mono f32 is 1.4 GB).
const MAX_SECONDS: i64 = 6 * 3600;

/// The file's first audio stream as 16 kHz mono; `progress(fraction)` returns false to cancel.
pub fn decode(path: &Path, progress: &mut dyn FnMut(f32) -> bool) -> Result<Vec<f32>, String> {
    decode_span(path, 0, None, progress)
}

fn open(path: &Path) -> Result<(String, filmcraft_media::SharedSource), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let reader = FileReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let src = filmcraft_media::reader::open_reader(&name, Arc::new(reader), &filmcraft_codecs::reader_openers(), &filmcraft_codecs::openers())
        .map_err(|e| format!("{name}: 열 수 없는 형식입니다 ({e})"))?;
    Ok((name, src))
}

/// The file's audio tracks (streams) as the user sees them: "트랙 1 · 스테레오 · 48 kHz · aac".
/// OBS and game capture often record the microphone and the game on separate tracks.
pub fn audio_tracks(path: &Path) -> Result<Vec<String>, String> {
    let (_, src) = open(path)?;
    Ok(src.info().audio_streams.iter().enumerate().map(|(i, a)| track_label(i, a)).collect())
}

/// Whether the sound is inside a video (its only audio track) or separate (one of several
/// tracks, or a sound-only file): see [`overfilm_pipeline::recognise::skip_silence_default`].
pub fn sound_source(path: &Path) -> Result<overfilm_pipeline::recognise::SoundSource, String> {
    let (_, src) = open(path)?;
    let info = src.info();
    Ok(overfilm_pipeline::recognise::SoundSource::of(info.has_video(), info.audio_streams.len()))
}

fn track_label(i: usize, a: &filmcraft_media::AudioStreamInfo) -> String {
    let ch = match a.channels {
        1 => "모노".to_string(),
        2 => "스테레오".to_string(),
        n => format!("{n}채널"),
    };
    let khz = f64::from(a.sample_rate) / 1000.0;
    let codec = if a.codec.is_empty() { String::new() } else { format!(" · {}", a.codec) };
    format!("트랙 {} · {ch} · {khz} kHz{codec}", i.saturating_add(1))
}

/// Audio stream `stream` (0 = the first; see [`audio_tracks`]) of `[from_ms, to_ms)` only
/// (clamped to the file), or of everything with `None`, as 16 kHz mono.
pub fn decode_span(path: &Path, stream: usize, span: Option<(i64, i64)>, progress: &mut dyn FnMut(f32) -> bool) -> Result<Vec<f32>, String> {
    let mut out = Vec::new();
    for_each_block(path, stream, span, &mut |block, f| {
        out.extend_from_slice(block);
        progress(f)
    })?;
    Ok(out)
}

/// [`decode_span`] handed over a block (up to a minute) at a time, so long files need no buffer
/// of their own; `each(block, fraction)` returns false to cancel.
pub fn for_each_block(path: &Path, stream: usize, span: Option<(i64, i64)>, each: &mut dyn FnMut(&[f32], f32) -> bool) -> Result<(), String> {
    let (name, src) = open(path)?;
    if !src.info().has_audio() {
        return Err(format!("{name}: 오디오 트랙이 없습니다"));
    }
    let tracks = src.info().audio_streams.len();
    if stream >= tracks {
        return Err(format!("{name}: 오디오 트랙 {}이(가) 없습니다 (트랙 {tracks}개)", stream.saturating_add(1)));
    }
    let sr = overfilm_asr::SAMPLE_RATE;
    let rate = i64::from(sr);
    let mut total = src.info().duration.to_units_floor(rate).clamp(0, MAX_SECONDS * rate);
    let mut pos = 0i64;
    if let Some((a, b)) = span {
        let per_ms = rate / 1000;
        pos = a.max(0).saturating_mul(per_ms).min(total);
        total = b.max(0).saturating_mul(per_ms).clamp(pos, total);
    }
    let first = pos;
    let block = BLOCK_SECONDS * rate;
    while pos < total {
        let n = block.min(total - pos);
        let buf = src.audio_stream(stream, pos, usize::try_from(n).unwrap_or(0), sr).map_err(|e| format!("{name}: 소리를 읽지 못했습니다 ({e})"))?;
        let mono = mono(&buf.channels);
        if mono.is_empty() {
            break;
        }
        pos += n;
        if !each(&mono, (pos - first) as f32 / (total - first).max(1) as f32) {
            return Err("취소했습니다".into());
        }
    }
    Ok(())
}

fn mono(channels: &[Vec<f32>]) -> Vec<f32> {
    let n = channels.iter().map(Vec::len).max().unwrap_or(0);
    let k = channels.len().max(1) as f32;
    (0..n).map(|i| channels.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() / k).map(|v| v.clamp(-1.0, 1.0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_wav() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.wav");
        // 1 s of 8 kHz stereo 16-bit
        let frames = 8000u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + frames * 4).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes());
        b.extend_from_slice(&32000u32.to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(frames * 4).to_le_bytes());
        for i in 0..frames {
            let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&p, b).unwrap();
        let a = decode(&p, &mut |_| true).unwrap();
        assert!((15_000..=16_100).contains(&a.len()), "{}", a.len());
        assert!(a.iter().any(|v| v.abs() > 0.1));
        let tracks = audio_tracks(&p).unwrap();
        assert_eq!(tracks.len(), 1);
        assert!(tracks[0].starts_with("트랙 1 · 스테레오 · 8 kHz"), "{tracks:?}");
        // a track the file does not have is an error, not a crash
        assert!(decode_span(&p, 1, None, &mut |_| true).unwrap_err().contains("트랙 2"));
        assert!(decode_span(&p, usize::MAX, None, &mut |_| true).is_err());
    }

    #[test]
    fn track_labels_read_plainly() {
        let a = |channels, codec: &str| filmcraft_media::AudioStreamInfo { sample_rate: 48_000, channels, codec: codec.into(), bits_per_sample: None };
        assert_eq!(track_label(0, &a(2, "aac")), "트랙 1 · 스테레오 · 48 kHz · aac");
        assert_eq!(track_label(1, &a(1, "")), "트랙 2 · 모노 · 48 kHz");
        assert_eq!(track_label(5, &a(6, "opus")), "트랙 6 · 6채널 · 48 kHz · opus");
    }

    #[test]
    fn rejects_non_media() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.mp4");
        std::fs::write(&p, b"not a video").unwrap();
        assert!(decode(&p, &mut |_| true).is_err());
        assert!(decode(&d.path().join("missing.mp3"), &mut |_| true).is_err());
    }
}

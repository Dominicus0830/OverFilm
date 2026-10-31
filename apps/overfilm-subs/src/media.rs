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
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let reader = FileReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let src = filmcraft_media::reader::open_reader(&name, Arc::new(reader), &filmcraft_codecs::reader_openers(), &filmcraft_codecs::openers())
        .map_err(|e| format!("{name}: 열 수 없는 형식입니다 ({e})"))?;
    if !src.info().has_audio() {
        return Err(format!("{name}: 오디오 트랙이 없습니다"));
    }
    let sr = overfilm_asr::SAMPLE_RATE;
    let rate = i64::from(sr);
    let total = src.info().duration.to_units_floor(rate).clamp(0, MAX_SECONDS * rate);
    let block = BLOCK_SECONDS * rate;
    let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    let mut pos = 0i64;
    while pos < total {
        let n = block.min(total - pos);
        let buf = src.audio(pos, usize::try_from(n).unwrap_or(0), sr).map_err(|e| format!("{name}: 소리를 읽지 못했습니다 ({e})"))?;
        let mono = mono(&buf.channels);
        if mono.is_empty() {
            break;
        }
        out.extend(mono);
        pos += n;
        if !progress(pos as f32 / total.max(1) as f32) {
            return Err("취소했습니다".into());
        }
    }
    Ok(out)
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

//! The review screen's waveform: loudness peaks of the whole file (100 per second), and short
//! stretches of sound for listening back.

use std::path::{Path, PathBuf};

use crate::media;

/// Peaks per second of sound.
pub const PER_SECOND: i64 = 100;
/// Samples (16 kHz) behind one peak.
const BUCKET: usize = (overfilm_asr::SAMPLE_RATE as usize) / PER_SECOND as usize;
/// Sound kept in memory for listening (16-bit, 16 kHz): up to 2 hours, about 230 MB.
const KEEP_SAMPLES: usize = 2 * 3600 * overfilm_asr::SAMPLE_RATE as usize;
/// Longest stretch played at once.
pub const MAX_PLAY_MS: i64 = 120_000;

/// A file's waveform, and its sound for listening when it was short enough to keep.
#[derive(Debug, Default)]
pub struct Sound {
    /// The loudest sample in each 10 ms, as 0–255.
    pub peaks: Vec<u8>,
    /// 16 kHz mono, `None` for files longer than two hours (read again when played).
    pub samples: Option<Vec<i16>>,
}

/// The loudest sample in each 10 ms, as 0–255.
pub fn peaks(samples: &[f32]) -> Vec<u8> {
    samples.chunks(BUCKET).map(|c| (c.iter().fold(0.0_f32, |m, v| m.max(v.abs())).min(1.0) * 255.0).round() as u8).collect()
}

/// Read the whole file once: peaks, and the sound when it is not too long.
/// `progress(fraction)` returns false to stop.
pub fn load(path: &Path, progress: &mut dyn FnMut(f32) -> bool) -> Result<Sound, String> {
    let mut sound = Sound { peaks: Vec::new(), samples: Some(Vec::new()) };
    // blocks are whole minutes, so 10 ms buckets never straddle two of them
    media::for_each_block(path, 0, None, &mut |block, f| {
        sound.peaks.extend(peaks(block));
        if let Some(s) = sound.samples.as_mut() {
            if s.len().saturating_add(block.len()) > KEEP_SAMPLES {
                sound.samples = None;
            } else {
                s.extend(block.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16));
            }
        }
        progress(f)
    })?;
    Ok(sound)
}

/// 16 kHz mono sound of `[from_ms, to_ms)` (at most [`MAX_PLAY_MS`]): from `kept` when the
/// sound was kept, else read from the file.
pub fn stretch(path: &Path, kept: Option<&[i16]>, from_ms: i64, to_ms: i64) -> Result<Vec<f32>, String> {
    let from = from_ms.max(0);
    let to = to_ms.clamp(from, from.saturating_add(MAX_PLAY_MS));
    if let Some(k) = kept {
        let at = |ms: i64| usize::try_from(ms.saturating_mul(16)).unwrap_or(0).min(k.len());
        return Ok(k.get(at(from)..at(to)).unwrap_or_default().iter().map(|&v| f32::from(v) / 32768.0).collect());
    }
    media::decode_span(path, 0, Some((from, to)), &mut |_| true)
}

/// The video or audio a result was made from: `input` when it is there, else a media file beside
/// the SRT with the same name (`이름.mp4` for `이름.srt`, also `이름.화자1.srt`).
pub fn source(srt: &Path, input: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = input
        && p.is_file()
        && !is_caption(p)
    {
        return Some(p.to_path_buf());
    }
    let dir = srt.parent()?;
    let stem = srt.file_stem()?.to_string_lossy().into_owned();
    let mut stems = vec![stem.clone()];
    if let Some((base, _)) = stem.rsplit_once('.') {
        stems.push(base.to_string());
    }
    for s in &stems {
        for ext in crate::MEDIA_EXTENSIONS {
            let p = dir.join(format!("{s}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn is_caption(p: &Path) -> bool {
    p.extension().is_some_and(|e| ["srt", "vtt", "ass", "txt", "json"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_are_per_ten_ms() {
        let mut s = vec![0.0_f32; 16_000];
        s[200] = -0.5;
        s[15_999] = 2.0;
        let p = peaks(&s);
        assert_eq!(p.len(), 100);
        assert_eq!(p[1], 128);
        assert_eq!(p[99], 255);
        assert_eq!(p[50], 0);
        assert!(peaks(&[]).is_empty());
    }

    #[test]
    fn finds_the_video_beside_the_captions() {
        let d = tempfile::tempdir().unwrap();
        let srt = d.path().join("경기.화자1.srt");
        std::fs::write(&srt, "").unwrap();
        assert_eq!(source(&srt, None), None);
        std::fs::write(d.path().join("경기.mkv"), b"x").unwrap();
        assert_eq!(source(&srt, None), Some(d.path().join("경기.mkv")));
        let other = d.path().join("원본.mp3");
        std::fs::write(&other, b"x").unwrap();
        assert_eq!(source(&srt, Some(&other)), Some(other));
        // an input that is itself captions, or gone, is not the sound
        assert_eq!(source(&srt, Some(&srt)), Some(d.path().join("경기.mkv")));
        assert_eq!(source(&srt, Some(&d.path().join("없음.mp4"))), Some(d.path().join("경기.mkv")));
    }

    #[test]
    fn loads_peaks_and_a_stretch_of_a_wav() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.wav");
        let rate = 16_000u32;
        let frames = rate * 3;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(frames * 2).to_le_bytes());
        for i in 0..frames {
            // silent first second, then a tone
            let v = if i < rate { 0 } else { ((i as f32 * 0.07).sin() * 16000.0) as i16 };
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&p, b).unwrap();
        let sound = load(&p, &mut |_| true).unwrap();
        let pk = &sound.peaks;
        assert!((295..=301).contains(&pk.len()), "{}", pk.len());
        assert!(pk[..95].iter().all(|&v| v < 3));
        assert!(pk[110..290].iter().all(|&v| v > 100));
        let kept = sound.samples.as_deref().unwrap();
        assert!((47_000..=48_100).contains(&kept.len()), "{}", kept.len());
        for k in [Some(kept), None] {
            let s = stretch(&p, k, 1500, 2000).unwrap();
            assert!((7_900..=8_100).contains(&s.len()), "{}", s.len());
            assert!(s.iter().any(|v| v.abs() > 0.3));
            assert!(stretch(&p, k, 2000, 1000).unwrap().is_empty());
        }
        // past the end is empty, not a crash
        assert!(stretch(&p, Some(kept), 9_000_000_000_000_000, i64::MAX).unwrap().is_empty());
    }
}

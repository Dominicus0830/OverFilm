//! Enrolled voices (see [`overfilm_pipeline::voices`]): the storage is shared with the editor;
//! enrolling from a sample runs here.

use std::path::Path;

pub use overfilm_pipeline::voices::*;

/// Longest sample used (seconds); more adds nothing.
pub const MAX_SAMPLE_SECONDS: u64 = 120;
/// Shortest useful sample (seconds).
pub const MIN_SAMPLE_SECONDS: u64 = 3;

/// "83", "1:23" or "1:02:03" (optionally with ",mmm" or ".mmm") as milliseconds.
pub fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    let (main, frac) = match s.split_once([',', '.']) {
        Some((m, f)) => (m, f),
        None => (s, ""),
    };
    let mut secs: u64 = 0;
    let parts: Vec<&str> = main.split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for p in parts {
        secs = secs.checked_mul(60)?.checked_add(p.parse::<u64>().ok()?)?;
    }
    let ms = if frac.is_empty() {
        0
    } else {
        let f: String = frac.chars().chain("000".chars()).take(3).collect();
        f.parse::<u64>().ok()?
    };
    secs.checked_mul(1000)?.checked_add(ms)
}

/// Make and save a voice from `file` (video or audio), optionally only `[from, to)` ms of it.
pub fn enroll(name: &str, file: &Path, from_ms: Option<u64>, to_ms: Option<u64>) -> Result<Voice, String> {
    let name = clean_name(name)?;
    let models = crate::job::models_dir();
    let m = &overfilm_asr::catalogue::SPEAKER;
    if !m.installed(&models) {
        overfilm_asr::catalogue::download(&models, m, &mut |_, _, _| true).map_err(|e| format!("화자 구별 모델 다운로드 실패: {e}"))?;
    }
    let audio = crate::media::decode(file, &mut |_| true)?;
    let per_ms = overfilm_asr::SAMPLE_RATE as usize / 1000;
    let a = from_ms.map_or(0, |t| usize::try_from(t).unwrap_or(usize::MAX).saturating_mul(per_ms)).min(audio.len());
    let max_len = usize::try_from(MAX_SAMPLE_SECONDS).unwrap_or(120) * overfilm_asr::SAMPLE_RATE as usize;
    let b = to_ms.map_or(audio.len(), |t| usize::try_from(t).unwrap_or(usize::MAX).saturating_mul(per_ms)).min(audio.len()).min(a.saturating_add(max_len));
    let sample = audio.get(a..b).unwrap_or(&[]);
    let seconds = sample.len() as f32 / overfilm_asr::SAMPLE_RATE as f32;
    if seconds < MIN_SAMPLE_SECONDS as f32 {
        return Err(format!("목소리 샘플이 너무 짧습니다 ({seconds:.1}초). {MIN_SAMPLE_SECONDS}초 이상, 10~30초를 권합니다."));
    }
    let d = overfilm_asr::speakers::Diarizer::load(&m.dir(&models)).map_err(|e| format!("화자 구별 모델: {e}"))?;
    let vector = d.profile(sample).map_err(|e| e.to_string())?.ok_or("샘플에서 말소리를 찾지 못했습니다")?;
    let range = match (from_ms, to_ms) {
        (None, None) => String::new(),
        _ => format!(
            " {}~{}",
            from_ms.map(|t| crate::review::clock(i64::try_from(t).unwrap_or(i64::MAX))).unwrap_or_default(),
            to_ms.map(|t| crate::review::clock(i64::try_from(t).unwrap_or(i64::MAX))).unwrap_or_default()
        ),
    };
    let v = Voice { name, vector, seconds, source: format!("{}{range}", file.display()) };
    save_in(&dir(), &v)?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("83"), Some(83_000));
        assert_eq!(parse_time("1:23"), Some(83_000));
        assert_eq!(parse_time("1:02:03,5"), Some(3_723_500));
        assert_eq!(parse_time("x"), None);
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time("99999999999999999999"), None);
    }
}

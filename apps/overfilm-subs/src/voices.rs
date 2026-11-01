//! Enrolled voices: a person's voice saved once (from a sample of them speaking) so every later
//! extraction names them ("아이치" instead of "화자1") and can keep only their speech (a
//! streamer's own voice over game audio and other people). Saved as `voices/<name>.json` beside
//! the models folder.

use std::path::{Path, PathBuf};

use overfilm_asr::speakers::KnownVoice;
use serde::{Deserialize, Serialize};

/// Most saved voices read.
const MAX_VOICES: usize = 100;
/// Longest sample used (seconds); more adds nothing.
pub const MAX_SAMPLE_SECONDS: u64 = 120;
/// Shortest useful sample (seconds).
pub const MIN_SAMPLE_SECONDS: u64 = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Voice {
    pub name: String,
    pub vector: Vec<f32>,
    /// Seconds of sample it was made from.
    #[serde(default)]
    pub seconds: f32,
    /// Where the sample came from.
    #[serde(default)]
    pub source: String,
}

/// The voices folder.
pub fn dir() -> PathBuf {
    crate::job::models_dir().parent().map(|p| p.join("voices")).unwrap_or_else(|| PathBuf::from("voices"))
}

/// A name as a file name: letters, digits, spaces, `-` and `_` only, at most 30 characters.
pub fn clean_name(name: &str) -> Result<String, String> {
    let n: String = name.trim().chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(30).collect();
    let n = n.trim().to_string();
    if n.is_empty() {
        return Err("이름이 비어 있습니다 (글자·숫자로 적어 주세요)".into());
    }
    Ok(n)
}

fn path_in(dir: &Path, name: &str) -> Result<PathBuf, String> {
    Ok(dir.join(format!("{}.json", clean_name(name)?)))
}

/// Saved voices in `dir`, by name.
pub fn list_in(dir: &Path) -> Vec<Voice> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Voice> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() < 1 << 20))
        .take(MAX_VOICES)
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .filter_map(|t| serde_json::from_str::<Voice>(&t).ok())
        .filter(|v| v.vector.len() == overfilm_asr::speakers::net_embed_size() && v.vector.iter().all(|x| x.is_finite()))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn list() -> Vec<Voice> {
    list_in(&dir())
}

pub fn save_in(dir: &Path, v: &Voice) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let p = path_in(dir, &v.name)?;
    let json = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(&p, json).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

pub fn remove_in(dir: &Path, name: &str) -> Result<(), String> {
    let p = path_in(dir, name)?;
    std::fs::remove_file(&p).map_err(|e| format!("{name}: {e}"))
}

/// The voices as the speaker labeller takes them.
pub fn known(voices: &[Voice]) -> Vec<KnownVoice> {
    voices.iter().map(|v| KnownVoice { name: v.name.clone(), vector: v.vector.clone() }).collect()
}

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
    fn names_times_and_storage() {
        assert_eq!(clean_name(" 아이치/../x ").unwrap(), "아이치x");
        assert!(clean_name("../").is_err());
        assert_eq!(parse_time("83"), Some(83_000));
        assert_eq!(parse_time("1:23"), Some(83_000));
        assert_eq!(parse_time("1:02:03,5"), Some(3_723_500));
        assert_eq!(parse_time("x"), None);
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time("99999999999999999999"), None);
        let d = tempfile::tempdir().unwrap();
        let n = overfilm_asr::speakers::net_embed_size();
        let v = Voice { name: "스트리머".into(), vector: vec![0.1; n], seconds: 12.0, source: "a.mp4".into() };
        save_in(d.path(), &v).unwrap();
        // broken and wrong-sized files are skipped
        std::fs::write(d.path().join("bad.json"), "{").unwrap();
        std::fs::write(d.path().join("short.json"), r#"{"name":"s","vector":[1.0]}"#).unwrap();
        let l = list_in(d.path());
        assert_eq!(l.len(), 1);
        assert_eq!(known(&l)[0].name, "스트리머");
        remove_in(d.path(), "스트리머").unwrap();
        assert!(list_in(d.path()).is_empty());
        assert!(list_in(Path::new("/nonexistent")).is_empty());
    }
}

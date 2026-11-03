//! Enrolled voices: a person's voice saved once (from a sample of them speaking) so every later
//! extraction names them ("아이치" instead of "화자1") and can keep only their speech (a
//! streamer's own voice over game audio and other people). Saved as `voices/<name>.json` beside
//! the models folder. Enrolling (which runs the speaker network) is the extractor's job.

use std::path::{Path, PathBuf};

use overfilm_asr::speakers::KnownVoice;
use serde::{Deserialize, Serialize};

/// Most saved voices read.
const MAX_VOICES: usize = 100;

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
    crate::data_dir().join("voices")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_storage() {
        assert_eq!(clean_name(" 아이치/../x ").unwrap(), "아이치x");
        assert!(clean_name("../").is_err());
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

//! Every saved state of a caption file, so any earlier one can come back: each save of an SRT made
//! or edited here is kept as a numbered copy under `history/` beside the models (not beside the
//! SRT, so the editing folder stays clean), with when and what changed. The newest
//! [`MAX_VERSIONS`] are kept per file.

use std::path::{Path, PathBuf};

use overfilm_captions::Caption;
use serde::{Deserialize, Serialize};

/// Versions kept per caption file.
pub const MAX_VERSIONS: usize = 100;
/// Largest index file read.
const MAX_INDEX: u64 = 1 << 20;

/// One saved state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub id: u32,
    /// Seconds since 1970 (UTC).
    pub time: i64,
    /// What changed ("글자 고침 #12", "찾아 바꾸기"…).
    pub label: String,
    pub captions: usize,
    /// Hash of the SRT text (an unchanged save is not kept twice).
    #[serde(default)]
    hash: u64,
}

impl Version {
    /// Local date and time, e.g. `11월 1일 14:05:09`.
    pub fn when(&self) -> String {
        chrono::DateTime::from_timestamp(self.time, 0).map(|t| t.with_timezone(&chrono::Local).format("%-m월 %-d일 %H:%M:%S").to_string()).unwrap_or_default()
    }
}

/// Where histories live.
pub fn root() -> PathBuf {
    if cfg!(test) { std::env::temp_dir().join("overfilm-test-history") } else { crate::edit::data_dir().join("history") }
}

/// 64-bit FNV-1a (stable across builds, unlike the standard hasher).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The folder for `srt`: its name and a hash of its full path.
pub fn dir_for(root: &Path, srt: &Path) -> PathBuf {
    let full = std::fs::canonicalize(srt).unwrap_or_else(|_| srt.to_path_buf());
    let stem: String = srt
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == ' ' { c } else { '_' })
        .take(30)
        .collect();
    root.join(format!("{stem}-{:016x}", fnv(full.to_string_lossy().as_bytes())))
}

fn read_index(dir: &Path) -> Vec<Version> {
    let p = dir.join("index.json");
    match std::fs::metadata(&p) {
        Ok(m) if m.len() <= MAX_INDEX => std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn version_file(dir: &Path, id: u32) -> PathBuf {
    dir.join(format!("{id:06}.srt"))
}

/// Keep `caps` as a new version of `srt` under `root`. A save identical to the last version is
/// not kept again. Returns the version (the last one when unchanged).
pub fn snapshot_in(root: &Path, srt: &Path, caps: &[Caption], label: &str) -> Result<Version, String> {
    let dir = dir_for(root, srt);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = overfilm_captions::to_srt(caps);
    let hash = fnv(text.as_bytes());
    let mut index = read_index(&dir);
    if let Some(last) = index.last()
        && last.hash == hash
    {
        return Ok(last.clone());
    }
    let id = index.last().map_or(1, |v| v.id.saturating_add(1));
    std::fs::write(version_file(&dir, id), &text).map_err(|e| format!("{}: {e}", dir.display()))?;
    let v = Version { id, time: chrono::Utc::now().timestamp(), label: label.chars().take(80).collect(), captions: caps.len(), hash };
    index.push(v.clone());
    while index.len() > MAX_VERSIONS {
        let old = index.remove(0);
        let _ = std::fs::remove_file(version_file(&dir, old.id));
    }
    let json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("index.json"), json).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(v)
}

pub fn snapshot(srt: &Path, caps: &[Caption], label: &str) -> Result<Version, String> {
    snapshot_in(&root(), srt, caps, label)
}

/// Versions of `srt`, newest first.
pub fn list_in(root: &Path, srt: &Path) -> Vec<Version> {
    let mut v = read_index(&dir_for(root, srt));
    v.reverse();
    v
}

pub fn list(srt: &Path) -> Vec<Version> {
    list_in(&root(), srt)
}

/// The captions of version `id`.
pub fn load_in(root: &Path, srt: &Path, id: u32) -> Result<Vec<Caption>, String> {
    let p = version_file(&dir_for(root, srt), id);
    if !p.is_file() {
        return Err(format!("기록 {id}번이 없습니다"));
    }
    crate::job::read_srt(&p)
}

pub fn load(srt: &Path, id: u32) -> Result<Vec<Caption>, String> {
    load_in(&root(), srt, id)
}

/// Before the first save of a file with no history yet: keep what is on disk now.
pub fn keep_original_in(root: &Path, srt: &Path) {
    if read_index(&dir_for(root, srt)).is_empty()
        && let Ok(caps) = crate::job::read_srt(srt)
    {
        let _ = snapshot_in(root, srt, &caps, "원본");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(t: &str) -> Vec<Caption> {
        vec![Caption { start_ms: 0, end_ms: 900, lines: vec![t.into()] }]
    }

    #[test]
    fn versions_are_kept_listed_and_loaded() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("history");
        let srt = d.path().join("경기 1.srt");
        crate::job::write_srt(&srt, &cap("처음")).unwrap();
        keep_original_in(&root, &srt);
        snapshot_in(&root, &srt, &cap("둘째"), "글자 고침 #1").unwrap();
        snapshot_in(&root, &srt, &cap("둘째"), "같은 내용").unwrap();
        let l = list_in(&root, &srt);
        assert_eq!(l.iter().map(|v| v.label.as_str()).collect::<Vec<_>>(), vec!["글자 고침 #1", "원본"]);
        assert_eq!(load_in(&root, &srt, l[1].id).unwrap(), cap("처음"));
        assert!(load_in(&root, &srt, 99).is_err());
        assert!(!l[0].when().is_empty());
        // another file of the same name elsewhere has its own history
        let other = d.path().join("sub");
        std::fs::create_dir_all(&other).unwrap();
        assert!(list_in(&root, &other.join("경기 1.srt")).is_empty());
    }

    #[test]
    fn only_the_newest_are_kept() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("h");
        let srt = d.path().join("a.srt");
        for k in 0..(MAX_VERSIONS + 5) {
            snapshot_in(&root, &srt, &cap(&k.to_string()), "고침").unwrap();
        }
        let l = list_in(&root, &srt);
        assert_eq!(l.len(), MAX_VERSIONS);
        assert_eq!(load_in(&root, &srt, l[0].id).unwrap(), cap(&(MAX_VERSIONS + 4).to_string()));
        let files = std::fs::read_dir(dir_for(&root, &srt)).unwrap().count();
        assert_eq!(files, MAX_VERSIONS + 1, "old copies are deleted");
    }
}

//! One extraction: models (downloaded on first use), sound, recognition, correction, caption
//! layout, files written next to the input.

use std::path::{Path, PathBuf};

use overfilm_captions::{Caption, Layout, TimedWord};
use serde::{Deserialize, Serialize};

use crate::pack::{self, Mode};

/// What to extract and how.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Options {
    pub input: PathBuf,
    /// SRT to write; default: the input's name with `.srt`.
    #[serde(default)]
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub mode: Mode,
    /// Extra names and terms for the recogniser (comma separated).
    #[serde(default)]
    pub extra_terms: String,
    /// Swear words as initial consonants (씨발 → ㅅㅂ).
    #[serde(default = "yes")]
    pub mask_profanity: bool,
    /// Folder of extra glossary `.tsv` tables.
    #[serde(default)]
    pub glossary_dir: Option<PathBuf>,
    /// Characters per caption line (Hangul).
    #[serde(default = "sixteen")]
    pub line_chars: u32,
    /// Lines per caption (1 or 2).
    #[serde(default = "two")]
    pub max_lines: u32,
}

fn yes() -> bool {
    true
}
fn sixteen() -> u32 {
    16
}
fn two() -> u32 {
    2
}

impl Options {
    pub fn new(input: PathBuf) -> Self {
        Self { input, output: None, mode: Mode::General, extra_terms: String::new(), mask_profanity: true, glossary_dir: None, line_chars: 16, max_lines: 2 }
    }

    pub fn srt_path(&self) -> PathBuf {
        self.output.clone().unwrap_or_else(|| self.input.with_extension("srt"))
    }

    pub fn layout(&self) -> Layout {
        Layout { max_line: self.line_chars.clamp(4, 60) * 2, max_lines: self.max_lines.clamp(1, 2), ..Layout::default() }
    }
}

/// The transcript file kept beside the SRT (word times, for review and re-layout).
pub fn transcript_path(srt: &Path) -> PathBuf {
    srt.with_extension("overfilm.json")
}

/// Result of a finished extraction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Done {
    pub srt: PathBuf,
    pub transcript: PathBuf,
    pub captions: usize,
    pub audio_seconds: u64,
    pub seconds_taken: u64,
    /// Glossary corrections made, `(from, to)`.
    pub corrections: Vec<(String, String)>,
}

/// Where the models live: `models` beside the program if that folder exists (portable copy),
/// else the user's local data folder.
pub fn models_dir() -> PathBuf {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("models")))
        && dir.is_dir()
    {
        return dir;
    }
    if let Some(d) = std::env::var_os("OVERFILM_MODELS") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("OverFilm").join("models")
}

/// Progress: (fraction 0–1, message); return false to cancel.
pub type Progress<'a> = &'a mut dyn FnMut(f32, &str) -> bool;

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1e9)
}

/// Download whatever model files are missing.
pub fn ensure_models(dir: &Path, progress: Progress) -> Result<(), String> {
    for m in overfilm_asr::catalogue::all() {
        if m.installed(dir) {
            continue;
        }
        let name = m.name;
        overfilm_asr::catalogue::download(dir, m, &mut |done, total, file| {
            progress(done as f32 / total.max(1) as f32, &format!("{name} 받는 중 ({file}, {} / {})", gb(done), gb(total)))
        })
        .map_err(|e| format!("{name} 다운로드 실패: {e}"))?;
    }
    Ok(())
}

/// Run one extraction. Stages share the progress bar: models 0–5 %, sound 5–10 %, recognition
/// and timing 10–98 %, writing the rest.
pub fn run(o: &Options, progress: Progress) -> Result<Done, String> {
    let t0 = std::time::Instant::now();
    let models = models_dir();
    ensure_models(&models, &mut |f, m| progress(0.05 * f, m))?;
    let audio = crate::media::decode(&o.input, &mut |f| progress(0.05 + 0.05 * f, "소리 읽는 중"))?;
    if audio.is_empty() {
        return Err("소리가 비어 있습니다".into());
    }
    let g = pack::glossary(o.glossary_dir.as_deref(), o.mask_profanity)?;
    let corrector = pack::corrector(&g);
    let context = pack::context(&g, o.mode, &o.extra_terms);
    let corrections = std::sync::Mutex::new(Vec::new());
    let correct = |t: &str| {
        let c = corrector.apply(t);
        corrections.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(c.changes);
        c.text
    };
    let settings = overfilm_asr::Settings {
        asr_dir: &overfilm_asr::catalogue::ASR.dir(&models),
        aligner_dir: &overfilm_asr::catalogue::ALIGNER.dir(&models),
        quantize: true,
        context: &context,
        language: Some("Korean"),
        correct: &correct,
    };
    let transcript = overfilm_asr::transcribe(&audio, &settings, &mut |f, m| progress(0.1 + 0.88 * f, m)).map_err(|e| e.to_string())?;
    let words: Vec<TimedWord> = transcript.words().map(|w| TimedWord { text: w.text.clone(), start_ms: w.start_ms, end_ms: w.end_ms }).collect();
    let caps = overfilm_captions::layout(&words, &o.layout());
    let srt = o.srt_path();
    write_srt(&srt, &caps)?;
    let tpath = transcript_path(&srt);
    let json = serde_json::to_string_pretty(&transcript).map_err(|e| e.to_string())?;
    std::fs::write(&tpath, json).map_err(|e| format!("{}: {e}", tpath.display()))?;
    progress(1.0, "끝");
    let corrections = corrections.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Done {
        srt,
        transcript: tpath,
        captions: caps.len(),
        audio_seconds: (audio.len() / overfilm_asr::SAMPLE_RATE as usize) as u64,
        seconds_taken: t0.elapsed().as_secs(),
        corrections,
    })
}

/// Write an SRT (UTF-8 with BOM, which Premiere and Windows tools read as UTF-8).
pub fn write_srt(path: &Path, caps: &[Caption]) -> Result<(), String> {
    let mut s = String::from("\u{feff}");
    s.push_str(&overfilm_captions::to_srt(caps));
    std::fs::write(path, s).map_err(|e| format!("{}: {e}", path.display()))
}

/// Read an SRT written by anyone.
pub fn read_srt(path: &Path) -> Result<Vec<Caption>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > 64 << 20 {
        return Err(format!("{}: 파일이 너무 큽니다", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    overfilm_captions::from_srt(text.trim_start_matches('\u{feff}'))
}

/// Run `f` so that a panic becomes an error instead of ending the program.
pub fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(p) => {
            Err(format!("내부 오류: {}", p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_files_round_trip_with_bom() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.srt");
        let caps = vec![Caption { start_ms: 0, end_ms: 900, lines: vec!["안녕".into()] }];
        write_srt(&p, &caps).unwrap();
        assert!(std::fs::read(&p).unwrap().starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(read_srt(&p).unwrap(), caps);
        assert_eq!(transcript_path(&p), d.path().join("a.overfilm.json"));
    }

    #[test]
    fn options_from_json_defaults() {
        let o: Options = serde_json::from_str(r#"{"input":"v.mp4"}"#).unwrap();
        assert!(o.mask_profanity);
        assert_eq!(o.srt_path(), PathBuf::from("v.srt"));
        assert_eq!(o.layout().max_line, 32);
    }

    #[test]
    fn panics_become_errors() {
        let r: Result<(), String> = guarded(|| panic!("boom"));
        assert!(r.unwrap_err().contains("boom"));
    }
}

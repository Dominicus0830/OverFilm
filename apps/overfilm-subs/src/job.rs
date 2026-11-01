//! One extraction: models (downloaded on first use), sound, recognition, correction, caption
//! layout, files written next to the input.

use std::path::{Path, PathBuf};

use overfilm_captions::{Caption, Layout};
use serde::{Deserialize, Serialize};

use crate::pack::{self, Mode};
use crate::speakers::{self, GameVoice};

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
    /// Spoken numbers before counters as digits (오 시즌 → 5시즌, 사십칠 퍼센트 → 47%).
    #[serde(default = "yes")]
    pub digits: bool,
    /// Folder of extra glossary `.tsv` tables.
    #[serde(default)]
    pub glossary_dir: Option<PathBuf>,
    /// Characters per caption line (Hangul).
    #[serde(default = "sixteen")]
    pub line_chars: u32,
    /// Lines per caption (1 or 2).
    #[serde(default = "two")]
    pub max_lines: u32,
    /// Use the GPU when there is one (Vulkan/DX12; falls back to the CPU).
    #[serde(default = "yes")]
    pub gpu: bool,
    /// Tell voices apart and label captions (화자1, 화자2…) when there are two or more people.
    #[serde(default = "yes")]
    pub speakers: bool,
    /// The number of speakers when known (`None`: counted automatically).
    #[serde(default)]
    pub num_speakers: Option<usize>,
    /// What captions do with in-game voice (hero lines, announcer).
    #[serde(default)]
    pub game_voice: GameVoice,
    /// Also write one SRT per person (`이름.화자1.srt`…).
    #[serde(default)]
    pub per_speaker: bool,
    /// Name speakers after the enrolled voices (`voices/`).
    #[serde(default = "yes")]
    pub use_voices: bool,
    /// Captions only for enrolled voices (e.g. the streamer's own speech).
    #[serde(default)]
    pub only_voices: bool,
    /// Leave hesitation sounds and stutters (어, 음, 그 그게) out of the captions.
    #[serde(default)]
    pub clean_fillers: bool,
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
        Self {
            input,
            output: None,
            mode: Mode::General,
            extra_terms: String::new(),
            mask_profanity: true,
            digits: true,
            glossary_dir: None,
            line_chars: 16,
            max_lines: 2,
            gpu: true,
            speakers: true,
            num_speakers: None,
            game_voice: GameVoice::Drop,
            per_speaker: false,
            use_voices: true,
            only_voices: false,
            clean_fillers: false,
        }
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
    /// Where recognition ran ("CPU" or "GPU <adapter>").
    pub device: String,
    /// Speakers found: (label, seconds, game voice).
    #[serde(default)]
    pub speakers: Vec<(String, f32, bool)>,
    /// In-game voice lines recognised.
    #[serde(default)]
    pub voice_lines: usize,
    /// Extra SRT files written, one per person.
    #[serde(default)]
    pub speaker_srts: Vec<PathBuf>,
    /// Something the user should know about the result (e.g. no enrolled voice was heard).
    #[serde(default)]
    pub note: String,
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
/// and timing 10–90 %, speakers 90–98 %, writing the rest.
pub fn run(o: &Options, progress: Progress) -> Result<Done, String> {
    let t0 = std::time::Instant::now();
    let models = models_dir();
    ensure_models(&models, &mut |f, m| progress(0.05 * f, m))?;
    let audio = crate::media::decode(&o.input, &mut |f| progress(0.05 + 0.05 * f, "소리 읽는 중"))?;
    if audio.is_empty() {
        return Err("소리가 비어 있습니다".into());
    }
    let g = pack::glossary(o.glossary_dir.as_deref(), o.mask_profanity)?;
    let corrector = pack::corrector_with(&g, o.digits);
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
        gpu: o.gpu,
    };
    let rec_share = if o.speakers { 0.8 } else { 0.88 };
    let mut transcript = overfilm_asr::transcribe(&audio, &settings, &mut |f, m| progress(0.1 + rec_share * f, m)).map_err(|e| e.to_string())?;
    let mut speaker_device = String::new();
    if o.speakers {
        let known = if o.use_voices || o.only_voices { crate::voices::known(&crate::voices::list()) } else { Vec::new() };
        let p = overfilm_asr::speakers::Params { speakers: o.num_speakers.filter(|&n| n > 0), known, ..Default::default() };
        let mut cancelled = false;
        let r = speakers::label(&audio, &mut transcript, &overfilm_asr::catalogue::SPEAKER.dir(&models), &p, o.gpu, &mut |f| {
            let go = progress(0.9 + 0.08 * f, "목소리 구별하는 중");
            cancelled |= !go;
            go
        });
        if cancelled {
            return Err("취소됨".into());
        }
        speaker_device = r?;
    }
    let voice_lines = speakers::mark_game_voice(&mut transcript, &pack::voicelines());
    let mut note = String::new();
    let keep = if o.only_voices {
        if transcript.speakers.iter().any(|s| s.enrolled && !s.game) {
            speakers::Keep::Enrolled
        } else {
            note = "등록한 목소리를 이 영상에서 찾지 못해 모든 사람의 자막을 만들었습니다.".into();
            speakers::Keep::All
        }
    } else {
        speakers::Keep::All
    };
    let caps = speakers::captions_keep(&transcript, o.game_voice, keep, &o.layout(), o.clean_fillers);
    let srt = o.srt_path();
    write_srt(&srt, &caps)?;
    let _ = crate::history::snapshot(&srt, &caps, "처음 만듦");
    let mut speaker_srts = Vec::new();
    if o.per_speaker && speakers::people(&transcript) > 1 {
        for (_, label, caps) in speakers::captions_per_speaker(&transcript, &o.layout(), o.clean_fillers) {
            let p = per_speaker_path(&srt, &label);
            write_srt(&p, &caps)?;
            speaker_srts.push(p);
        }
    }
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
        device: if speaker_device.is_empty() || speaker_device == transcript.device {
            transcript.device.clone()
        } else {
            format!("{}, 화자 구별 {}", transcript.device, speaker_device.split(" (").next().unwrap_or(&speaker_device))
        },
        speakers: transcript.speakers.iter().map(|s| (s.label.clone(), s.seconds, s.game)).collect(),
        voice_lines,
        speaker_srts,
        note,
    })
}

/// `경기.srt` → `경기.화자1.srt` (characters a file name cannot hold become `_`).
pub fn per_speaker_path(srt: &Path, label: &str) -> PathBuf {
    let safe: String = label.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).take(40).collect();
    let stem = srt.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    srt.with_file_name(format!("{stem}.{safe}.srt"))
}

/// One-line summary of the speakers found ("화자 3명: 화자1 4분 12초, …; 게임 음성 2줄 뺌").
pub fn speaker_summary(d: &Done) -> String {
    let people: Vec<String> = d
        .speakers
        .iter()
        .filter(|s| !s.2)
        .map(|(l, secs, _)| {
            let s = secs.max(0.0) as u64;
            if s >= 60 { format!("{l} {}분 {}초", s / 60, s % 60) } else { format!("{l} {s}초") }
        })
        .collect();
    let mut out = if people.is_empty() { String::new() } else { format!("화자 {}명: {}", people.len(), people.join(", ")) };
    if d.voice_lines > 0 || d.speakers.iter().any(|s| s.2) {
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(&format!("게임 음성 대사 {}줄 찾음", d.voice_lines));
    }
    if !d.note.is_empty() {
        if !out.is_empty() {
            out.push_str(" · ");
        }
        out.push_str(&d.note);
    }
    out
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

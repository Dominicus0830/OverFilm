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
    /// Only this part of the input, `(from_ms, to_ms)`; caption times stay on the input's clock.
    #[serde(default)]
    pub range: Option<(i64, i64)>,
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
            range: None,
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

/// Where the models live (shared with the editor; see [`overfilm_pipeline::models_dir`]).
pub fn models_dir() -> PathBuf {
    overfilm_pipeline::models_dir()
}

/// Progress: (fraction 0–1, message); return false to cancel.
pub type Progress<'a> = &'a mut dyn FnMut(f32, &str) -> bool;

/// Download whatever model files are missing.
pub fn ensure_models(dir: &Path, progress: Progress) -> Result<(), String> {
    overfilm_pipeline::recognise::ensure_models(dir, progress)
}

/// Run one extraction. Stages share the progress bar: models 0–5 %, sound 5–10 %, recognition
/// and timing 10–90 %, speakers 90–98 %, writing the rest.
pub fn run(o: &Options, progress: Progress) -> Result<Done, String> {
    let t0 = std::time::Instant::now();
    let models = models_dir();
    ensure_models(&models, &mut |f, m| progress(0.05 * f, m))?;
    let range = o.range.map(|(a, b)| (a.max(0), b.max(a.max(0))));
    let audio = crate::media::decode_span(&o.input, range, &mut |f| progress(0.05 + 0.05 * f, "소리 읽는 중"))?;
    if audio.is_empty() {
        return Err("소리가 비어 있습니다".into());
    }
    let known = if o.speakers && (o.use_voices || o.only_voices) { crate::voices::known(&crate::voices::list()) } else { Vec::new() };
    let req = overfilm_pipeline::recognise::Request {
        mode: o.mode,
        extra_terms: o.extra_terms.clone(),
        mask_profanity: o.mask_profanity,
        digits: o.digits,
        glossary_dir: o.glossary_dir.clone(),
        gpu: o.gpu,
        speakers: o.speakers,
        num_speakers: o.num_speakers,
        known,
    };
    let rec = overfilm_pipeline::recognise::recognise(&audio, &req, &models, &mut |f, m| progress(0.1 + 0.88 * f, m))?;
    let overfilm_pipeline::recognise::Recognised { mut transcript, corrections, device, voice_lines, .. } = rec;
    if let Some((from, _)) = range {
        // the words were timed on the cut-out sound: put them on the input's clock
        shift(&mut transcript, from);
    }
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
    Ok(Done {
        srt,
        transcript: tpath,
        captions: caps.len(),
        audio_seconds: (audio.len() / overfilm_asr::SAMPLE_RATE as usize) as u64,
        seconds_taken: t0.elapsed().as_secs(),
        corrections,
        device,
        speakers: transcript.speakers.iter().map(|s| (s.label.clone(), s.seconds, s.game)).collect(),
        voice_lines,
        speaker_srts,
        note,
    })
}

/// Move every time in `t` by `ms`.
pub fn shift(t: &mut overfilm_asr::Transcript, ms: i64) {
    for c in &mut t.chunks {
        c.start_ms = c.start_ms.saturating_add(ms);
        c.end_ms = c.end_ms.saturating_add(ms);
        for w in &mut c.words {
            w.start_ms = w.start_ms.saturating_add(ms);
            w.end_ms = w.end_ms.saturating_add(ms);
        }
    }
}

/// Sound kept around a part recognised again, so its first and last words are heard whole (ms).
const REDO_PAD_MS: i64 = 400;

/// Recognise `[from_ms, to_ms)` of `o.input` again (with `o.extra_terms` as extra context) and
/// return its words on the input's clock, corrected like a normal run. Words that start in the
/// padding around the part are left out. No speakers: [`splice`] takes them from the old words.
pub fn recognise_span(o: &Options, from_ms: i64, to_ms: i64, progress: Progress) -> Result<Vec<overfilm_asr::Word>, String> {
    if to_ms <= from_ms {
        return Err("구간이 비어 있습니다".into());
    }
    if to_ms - from_ms > 30 * 60_000 {
        return Err("다시 인식은 30분까지만 됩니다. 더 길면 새로 만드세요.".into());
    }
    let models = models_dir();
    ensure_models(&models, &mut |f, m| progress(0.05 * f, m))?;
    let start = from_ms.saturating_sub(REDO_PAD_MS).max(0);
    let audio = crate::media::decode_span(&o.input, Some((start, to_ms.saturating_add(REDO_PAD_MS))), &mut |f| progress(0.05 + 0.05 * f, "소리 읽는 중"))?;
    if audio.is_empty() {
        return Err("그 구간에 소리가 없습니다".into());
    }
    let g = pack::glossary(o.glossary_dir.as_deref(), o.mask_profanity)?;
    let corrector = pack::corrector_with(&g, o.digits);
    let context = pack::context(&g, o.mode, &o.extra_terms);
    let correct = |t: &str| corrector.apply(t).text;
    let settings = overfilm_asr::Settings {
        asr_dir: &overfilm_asr::catalogue::ASR.dir(&models),
        aligner_dir: &overfilm_asr::catalogue::ALIGNER.dir(&models),
        quantize: true,
        context: &context,
        language: Some("Korean"),
        correct: &correct,
        gpu: o.gpu,
    };
    let mut t = overfilm_asr::transcribe(&audio, &settings, &mut |f, m| progress(0.1 + 0.9 * f, m)).map_err(|e| e.to_string())?;
    shift(&mut t, start);
    Ok(t.chunks.into_iter().flat_map(|c| c.words).filter(|w| w.start_ms >= from_ms && w.start_ms < to_ms).collect())
}

/// Put `new` words in place of the words of `t` that start in `[from_ms, to_ms)`. Each new word
/// takes the speaker (and game-voice mark) of the nearest old word in time.
pub fn splice(t: &mut overfilm_asr::Transcript, from_ms: i64, to_ms: i64, mut new: Vec<overfilm_asr::Word>) {
    let inside = |w: &overfilm_asr::Word| w.start_ms >= from_ms && w.start_ms < to_ms;
    let old: Vec<overfilm_asr::Word> = t.words().filter(|w| inside(w)).cloned().collect();
    let around: Vec<overfilm_asr::Word> = if old.is_empty() { t.words().cloned().collect() } else { old };
    for w in &mut new {
        if let Some(n) = around.iter().min_by_key(|o| (o.start_ms - w.start_ms).abs()) {
            w.speaker = n.speaker;
            w.game = n.game;
            w.also = None;
        }
    }
    for c in &mut t.chunks {
        c.words.retain(|w| !inside(w));
    }
    // into the chunk the part starts in (or the last one starting before it)
    let k = t.chunks.iter().rposition(|c| c.start_ms <= from_ms).unwrap_or(0);
    match t.chunks.get_mut(k) {
        Some(c) => {
            c.words.extend(new);
            c.words.sort_by_key(|w| w.start_ms);
        }
        None => t.chunks.push(overfilm_asr::ChunkText { start_ms: from_ms, end_ms: to_ms, text: String::new(), words: new }),
    }
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
    fn splice_replaces_words_and_keeps_speakers() {
        use overfilm_asr::{ChunkText, Transcript, Word};
        let w = |t: &str, s: i64, sp: usize| Word { text: t.into(), start_ms: s, end_ms: s + 300, speaker: Some(sp), ..Word::default() };
        let mut t = Transcript {
            chunks: vec![
                ChunkText { start_ms: 0, end_ms: 5000, text: String::new(), words: vec![w("앞", 100, 0), w("전국퀸", 1000, 1), w("나왔다", 1500, 1)] },
                ChunkText { start_ms: 5000, end_ms: 9000, text: String::new(), words: vec![w("뒤", 6000, 0)] },
            ],
            ..Transcript::default()
        };
        let new = vec![
            Word { text: "정커퀸이".into(), start_ms: 1010, end_ms: 1400, ..Word::default() },
            Word { text: "나왔다".into(), start_ms: 1450, end_ms: 1800, ..Word::default() },
        ];
        splice(&mut t, 900, 2000, new);
        let words: Vec<(&str, Option<usize>)> = t.words().map(|w| (w.text.as_str(), w.speaker)).collect();
        assert_eq!(words, vec![("앞", Some(0)), ("정커퀸이", Some(1)), ("나왔다", Some(1)), ("뒤", Some(0))]);
        // nothing new: the part is just emptied
        splice(&mut t, 5500, 7000, Vec::new());
        assert_eq!(t.words().count(), 3);
        let mut empty = Transcript::default();
        splice(&mut empty, 0, 10, vec![Word { text: "가".into(), start_ms: 1, end_ms: 5, ..Word::default() }]);
        assert_eq!(empty.words().count(), 1);
    }

    #[test]
    fn panics_become_errors() {
        let r: Result<(), String> = guarded(|| panic!("boom"));
        assert!(r.unwrap_err().contains("boom"));
    }
}

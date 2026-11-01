//! One recognition of a whole sound (feature `qwen`): Qwen3-ASR with the pack's context, word
//! times, glossary correction, speaker labels and game voice. The extractor writes the result as
//! SRT; the editor stores it as the clip's transcript.

use std::path::Path;

use overfilm_asr::Transcript;

use crate::pack::{self, Mode};
use crate::speakers;

/// How to recognise.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub mode: Mode,
    /// Extra names and terms for the recogniser (comma separated).
    pub extra_terms: String,
    /// Swear words as initial consonants (씨발 → ㅅㅂ).
    pub mask_profanity: bool,
    /// Spoken numbers before counters as digits.
    pub digits: bool,
    /// Folder of extra glossary `.tsv` tables.
    pub glossary_dir: Option<std::path::PathBuf>,
    /// Use the GPU when there is one (falls back to the CPU).
    pub gpu: bool,
    /// Tell voices apart.
    pub speakers: bool,
    /// The number of speakers when known.
    pub num_speakers: Option<usize>,
    /// Enrolled voices to name speakers after.
    pub known: Vec<overfilm_asr::speakers::KnownVoice>,
}

impl Request {
    /// The extractor's defaults: profanity masked, digits, GPU, speakers on.
    pub fn new(mode: Mode) -> Self {
        Self { mode, mask_profanity: true, digits: true, gpu: true, speakers: true, ..Default::default() }
    }
}

/// What came out.
#[derive(Clone, Debug, Default)]
pub struct Recognised {
    /// Words with times (ms from the first sample), speakers and game voice marked.
    pub transcript: Transcript,
    /// Glossary corrections made, `(from, to)`.
    pub corrections: Vec<(String, String)>,
    /// Where recognition and speaker labelling ran.
    pub device: String,
    /// In-game voice lines recognised.
    pub voice_lines: usize,
    /// Pure recognition of each chunk (before the glossary correction), in the order of
    /// `transcript.chunks`; Claude Code review shows it beside the corrected captions.
    pub raw_texts: Vec<String>,
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

/// Recognise mono 16 kHz `audio` with the models in `models` (already downloaded, see
/// [`ensure_models`]). Progress runs 0–1: recognition and timing first, speakers last.
pub fn recognise(audio: &[f32], r: &Request, models: &Path, progress: Progress) -> Result<Recognised, String> {
    if audio.is_empty() {
        return Err("소리가 비어 있습니다".into());
    }
    let g = pack::glossary(r.glossary_dir.as_deref(), r.mask_profanity)?;
    let corrector = pack::corrector_with(&g, r.digits);
    let context = pack::context(&g, r.mode, &r.extra_terms);
    let corrections = std::sync::Mutex::new(Vec::new());
    let raw_texts = std::sync::Mutex::new(Vec::new());
    let correct = |t: &str| {
        raw_texts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(t.to_string());
        let c = corrector.apply(t);
        corrections.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(c.changes);
        c.text
    };
    let settings = overfilm_asr::Settings {
        asr_dir: &overfilm_asr::catalogue::ASR.dir(models),
        aligner_dir: &overfilm_asr::catalogue::ALIGNER.dir(models),
        quantize: true,
        context: &context,
        language: Some("Korean"),
        correct: &correct,
        gpu: r.gpu,
    };
    let rec_share = if r.speakers { 0.9 } else { 1.0 };
    let mut transcript = overfilm_asr::transcribe(audio, &settings, &mut |f, m| progress(rec_share * f, m)).map_err(|e| e.to_string())?;
    let mut speaker_device = String::new();
    if r.speakers {
        let p = overfilm_asr::speakers::Params { speakers: r.num_speakers.filter(|&n| n > 0), known: r.known.clone(), ..Default::default() };
        let mut cancelled = false;
        let res = speakers::label(audio, &mut transcript, &overfilm_asr::catalogue::SPEAKER.dir(models), &p, r.gpu, &mut |f| {
            let go = progress(0.9 + 0.1 * f, "목소리 구별하는 중");
            cancelled |= !go;
            go
        });
        if cancelled {
            return Err("취소됨".into());
        }
        speaker_device = res?;
    }
    let voice_lines = speakers::mark_game_voice(&mut transcript, &pack::voicelines());
    let device = if speaker_device.is_empty() || speaker_device == transcript.device {
        transcript.device.clone()
    } else {
        format!("{}, 화자 구별 {}", transcript.device, speaker_device.split(" (").next().unwrap_or(&speaker_device))
    };
    let corrections = corrections.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    let raw_texts = raw_texts.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(Recognised { transcript, corrections, device, voice_lines, raw_texts })
}

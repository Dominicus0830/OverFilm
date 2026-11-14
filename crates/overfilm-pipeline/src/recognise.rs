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
    /// The number of speakers when known. One speaker and no enrolled voices skips telling voices
    /// apart (and overlap detection) altogether: faster, and there is nobody to tell apart.
    pub num_speakers: Option<usize>,
    /// Enrolled voices to name speakers after.
    pub known: Vec<overfilm_asr::speakers::KnownVoice>,
    /// Leave out long quiet stretches (see [`overfilm_asr::Settings::skip_silence`]; decide with
    /// [`skip_silence_default`]).
    pub skip_silence: bool,
    /// Teams, players and heroes read off the screen (a soft prior, see [`crate::game`]).
    pub prior: Option<crate::game::MatchPrior>,
}

impl Request {
    /// The extractor's defaults: profanity masked, digits, GPU, speakers on.
    pub fn new(mode: Mode) -> Self {
        Self { mode, mask_profanity: true, digits: true, gpu: true, speakers: true, ..Default::default() }
    }
}

/// Where the sound to recognise comes from, for [`skip_silence_default`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundSource {
    /// A video's only audio track: the voice is mixed with the game.
    Embedded,
    /// One of a video's several audio tracks (OBS-style: the microphone on a track of its own),
    /// or a sound-only file.
    Separate,
}

impl SoundSource {
    /// From what a file holds: video or not, and how many audio tracks.
    pub fn of(has_video: bool, audio_tracks: usize) -> Self {
        if has_video && audio_tracks <= 1 { Self::Embedded } else { Self::Separate }
    }
}

/// Whether to leave out long quiet stretches when the user has not said: on for a separate
/// voice track (its quiet is real silence), off for sound inside the video (a soft voice under
/// loud game sound could be taken for quiet).
pub fn skip_silence_default(src: SoundSource) -> bool {
    src == SoundSource::Separate
}

/// Shown when the user turns skipping silence on for sound inside the video.
pub const SKIP_SILENCE_WARNING: &str =
    "영상에 섞인 소리에서 무음 건너뛰기를 켰습니다. 게임 소리 밑의 작은 말소리까지 건너뛸 수 있으니, 말소리만 따로 녹음된 트랙에서 쓰는 것을 권합니다.";

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

/// Whether [`recognise`] tells voices apart: speakers on, unless the user said there is one
/// person and there are no enrolled voices to name them after.
pub fn tell_apart(r: &Request) -> bool {
    r.speakers && !(r.num_speakers == Some(1) && r.known.is_empty())
}

/// Recognise mono 16 kHz `audio` with the models in `models` (already downloaded, see
/// [`ensure_models`]). Progress runs 0–1: recognition and timing first, speakers last.
pub fn recognise(audio: &[f32], r: &Request, models: &Path, progress: Progress) -> Result<Recognised, String> {
    recognise_live(audio, r, models, progress, None)
}

/// [`recognise`] that shows `live` each chunk's text while it is generated and when it is done
/// (see [`overfilm_asr::Settings::live`]).
pub fn recognise_live(
    audio: &[f32],
    r: &Request,
    models: &Path,
    progress: Progress,
    live: Option<&dyn Fn(&overfilm_asr::LiveText)>,
) -> Result<Recognised, String> {
    if audio.is_empty() {
        return Err("소리가 비어 있습니다".into());
    }
    let g = pack::glossary(r.glossary_dir.as_deref(), r.mask_profanity)?;
    let corrector = pack::corrector_with(&g, r.digits);
    let prior = r.prior.clone().unwrap_or_default();
    let context = pack::context_with(&g, r.mode, &r.extra_terms, &prior.context());
    let corrections = std::sync::Mutex::new(Vec::new());
    let raw_texts = std::sync::Mutex::new(Vec::new());
    let correct = |t: &str, start: i64, end: i64| {
        raw_texts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(t.to_string());
        let c = if prior.is_empty() { corrector.apply(t) } else { corrector.apply_preferring(t, &prior.preferred(start, end)) };
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
        skip_silence: r.skip_silence,
        live,
    };
    let label_speakers = tell_apart(r);
    let rec_share = if label_speakers { 0.9 } else { 1.0 };
    let mut transcript = overfilm_asr::transcribe(audio, &settings, &mut |f, m| progress(rec_share * f, m)).map_err(|e| e.to_string())?;
    let mut speaker_device = String::new();
    if label_speakers {
        let p = overfilm_asr::speakers::Params { speakers: r.num_speakers.filter(|&n| n > 0), known: r.known.clone(), ..Default::default() };
        let mut cancelled = false;
        let t = std::time::Instant::now();
        let res = speakers::label(audio, &mut transcript, &overfilm_asr::catalogue::SPEAKER.dir(models), &p, r.gpu, &mut |f| {
            let go = progress(0.9 + 0.1 * f, "목소리 구별하는 중");
            cancelled |= !go;
            go
        });
        if cancelled {
            return Err("취소됨".into());
        }
        speaker_device = res?;
        transcript.add_stage("화자 구별", t.elapsed().as_secs_f32(), 0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_skipped_by_default_only_on_separate_sound() {
        assert_eq!(SoundSource::of(true, 1), SoundSource::Embedded);
        assert_eq!(SoundSource::of(true, 2), SoundSource::Separate);
        assert_eq!(SoundSource::of(false, 1), SoundSource::Separate, "a sound-only file");
        assert!(!skip_silence_default(SoundSource::Embedded));
        assert!(skip_silence_default(SoundSource::Separate));
    }

    #[test]
    fn one_speaker_skips_telling_voices_apart() {
        let mut r = Request::new(Mode::General);
        assert!(tell_apart(&r), "counted automatically");
        r.num_speakers = Some(2);
        assert!(tell_apart(&r));
        r.num_speakers = Some(1);
        assert!(!tell_apart(&r), "one person: nothing to tell apart");
        r.known = vec![overfilm_asr::speakers::KnownVoice { name: "나".into(), vector: vec![1.0] }];
        assert!(tell_apart(&r), "an enrolled voice still names the one person");
        r.speakers = false;
        r.known.clear();
        r.num_speakers = None;
        assert!(!tell_apart(&r));
    }
}

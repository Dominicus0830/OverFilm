//! OverFilm speech recognition: Qwen3-ASR-1.7B and Qwen3-ForcedAligner-0.6B in pure Rust on
//! candle (CPU), with the model catalogue and download, low-energy chunking and a word-timed
//! transcript pipeline.
//!
//! Recognition ([`Asr`]) takes a piece (≤ [`MAX_CHUNK_SECONDS`]) of 16 kHz mono audio, an
//! optional context text (glossary terms; it measurably fixes hero and term names) and a forced
//! language. Alignment
//! ([`Aligner`]) gives each word of that text a start and end time. [`transcribe`] runs both over
//! a whole file's audio. See `OVERFILM.md` → "Speech recognition".
//!
//! [`gpu`] runs both networks on any Vulkan, DX12 or Metal GPU through wgpu (AMD included);
//! without one, or on any GPU error, the CPU (candle) does the work.
//!
//! [`speakers`] labels who spoke each word (WeSpeaker ResNet34 voice vectors and clustering; the
//! network needs feature `qwen`). In-game voice lines are found by text in `overfilm-glossary`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod catalogue;
pub mod chunk;
#[cfg(feature = "qwen")]
mod engine;
#[cfg(feature = "qwen")]
pub mod gpu;
#[cfg(feature = "qwen")]
pub mod model;
#[cfg(feature = "qwen")]
pub mod qmm;
pub mod speakers;
pub mod text;
pub mod tokenizer;
#[cfg(feature = "qwen")]
pub mod weights;

#[cfg(feature = "qwen")]
pub use engine::{Aligner, Asr, gpu_model_check, transcribe};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Input sample rate.
pub const SAMPLE_RATE: u32 = 16_000;
/// Longest audio handed to the models at once. The models take up to 3 minutes; 60 s pieces
/// gave fewer wrong characters than 30 s ones on the M1 set (CER 6.6 → 6.25 %, 120 s: 6.20 %)
/// and still leave room to decode three or four side by side on the GPU.
pub const MAX_CHUNK_SECONDS: f32 = 60.0;

#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("model: {0}")]
    Model(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("download: {0}")]
    Download(String),
    #[error("cancelled")]
    Cancelled,
}

#[cfg(feature = "qwen")]
impl From<candle_core::Error> for AsrError {
    fn from(e: candle_core::Error) -> Self {
        AsrError::Model(e.to_string())
    }
}

/// Progress callback: (fraction 0–1, message); return false to cancel.
pub type Progress<'a> = &'a mut dyn FnMut(f32, &str) -> bool;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// Speaker (0-based, see [`Transcript::speakers`]) when speakers were labelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<usize>,
    /// In-game voice (a hero line or the announcer), not a person in the room.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub game: bool,
    /// Another speaker talking at the same time (overlapped speech).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub also: Option<usize>,
    /// How sure the recogniser was of this word: the lowest probability of the tokens it was
    /// written with (0–1). `None` for words the correction step wrote or old transcripts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conf: Option<f32>,
}

/// A whole input's transcript, chunk by chunk: the text as recognised (and corrected) and its
/// words with times.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Transcript {
    pub language: String,
    /// where recognition ran ("CPU" or "GPU <adapter>")
    #[serde(default)]
    pub device: String,
    pub chunks: Vec<ChunkText>,
    /// Speakers found (empty when speakers were not labelled).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<Speaker>,
}

/// One voice in the transcript.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Speaker {
    /// Shown in captions, e.g. "화자1" (the user can rename it).
    pub label: String,
    /// Seconds of words.
    pub seconds: f32,
    pub words: usize,
    /// Mostly in-game voice lines: a hero or the announcer, not a person.
    #[serde(default)]
    pub game: bool,
    /// Matched a voice the user enrolled (its label is that voice's name).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enrolled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkText {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub words: Vec<Word>,
}

impl Transcript {
    pub fn words(&self) -> impl Iterator<Item = &Word> {
        self.chunks.iter().flat_map(|c| c.words.iter())
    }
}

/// Model directories and settings for [`transcribe`].
pub struct Settings<'a> {
    pub asr_dir: &'a Path,
    pub aligner_dir: &'a Path,
    /// 8-bit decoder weights (see [`Asr::load`])
    pub quantize: bool,
    pub context: &'a str,
    pub language: Option<&'a str>,
    /// Applied to each chunk's text before alignment (glossary correction, profanity masking).
    pub correct: &'a dyn Fn(&str) -> String,
    /// Use the GPU when there is one (falls back to the CPU).
    pub gpu: bool,
}

#[cfg_attr(not(feature = "qwen"), allow(dead_code))]
pub(crate) fn ms_of_sample(n: usize) -> i64 {
    // 16 samples per millisecond at 16 kHz
    i64::try_from(n / (SAMPLE_RATE as usize / 1000)).unwrap_or(i64::MAX)
}

/// Words of `text` with the times of their aligner units.
#[cfg_attr(not(feature = "qwen"), allow(dead_code))]
pub(crate) fn timed_words(text: &str, times: &[(i64, i64)], offset_ms: i64) -> Vec<Word> {
    let mut out: Vec<Word> = Vec::new();
    let mut next = 0usize;
    for (word, n) in text::word_units(text) {
        if n == 0 {
            // punctuation on its own: part of the previous word
            if let Some(last) = out.last_mut() {
                last.text.push(' ');
                last.text.push_str(word);
                continue;
            }
        }
        let first = times.get(next).copied();
        let last = times.get(next + n.max(1) - 1).copied().or(first);
        next += n;
        let (s, e) = match (first, last) {
            (Some((s, _)), Some((_, e))) => (s, e),
            _ => out.last().map(|w| (w.end_ms - offset_ms, w.end_ms - offset_ms)).unwrap_or((0, 0)),
        };
        out.push(Word { text: word.to_string(), start_ms: offset_ms.saturating_add(s), end_ms: offset_ms.saturating_add(e), ..Word::default() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_keep_punctuation_and_times() {
        let w = timed_words("자, 겐지가 용검 ! 써요.", &[(0, 80), (100, 400), (400, 640), (700, 900)], 1000);
        let t: Vec<_> = w.iter().map(|w| (w.text.as_str(), w.start_ms, w.end_ms)).collect();
        assert_eq!(t, vec![("자,", 1000, 1080), ("겐지가", 1100, 1400), ("용검 !", 1400, 1640), ("써요.", 1700, 1900)]);
        // fewer times than words (hostile aligner output) still gives every word a time
        assert_eq!(timed_words("하나 둘 셋", &[(0, 10)], 0).len(), 3);
    }
}

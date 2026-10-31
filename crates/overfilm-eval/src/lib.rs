//! OverFilm transcript evaluation.
//!
//! Scores a recognized transcript (the hypothesis) against a hand-checked one (the reference):
//!
//! - **CER**, character error rate, on letters and digits with word breaks removed. This is the
//!   headline number for Korean, where spacing is inconsistent even in references.
//! - **WER**, word (eojeol) error rate, for comparison with published English numbers.
//! - **Term accuracy**: the share of the reference's game terms (heroes, abilities, maps…) that
//!   the hypothesis spells right, from a [term list](terms).
//!
//! Both sides go through the same [normalization](normalize). Inputs are caption files filmcraft
//! reads (SRT, WebVTT, TTML…; cue texts joined in order) or plain UTF-8 text. Corpus totals add up
//! edits and reference lengths over all files, so long files weigh more, as in published ASR
//! benchmarks. The `overfilm-eval` binary is the command-line front end (see `OVERFILM.md`).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod distance;
pub mod normalize;
pub mod numbers;
pub mod terms;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use distance::EditCounts;
pub use terms::{TermList, TermScore};

/// The largest input file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// An evaluation failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: larger than {MAX_FILE_BYTES} bytes")]
    TooBig { path: PathBuf },
    #[error("{path}: {source}")]
    Captions { path: PathBuf, source: filmcraft_captions::Error },
    #[error("{path}: {source}")]
    Terms { path: PathBuf, source: terms::TermError },
    #[error("{path}: {source}")]
    Glossary { path: PathBuf, source: overfilm_glossary::GlossaryError },
    #[error("{0}")]
    TooLong(#[from] distance::TooLong),
    #[error("{0}: no reference files (.srt, .vtt, .ttml, .dfxp, .txt)")]
    NoFiles(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;

fn read_capped(path: &Path) -> Result<Vec<u8>> {
    let io = |source| Error::Io { path: path.to_path_buf(), source };
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > MAX_FILE_BYTES {
        return Err(Error::TooBig { path: path.to_path_buf() });
    }
    std::fs::read(path).map_err(io)
}

/// The transcript text of `bytes`: the cues of a caption file in order, one per line, or the
/// text itself. `ext` (without the dot) helps tell caption formats apart.
pub fn transcript_text(bytes: &[u8], ext: Option<&str>) -> std::result::Result<String, filmcraft_captions::Error> {
    if let Some(format) = filmcraft_captions::detect(bytes, ext) {
        let doc = filmcraft_captions::parse(bytes, format)?;
        return Ok(doc.cues.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n"));
    }
    let text = String::from_utf8_lossy(bytes);
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

/// Read a transcript file (see [`transcript_text`]).
pub fn load_transcript(path: &Path) -> Result<String> {
    let bytes = read_capped(path)?;
    let ext = path.extension().and_then(|e| e.to_str());
    transcript_text(&bytes, ext).map_err(|source| Error::Captions { path: path.to_path_buf(), source })
}

/// Read a term list file.
///
/// `path` is a term list file, or a glossary directory of `.tsv` tables (`overfilm_glossary`)
/// whose [`overfilm_glossary::Glossary::term_list`] is used.
pub fn load_terms(path: &Path) -> Result<TermList> {
    let text = if path.is_dir() { load_glossary(path)?.term_list() } else { String::from_utf8_lossy(&read_capped(path)?).into_owned() };
    TermList::parse(&text).map_err(|source| Error::Terms { path: path.to_path_buf(), source })
}

/// The glossary in directory `dir`: every `*.tsv` file, in name order, as the category named by
/// its file stem.
pub fn load_glossary(dir: &Path) -> Result<overfilm_glossary::Glossary> {
    let io = |source| Error::Io { path: dir.to_path_buf(), source };
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(dir).map_err(io)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "tsv")).collect();
    files.sort();
    let mut glossary = overfilm_glossary::Glossary::default();
    for f in files {
        let category = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let text = String::from_utf8_lossy(&read_capped(&f)?).into_owned();
        glossary.add_table(&category, &text).map_err(|source| Error::Glossary { path: f.clone(), source })?;
    }
    Ok(glossary)
}

/// Scores of one hypothesis against its reference.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Score {
    pub chars: EditCounts,
    pub words: EditCounts,
    /// Present when a term list was given.
    pub terms: Option<TermScore>,
}

impl Score {
    pub fn cer(&self) -> f64 {
        self.chars.rate()
    }
    pub fn wer(&self) -> f64 {
        self.words.rate()
    }
}

/// Score `hyp` against `reference`.
pub fn score(reference: &str, hyp: &str, terms: Option<&TermList>) -> Result<Score> {
    let r = normalize::normalize(reference);
    let h = normalize::normalize(hyp);
    let chars = distance::align(&r.chars(), &h.chars())?;
    let words = distance::align(&r.words, &h.words)?;
    Ok(Score { chars, words, terms: terms.map(|t| t.score(reference, hyp)) })
}

/// One scored file pair.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FileScore {
    pub name: String,
    pub cer: f64,
    pub wer: f64,
    pub term_accuracy: Option<f64>,
    #[serde(flatten)]
    pub score: Score,
}

/// Scores of a set of file pairs and their corpus totals.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    pub files: Vec<FileScore>,
    pub total: Score,
    pub cer: f64,
    pub wer: f64,
    pub term_accuracy: Option<f64>,
    /// Reference files with no hypothesis of the same name (directory mode).
    pub missing: Vec<String>,
}

impl Report {
    fn push(&mut self, name: String, s: Score) {
        self.total.chars.add(&s.chars);
        self.total.words.add(&s.words);
        if let Some(t) = &s.terms {
            let tot = self.total.terms.get_or_insert_with(TermScore::default);
            tot.reference = tot.reference.saturating_add(t.reference);
            tot.hypothesis = tot.hypothesis.saturating_add(t.hypothesis);
            tot.matched = tot.matched.saturating_add(t.matched);
            tot.mismatched.extend(t.mismatched.iter().cloned());
        }
        self.cer = self.total.cer();
        self.wer = self.total.wer();
        self.term_accuracy = self.total.terms.as_ref().map(TermScore::accuracy);
        self.files.push(FileScore { name, cer: s.cer(), wer: s.wer(), term_accuracy: s.terms.as_ref().map(TermScore::accuracy), score: s });
    }
}

const TRANSCRIPT_EXT: &[&str] = &["srt", "vtt", "ttml", "dfxp", "txt"];

fn transcript_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let io = |source| Error::Io { path: dir.to_path_buf(), source };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        if path.is_file() && ext.is_some_and(|e| TRANSCRIPT_EXT.contains(&e.as_str())) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Evaluate `hyp` against `reference`. Two files are one pair; two directories pair files by
/// name without extension (`clip01.srt` with `clip01.txt`), and references with no hypothesis are
/// listed in [`Report::missing`].
pub fn evaluate(reference: &Path, hyp: &Path, terms: Option<&TermList>) -> Result<Report> {
    let mut report = Report::default();
    if reference.is_dir() {
        let refs = transcript_files(reference)?;
        if refs.is_empty() {
            return Err(Error::NoFiles(reference.to_path_buf()));
        }
        let hyps = transcript_files(hyp)?;
        for r in refs {
            let name = stem(&r);
            let Some(h) = hyps.iter().find(|h| stem(h) == name) else {
                report.missing.push(name);
                continue;
            };
            let s = score(&load_transcript(&r)?, &load_transcript(h)?, terms)?;
            report.push(name, s);
        }
    } else {
        let s = score(&load_transcript(reference)?, &load_transcript(hyp)?, terms)?;
        report.push(stem(reference), s);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("samples").join(name)
    }

    #[test]
    fn glossary_directory_is_a_term_list() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/overwatch/ko/glossary");
        let terms = load_terms(&dir).unwrap();
        // Old names count as the hero; one-character short forms do not count at all.
        assert_eq!(terms.score("맥크리 궁", "캐서디 궁").accuracy(), 1.0);
        assert_eq!(terms.score("궁", "").reference, 0);
    }

    #[test]
    fn srt_cues_are_joined_in_order() {
        let srt = b"1\n00:00:01,000 --> 00:00:02,000\n<i>\xea\xb2\x90\xec\xa7\x80</i>\n\n2\n00:00:02,000 --> 00:00:03,000\nB\n";
        assert_eq!(transcript_text(srt, Some("srt")).unwrap(), "<i>겐지</i>\nB");
        assert_eq!(transcript_text("그냥 텍스트".as_bytes(), Some("txt")).unwrap(), "그냥 텍스트");
    }

    #[test]
    fn sample_pair_scores() {
        let terms = load_terms(&sample("terms.txt")).unwrap();
        let r = evaluate(&sample("ref"), &sample("hyp"), Some(&terms)).unwrap();
        assert_eq!(r.files.len(), 2);
        assert_eq!(r.missing, ["clip03"]);
        // Counted by hand: samples/README.md.
        let clip01 = &r.files[0];
        assert_eq!(clip01.name, "clip01");
        assert_eq!((clip01.score.chars.reference, clip01.score.chars.substitutions, clip01.score.chars.errors()), (38, 3, 3));
        assert_eq!((clip01.score.words.reference, clip01.score.words.errors()), (16, 4));
        let t = clip01.score.terms.as_ref().unwrap();
        assert_eq!((t.reference, t.matched), (6, 4));
        let clip02 = &r.files[1];
        assert_eq!((clip02.score.chars.reference, clip02.score.chars.errors()), (23, 0));
        assert_eq!((clip02.score.words.reference, clip02.score.words.errors()), (7, 3));
        assert!((r.cer - 3.0 / 61.0).abs() < 1e-12);
        assert!((r.wer - 7.0 / 23.0).abs() < 1e-12);
        assert_eq!(r.term_accuracy, Some(0.75));
    }

    #[test]
    fn single_file_pair() {
        let r = evaluate(&sample("ref/clip02.srt"), &sample("hyp/clip02.txt"), None).unwrap();
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.cer, 0.0);
        assert_eq!(r.term_accuracy, None);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(matches!(evaluate(&sample("nope.srt"), &sample("nope.txt"), None), Err(Error::Io { .. })));
    }
}

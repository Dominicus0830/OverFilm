//! Overlapped speech: who else is talking while a word is said.
//!
//! Runs after the words have their speakers. [`seg`](super::seg) (pyannote segmentation-3.0)
//! looks at 10 s chunks every [`OverlapParams::step_ms`] and says, every ~17 ms, which of up to
//! three *local* speakers are talking, two at once included. Then, in each chunk with overlap:
//!
//! 1. every local speaker is tied to a speaker of the transcript: by the word labels under its
//!    clean (single-voice) frames when it has enough of them, otherwise by its voice vector
//!    (WeSpeaker over its frames) against the voices of the speakers already tied;
//! 2. a word whose frames are at least [`OverlapParams::word_share`] overlapped gets the other
//!    voice of those frames as its second speaker.
//!
//! A local speaker that cannot be tied leaves the second speaker unknown; the word keeps its own
//! label. The recogniser writes one stream of words, so in an overlap the words of both people
//! carry the same pair of speakers.

use serde::{Deserialize, Serialize};

use crate::{SAMPLE_RATE, Word};

use super::cluster;
#[cfg(feature = "qwen")]
use super::seg;
use super::seg::{CHUNK_SAMPLES, CLASSES, LOCAL, frame_centre};

/// Tunables (measured on synthetic Korean conversations with overlaps; see `OVERFILM.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlapParams {
    /// Chunk start every this many ms (chunks are 10 s).
    pub step_ms: i64,
    /// Clean frames a local speaker needs to be tied by the word labels.
    pub min_vote_frames: usize,
    /// Share of the clean votes the winning speaker needs.
    pub vote_share: f32,
    /// A frame is overlapped from this probability of two talkers at once.
    pub overlap_prob: f32,
    /// Share of a word's frames that must be overlapped.
    pub word_share: f32,
    /// Least cosine similarity for tying a local speaker by its voice.
    pub min_similarity: f32,
}

impl Default for OverlapParams {
    fn default() -> Self {
        Self { step_ms: 5000, min_vote_frames: 12, vote_share: 0.6, overlap_prob: 0.3, word_share: 0.2, min_similarity: 0.2 }
    }
}

/// Which local speakers talk in each frame (the most likely powerset class).
pub fn activity(logp: &[[f32; 7]]) -> Vec<[bool; LOCAL]> {
    logp.iter()
        .map(|row| {
            let best = row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |x| x.0);
            let mut a = [false; LOCAL];
            for &s in CLASSES.get(best).copied().unwrap_or(&[]) {
                if let Some(x) = a.get_mut(s) {
                    *x = true;
                }
            }
            a
        })
        .collect()
}

/// Time (ms from the input start) of frame `f` of a chunk starting at `chunk_ms`.
pub fn frame_ms(chunk_ms: i64, f: usize) -> i64 {
    chunk_ms + i64::try_from(frame_centre(f) / (SAMPLE_RATE as usize / 1000)).unwrap_or(i64::MAX / 4)
}

/// Chunk starts (ms) over `len_ms` of audio that hold at least one word.
pub fn chunk_starts(len_ms: i64, words: &[Word], step_ms: i64) -> Vec<i64> {
    let step = step_ms.max(1000);
    let mut out = Vec::new();
    let mut s = 0i64;
    while s < len_ms && out.len() < 1_000_000 {
        if words.iter().any(|w| w.end_ms > s && w.start_ms < s + CHUNK_MS) {
            out.push(s);
        }
        if s + CHUNK_MS >= len_ms {
            break;
        }
        s += step;
    }
    out
}

/// Index of the word whose span holds `t` (words sorted by start), if any.
fn word_at(words: &[Word], t: i64) -> Option<usize> {
    let i = words.partition_point(|w| w.start_ms <= t);
    // a few words back in case spans overlap
    (i.saturating_sub(4)..i).rev().find(|&j| words.get(j).is_some_and(|w| w.start_ms <= t && t < w.end_ms.max(w.start_ms + 1)))
}

/// Clean-frame votes of each local speaker for the transcript's speakers: `votes[s][k]`.
pub fn votes(act: &[[bool; LOCAL]], chunk_ms: i64, words: &[Word], labels: &[Option<usize>], k: usize) -> [Vec<usize>; LOCAL] {
    let mut v: [Vec<usize>; LOCAL] = std::array::from_fn(|_| vec![0; k]);
    for (f, a) in act.iter().enumerate() {
        let on: Vec<usize> = (0..LOCAL).filter(|&s| a.get(s).copied().unwrap_or(false)).collect();
        if let [s] = on[..]
            && let Some(Some(l)) = word_at(words, frame_ms(chunk_ms, f)).and_then(|i| labels.get(i))
            && let Some(c) = v.get_mut(s).and_then(|x| x.get_mut(*l))
        {
            *c += 1;
        }
    }
    v
}

/// The speaker a local speaker's votes decide, if decisive.
pub fn by_votes(v: &[usize], p: &OverlapParams) -> Option<usize> {
    let total: usize = v.iter().sum();
    let (k, &n) = v.iter().enumerate().max_by_key(|x| *x.1)?;
    (total >= p.min_vote_frames && n as f32 >= p.vote_share * total as f32).then_some(k)
}

/// Tie the local speakers of one chunk: decisive votes first, then voices (unit vectors) to the
/// nearest centroid not already taken in this chunk.
pub fn tie(votes: &[Vec<usize>; LOCAL], voices: &[Option<Vec<f32>>; LOCAL], cents: &[Option<Vec<f32>>], p: &OverlapParams) -> [Option<usize>; LOCAL] {
    let mut out = [None; LOCAL];
    for (s, o) in out.iter_mut().enumerate() {
        *o = votes.get(s).and_then(|v| by_votes(v, p));
    }
    for s in 0..LOCAL {
        if out.get(s).copied().flatten().is_some() {
            continue;
        }
        let Some(Some(v)) = voices.get(s) else { continue };
        let best = cents
            .iter()
            .enumerate()
            .filter(|(k, _)| !out.contains(&Some(*k)))
            .filter_map(|(k, c)| c.as_ref().map(|c| (k, 1.0 - cluster::cosine_distance(v, c))))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((k, sim)) = best
            && sim >= p.min_similarity
            && let Some(o) = out.get_mut(s)
        {
            *o = Some(k);
        }
    }
    out
}

/// One analysed chunk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chunk {
    pub start_ms: i64,
    /// most likely set of talkers per frame
    pub act: Vec<[bool; LOCAL]>,
    /// probability that two talk at once, per frame
    pub overlap: Vec<f32>,
    /// probability that each local speaker talks, per frame
    pub talk: Vec<[f32; LOCAL]>,
    /// transcript speaker of each local speaker
    pub ties: [Option<usize>; LOCAL],
}

impl Chunk {
    pub fn new(start_ms: i64, logp: &[[f32; 7]]) -> Self {
        let mut overlap = Vec::with_capacity(logp.len());
        let mut talk = Vec::with_capacity(logp.len());
        for row in logp {
            let pr: Vec<f32> = row.iter().map(|&l| l.exp()).collect();
            let mut t = [0f32; LOCAL];
            let mut ov = 0f32;
            for (c, &q) in pr.iter().enumerate() {
                let set = CLASSES.get(c).copied().unwrap_or(&[]);
                if set.len() >= 2 {
                    ov += q;
                }
                for &s in set {
                    if let Some(x) = t.get_mut(s) {
                        *x += q;
                    }
                }
            }
            overlap.push(ov);
            talk.push(t);
        }
        Self { start_ms, act: activity(logp), overlap, talk, ties: [None; LOCAL] }
    }

    /// Weight of time `t` in this chunk: highest at its centre (the model sees most context
    /// there), 0 outside.
    fn weight(&self, t: i64) -> f32 {
        let x = (t - self.start_ms) as f32 / CHUNK_MS as f32;
        if (0.0..1.0).contains(&x) { 0.02 + (std::f32::consts::PI * x).sin().powi(2) } else { 0.0 }
    }
}

/// Second speaker of each word: averaged over the chunks that hold the word, the share of its
/// frames with overlap probability at least [`OverlapParams::overlap_prob`] must reach
/// [`OverlapParams::word_share`]; the second speaker is the tied speaker other than the word's
/// own that is most likely talking in those frames.
pub fn second_speakers(words: &[Word], labels: &[Option<usize>], chunks: &[Chunk], p: &OverlapParams) -> Vec<Option<usize>> {
    let mut out = vec![None; words.len()];
    for (i, w) in words.iter().enumerate() {
        let c = (w.start_ms + w.end_ms) / 2;
        let mine = labels.get(i).copied().flatten();
        let (mut share, mut wsum) = (0f32, 0f32);
        let mut other: Vec<f32> = Vec::new();
        // chunks are sorted by start; only those around the word can hold it
        let from = chunks.partition_point(|x| x.start_ms + CHUNK_MS <= w.start_ms);
        for ch in chunks.iter().skip(from).take_while(|x| x.start_ms <= w.end_ms) {
            let wt = ch.weight(c);
            if wt <= 0.0 {
                continue;
            }
            let (mut n, mut n_ov) = (0usize, 0usize);
            for (f, (&ov, talk)) in ch.overlap.iter().zip(&ch.talk).enumerate() {
                let t = frame_ms(ch.start_ms, f);
                if t < w.start_ms || t >= w.end_ms {
                    continue;
                }
                n += 1;
                if ov < p.overlap_prob {
                    continue;
                }
                n_ov += 1;
                for (s, &q) in talk.iter().enumerate() {
                    if let Some(Some(k)) = ch.ties.get(s)
                        && Some(*k) != mine
                    {
                        if other.len() <= *k {
                            other.resize(*k + 1, 0.0);
                        }
                        if let Some(o) = other.get_mut(*k) {
                            *o += q * wt;
                        }
                    }
                }
            }
            if n > 0 {
                share += wt * n_ov as f32 / n as f32;
                wsum += wt;
            }
        }
        if wsum > 0.0 && share / wsum >= p.word_share {
            let best = other.iter().enumerate().filter(|x| *x.1 > 0.0).max_by(|a, b| a.1.total_cmp(b.1)).map(|x| x.0);
            if let Some(o) = out.get_mut(i) {
                *o = best;
            }
        }
    }
    out
}

/// Chunk length in ms.
pub const CHUNK_MS: i64 = (CHUNK_SAMPLES / (SAMPLE_RATE as usize / 1000)) as i64;

/// The voice of a local speaker's frames in a WeSpeaker frame map: the 80 ms map frames whose
/// segmentation frames all have `want(activity)` (at least one).
pub fn map_frames(act: &[[bool; LOCAL]], map_frames: usize, want: &dyn Fn(&[bool; LOCAL]) -> bool) -> Vec<usize> {
    let mut good = vec![(0usize, true); map_frames];
    for (f, a) in act.iter().enumerate() {
        let ms = frame_centre(f) / (SAMPLE_RATE as usize / 1000);
        if let Some(g) = good.get_mut(ms / 80) {
            g.0 += 1;
            g.1 &= want(a);
        }
    }
    good.iter().enumerate().filter(|(_, g)| g.0 > 0 && g.1).map(|(j, _)| j).collect()
}

#[cfg(feature = "qwen")]
impl super::Diarizer {
    /// Second speaker of each word (see the module docs); `labels` are the words' speakers.
    /// `progress(fraction)` returns false to cancel.
    pub fn overlaps(
        &self,
        net: &seg::SegNet,
        audio: &[f32],
        words: &[Word],
        labels: &[Option<usize>],
        p: &OverlapParams,
        progress: &mut dyn FnMut(f32) -> bool,
    ) -> Result<Vec<Option<usize>>, crate::AsrError> {
        let k = labels.iter().flatten().max().map_or(0, |m| m + 1);
        if k < 2 || words.is_empty() {
            return Ok(vec![None; words.len()]);
        }
        let per_ms = SAMPLE_RATE as usize / 1000;
        let len_ms = i64::try_from(audio.len() / per_ms).unwrap_or(0);
        let starts = chunk_starts(len_ms, words, p.step_ms);
        let mut chunks = Vec::new();
        // pass 1: activity, votes; voices of decisively tied local speakers make the centroids
        let mut sums: Vec<(Vec<f32>, usize)> = vec![(Vec::new(), 0); k];
        let mut pending = Vec::new();
        for (n, &cs) in starts.iter().enumerate() {
            if !progress(n as f32 / starts.len().max(1) as f32) {
                return Err(crate::AsrError::Cancelled);
            }
            let a = usize::try_from(cs).unwrap_or(0).saturating_mul(per_ms).min(audio.len());
            let b = a.saturating_add(CHUNK_SAMPLES).min(audio.len());
            let Some(block) = audio.get(a..b) else { continue };
            let mut ch = Chunk::new(cs, &net.forward(block)?);
            let act = ch.act.clone();
            let v = votes(&act, cs, words, labels, k);
            let ties0: [Option<usize>; LOCAL] = std::array::from_fn(|s| v.get(s).and_then(|v| by_votes(v, p)));
            ch.ties = ties0;
            // voices are needed only for a local speaker that talks in an overlap and the word
            // labels could not tie
            let untied = (0..LOCAL).any(|s| {
                ties0.get(s).copied().flatten().is_none()
                    && ch.overlap.iter().zip(&ch.talk).filter(|(o, t)| **o >= p.overlap_prob && t.get(s).is_some_and(|&q| q >= 0.5)).count() >= 3
            });
            if !untied {
                chunks.push(ch);
                continue;
            }
            // voices of every local speaker here: clean frames, or all its frames when it has
            // too few clean ones
            let (mf, map) = self.frame_map(block)?;
            let mut voices: [Option<Vec<f32>>; LOCAL] = Default::default();
            for (s, slot) in voices.iter_mut().enumerate() {
                let only = |a: &[bool; LOCAL]| a.get(s).copied().unwrap_or(false) && a.iter().filter(|&&x| x).count() == 1;
                let any = |a: &[bool; LOCAL]| a.get(s).copied().unwrap_or(false);
                let mut idx = map_frames(&act, mf, &only);
                if idx.len() < 6 {
                    idx = map_frames(&act, mf, &any);
                }
                if idx.len() < 3 {
                    continue;
                }
                let mut rows = Vec::with_capacity(idx.len() * super::net::FEAT);
                for j in &idx {
                    rows.extend_from_slice(map.get(j * super::net::FEAT..(j + 1) * super::net::FEAT).unwrap_or(&[]));
                }
                *slot = self.pool(&rows, 0, rows.len() / super::net::FEAT).map(|mut v| {
                    cluster::normalize(&mut v);
                    v
                });
                if let (Some(Some(t)), Some(v)) = (ties0.get(s), slot.as_ref())
                    && let Some(acc) = sums.get_mut(*t)
                {
                    if acc.0.is_empty() {
                        acc.0 = vec![0.0; v.len()];
                    }
                    acc.0.iter_mut().zip(v).for_each(|(a, b)| *a += b);
                    acc.1 += 1;
                }
            }
            pending.push((chunks.len(), v, voices));
            chunks.push(ch);
        }
        let cents: Vec<Option<Vec<f32>>> = sums
            .into_iter()
            .map(|(mut s, n)| {
                (n > 0).then(|| {
                    cluster::normalize(&mut s);
                    s
                })
            })
            .collect();
        // pass 2: tie the rest by voice
        for (ci, v, voices) in pending {
            if let Some(c) = chunks.get_mut(ci) {
                c.ties = tie(&v, &voices, &cents, p);
            }
        }
        Ok(second_speakers(words, labels, &chunks, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speakers::seg::CHUNK_FRAMES;

    fn w(s: i64, e: i64) -> Word {
        Word { text: "말".into(), start_ms: s, end_ms: e, ..Default::default() }
    }

    fn logp(act: &[[bool; 3]]) -> Vec<[f32; 7]> {
        act.iter()
            .map(|a| {
                let on: Vec<usize> = (0..3).filter(|&s| a[s]).collect();
                let c = CLASSES.iter().position(|x| *x == on.as_slice()).unwrap();
                let mut r = [-20f32; 7];
                r[c] = 0.0;
                r
            })
            .collect()
    }

    #[test]
    fn activity_from_powerset() {
        let mut r = [[-9f32; 7]; 3];
        r[0][0] = 0.0;
        r[1][2] = 0.0;
        r[2][5] = 0.0;
        assert_eq!(activity(&r), vec![[false; 3], [false, true, false], [true, false, true]]);
    }

    #[test]
    fn votes_ties_and_second_speakers() {
        // words of speaker 0 then 1; local A clean over the first, B over the second, both at once
        // around the boundary
        let words = vec![w(0, 3000), w(3000, 6000)];
        let labels = vec![Some(0), Some(1)];
        let act: Vec<[bool; 3]> = (0..CHUNK_FRAMES)
            .map(|f| {
                let t = frame_ms(0, f);
                [t < 3300, (2700..6000).contains(&t), false]
            })
            .collect();
        let v = votes(&act, 0, &words, &labels, 2);
        let p = OverlapParams::default();
        assert_eq!(by_votes(&v[0], &p), Some(0));
        assert_eq!(by_votes(&v[1], &p), Some(1));
        let ties = tie(&v, &[None, None, None], &[], &p);
        assert_eq!(ties, [Some(0), Some(1), None]);
        let mut ch = Chunk::new(0, &logp(&act));
        assert_eq!(ch.act, act);
        assert!(ch.overlap.iter().zip(&act).all(|(&o, a)| (o > 0.99) == (a[0] && a[1])));
        ch.ties = ties;
        let chunks = vec![ch];
        // 2.7–3.0 of word 0 is overlapped (10 %): below the share
        assert_eq!(second_speakers(&words, &labels, &chunks, &p), vec![None, None]);
        let words2 = vec![w(2000, 3000), w(3000, 4000)];
        assert_eq!(second_speakers(&words2, &labels, &chunks, &p), vec![Some(1), Some(0)]);
        // a local speaker with no clean frames is tied by its voice, never to a taken speaker
        let vb = [vec![30, 0], vec![0, 0], vec![0, 0]];
        let cents = vec![Some(vec![1.0, 0.0]), Some(vec![0.0, 1.0])];
        assert_eq!(tie(&vb, &[None, Some(vec![0.9, 0.436]), None], &cents, &p), [Some(0), Some(1), None]);
        assert_eq!(tie(&vb, &[None, Some(vec![-1.0, 0.0]), None], &cents, &p), [Some(0), None, None]);
    }

    #[test]
    fn chunks_and_map_frames() {
        let words = vec![w(1000, 1500), w(31_000, 31_500)];
        assert_eq!(chunk_starts(40_000, &words, 5000), vec![0, 25_000, 30_000]);
        assert!(chunk_starts(0, &words, 5000).is_empty());
        assert!(chunk_starts(i64::MAX / 2, &[], 0).len() <= 1_000_000);
        let act: Vec<[bool; 3]> = (0..CHUNK_FRAMES).map(|f| [frame_ms(0, f) < 1000, false, false]).collect();
        let idx = map_frames(&act, 125, &|a| a[0]);
        assert_eq!(idx.first(), Some(&0));
        assert!(idx.iter().all(|&j| j < 13), "{idx:?}");
        assert!(second_speakers(&[w(0, 100)], &[None], &[], &OverlapParams::default()).iter().all(Option::is_none));
    }
}

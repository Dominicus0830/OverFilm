//! Who spoke when: speaker labels for a word-timed transcript.
//!
//! 1. **Speech**: the recognised words, joined where the pause between them is short.
//! 2. **Voice vectors**: [`net`] (WeSpeaker ResNet34) runs over 10 s blocks of filterbank
//!    features ([`fbank`]); every [`Params::window_ms`] window of speech, every
//!    [`Params::step_ms`], gets a 256-value voice vector by statistics pooling over its frames.
//! 3. **Clustering** ([`cluster`]): average linkage on cosine distance, cut at
//!    [`Params::threshold`] (or at the speaker count the user gave); windows are then reassigned
//!    to the nearest speaker centroid, and speakers with too little speech are folded into their
//!    nearest neighbour.
//! 4. **Words**: each word takes the speaker whose windows overlap it most; very short islands
//!    between two stretches of the same speaker are absorbed.
//!
//! Speakers are numbered by first appearance. Game voice (hero lines) is detected separately by
//! text (`overfilm_glossary::voicelines`) and can mark a whole speaker as game audio.

pub mod cluster;
pub mod fbank;
#[cfg(feature = "qwen")]
pub mod net;

use serde::{Deserialize, Serialize};

use crate::{SAMPLE_RATE, Word};

/// Tunables (defaults measured on synthetic Korean conversations; see `OVERFILM.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub window_ms: i64,
    pub step_ms: i64,
    /// A pause at least this long separates speech regions; windows never cross one, so a
    /// window rarely holds two voices.
    pub pause_ms: i64,
    /// Cosine distance at which clustering stops merging.
    pub threshold: f32,
    /// The number of speakers when the user knows it.
    pub speakers: Option<usize>,
    /// At most this many speakers are kept.
    pub max_speakers: usize,
    /// A speaker needs at least this much speech (seconds) to be kept, or 8 % of all speech when
    /// that is less (short clips).
    pub min_speaker_seconds: f32,
    /// Words of another speaker shorter than this between two stretches of one speaker are
    /// absorbed.
    pub min_island_ms: i64,
}

impl Default for Params {
    fn default() -> Self {
        Self { window_ms: 1500, step_ms: 500, pause_ms: 250, threshold: 0.7, speakers: None, max_speakers: 8, min_speaker_seconds: 10.0, min_island_ms: 500 }
    }
}

/// Result: one label per word (`None` when no window reached it) and per-speaker totals.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Labels {
    pub words: Vec<Option<usize>>,
    pub speakers: Vec<SpeakerStat>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeakerStat {
    /// 0-based; shown as "화자 {id + 1}"
    pub id: usize,
    /// Seconds of words.
    pub seconds: f32,
    pub words: usize,
}

/// Speech regions `[start, end)` in ms: the words, joined across pauses shorter than `pause`.
pub fn speech_regions(words: &[Word], pause: i64) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = Vec::new();
    for w in words {
        let (s, e) = (w.start_ms.max(0), w.end_ms.max(w.start_ms).max(0));
        match out.last_mut() {
            Some(last) if s - last.1 < pause => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// Most analysis windows per input (a day of speech at the default step).
pub const MAX_WINDOWS: usize = 200_000;

/// Analysis windows over the regions: every `step` ms a `win` ms window; a region shorter than
/// the window gets one window over the whole region (if it is at least 300 ms).
pub fn windows(regions: &[(i64, i64)], win: i64, step: i64) -> Vec<(i64, i64)> {
    let (win, step) = (win.max(200), step.max(50));
    let mut out = Vec::new();
    for &(s, e) in regions {
        if out.len() >= MAX_WINDOWS {
            break;
        }
        let len = e - s;
        if len < 300 {
            continue;
        }
        if len <= win {
            out.push((s, e));
            continue;
        }
        let mut t = s;
        while t + win < e && out.len() < MAX_WINDOWS {
            out.push((t, t + win));
            t += step;
        }
        out.push((e - win, e));
    }
    out
}

/// Block layout for the network: `BLOCK_MS` blocks every `BLOCK_STEP_MS`, so every window up to
/// `BLOCK_MS - BLOCK_STEP_MS` long lies wholly inside one block.
pub const BLOCK_MS: i64 = 10_000;
pub const BLOCK_STEP_MS: i64 = 8_000;

/// The block (index) that holds window `(s, e)` with its centre nearest the block centre.
pub fn block_of(s: i64, e: i64) -> usize {
    let c = (s + e) / 2;
    let k = usize::try_from(((c - BLOCK_MS / 2) as f64 / BLOCK_STEP_MS as f64).round().max(0.0) as i64).unwrap_or(0);
    // the nearest-centred block may not contain the whole window near its edges
    let start = |k: usize| k as i64 * BLOCK_STEP_MS;
    if s < start(k) && k > 0 {
        k - 1
    } else if e > start(k) + BLOCK_MS {
        k + 1
    } else {
        k
    }
}

fn ms_to_sample(ms: i64) -> usize {
    usize::try_from(ms.max(0)).unwrap_or(0).saturating_mul(SAMPLE_RATE as usize / 1000)
}

/// Voice vectors of `wins` (unit length; `None` where the window had no frames), computing each
/// block's frame map with `frame_map(block_audio) -> (out_frames, map)` and pooling with
/// `pool(map, a, b)`. `progress(done, total)` returns false to cancel.
pub fn window_vectors(
    audio: &[f32],
    wins: &[(i64, i64)],
    frame_map: &mut dyn FnMut(&[f32]) -> Result<(usize, Vec<f32>), crate::AsrError>,
    pool: &dyn Fn(&[f32], usize, usize) -> Option<Vec<f32>>,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<Vec<Option<Vec<f32>>>, crate::AsrError> {
    let mut out: Vec<Option<Vec<f32>>> = vec![None; wins.len()];
    let mut by_block: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for (i, &(s, e)) in wins.iter().enumerate() {
        by_block.entry(block_of(s, e)).or_default().push(i);
    }
    let total = by_block.len();
    // output frame length in ms (8 feature frames of 10 ms)
    let frame_ms = 80i64;
    for (done, (k, idx)) in by_block.into_iter().enumerate() {
        if !progress(done, total) {
            return Err(crate::AsrError::Cancelled);
        }
        let bs = k as i64 * BLOCK_STEP_MS;
        let a = ms_to_sample(bs).min(audio.len());
        let b = ms_to_sample(bs + BLOCK_MS).min(audio.len());
        let Some(block) = audio.get(a..b) else { continue };
        let (frames, map) = frame_map(block)?;
        for i in idx {
            let Some(&(s, e)) = wins.get(i) else { continue };
            let fa = usize::try_from(((s - bs) / frame_ms).max(0)).unwrap_or(0).min(frames);
            let fb = usize::try_from(((e - bs) + frame_ms - 1) / frame_ms).unwrap_or(0).min(frames);
            if fb > fa
                && let Some(mut v) = pool(&map, fa, fb)
            {
                cluster::normalize(&mut v);
                if let Some(slot) = out.get_mut(i) {
                    *slot = Some(v);
                }
            }
        }
    }
    Ok(out)
}

/// Speaker of each window from its voice vector (`None` stays `None`).
pub fn cluster_windows(vecs: &[Option<Vec<f32>>], wins: &[(i64, i64)], p: &Params) -> Vec<Option<usize>> {
    let have: Vec<usize> = (0..vecs.len()).filter(|&i| vecs.get(i).is_some_and(Option::is_some)).collect();
    if have.is_empty() {
        return vec![None; vecs.len()];
    }
    let dim = vecs.iter().flatten().next().map_or(0, Vec::len);
    // subsample evenly for the tree when there are too many windows
    let stride = have.len().div_ceil(cluster::MAX_POINTS).max(1);
    let sample: Vec<Vec<f32>> = have.iter().step_by(stride).filter_map(|&i| vecs.get(i).cloned().flatten()).collect();
    let merges = cluster::average_linkage(&sample);
    let want = p.speakers.map(|k| k.clamp(1, p.max_speakers.max(1)));
    let labels = cluster::cut(sample.len(), &merges, p.threshold, want);
    let k = labels.iter().max().map_or(1, |m| m + 1);
    let mut cents = cluster::centroids(&sample, &labels, k, dim);
    // assign every window to its nearest centroid, then fold small speakers away
    let dur = |i: usize| wins.get(i).map_or(0, |w| w.1 - w.0) as f32 / 1000.0;
    let assign = |cents: &[Vec<f32>]| -> Vec<Option<usize>> {
        (0..vecs.len()).map(|i| vecs.get(i).and_then(|v| v.as_ref()).and_then(|v| cluster::nearest(v, cents))).collect()
    };
    let mut lab = assign(&cents);
    loop {
        let mut secs = vec![0f32; cents.len()];
        for (i, l) in lab.iter().enumerate() {
            if let Some(s) = l.and_then(|l| secs.get_mut(l)) {
                // windows overlap; scale by step so the total approximates speech time
                *s += dur(i).min(p.step_ms.max(1) as f32 / 1000.0);
            }
        }
        let total: f32 = secs.iter().sum();
        let keep_min = if want.is_some() { 0.0 } else { p.min_speaker_seconds.min(0.08 * total) };
        let over = cents.len() > p.max_speakers.max(1);
        let smallest = secs.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1)).map(|(i, &s)| (i, s));
        match smallest {
            Some((i, s)) if cents.len() > 1 && (s < keep_min || over) => {
                cents.remove(i);
                lab = assign(&cents);
            }
            _ => break,
        }
    }
    lab
}

/// Label each word with the speaker whose windows overlap it most, absorb short islands and
/// number speakers by first appearance.
pub fn label_words(words: &[Word], wins: &[(i64, i64)], win_labels: &[Option<usize>], p: &Params) -> Labels {
    let k = win_labels.iter().flatten().max().map_or(0, |m| m + 1);
    let mut raw: Vec<Option<usize>> = Vec::with_capacity(words.len());
    // windows are sorted by start within each region; scan with a moving lower bound
    let mut lo = 0usize;
    for w in words {
        while wins.get(lo).is_some_and(|x| x.1 <= w.start_ms) && lo + 1 < wins.len() && wins.get(lo + 1).is_some_and(|n| n.0 <= w.start_ms) {
            lo += 1;
        }
        let mut score = vec![0f64; k];
        let mut j = lo.saturating_sub(8);
        while let Some(&(s, e)) = wins.get(j) {
            if s > w.end_ms + 2000 {
                break;
            }
            let ov = (e.min(w.end_ms.max(w.start_ms + 1)) - s.max(w.start_ms)).max(0) as f64;
            // a window counts more the nearer its centre is to the word's centre
            let dc = (((s + e) / 2) - (w.start_ms + w.end_ms) / 2).abs() as f64;
            if ov > 0.0
                && let Some(Some(l)) = win_labels.get(j)
                && let Some(sc) = score.get_mut(*l)
            {
                *sc += ov / (1.0 + dc / 500.0);
            }
            j += 1;
        }
        let best = score.iter().enumerate().filter(|x| *x.1 > 0.0).max_by(|a, b| a.1.total_cmp(b.1)).map(|x| x.0);
        raw.push(best);
    }
    // words no window reached take the previous label
    let mut last = None;
    for l in raw.iter_mut() {
        if l.is_none() {
            *l = last;
        } else {
            last = *l;
        }
    }
    // absorb short islands between two runs of the same speaker
    let mut i = 0;
    while i < raw.len() {
        let mut j = i;
        while j < raw.len() && raw.get(j) == raw.get(i) {
            j += 1;
        }
        let dur = match (words.get(i), words.get(j.saturating_sub(1))) {
            (Some(a), Some(b)) => b.end_ms - a.start_ms,
            _ => 0,
        };
        if i > 0 && j < raw.len() && raw.get(i - 1) == raw.get(j) && dur < p.min_island_ms {
            let v = raw.get(i - 1).copied().flatten();
            raw.iter_mut().take(j).skip(i).for_each(|x| *x = v);
        }
        i = j;
    }
    // number by first appearance
    let mut order: Vec<usize> = Vec::new();
    for l in raw.iter().flatten() {
        if !order.contains(l) {
            order.push(*l);
        }
    }
    let words_out: Vec<Option<usize>> = raw.iter().map(|l| l.and_then(|l| order.iter().position(|&o| o == l))).collect();
    let mut speakers: Vec<SpeakerStat> = (0..order.len()).map(|id| SpeakerStat { id, ..Default::default() }).collect();
    for (w, l) in words.iter().zip(&words_out) {
        if let Some(s) = l.and_then(|l| speakers.get_mut(l)) {
            s.words += 1;
            s.seconds += (w.end_ms - w.start_ms).max(0) as f32 / 1000.0;
        }
    }
    Labels { words: words_out, speakers }
}

/// The whole method with the network loaded (feature `qwen`).
#[cfg(feature = "qwen")]
pub struct Diarizer {
    net: net::Net,
    fbank: fbank::Fbank,
}

#[cfg(feature = "qwen")]
impl Diarizer {
    /// Load the speaker model from its folder (see [`crate::catalogue::SPEAKER`]).
    pub fn load(dir: &std::path::Path) -> Result<Self, crate::AsrError> {
        Ok(Self { net: net::Net::load(&dir.join("pytorch_model.bin"))?, fbank: fbank::Fbank::new() })
    }

    /// Frame map of a block of audio: (output frames, map).
    pub fn frame_map(&self, block: &[f32]) -> Result<(usize, Vec<f32>), crate::AsrError> {
        let frames = fbank::frames(block.len());
        let mut feats = self.fbank.compute(block);
        fbank::subtract_mean(&mut feats);
        let map = self.net.frame_map(&feats, frames)?;
        Ok((map.len() / net::FEAT, map))
    }

    /// Voice vector of output frames `[a, b)` of a frame map (not normalised).
    pub fn pool(&self, map: &[f32], a: usize, b: usize) -> Option<Vec<f32>> {
        self.net.embed(map, a, b)
    }

    /// Voice vector of a whole stretch of audio (unit length).
    pub fn embed(&self, audio: &[f32]) -> Result<Option<Vec<f32>>, crate::AsrError> {
        let (n, map) = self.frame_map(audio)?;
        Ok(self.net.embed(&map, 0, n).map(|mut v| {
            cluster::normalize(&mut v);
            v
        }))
    }

    /// Label the words of `audio` (16 kHz mono). `progress(fraction)` returns false to cancel.
    pub fn run(&self, audio: &[f32], words: &[Word], p: &Params, progress: &mut dyn FnMut(f32) -> bool) -> Result<Labels, crate::AsrError> {
        let wins = windows(&speech_regions(words, p.pause_ms), p.window_ms, p.step_ms);
        let vecs =
            window_vectors(audio, &wins, &mut |b| self.frame_map(b), &|m, a, b| self.net.embed(m, a, b), &mut |d, t| progress(d as f32 / t.max(1) as f32))?;
        let wl = cluster_windows(&vecs, &wins, p);
        Ok(label_words(words, &wins, &wl, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: i64, e: i64) -> Word {
        Word { text: "말".into(), start_ms: s, end_ms: e, ..Default::default() }
    }

    #[test]
    fn regions_windows_and_blocks() {
        let r = speech_regions(&[w(0, 500), w(700, 1200), w(3000, 3100), w(5000, 9000)], 400);
        assert_eq!(r, vec![(0, 1200), (3000, 3100), (5000, 9000)]);
        let wins = windows(&r, 1500, 500);
        assert_eq!(wins.first(), Some(&(0, 1200)));
        assert!(!wins.contains(&(3000, 3100)), "too short");
        assert_eq!(wins.last(), Some(&(7500, 9000)));
        assert!(wins.iter().all(|&(s, e)| e - s <= 1500));
        for &(s, e) in &[(0, 1500), (7900, 9400), (8500, 10_000), (9_999, 11_499), (123_456, 124_956)] {
            let k = block_of(s, e) as i64;
            assert!(s >= k * BLOCK_STEP_MS && e <= k * BLOCK_STEP_MS + BLOCK_MS, "{s}-{e} in block {k}");
        }
    }

    #[test]
    fn two_voices_become_two_speakers() {
        // fake network: each block's "map" is one value per 80 ms frame telling the voice
        let words: Vec<Word> = (0..40).map(|i| w(i * 500, i * 500 + 450)).collect();
        let voice = |ms: i64| if (ms / 5000) % 2 == 0 { [1.0f32, 0.1] } else { [0.1, 1.0] };
        let audio = vec![0f32; 16 * 20_000];
        let wins = windows(&speech_regions(&words, 250), 1500, 500);
        let mut block_start = Vec::new();
        let vecs = window_vectors(
            &audio,
            &wins,
            &mut |b| {
                block_start.push(b.len());
                Ok((b.len() / 16 / 80, Vec::new()))
            },
            &|_, _, _| None,
            &mut |_, _| true,
        )
        .unwrap();
        assert!(vecs.iter().all(Option::is_none));
        // pool by window directly
        let vecs: Vec<Option<Vec<f32>>> = wins
            .iter()
            .map(|&(s, e)| {
                let mut v = voice((s + e) / 2).to_vec();
                cluster::normalize(&mut v);
                Some(v)
            })
            .collect();
        let p = Params { min_speaker_seconds: 0.5, ..Params::default() };
        let wl = cluster_windows(&vecs, &wins, &p);
        let l = label_words(&words, &wins, &wl, &p);
        assert_eq!(l.speakers.len(), 2);
        assert_eq!(l.words[0], Some(0));
        assert_eq!(l.words[12], Some(1));
        assert_eq!(l.words[25], Some(0));
        // the user's count wins
        let one = cluster_windows(&vecs, &wins, &Params { speakers: Some(1), ..p.clone() });
        assert_eq!(label_words(&words, &wins, &one, &p).speakers.len(), 1);
    }

    #[test]
    fn islands_absorbed_and_empty_inputs() {
        let words = vec![w(0, 1000), w(1000, 1200), w(1200, 2500)];
        let wins = vec![(0, 1000), (1000, 1200), (1200, 2500)];
        let l = label_words(&words, &wins, &[Some(0), Some(1), Some(0)], &Params::default());
        assert_eq!(l.words, vec![Some(0); 3]);
        assert_eq!(l.speakers.len(), 1);
        let e = label_words(&[], &[], &[], &Params::default());
        assert!(e.words.is_empty() && e.speakers.is_empty());
        assert!(cluster_windows(&[None, None], &[(0, 1), (1, 2)], &Params::default()).iter().all(Option::is_none));
        // hostile times
        let odd = vec![Word { start_ms: i64::MIN / 4, end_ms: i64::MAX / 4, ..w(0, 0) }];
        let _ = windows(&speech_regions(&odd, 250), 1500, 500);
    }
}

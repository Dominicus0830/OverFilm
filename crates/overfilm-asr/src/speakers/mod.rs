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
//! 4. **Words**: each word scores every speaker by its windows' similarity to the speaker's
//!    centroid; a Viterbi pass makes speaker changes cheap at pauses and sentence ends and costly
//!    inside fluent speech ([`label_words`]).
//!
//! 5. **Overlap** ([`overlap`], optional): [`seg`] (pyannote segmentation-3.0) finds where two
//!    people talk at once, and such words get the other voice as a second speaker.
//!
//! Speakers are numbered by first appearance. Game voice (hero lines) is detected separately by
//! text (`overfilm_glossary::voicelines`) and can mark a whole speaker as game audio.

pub mod cluster;
pub mod fbank;
#[cfg(feature = "qwen")]
pub mod net;
pub mod overlap;
pub mod seg;

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
    /// Cost (similarity × seconds) of changing speaker between two words of fluent speech; at a
    /// pause or sentence end it is nearly free.
    pub switch_cost: f32,
    /// Enrolled voices: a speaker whose centroid is within [`KNOWN_MATCH`] of one takes its name.
    #[serde(default)]
    pub known: Vec<KnownVoice>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            window_ms: 1500,
            step_ms: 500,
            pause_ms: 250,
            threshold: 0.7,
            speakers: None,
            max_speakers: 8,
            min_speaker_seconds: 10.0,
            switch_cost: 0.4,
            known: Vec::new(),
        }
    }
}

/// Result: one label per word (`None` when no window reached it) and per-speaker totals.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Labels {
    pub words: Vec<Option<usize>>,
    pub speakers: Vec<SpeakerStat>,
    /// Second speaker per word where two talk at once ([`overlap`]; empty when not looked for).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<Option<usize>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpeakerStat {
    /// 0-based; shown as "화자 {id + 1}"
    pub id: usize,
    /// Seconds of words.
    pub seconds: f32,
    pub words: usize,
    /// The enrolled voice this speaker matched ([`Params::known`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A voice the user enrolled: a name and the mean voice vector of a sample (unit length).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KnownVoice {
    pub name: String,
    pub vector: Vec<f32>,
}

/// Size of a voice vector (also without the network compiled in).
pub const fn net_embed_size() -> usize {
    256
}

/// Cosine distance at which a speaker is taken to be an enrolled voice.
pub const KNOWN_MATCH: f32 = 0.55;

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

/// Fewest frames a window keeps in [`window_vectors_skipping`] (0.4 s).
pub const MIN_KEPT_FRAMES: usize = 5;

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

#[cfg_attr(not(feature = "qwen"), allow(dead_code))]
fn ms_of(samples: usize) -> i64 {
    i64::try_from(samples / (SAMPLE_RATE as usize / 1000)).unwrap_or(i64::MAX)
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
    window_vectors_skipping(audio, wins, frame_map, pool, None, progress)
}

/// [`window_vectors`] leaving out the 80 ms frames whose centre (ms) `skip` rejects (overlapped
/// speech); a window left with fewer than [`MIN_KEPT_FRAMES`] frames (or under half of its own)
/// gets no vector.
pub fn window_vectors_skipping(
    audio: &[f32],
    wins: &[(i64, i64)],
    frame_map: &mut dyn FnMut(&[f32]) -> Result<(usize, Vec<f32>), crate::AsrError>,
    pool: &dyn Fn(&[f32], usize, usize) -> Option<Vec<f32>>,
    skip: Option<&dyn Fn(i64) -> bool>,
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
            let pooled = match skip {
                None => (fb > fa).then(|| pool(&map, fa, fb)).flatten(),
                Some(skip) => {
                    let feat = map.len() / frames.max(1);
                    let keep: Vec<usize> = (fa..fb).filter(|&j| !skip(bs + j as i64 * frame_ms + frame_ms / 2)).collect();
                    if keep.len() < MIN_KEPT_FRAMES || keep.len() * 2 < fb.saturating_sub(fa) {
                        None
                    } else if keep.len() == fb - fa {
                        pool(&map, fa, fb)
                    } else {
                        let mut rows = Vec::with_capacity(keep.len() * feat);
                        for j in &keep {
                            rows.extend_from_slice(map.get(j * feat..(j + 1) * feat).unwrap_or(&[]));
                        }
                        pool(&rows, 0, rows.len() / feat.max(1))
                    }
                }
            };
            if let Some(mut v) = pooled {
                cluster::normalize(&mut v);
                if let Some(slot) = out.get_mut(i) {
                    *slot = Some(v);
                }
            }
        }
    }
    Ok(out)
}

/// Clustering result: the speaker of each window (`None` where it had no vector) and each
/// speaker's centroid (unit length).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clusters {
    pub labels: Vec<Option<usize>>,
    pub centroids: Vec<Vec<f32>>,
    /// Enrolled name of each centroid's speaker, if it matched one.
    pub names: Vec<Option<String>>,
}

/// Give enrolled names to centroids: closest pairs first, each voice and centroid used once,
/// only within [`KNOWN_MATCH`].
pub fn match_known(centroids: &[Vec<f32>], known: &[KnownVoice]) -> Vec<Option<String>> {
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (ci, c) in centroids.iter().enumerate() {
        for (ki, k) in known.iter().enumerate() {
            if k.vector.len() == c.len() {
                let d = cluster::cosine_distance(c, &k.vector);
                if d <= KNOWN_MATCH {
                    pairs.push((d, ci, ki));
                }
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut names = vec![None; centroids.len()];
    let mut used = vec![false; known.len()];
    for (_, ci, ki) in pairs {
        if names.get(ci).is_some_and(Option::is_none)
            && !used.get(ki).copied().unwrap_or(true)
            && let (Some(n), Some(u), Some(k)) = (names.get_mut(ci), used.get_mut(ki), known.get(ki))
        {
            *n = Some(k.name.clone());
            *u = true;
        }
    }
    names
}

/// Speaker of each window from its voice vector (`None` stays `None`).
pub fn cluster_windows(vecs: &[Option<Vec<f32>>], wins: &[(i64, i64)], p: &Params) -> Clusters {
    let have: Vec<usize> = (0..vecs.len()).filter(|&i| vecs.get(i).is_some_and(Option::is_some)).collect();
    if have.is_empty() {
        return Clusters { labels: vec![None; vecs.len()], centroids: Vec::new(), names: Vec::new() };
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
        // a speaker that is an enrolled voice is kept however little it says
        let named = match_known(&cents, &p.known);
        for (s, n) in secs.iter_mut().zip(&named) {
            if n.is_some() {
                *s = f32::INFINITY;
            }
        }
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
    let names = match_known(&cents, &p.known);
    Clusters { labels: lab, centroids: cents, names }
}

/// How strongly the boundary after word `a` (before `b`) separates turns: 1 at a sentence end or
/// a long pause, 0 inside fluent speech.
fn boundary(a: &Word, b: &Word) -> f64 {
    let t = a.text.trim_end();
    if t.ends_with(['.', '?', '!', '…']) {
        return 1.0;
    }
    let gap = (b.start_ms - a.end_ms).max(0) as f64;
    let g = (gap / 400.0).min(1.0);
    if t.ends_with(',') { g.max(0.6) } else { g }
}

/// Label the words: each word scores every speaker by the similarity of the windows over it to
/// the speaker's centroid (weighted by overlap and nearness), and a Viterbi pass picks the best
/// labelling where changing speaker costs [`Params::switch_cost`] inside fluent speech and little
/// at a pause or sentence end. So turns change where people actually stop, and a stray word in
/// the middle of a sentence does not become a speaker change. Speakers are numbered by first
/// appearance; speakers no word ends up with are dropped.
pub fn label_words(words: &[Word], wins: &[(i64, i64)], vecs: &[Option<Vec<f32>>], c: &Clusters, p: &Params) -> Labels {
    let k = c.centroids.len();
    if k == 0 || words.is_empty() {
        return Labels { words: vec![None; words.len()], ..Labels::default() };
    }
    // per-word similarity to each centroid (None: no window reached it)
    let mut emit: Vec<Option<Vec<f64>>> = Vec::with_capacity(words.len());
    let mut lo = 0usize;
    for w in words {
        while wins.get(lo).is_some_and(|x| x.1 <= w.start_ms) && lo + 1 < wins.len() && wins.get(lo + 1).is_some_and(|n| n.0 <= w.start_ms) {
            lo += 1;
        }
        let mut score = vec![0f64; k];
        let mut total = 0f64;
        let mut j = lo.saturating_sub(8);
        while let Some(&(s, e)) = wins.get(j) {
            if s > w.end_ms + 2000 {
                break;
            }
            let ov = (e.min(w.end_ms.max(w.start_ms + 1)) - s.max(w.start_ms)).max(0) as f64;
            let dc = (((s + e) / 2) - (w.start_ms + w.end_ms) / 2).abs() as f64;
            if ov > 0.0
                && let Some(Some(v)) = vecs.get(j)
            {
                let wt = ov / (1.0 + dc / 500.0);
                total += wt;
                for (sc, cent) in score.iter_mut().zip(&c.centroids) {
                    *sc += wt * f64::from(1.0 - cluster::cosine_distance(v, cent));
                }
            }
            j += 1;
        }
        emit.push((total > 0.0).then(|| score.iter().map(|x| x / total).collect()));
    }
    // Viterbi: maximise Σ duration·similarity − switch costs
    let n = words.len();
    let mut best: Vec<f64> = vec![0.0; k];
    let mut back: Vec<Vec<usize>> = Vec::with_capacity(n);
    for (i, w) in words.iter().enumerate() {
        let gain = |s: usize| -> f64 {
            let d = ((w.end_ms - w.start_ms).max(0) as f64 / 1000.0).clamp(0.1, 3.0);
            emit.get(i).and_then(|e| e.as_ref()).and_then(|e| e.get(s)).map_or(0.0, |x| d * x)
        };
        let cost = match (i.checked_sub(1).and_then(|j| words.get(j)), Some(w)) {
            (Some(a), Some(b)) => f64::from(p.switch_cost) * (1.0 - boundary(a, b)) + 0.02,
            _ => 0.0,
        };
        let mut next = vec![0f64; k];
        let mut from = vec![0usize; k];
        let (top, top_val) = best.iter().enumerate().fold((0, f64::NEG_INFINITY), |m, (s, &v)| if v > m.1 { (s, v) } else { m });
        for s in 0..k {
            let stay = best.get(s).copied().unwrap_or(f64::NEG_INFINITY);
            let (src, v) = if i > 0 && top != s && top_val - cost > stay { (top, top_val - cost) } else { (s, stay) };
            if let (Some(nx), Some(fr)) = (next.get_mut(s), from.get_mut(s)) {
                *nx = v + gain(s);
                *fr = src;
            }
        }
        best = next;
        back.push(from);
    }
    let mut raw = vec![0usize; n];
    let mut cur = best.iter().enumerate().fold((0, f64::NEG_INFINITY), |m, (s, &v)| if v > m.1 { (s, v) } else { m }).0;
    for i in (0..n).rev() {
        if let Some(r) = raw.get_mut(i) {
            *r = cur;
        }
        cur = back.get(i).and_then(|b| b.get(cur)).copied().unwrap_or(cur);
    }
    // number by first appearance
    let mut order: Vec<usize> = Vec::new();
    for &l in &raw {
        if !order.contains(&l) {
            order.push(l);
        }
    }
    let words_out: Vec<Option<usize>> = raw.iter().map(|l| order.iter().position(|o| o == l)).collect();
    let mut speakers: Vec<SpeakerStat> =
        order.iter().enumerate().map(|(id, &o)| SpeakerStat { id, name: c.names.get(o).cloned().flatten(), ..Default::default() }).collect();
    for (w, l) in words.iter().zip(&words_out) {
        if let Some(s) = l.and_then(|l| speakers.get_mut(l)) {
            s.words += 1;
            s.seconds += (w.end_ms - w.start_ms).max(0) as f32 / 1000.0;
        }
    }
    Labels { words: words_out, speakers, also: Vec::new() }
}

/// The whole method with the network loaded (feature `qwen`).
#[cfg(feature = "qwen")]
pub struct Diarizer {
    net: net::Net,
    fbank: fbank::Fbank,
    /// The convolutions on the GPU, when [`Diarizer::use_gpu`] found one that matches the CPU.
    gpu: Option<crate::gpu::speaker::SpeakerNet>,
    /// Where the network runs ("CPU" or "GPU <adapter>").
    pub device: String,
}

#[cfg(feature = "qwen")]
impl Diarizer {
    /// Load the speaker model from its folder (see [`crate::catalogue::SPEAKER`]).
    pub fn load(dir: &std::path::Path) -> Result<Self, crate::AsrError> {
        Ok(Self { net: net::Net::load(&dir.join("pytorch_model.bin"))?, fbank: fbank::Fbank::new(), gpu: None, device: "CPU".into() })
    }

    /// Run the convolutions on the GPU (Vulkan, DX12, Metal) if there is one and it gives the
    /// CPU's numbers on a test block; otherwise stay on the CPU. Returns where it runs.
    pub fn use_gpu(&mut self) -> String {
        match self.try_gpu() {
            Ok((g, name)) => {
                self.gpu = Some(g);
                self.device = format!("GPU {name}");
            }
            Err(e) => {
                self.gpu = None;
                self.device = format!("CPU ({e})");
            }
        }
        self.device.clone()
    }

    fn try_gpu(&self) -> Result<(crate::gpu::speaker::SpeakerNet, String), crate::AsrError> {
        let ctx = crate::gpu::Ctx::new()?;
        self.gpu_with(&ctx)
    }

    /// Build the GPU network on `ctx` and compare it with the CPU on 2 s of test features.
    pub fn gpu_with(&self, ctx: &std::sync::Arc<crate::gpu::Ctx>) -> Result<(crate::gpu::speaker::SpeakerNet, String), crate::AsrError> {
        let (stem, blocks) = self.net.specs()?;
        let g = crate::gpu::speaker::SpeakerNet::new(ctx, &stem, &blocks)?;
        // a chirp with harmonics: varied, speech-like features
        let audio: Vec<f32> = (0..32_000)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                let f = 120.0 + 80.0 * t;
                (1..6).map(|h| (std::f32::consts::TAU * f * h as f32 * t).sin() / h as f32).sum::<f32>() * 0.2
            })
            .collect();
        let frames = fbank::frames(audio.len());
        let mut feats = self.fbank.compute(&audio);
        fbank::subtract_mean(&mut feats);
        let cpu = self.net.frame_map(&feats, frames)?;
        let gpu = Self::gpu_map(&g, &feats, frames)?;
        let scale = cpu.iter().map(|v| v.abs()).fold(0f32, f32::max).max(1e-6);
        let err = cpu.iter().zip(&gpu).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max) / scale;
        if cpu.len() != gpu.len() || !err.is_finite() || err > 1e-3 {
            return Err(crate::AsrError::Model(format!("GPU speaker network differs from the CPU (relative error {err:.2e})")));
        }
        Ok((g, ctx.name.clone()))
    }

    /// GPU frame map rearranged like [`net::Net::frame_map`]: `out_frames × FEAT`.
    fn gpu_map(g: &crate::gpu::speaker::SpeakerNet, feats: &[f32], frames: usize) -> Result<Vec<f32>, crate::AsrError> {
        let (raw, t) = g.forward(feats, frames, fbank::N_MELS)?;
        if raw.len() != net::FEAT * t {
            return Err(crate::AsrError::Model("GPU speaker network: unexpected output size".into()));
        }
        let mut out = vec![0f32; raw.len()];
        for (r, row) in raw.chunks(t.max(1)).enumerate() {
            for (f, &v) in row.iter().enumerate() {
                if let Some(slot) = out.get_mut(f * net::FEAT + r) {
                    *slot = v;
                }
            }
        }
        Ok(out)
    }

    /// Frame map of a block of audio: (output frames, map). On the GPU when enabled; a GPU error
    /// falls back to the CPU for that block.
    pub fn frame_map(&self, block: &[f32]) -> Result<(usize, Vec<f32>), crate::AsrError> {
        let frames = fbank::frames(block.len());
        let mut feats = self.fbank.compute(block);
        fbank::subtract_mean(&mut feats);
        let map = match self.gpu.as_ref().filter(|_| frames > 0).map(|g| Self::gpu_map(g, &feats, frames)) {
            Some(Ok(m)) => m,
            _ => self.net.frame_map(&feats, frames)?,
        };
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

    /// Voice profile of a sample of one person speaking (a few seconds to a minute): the mean of
    /// its 1.5 s windows' voice vectors, skipping quiet windows. `None` if it has no speech.
    pub fn profile(&self, audio: &[f32]) -> Result<Option<Vec<f32>>, crate::AsrError> {
        let len = ms_of(audio.len());
        let rms = |s: i64, e: i64| -> f32 {
            let x = audio.get(ms_to_sample(s).min(audio.len())..ms_to_sample(e).min(audio.len())).unwrap_or(&[]);
            if x.is_empty() { 0.0 } else { (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt() }
        };
        let all = windows(&[(0, len)], 1500, 500);
        let loud = all.iter().map(|&(s, e)| rms(s, e)).fold(0f32, f32::max);
        let wins: Vec<(i64, i64)> = all.into_iter().filter(|&(s, e)| loud > 0.0 && rms(s, e) >= 0.1 * loud).collect();
        let vecs = window_vectors(audio, &wins, &mut |b| self.frame_map(b), &|m, a, b| self.net.embed(m, a, b), &mut |_, _| true)?;
        let mut sum = vec![0f32; net::EMBED];
        let mut n = 0;
        for v in vecs.iter().flatten() {
            sum.iter_mut().zip(v).for_each(|(a, b)| *a += b);
            n += 1;
        }
        if n == 0 {
            return Ok(None);
        }
        cluster::normalize(&mut sum);
        Ok(Some(sum))
    }

    /// Label the words of `audio` (16 kHz mono). `progress(fraction)` returns false to cancel.
    pub fn run(&self, audio: &[f32], words: &[Word], p: &Params, progress: &mut dyn FnMut(f32) -> bool) -> Result<Labels, crate::AsrError> {
        let wins = windows(&speech_regions(words, p.pause_ms), p.window_ms, p.step_ms);
        let vecs =
            window_vectors(audio, &wins, &mut |b| self.frame_map(b), &|m, a, b| self.net.embed(m, a, b), &mut |d, t| progress(d as f32 / t.max(1) as f32))?;
        let c = cluster_windows(&vecs, &wins, p);
        Ok(label_words(words, &wins, &vecs, &c, p))
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
        let c = cluster_windows(&vecs, &wins, &p);
        assert_eq!(c.centroids.len(), 2);
        let l = label_words(&words, &wins, &vecs, &c, &p);
        assert_eq!(l.speakers.len(), 2);
        assert_eq!(l.words[0], Some(0));
        assert_eq!(l.words[12], Some(1));
        assert_eq!(l.words[25], Some(0));
        // the user's count wins
        let one = cluster_windows(&vecs, &wins, &Params { speakers: Some(1), ..p.clone() });
        assert_eq!(label_words(&words, &wins, &vecs, &one, &p).speakers.len(), 1);
    }

    #[test]
    fn skipped_frames_stay_out_of_the_vectors() {
        // 125 frames of 80 ms per 10 s block, each frame's one feature is 1 (clean) or 9 (overlap)
        let audio = vec![0f32; 16 * 10_000];
        let overlapped = |ms: i64| (2000..2400).contains(&ms) || (5000..6500).contains(&ms);
        let mut map = |b: &[f32]| {
            let n = b.len() / 16 / 80;
            Ok((n, (0..n).map(|j| if overlapped(j as i64 * 80 + 40) { 9.0 } else { 1.0 }).collect()))
        };
        let mean = |m: &[f32], a: usize, b: usize| Some(vec![m.get(a..b)?.iter().sum::<f32>() / (b - a) as f32, 1.0]);
        let wins = [(1500, 3000), (5000, 6500), (7000, 8500)];
        let mixed = window_vectors(&audio, &wins, &mut map, &mean, &mut |_, _| true).unwrap();
        let first = mixed[0].as_ref().unwrap();
        assert!(first[0] > first[1], "the overlap pulls the plain vector");
        let skip = |ms: i64| overlapped(ms);
        let clean = window_vectors_skipping(&audio, &wins, &mut map, &mean, Some(&skip), &mut |_, _| true).unwrap();
        let first = clean[0].as_ref().unwrap();
        assert!((first[0] - first[1]).abs() < 1e-6, "only clean frames: {first:?}");
        assert_eq!(clean[1], None, "a wholly overlapped window has no vector");
        assert_eq!(clean[2], mixed[2], "an untouched window is the same");
    }

    #[test]
    fn enrolled_voices_name_the_nearest_speaker_only() {
        let unit = |v: &[f32]| {
            let mut v = v.to_vec();
            cluster::normalize(&mut v);
            v
        };
        let cents = vec![unit(&[1.0, 0.0, 0.0]), unit(&[0.0, 1.0, 0.0])];
        let known = vec![
            KnownVoice { name: "스트리머".into(), vector: unit(&[0.1, 1.0, 0.0]) },
            KnownVoice { name: "없는 사람".into(), vector: unit(&[0.0, 0.0, 1.0]) },
            KnownVoice { name: "짧은 벡터".into(), vector: vec![1.0] },
        ];
        assert_eq!(match_known(&cents, &known), vec![None, Some("스트리머".to_string())]);
        assert_eq!(match_known(&cents, &[]), vec![None, None]);
        // two voices close to one speaker: only the closer one gets it
        let twins = vec![KnownVoice { name: "a".into(), vector: unit(&[1.0, 0.2, 0.0]) }, KnownVoice { name: "b".into(), vector: unit(&[1.0, 0.0, 0.0]) }];
        assert_eq!(match_known(&cents, &twins), vec![Some("b".to_string()), None]);
    }

    #[test]
    fn turns_change_at_pauses_not_mid_sentence() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        let c = Clusters { labels: Vec::new(), centroids: vec![a.clone(), b.clone()], names: vec![None, Some("아이치".into())] };
        let word = |t: &str, s: i64, e: i64| Word { text: t.into(), start_ms: s, end_ms: e, ..Default::default() };
        // A speaks two fluent words, one window in the middle of a sentence sounds like B
        let words = vec![word("이거", 0, 400), word("진짜", 400, 800), word("좋네요.", 800, 1200), word("네", 1800, 2200), word("맞아요", 2200, 2600)];
        let wins = vec![(0, 400), (400, 800), (800, 1200), (1800, 2200), (2200, 2600)];
        let vecs = vec![Some(a.clone()), Some(b.clone()), Some(a.clone()), Some(b.clone()), Some(b.clone())];
        let l = label_words(&words, &wins, &vecs, &c, &Params::default());
        assert_eq!(l.words, vec![Some(0), Some(0), Some(0), Some(1), Some(1)], "the stray window does not split the sentence");
        assert_eq!(l.speakers.len(), 2);
        assert_eq!(l.speakers[1].name.as_deref(), Some("아이치"));
        // without a pause the same evidence over two words is still a change only if it is strong
        assert!(boundary(&words[2], &words[3]) > 0.99);
        assert!(boundary(&words[0], &words[1]) < 0.01);
        let e = label_words(&[], &[], &[], &c, &Params::default());
        assert!(e.words.is_empty() && e.speakers.is_empty());
        let none = cluster_windows(&[None, None], &[(0, 1), (1, 2)], &Params::default());
        assert!(none.labels.iter().all(Option::is_none) && none.centroids.is_empty());
        assert_eq!(label_words(&words, &wins, &vecs, &none, &Params::default()).words, vec![None; 5]);
        // hostile times
        let odd = vec![Word { start_ms: i64::MIN / 4, end_ms: i64::MAX / 4, ..w(0, 0) }];
        let _ = windows(&speech_regions(&odd, 250), 1500, 500);
    }
}

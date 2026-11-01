//! OverFilm editing assistants: the analysis behind the editor's suggestions (milestone M6).
//! Pure functions on loudness and transcript words; the engine (`overfilm_assist`) gathers them
//! from the sequence, runs these and puts the result on the timeline as markers to review.
//!
//! - [`silences`]: quiet stretches (no speech, no game sound) to cut, kept clear of words.
//! - [`highlights`]: the moments worth a look: the casters or the room get loud, hype words
//!   ("미쳤다", "전멸", "나이스"…), hero ultimates heard in the game voice.
//!
//! Loudness is a list of levels in dBFS, one per [`BLOCK_MS`] block (see [`levels`]).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use serde::{Deserialize, Serialize};

/// One loudness block (ms).
pub const BLOCK_MS: i64 = 100;
/// Level of digital silence (dBFS).
pub const FLOOR_DB: f32 = -100.0;
/// Most blocks analysed (12 hours).
pub const MAX_BLOCKS: usize = 12 * 3600 * 10;

/// RMS level in dBFS of each [`BLOCK_MS`] block of mono `samples` at `rate` Hz (the last, partial
/// block included).
pub fn levels(samples: &[f32], rate: u32) -> Vec<f32> {
    let n = (rate as usize / 10).max(1);
    samples.chunks(n).take(MAX_BLOCKS).map(rms_db).collect()
}

/// RMS level of a block in dBFS ([`FLOOR_DB`] for silence).
pub fn rms_db(block: &[f32]) -> f32 {
    if block.is_empty() {
        return FLOOR_DB;
    }
    let sum: f64 = block.iter().filter(|x| x.is_finite()).map(|&x| f64::from(x) * f64::from(x)).sum();
    let ms = sum / block.len() as f64;
    if ms <= 1e-10 { FLOOR_DB } else { ((10.0 * ms.log10()) as f32).max(FLOOR_DB) }
}

/// A word on the timeline (ms), with its speaker and whether that speaker is in-game voice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub game: bool,
}

/// A time range in ms, `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Span {
    pub fn len(&self) -> i64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
    pub fn is_empty(&self) -> bool {
        self.len() <= 0
    }
}

/// Silence tunables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SilenceParams {
    /// Blocks below this are quiet (dBFS).
    pub threshold_db: f32,
    /// Shortest quiet stretch suggested, after padding (ms).
    pub min_ms: i64,
    /// Sound kept on each side, so breaths and word ends are not clipped (ms).
    pub pad_ms: i64,
}

impl Default for SilenceParams {
    fn default() -> Self {
        Self { threshold_db: -45.0, min_ms: 1000, pad_ms: 200 }
    }
}

/// Quiet stretches of `levels` to cut: below the threshold for at least `min_ms` once `pad_ms` is
/// left on each side, and never over a word.
pub fn silences(levels: &[f32], words: &[Word], p: &SilenceParams) -> Vec<Span> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < levels.len() {
        if levels.get(i).is_some_and(|&l| l < p.threshold_db) {
            let mut j = i;
            while levels.get(j).is_some_and(|&l| l < p.threshold_db) {
                j += 1;
            }
            let span = Span { start_ms: i as i64 * BLOCK_MS + p.pad_ms.max(0), end_ms: j as i64 * BLOCK_MS - p.pad_ms.max(0) };
            // words inside split the stretch; keep the parts between them
            let mut parts = vec![span];
            for w in words.iter().filter(|w| w.end_ms > span.start_ms && w.start_ms < span.end_ms) {
                let mut next = Vec::new();
                for s in parts {
                    if w.end_ms <= s.start_ms || w.start_ms >= s.end_ms {
                        next.push(s);
                        continue;
                    }
                    next.push(Span { start_ms: s.start_ms, end_ms: w.start_ms - p.pad_ms.max(0) });
                    next.push(Span { start_ms: w.end_ms + p.pad_ms.max(0), end_ms: s.end_ms });
                }
                parts = next;
            }
            out.extend(parts.into_iter().filter(|s| s.len() >= p.min_ms.max(1)));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Words the casters and players shout at a big moment (matched inside a word, spacing ignored).
pub const HYPE_WORDS: &[&str] = &[
    "미쳤",
    "미친",
    "대박",
    "나이스",
    "말도안",
    "와아",
    "우와",
    "전멸",
    "다잡",
    "다잡아",
    "한타",
    "역전",
    "이겼",
    "끝났",
    "끝냅",
    "트리플",
    "쿼드라",
    "멀티킬",
    "팀킬",
    "살렸",
    "살아남",
    "막아냅",
    "막아",
    "슈퍼플레이",
    "엄청",
    "어마어마",
    "기가막",
    "레전드",
    "결정적",
    "뒤집",
    "따냅",
    "점령",
    "환상",
];

/// Highlight tunables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HighlightParams {
    /// How many to suggest at most.
    pub count: usize,
    /// Length of each highlight (ms); the peak sits at two thirds.
    pub length_ms: i64,
    /// Points per dB the room is louder than its surroundings.
    pub loud_weight: f32,
    /// Points per hype word.
    pub word_points: f32,
    /// Points per in-game voice word (hero ultimates, the announcer).
    pub game_points: f32,
    /// Points per death read off the HUD (`overfilm.kills.scan`).
    pub kill_points: f32,
    /// Smallest score suggested.
    pub min_score: f32,
}

impl Default for HighlightParams {
    fn default() -> Self {
        Self { count: 10, length_ms: 20_000, loud_weight: 1.0, word_points: 4.0, game_points: 1.5, kill_points: 5.0, min_score: 8.0 }
    }
}

/// One suggested highlight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub span: Span,
    /// Where the score peaks (ms).
    pub peak_ms: i64,
    pub score: f32,
    /// How much louder than the surrounding minute (dB).
    pub louder_db: f32,
    /// Hype words heard, in order (at most 5).
    pub words: Vec<String>,
    /// In-game voice words in it.
    pub game_words: usize,
    /// Deaths in it.
    #[serde(default)]
    pub kills: usize,
}

/// Median of a slice (0 for empty).
fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    v.select_nth_unstable_by(mid, f32::total_cmp);
    v.get(mid).copied().unwrap_or(0.0)
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Whether `word` is a hype word.
pub fn is_hype(word: &str) -> bool {
    let w = squash(word);
    !w.is_empty() && HYPE_WORDS.iter().any(|h| w.contains(h))
}

/// The best moments: each second scores how much louder it is than the minute around it, plus hype
/// words and in-game voice; the score is smoothed over 5 s and the highest peaks become
/// non-overlapping highlights of `length_ms`, in time order.
pub fn highlights(levels: &[f32], words: &[Word], p: &HighlightParams) -> Vec<Highlight> {
    highlights_with_kills(levels, words, &[], p)
}

/// [`highlights`] with the times (ms) of deaths read off the HUD: each scores `kill_points`, so a
/// team fight stands out even when the casters stay calm.
pub fn highlights_with_kills(levels: &[f32], words: &[Word], kills_ms: &[i64], p: &HighlightParams) -> Vec<Highlight> {
    let secs = levels.len().div_ceil(10);
    if secs == 0 {
        return Vec::new();
    }
    // per-second loudness (energy average of the ten blocks)
    let sec_db: Vec<f32> = levels
        .chunks(10)
        .map(|c| {
            let e: f32 = c.iter().map(|&d| 10f32.powf(d / 10.0)).sum::<f32>() / c.len().max(1) as f32;
            if e <= 1e-10 { FLOOR_DB } else { 10.0 * e.log10() }
        })
        .collect();
    let mut score = vec![0f32; secs];
    let mut louder = vec![0f32; secs];
    // loudness above the median of the surrounding 60 s, counting only sound (quiet seconds would
    // pull the median down and make any speech look exciting)
    for i in 0..secs {
        let a = i.saturating_sub(30);
        let b = (i + 30).min(secs);
        let mut around: Vec<f32> = sec_db.get(a..b).unwrap_or(&[]).iter().copied().filter(|&d| d > -60.0).collect();
        let base = median(&mut around);
        let d = sec_db.get(i).copied().unwrap_or(FLOOR_DB);
        if d > -60.0 && !around.is_empty() {
            let up = (d - base - 3.0).max(0.0);
            louder[i] = d - base;
            score[i] += up * p.loud_weight;
        }
    }
    let at = |ms: i64| usize::try_from(ms.max(0) / 1000).unwrap_or(usize::MAX).min(secs - 1);
    for w in words {
        let i = at(w.start_ms);
        if let Some(s) = score.get_mut(i) {
            if w.game {
                *s += p.game_points;
            } else if is_hype(&w.text) {
                *s += p.word_points;
            }
        }
    }
    for &k in kills_ms {
        if let Some(s) = score.get_mut(at(k)) {
            *s += p.kill_points;
        }
    }
    // smooth over 5 s
    let smooth: Vec<f32> = (0..secs).map(|i| score.get(i.saturating_sub(2)..(i + 3).min(secs)).unwrap_or(&[]).iter().sum::<f32>()).collect();
    let len_s = usize::try_from((p.length_ms / 1000).max(4)).unwrap_or(20);
    let before = len_s * 2 / 3;
    let mut taken = vec![false; secs];
    let mut order: Vec<usize> = (0..secs).collect();
    order.sort_by(|&a, &b| smooth.get(b).copied().unwrap_or(0.0).total_cmp(&smooth.get(a).copied().unwrap_or(0.0)).then(a.cmp(&b)));
    let mut out = Vec::new();
    for peak in order {
        if out.len() >= p.count {
            break;
        }
        let sc = smooth.get(peak).copied().unwrap_or(0.0);
        if sc < p.min_score {
            break;
        }
        let a = peak.saturating_sub(before);
        let b = (a + len_s).min(secs);
        if taken.get(a..b).is_none_or(|t| t.iter().any(|&x| x)) {
            continue;
        }
        taken.iter_mut().take(b).skip(a).for_each(|x| *x = true);
        let span = Span { start_ms: a as i64 * 1000, end_ms: b as i64 * 1000 };
        let inside: Vec<&Word> = words.iter().filter(|w| w.start_ms >= span.start_ms && w.start_ms < span.end_ms).collect();
        out.push(Highlight {
            span,
            peak_ms: peak as i64 * 1000,
            score: sc,
            louder_db: louder.get(a..b).unwrap_or(&[]).iter().copied().fold(0.0, f32::max),
            words: inside.iter().filter(|w| !w.game && is_hype(&w.text)).take(5).map(|w| w.text.clone()).collect(),
            game_words: inside.iter().filter(|w| w.game).count(),
            kills: kills_ms.iter().filter(|&&k| k >= span.start_ms && k < span.end_ms).count(),
        });
    }
    out.sort_by_key(|h| h.span.start_ms);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, s: i64, game: bool) -> Word {
        Word { text: t.into(), start_ms: s, end_ms: s + 300, game }
    }

    #[test]
    fn levels_and_rms() {
        assert_eq!(rms_db(&[]), FLOOR_DB);
        assert_eq!(rms_db(&[0.0; 10]), FLOOR_DB);
        assert!((rms_db(&[1.0, -1.0]) - 0.0).abs() < 1e-4);
        assert!((rms_db(&[0.1; 100]) + 20.0).abs() < 1e-3);
        assert!(rms_db(&[f32::NAN, 0.5]) < 0.0);
        let l = levels(&vec![0.1; 16_000 + 800], 16_000);
        assert_eq!(l.len(), 11, "ten full blocks and a partial one");
        assert!(levels(&[0.1; 10], 0).len() == 10, "rate 0 does not divide by zero");
    }

    #[test]
    fn silences_are_padded_and_never_cut_words() {
        // 1 s sound, 3 s quiet, 1 s sound, 0.8 s quiet, 1 s sound
        let mut l = vec![-20.0; 10];
        l.extend(vec![-70.0; 30]);
        l.extend(vec![-20.0; 10]);
        l.extend(vec![-70.0; 8]);
        l.extend(vec![-20.0; 10]);
        let s = silences(&l, &[], &SilenceParams::default());
        assert_eq!(s, vec![Span { start_ms: 1200, end_ms: 3800 }], "the 0.8 s gap is too short");
        // a whispered word inside splits it; the halves stay only if long enough
        let s = silences(&l, &[w("음", 2400, false)], &SilenceParams { min_ms: 500, ..Default::default() });
        assert_eq!(s, vec![Span { start_ms: 1200, end_ms: 2200 }, Span { start_ms: 2900, end_ms: 3800 }]);
        assert!(silences(&[], &[], &SilenceParams::default()).is_empty());
        assert!(silences(&[-90.0; 5], &[], &SilenceParams::default()).is_empty());
    }

    #[test]
    fn highlights_follow_loud_moments_and_hype_words() {
        // five minutes of steady commentary, a roar at 100 s and hype words at 220 s
        let mut l = vec![-30.0; 3000];
        for x in l.iter_mut().skip(1000).take(40) {
            *x = -14.0;
        }
        let words = vec![w("미쳤다", 220_000, false), w("전멸입니다", 221_000, false), w("나이스", 222_500, false), w("석양이", 223_000, true)];
        let h = highlights(&l, &words, &HighlightParams::default());
        assert_eq!(h.len(), 2, "{h:?}");
        assert!(h[0].span.start_ms <= 100_000 && 104_000 <= h[0].span.end_ms, "{:?}", h[0]);
        assert!(h[0].louder_db > 15.0);
        assert!(h[1].span.start_ms <= 220_000 && 223_000 < h[1].span.end_ms);
        assert_eq!(h[1].words, ["미쳤다", "전멸입니다", "나이스"]);
        assert_eq!(h[1].game_words, 1);
        assert_eq!(h[0].span.len(), 20_000);
        // nothing stands out: nothing suggested
        assert!(highlights(&vec![-30.0; 3000], &[], &HighlightParams::default()).is_empty());
        assert!(highlights(&[], &words, &HighlightParams::default()).is_empty());
        // words far past the end are clamped, not indexed out of range
        assert!(
            !highlights(&[-30.0; 50], &[w("미쳤다", i64::MAX / 2, false), w("대박", -5, false), w("와아", 100, false)], &HighlightParams::default()).is_empty()
        );
    }

    #[test]
    fn hype_words() {
        assert!(is_hype("미쳤어요!"));
        assert!(is_hype("말도 안 돼") || is_hype("말도안돼"));
        assert!(!is_hype("오늘은"));
        assert!(!is_hype("..."));
    }

    #[test]
    fn deaths_make_a_quiet_team_fight_a_highlight() {
        // two minutes at one level: nothing stands out by sound
        let l = vec![-30.0; 1200];
        assert!(highlights(&l, &[], &HighlightParams::default()).is_empty());
        let kills = [70_000, 72_000, 75_000, 76_500, -5, i64::MAX];
        let h = highlights_with_kills(&l, &[], &kills, &HighlightParams::default());
        assert_eq!(h.len(), 1);
        assert!(h[0].span.start_ms <= 70_000 && 76_500 < h[0].span.end_ms, "{h:?}");
        assert_eq!(h[0].kills, 4);
    }
}

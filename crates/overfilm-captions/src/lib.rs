//! OverFilm Korean caption layout (milestone M4).
//!
//! Timed words become captions of at most two lines:
//!
//! - **Width** is counted in half-width units: Hangul, kana and ideographs are 2, everything else
//!   (Latin letters, digits, spaces, punctuation) 1. The default line is 32 units (16 Hangul
//!   syllables, the usual Korean subtitle limit).
//! - **Captions** end at a sentence end (`.`, `?`, `!`, `…`) or a pause; inside a sentence the
//!   words are split into captions by dynamic programming over break costs: a break after a
//!   comma is free, after a connective ending or particle (`…고`, `…는데`, `…를`) cheap, anywhere
//!   else expensive, and every extra caption and every very short caption costs something, so
//!   captions come out full but end at natural places. A caption never runs longer than
//!   `max_cue_ms` unless it is a single word.
//! - **Lines**: a caption wider than one line is split into two at the break that balances the
//!   lines best, with the same preference for natural break points.
//! - **Timing**: from the first word's start to the last word's end plus a short hold, at least
//!   `min_cue_ms`, never overlapping the next caption.
//!
//! Shot-change snapping and reading-speed limits are planned.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

use filmcraft_captions::{Cue, Document};
use filmcraft_time::Tick;
use serde::{Deserialize, Serialize};

/// A word with its time (milliseconds), as written (punctuation attached).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimedWord {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Layout limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// Line width in half-width units (Hangul 2, Latin 1).
    pub max_line: u32,
    /// 1 or 2.
    pub max_lines: u32,
    /// Longest caption (ms), single words excepted.
    pub max_cue_ms: i64,
    /// A silence at least this long ends a caption (ms).
    pub pause_ms: i64,
    pub min_cue_ms: i64,
    /// Time a caption stays up after its last word (ms).
    pub hold_ms: i64,
}

impl Default for Layout {
    fn default() -> Self {
        Self { max_line: 32, max_lines: 2, max_cue_ms: 7000, pause_ms: 700, min_cue_ms: 700, hold_ms: 250 }
    }
}

/// One caption.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Caption {
    pub start_ms: i64,
    pub end_ms: i64,
    pub lines: Vec<String>,
}

impl Caption {
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

fn wide(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0x20000..=0x3FFFD)
}

/// Display width in half-width units.
pub fn width(s: &str) -> u32 {
    s.chars().map(|c| if wide(c) { 2 } else { 1 }).fold(0u32, u32::saturating_add)
}

fn sentence_end(w: &str) -> bool {
    w.trim_end_matches(['"', '\'', ')', '」', '』']).ends_with(['.', '?', '!', '…', '。', '？', '！'])
}

/// Connective endings and particles after which a Korean line or caption breaks naturally.
const SOFT_ENDINGS: &[&str] = &[
    "는데", "은데", "지만", "니까", "어서", "아서", "면서", "다가", "고", "며", "면", "서", "게", "도", "은", "는", "이", "가", "을", "를", "에", "에서", "로",
    "으로", "와", "과", "의", "랑", "한테", "까지", "부터", "보다", "처럼", "요",
];

/// Cost of breaking after word `w` (0 free … 80 bad).
fn break_cost(w: &str) -> u32 {
    let core = w.trim_end_matches(['"', '\'', ')']);
    if core.ends_with([',', '、', '，', ':', ';']) || sentence_end(core) {
        0
    } else if SOFT_ENDINGS.iter().any(|e| core.ends_with(e)) {
        15
    } else {
        80
    }
}

fn joined_width(words: &[TimedWord]) -> u32 {
    let spaces = u32::try_from(words.len().saturating_sub(1)).unwrap_or(u32::MAX);
    words.iter().map(|w| width(&w.text)).fold(spaces, u32::saturating_add)
}

/// Split one sentence-or-pause segment into captions (DP over break points).
fn split_segment(words: &[TimedWord], o: &Layout) -> Vec<std::ops::Range<usize>> {
    let n = words.len();
    let cap = o.max_line.saturating_mul(o.max_lines.clamp(1, 2));
    // best[i] = (cost, previous break) for words[..i]
    let mut best: Vec<(u64, usize)> = vec![(u64::MAX, 0); n + 1];
    if let Some(b) = best.get_mut(0) {
        *b = (0, 0);
    }
    for end in 1..=n {
        for start in (0..end).rev() {
            let (Some(slice), Some(&(prev, _))) = (words.get(start..end), best.get(start)) else { continue };
            let single = end - start == 1;
            let dur = slice.last().map_or(0, |l| l.end_ms) - slice.first().map_or(0, |f| f.start_ms);
            let w = joined_width(slice);
            if !single && (w > cap || dur > o.max_cue_ms) {
                break;
            }
            if prev == u64::MAX {
                continue;
            }
            let brk = if end == n { 0 } else { slice.last().map_or(80, |l| break_cost(&l.text)) };
            let short = if w < 12 && end != n { 40 } else { 0 };
            let cost = prev + 60 + u64::from(brk) + short;
            if let Some(b) = best.get_mut(end)
                && cost < b.0
            {
                *b = (cost, start);
            }
        }
    }
    let mut out = Vec::new();
    let mut end = n;
    while end > 0 {
        let start = best.get(end).map_or(0, |b| b.1).min(end - 1);
        out.push(start..end);
        end = start;
    }
    out.reverse();
    out
}

/// Hard-wrap one over-long word to `max` units per line.
fn wrap_chars(s: &str, max: u32) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut w = 0u32;
    for c in s.chars() {
        let cw = if wide(c) { 2 } else { 1 };
        if w + cw > max.max(2) && lines.last().is_some_and(|l| !l.is_empty()) {
            lines.push(String::new());
            w = 0;
        }
        if let Some(l) = lines.last_mut() {
            l.push(c);
        }
        w += cw;
    }
    lines
}

fn join(words: &[TimedWord]) -> String {
    words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// Lines of one caption.
fn lines_of(words: &[TimedWord], o: &Layout) -> Vec<String> {
    let all = join(words);
    if joined_width(words) <= o.max_line || o.max_lines < 2 {
        return if words.len() == 1 && width(&all) > o.max_line { wrap_chars(&all, o.max_line) } else { vec![all] };
    }
    let best = (1..words.len())
        .filter_map(|k| {
            let (a, b) = (words.get(..k)?, words.get(k..)?);
            let (wa, wb) = (joined_width(a), joined_width(b));
            if wa > o.max_line || wb > o.max_line {
                return None;
            }
            Some((wa.abs_diff(wb) + break_cost(&a.last()?.text) / 2, k))
        })
        .min();
    match best {
        Some((_, k)) => vec![join(words.get(..k).unwrap_or(&[])), join(words.get(k..).unwrap_or(&[]))],
        None => wrap_chars(&all, o.max_line),
    }
}

/// Lay out timed words as captions.
pub fn layout(words: &[TimedWord], o: &Layout) -> Vec<Caption> {
    let mut caps = Vec::new();
    let mut seg_start = 0;
    for i in 0..words.len() {
        let (Some(w), next) = (words.get(i), words.get(i + 1)) else { continue };
        let pause = next.is_some_and(|n| n.start_ms - w.end_ms >= o.pause_ms);
        if next.is_none() || pause || sentence_end(&w.text) {
            let seg = words.get(seg_start..=i).unwrap_or(&[]);
            for r in split_segment(seg, o) {
                let part = seg.get(r).unwrap_or(&[]);
                let (Some(f), Some(l)) = (part.first(), part.last()) else { continue };
                caps.push(Caption { start_ms: f.start_ms, end_ms: l.end_ms, lines: lines_of(part, o) });
            }
            seg_start = i + 1;
        }
    }
    // timing: hold, minimum duration, no overlap
    for i in 0..caps.len() {
        let next_start = caps.get(i + 1).map(|c| c.start_ms);
        if let Some(c) = caps.get_mut(i) {
            let mut end = (c.end_ms + o.hold_ms).max(c.start_ms + o.min_cue_ms);
            if let Some(ns) = next_start {
                end = end.min(ns.max(c.start_ms + 1));
            }
            c.end_ms = end.max(c.start_ms + 1);
        }
    }
    caps
}

fn tick(ms: i64) -> Tick {
    Tick(ms.saturating_mul(filmcraft_time::TICKS_PER_SECOND / 1000))
}

/// SubRip text of `caps`.
pub fn to_srt(caps: &[Caption]) -> String {
    let doc = Document {
        cues: caps.iter().map(|c| Cue { start: tick(c.start_ms), end: tick(c.end_ms), text: c.text(), ..Default::default() }).collect(),
        ..Default::default()
    };
    filmcraft_captions::srt::write(&doc)
}

/// Captions of SubRip text (times rounded to milliseconds).
pub fn from_srt(text: &str) -> Result<Vec<Caption>, String> {
    let doc = filmcraft_captions::srt::parse(text).map_err(|e| e.to_string())?;
    let ms = |t: Tick| t.0 / (filmcraft_time::TICKS_PER_SECOND / 1000);
    Ok(doc.cues.iter().map(|c| Caption { start_ms: ms(c.start), end_ms: ms(c.end), lines: c.text.lines().map(str::to_string).collect() }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(spec: &[(&str, i64, i64)]) -> Vec<TimedWord> {
        spec.iter().map(|(t, s, e)| TimedWord { text: t.to_string(), start_ms: *s, end_ms: *e }).collect()
    }

    fn seq(text: &str, step: i64) -> Vec<TimedWord> {
        text.split_whitespace().enumerate().map(|(i, t)| TimedWord { text: t.into(), start_ms: i as i64 * step, end_ms: i as i64 * step + step - 20 }).collect()
    }

    #[test]
    fn widths() {
        assert_eq!(width("겐지"), 4);
        assert_eq!(width("D.Va 76"), 7);
    }

    #[test]
    fn sentences_and_pauses_end_captions() {
        let w = words(&[("반갑습니다.", 0, 800), ("오늘은", 900, 1200), ("티어리스트", 1200, 1800), ("갑니다", 4000, 4500)]);
        let c = layout(&w, &Layout::default());
        let t: Vec<String> = c.iter().map(Caption::text).collect();
        assert_eq!(t, vec!["반갑습니다.", "오늘은 티어리스트", "갑니다"]);
        assert!(c.windows(2).all(|p| p[0].end_ms <= p[1].start_ms));
        assert_eq!(c[0].end_ms, 900, "hold is cut at the next caption");
    }

    #[test]
    fn at_most_two_lines_within_width() {
        let w = seq(
            "이번 시즌 상황부터 리마인드하고 들어가겠습니다 독트린이 출시했고 솜브라 로드호그가 리워크 됐습니다 이전 시즌 디몬 출시 이후 핫픽스로 메타가 됐었고",
            300,
        );
        let o = Layout::default();
        for c in layout(&w, &o) {
            assert!(c.lines.len() <= 2, "{c:?}");
            assert!(c.lines.iter().all(|l| width(l) <= o.max_line), "{c:?}");
            assert!(c.end_ms - c.start_ms <= o.max_cue_ms + o.hold_ms);
        }
    }

    #[test]
    fn breaks_prefer_commas_and_endings() {
        // 26 + 22 units: must split; the break after the comma wins
        let w = seq("디몬이 출시되고 나서, 메타가 완전히 바뀌었는데 지금은 다시 돌아왔습니다", 200);
        let c = layout(&w, &Layout { max_line: 20, ..Layout::default() });
        assert!(c.iter().any(|c| c.lines.last().is_some_and(|l| l.ends_with("나서,"))), "{c:?}");
    }

    #[test]
    fn hostile_input() {
        assert!(layout(&[], &Layout::default()).is_empty());
        let long = words(&[(&"가".repeat(100), 0, 10)]);
        let c = layout(&long, &Layout::default());
        assert_eq!(c.len(), 1);
        assert!(c[0].lines.iter().all(|l| width(l) <= 32));
        let o = Layout { max_line: 0, max_lines: 0, max_cue_ms: -5, ..Layout::default() };
        let _ = layout(&seq("가 나 다 라", 10), &o);
    }

    #[test]
    fn srt_round_trip() {
        let caps = vec![Caption { start_ms: 1000, end_ms: 2500, lines: vec!["가나".into(), "다라".into()] }];
        let s = to_srt(&caps);
        assert_eq!(s, "1\n00:00:01,000 --> 00:00:02,500\n가나\n다라\n\n");
        assert_eq!(from_srt(&s).unwrap(), caps);
    }
}

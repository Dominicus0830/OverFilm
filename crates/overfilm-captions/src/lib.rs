//! OverFilm Korean caption layout (milestone M4).
//!
//! Timed words become captions of at most two lines:
//!
//! - **Width** is counted in half-width units: Hangul, kana and ideographs are 2, everything else
//!   (Latin letters, digits, spaces, punctuation) 1. The default line is 32 units (16 Hangul
//!   syllables, the usual Korean subtitle limit).
//! - **Captions** end at a sentence end (`.`, `?`, `!`, `…`) or a pause; inside a sentence the
//!   words are split into captions by dynamic programming over break costs. Breaks follow Korean
//!   grammar: after a comma, a sentence-final ending (`…습니다`, `…요`) or a clause-closing
//!   connective (`…는데`, `…지만`, `…해서`, `…고`) are natural, after a particle (`…를`, `…에서`)
//!   fine, inside a noun phrase poor, and a phrase is never torn: not after a determiner or adverb
//!   (`이`, `그런`, `너무`), not before a bound noun (`할 수`, `하는 걸`), not inside a verb
//!   with its auxiliary (`하고 있다`, `해 보다`, `할 수 있다`, `하는 것 같다`). A silence between
//!   two words makes a break there more natural; a break before `그리고`, `근데`… is natural too.
//!   Every caption, the line break inside it and every very short caption cost something, so
//!   captions come out full but end at natural places. A caption never runs longer than
//!   `max_cue_ms` unless it is a single word.
//! - **Lines**: a caption wider than one line is split into two at the most natural break that
//!   fits, preferring balanced lines and no one-word stub.
//! - **Timing**: a caption appears `lead_ms` before its first word and stays `hold_ms` after its
//!   last, longer when its text needs time to read at `reading_cps` (up to `max_linger_ms`), at
//!   least `min_cue_ms`; it never overlaps the next caption, and short gaps between captions are
//!   closed so the screen doesn't blink.
//!
//! Shot-change snapping is planned.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod aep;
pub mod aep_expr;
pub mod aep_look;
pub mod premiere;
pub mod styles;

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

/// Layout limits and timing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
    /// Time a caption appears before its first word (ms).
    pub lead_ms: i64,
    /// Reading speed (letters per second): a caption stays up long enough to be read at this
    /// speed when the next caption leaves room. 0 turns it off.
    pub reading_cps: u32,
    /// The longest a caption stays after its last word so it can be read (ms).
    pub max_linger_ms: i64,
    /// A gap to the next caption shorter than this is closed, so the screen doesn't blink (ms).
    pub chain_ms: i64,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            max_line: 32,
            max_lines: 2,
            max_cue_ms: 7000,
            pause_ms: 700,
            min_cue_ms: 1000,
            hold_ms: 400,
            lead_ms: 120,
            reading_cps: 12,
            max_linger_ms: 1500,
            chain_ms: 500,
        }
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
    w.trim_end_matches(CLOSERS).ends_with(['.', '?', '!', '…', '。', '？', '！'])
}

const CLOSERS: &[char] = &['"', '\'', ')', ']', '」', '』', '”', '’'];
const OPENERS: &[char] = &['"', '\'', '(', '[', '「', '『', '“', '‘'];

/// Sentence-final endings: the speaker finished a sentence even when recognition left no period.
const FINAL_ENDINGS: &[&str] = &[
    "습니다",
    "습니까",
    "니다",
    "어요",
    "아요",
    "에요",
    "예요",
    "해요",
    "워요",
    "죠",
    "까요",
    "네요",
    "군요",
    "래요",
    "세요",
    "거든",
    "잖아",
    "는다",
    "었다",
    "았다",
    "했다",
    "였다",
    "이다",
    "한다",
    "된다",
    "겠다",
];
/// Connective endings that close a clause: the most natural places to break inside a sentence.
const CLAUSE_ENDINGS: &[&str] = &[
    "는데",
    "은데",
    "던데",
    "지만",
    "니까",
    "어서",
    "아서",
    "해서",
    "워서",
    "와서",
    "려서",
    "면서",
    "다가",
    "도록",
    "려고",
    "거나",
    "든지",
    "어도",
    "아도",
    "해도",
    "더라도",
    "는지",
    "은지",
    "자마자",
    "므로",
    "는데요",
    "거든요",
    "잖아요",
    "니까요",
    "고요",
    "구요",
    "더니",
];
/// Shorter connectives (`…하고`, `…하며`, `…하면`): still a clause break, a little weaker.
const LIGHT_ENDINGS: &[&str] = &["고", "며", "면", "서"];
/// Particles: the break comes after a whole noun phrase.
const PARTICLES: &[&str] = &[
    "에서", "에게", "한테", "까지", "부터", "보다", "처럼", "마다", "밖에", "으로", "이랑", "랑", "은", "는", "이", "가", "을", "를", "도", "만", "에", "로",
    "와", "과", "께",
];
/// Words that modify the next word (determiners, adverbs, conjunctions, fillers): a line never
/// ends on them, they go with what follows.
const GLUE_AFTER: &[&str] = &[
    "이",
    "그",
    "저",
    "이런",
    "그런",
    "저런",
    "이러한",
    "그러한",
    "어떤",
    "무슨",
    "몇",
    "모든",
    "각",
    "온갖",
    "한",
    "두",
    "세",
    "첫",
    "매우",
    "아주",
    "너무",
    "정말",
    "진짜",
    "완전",
    "되게",
    "엄청",
    "좀",
    "조금",
    "더",
    "덜",
    "가장",
    "제일",
    "안",
    "못",
    "잘",
    "꼭",
    "다시",
    "또",
    "그냥",
    "바로",
    "같은",
    "다른",
    "약간",
    "거의",
    "특히",
    "그리고",
    "그래서",
    "그런데",
    "근데",
    "하지만",
    "그러면",
    "그럼",
    "그러니까",
    "그니까",
    "왜냐하면",
    "그러나",
    "그래도",
    "자",
    "막",
    "뭐",
    "이제",
    "일단",
    "사실",
    "또한",
    "아니면",
];
/// Words that open a clause: breaking right before them is natural.
const CONNECTORS: &[&str] = &[
    "그리고",
    "그래서",
    "그런데",
    "근데",
    "하지만",
    "그러면",
    "그럼",
    "그러니까",
    "그니까",
    "왜냐하면",
    "그러나",
    "그래도",
    "결국",
    "또한",
    "아니면",
    "그다음",
    "그다음에",
    "자",
];
/// Bound nouns (의존 명사) with their usual particles: they belong to the word before
/// (`할 수`, `하는 걸`, `본 적`), so a break never comes right before them.
const BOUND_NOUNS: &[&str] = &[
    "것",
    "것이",
    "것을",
    "것은",
    "것도",
    "것만",
    "거",
    "거야",
    "거죠",
    "거예요",
    "거에요",
    "거고",
    "거는",
    "거를",
    "거든요",
    "걸",
    "걸요",
    "건",
    "건데",
    "게",
    "겁니다",
    "겁니까",
    "수",
    "수가",
    "수도",
    "수는",
    "수를",
    "수밖에",
    "때",
    "때는",
    "때도",
    "때가",
    "때문",
    "때문에",
    "때문이죠",
    "때문입니다",
    "줄",
    "줄은",
    "데",
    "데는",
    "데도",
    "데서",
    "뿐",
    "뿐이",
    "뿐입니다",
    "만큼",
    "정도",
    "정도로",
    "정도예요",
    "정도의",
    "정도는",
    "정도가",
    "듯",
    "듯이",
    "듯한",
    "듯해요",
    "듯합니다",
    "중",
    "중에",
    "중에서",
    "적",
    "적도",
    "적은",
    "지",
    "바",
    "대로",
    "채",
    "척",
    "편",
    "편이",
    "편입니다",
    "김에",
];

/// Whether `w` and `next` form one verb phrase with an auxiliary (`하고 있다`, `하지 않다`,
/// `하게 되다`, `해 보다`, `할 수 있다`, `하는 것 같다`), which a break would tear apart.
fn verb_phrase(w: &str, next: &str) -> bool {
    let starts = |stems: &[&str]| stems.iter().any(|s| next.starts_with(s));
    let ends = |e: &[&str]| e.iter().any(|x| w.ends_with(x));
    (ends(&["고"]) && starts(&["있", "싶", "계", "나서", "나니", "나면", "나도", "난 "]))
        || (ends(&["지"]) && starts(&["않", "못", "말", "마"]))
        || (ends(&["도록"]) && starts(&["하", "해", "했", "할", "합"]))
        || (ends(&["만", "라고", "다고"]) && matches!(next, "해도" | "하면" | "하더라도"))
        // `존재는 하지만`: here `하지만` is the verb, not the conjunction
        || (ends(&["는", "은", "도", "만"]) && matches!(next, "하지만" | "하는데" | "하고" | "해요" | "합니다" | "하죠" | "했지만" | "했는데" | "했어요" | "했습니다"))
        || (ends(&["게"]) && starts(&["되", "돼", "됐", "된", "될", "만들", "만드", "하", "해", "했", "할", "합"]))
        || (ends(&["어", "아", "여", "해", "워", "와", "봐", "줘", "놔", "둬", "내"])
            && starts(&["보", "봐", "봤", "볼", "주", "줘", "줬", "버리", "버려", "버렸", "버린", "놓", "놔", "놨", "둬", "뒀", "있"]))
        || (matches!(w, "수" | "수가" | "수도" | "수는") && starts(&["있", "없"]))
        || (matches!(w, "것" | "거" | "걸" | "건" | "줄") && starts(&["같", "알", "몰"]))
}

/// Cost of breaking a caption or a line between `w` and `next`: 0 natural … 70 inside a noun
/// phrase; 100 and more tears a phrase apart and is used only when nothing else fits.
fn break_cost(w: &str, next: Option<&str>) -> u32 {
    let core = w.trim_end_matches(CLOSERS);
    if sentence_end(core) {
        return 0;
    }
    if core.ends_with([',', '、', '，', ':', ';']) {
        return 2;
    }
    let Some(n) = next.map(|n| n.trim_start_matches(OPENERS)) else { return 0 };
    if BOUND_NOUNS.contains(&n) || verb_phrase(core, n) {
        return 200;
    }
    if GLUE_AFTER.contains(&core) {
        return 150;
    }
    let ends = |list: &[&str]| list.iter().any(|e| core.ends_with(e));
    let cost = if ends(FINAL_ENDINGS) {
        5
    } else if ends(CLAUSE_ENDINGS) || core.ends_with('요') {
        8
    } else if core.ends_with('다') {
        10
    } else if ends(&["에서", "으로"]) || (ends(LIGHT_ENDINGS) && !ends(PARTICLES)) {
        14
    } else if core.ends_with('의') {
        55
    } else if core.ends_with('히') && core.chars().count() > 1 {
        // an adverb before its verb (`완전히 바뀌었다`)
        100
    } else if core.ends_with('게') && core.chars().count() > 1 {
        // `빠르게 넘어가다`, `쉽게 하다`
        60
    } else if ends(PARTICLES) {
        25
    } else if !core.is_empty() && core.chars().all(|c| c.is_ascii_digit()) {
        // a number before its counter (`5 시즌`)
        80
    } else {
        70
    };
    if CONNECTORS.contains(&n) { cost.min(6) } else { cost }
}

/// How much a silence between two words makes a break there more natural.
fn pause_bonus(gap_ms: i64) -> u32 {
    u32::try_from(((gap_ms - 120) / 8).clamp(0, 30)).unwrap_or(0)
}

/// Cost of breaking after `words[i]` (inside `words`); a silence there counts in full between
/// captions and half between the two lines of one caption.
fn cost_after(words: &[TimedWord], i: usize, line: bool) -> u32 {
    let (Some(w), next) = (words.get(i), words.get(i + 1)) else { return 0 };
    let gap = next.map_or(0, |n| n.start_ms - w.end_ms);
    let bonus = if line { pause_bonus(gap) / 2 } else { pause_bonus(gap) };
    break_cost(&w.text, next.map(|n| n.text.as_str())).saturating_sub(bonus)
}

fn joined_width(words: &[TimedWord]) -> u32 {
    let spaces = u32::try_from(words.len().saturating_sub(1)).unwrap_or(u32::MAX);
    words.iter().map(|w| width(&w.text)).fold(spaces, u32::saturating_add)
}

/// The best place to split `words` into two lines of at most `o.max_line`: `(cost, k)` puts
/// `words[..k]` on the first line. Natural break points win; among them, balanced lines and no
/// one-word stub.
fn line_split(words: &[TimedWord], o: &Layout) -> Option<(u32, usize)> {
    (1..words.len())
        .filter_map(|k| {
            let (wa, wb) = (joined_width(words.get(..k)?), joined_width(words.get(k..)?));
            if wa > o.max_line || wb > o.max_line {
                return None;
            }
            let stub = if wa.min(wb) < 8 { 25 } else { 0 };
            Some((cost_after(words, k - 1, true) + wa.abs_diff(wb) / 2 + stub, k))
        })
        .min()
}

/// Whether `words` fit in `o.max_lines` lines of `o.max_line` without breaking a word, and what
/// the line break inside costs (0 for one line).
fn fit_cost(words: &[TimedWord], o: &Layout) -> Option<u32> {
    if joined_width(words) <= o.max_line {
        return Some(0);
    }
    if o.max_lines < 2 {
        return None;
    }
    line_split(words, o).map(|(c, _)| c)
}

/// Split one sentence-or-pause segment into captions (DP over break points): every caption costs
/// something, a break costs what [`break_cost`] says, the line break inside a caption counts too,
/// and very short captions cost extra, so captions come out full but end at natural places.
fn split_segment(words: &[TimedWord], o: &Layout) -> Vec<std::ops::Range<usize>> {
    let n = words.len();
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
            let fit = fit_cost(slice, o);
            // Adding words in front never makes a caption fit again, so stop here.
            if !single && (fit.is_none() || dur > o.max_cue_ms) {
                break;
            }
            if prev == u64::MAX {
                continue;
            }
            let brk = if end == n { 0 } else { cost_after(words, end - 1, false) };
            let short = if joined_width(slice) < 12 && !(start == 0 && end == n) { 40 } else { 0 };
            let cost = prev + 50 + u64::from(brk) + u64::from(fit.unwrap_or(0)) + short;
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
    match line_split(words, o) {
        Some((_, k)) => vec![join(words.get(..k).unwrap_or(&[])), join(words.get(k..).unwrap_or(&[]))],
        None => wrap_chars(&all, o.max_line),
    }
}

/// Lay out timed words as captions.
pub fn layout(words: &[TimedWord], o: &Layout) -> Vec<Caption> {
    let mut caps = segment(words, o);
    fix_timing(&mut caps, o);
    caps
}

/// Captions for speaker turns: each turn is laid out on its own, so no caption mixes two
/// speakers, and a turn with a label (`"화자1"`) starts with it in parentheses.
pub fn layout_turns(turns: &[(Option<String>, Vec<TimedWord>)], o: &Layout) -> Vec<Caption> {
    let mut caps = Vec::new();
    for (label, words) in turns {
        let Some(first) = words.first() else { continue };
        let mut ws = Vec::with_capacity(words.len() + 1);
        if let Some(l) = label {
            ws.push(TimedWord { text: format!("({l})"), start_ms: first.start_ms, end_ms: first.start_ms });
        }
        ws.extend(words.iter().cloned());
        caps.extend(segment(&ws, o));
    }
    caps.sort_by_key(|c| c.start_ms);
    fix_timing(&mut caps, o);
    caps
}

fn segment(words: &[TimedWord], o: &Layout) -> Vec<Caption> {
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
    caps
}

/// Letters and digits in `s` (what a reader actually reads).
fn letters(s: &str) -> i64 {
    i64::try_from(s.chars().filter(|c| c.is_alphanumeric()).count()).unwrap_or(i64::MAX)
}

/// Display times from speech times (`caps` come in with the first word's start and the last
/// word's end, sorted by start):
///
/// - a caption appears `lead_ms` before its first word, but not before the previous caption's
///   speech is over nor so early that the previous caption is up less than `min_cue_ms`;
/// - it stays `hold_ms` after its last word, longer when its text needs more time to read at
///   `reading_cps` (at most `max_linger_ms` past the speech), and at least `min_cue_ms`;
/// - it never overlaps the next caption, and a gap to the next one shorter than `chain_ms` is
///   closed so the screen doesn't blink between two captions.
fn fix_timing(caps: &mut [Caption], o: &Layout) {
    let speech: Vec<(i64, i64)> = caps.iter().map(|c| (c.start_ms, c.end_ms)).collect();
    let lead = o.lead_ms.clamp(0, 2000);
    for i in 0..caps.len() {
        let Some(&(s, _)) = speech.get(i) else { continue };
        let mut start = s.saturating_sub(lead).max(0).min(s);
        if let Some(&(_, prev_end)) = i.checked_sub(1).and_then(|j| speech.get(j)) {
            start = start.max(prev_end.min(s));
        }
        // nor so early that the previous caption is cut below its minimum
        if let Some(prev) = i.checked_sub(1).and_then(|j| caps.get(j)) {
            start = start.max(prev.start_ms.saturating_add(o.min_cue_ms.clamp(0, 10_000)).min(s));
        }
        if let Some(c) = caps.get_mut(i) {
            c.start_ms = start;
        }
    }
    for i in 0..caps.len() {
        let next_start = caps.get(i + 1).map(|c| c.start_ms);
        let (Some(&(_, e)), Some(c)) = (speech.get(i), caps.get_mut(i)) else { continue };
        let start = c.start_ms;
        let read_ms = match i64::from(o.reading_cps) {
            0 => 0,
            cps => letters(&c.text()).saturating_mul(1000) / cps,
        };
        let hold = o.hold_ms.clamp(0, 10_000);
        let linger = e.saturating_add(o.max_linger_ms.clamp(0, 10_000).max(hold));
        let mut end = e.saturating_add(hold).max(start.saturating_add(read_ms).min(linger)).max(start.saturating_add(o.min_cue_ms.clamp(0, 10_000)));
        if let Some(ns) = next_start {
            end = end.min(ns.max(start + 1));
            if ns > end && ns - end < o.chain_ms {
                end = ns;
            }
        }
        c.end_ms = end.max(start + 1);
    }
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

    #[test]
    fn turns_never_share_a_caption_and_carry_labels() {
        let a = words(&[("이거", 0, 300), ("보세요", 300, 700)]);
        let b = words(&[("네", 750, 900), ("맞아요", 900, 1300)]);
        let caps = layout_turns(&[(Some("화자1".into()), a), (Some("화자2".into()), b), (None, Vec::new())], &Layout::default());
        assert_eq!(caps.len(), 2, "{caps:?}");
        assert_eq!(caps[0].lines.join(" "), "(화자1) 이거 보세요");
        assert_eq!(caps[1].lines.join(" "), "(화자2) 네 맞아요");
        assert!(caps[0].end_ms <= caps[1].start_ms);
        let plain = layout_turns(&[(None, words(&[("하나", 0, 100)]))], &Layout::default());
        assert_eq!(plain[0].lines, vec!["하나".to_string()]);
    }

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
        assert_eq!(c[1].start_ms, 900, "no lead-in: the first caption needs its minimum time");
        assert_eq!(c[0].end_ms, 900, "hold is cut where the next caption appears");
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
    fn full_width_caption_that_cannot_split_in_two_is_not_cut_mid_word() {
        // 64 units = two full lines, but no word boundary gives two lines of at most 32: this used
        // to come out as three lines with `대한` cut in half.
        let w = seq("거기에서 좀 순위가 많이 갈리고 있고 경기력 그리고 뭐 전략에 대한", 300);
        let o = Layout::default();
        let c = layout(&w, &o);
        assert!(c.len() >= 2);
        for c in &c {
            assert!(c.lines.len() <= 2 && c.lines.iter().all(|l| width(l) <= o.max_line), "{c:?}");
        }
        let all: Vec<String> = c.iter().flat_map(|c| c.lines.iter().flat_map(|l| l.split(' ').map(String::from))).collect();
        assert_eq!(all.join(" "), "거기에서 좀 순위가 많이 갈리고 있고 경기력 그리고 뭐 전략에 대한");
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

    fn texts(c: &[Caption]) -> Vec<String> {
        c.iter().map(|c| c.lines.join(" / ")).collect()
    }

    #[test]
    fn breaks_never_tear_a_phrase() {
        let o = Layout { max_line: 20, ..Layout::default() };
        for (text, torn) in [
            ("영상 꼭 보고 경쟁하는 걸 추천할 정도예요", ["경쟁하는 / 걸", "추천할 / 정도예요"]),
            ("지금은 다들 이 영웅을 할 수 있는 상황이에요", ["할 / 수", "수 / 있는"]),
            ("요즘 다들 솔저를 하고 있는 거 같아요 진짜로", ["하고 / 있는", "거 / 같아요"]),
            ("이번 시즌 너무 강한 영웅이 너무 많아서 힘들어요", ["너무 / 강한", "너무 / 많아서"]),
            ("나머지 영웅들은 다음에 넘어가도록 할게요", ["넘어가도록 / 할게요", "x"]),
            ("이전 시즌까지만 해도 캐서디 시온 조합이 괜찮았어요", ["시즌까지만 / 해도", "x"]),
            ("점수 차이가 분명히 존재는 하지만 지금 눈여겨볼 수치는 아니에요", ["존재는 / 하지만", "x"]),
            ("그래서 이번 시즌 설명 좀 쉽게 하기 위해서 과거 사례를 가져올게요", ["쉽게 / 하기", "x"]),
        ] {
            let c = layout(&seq(text, 250), &o);
            let all = texts(&c).join(" | ");
            for t in torn {
                assert!(!all.contains(t), "{text}: {all}");
                assert!(!all.contains(&t.replace(" / ", " | ")), "{text}: {all}");
            }
        }
    }

    #[test]
    fn lines_break_at_clause_ends_not_by_count() {
        // Fits on two lines of 17 Hangul either way; the clause end (`출시했고`) wins over the balanced break
        // inside the list of heroes.
        let c = layout(&seq("독트린이 출시했고 솜브라 로드호그가 리워크됐습니다.", 250), &Layout { max_line: 34, ..Layout::default() });
        assert_eq!(texts(&c), vec!["독트린이 출시했고 / 솜브라 로드호그가 리워크됐습니다.".to_string()], "{c:?}");
        // `6개의 / 전환` splits a noun phrase; the break after the particle is better
        let c = layout(&seq("측정 가능한 여섯 개의 전환 중에 1위입니다.", 250), &Layout::default());
        assert_eq!(texts(&c), vec!["측정 가능한 여섯 개의 전환 중에 / 1위입니다.".to_string()]);
    }

    #[test]
    fn a_pause_draws_the_break() {
        // Same words; a 600 ms breath after `분석` moves the line break there.
        let mut w = seq("티어리스트와 메타 분석 빠르게 보도록 하겠습니다", 250);
        for x in w.iter_mut().skip(3) {
            x.start_ms += 600;
            x.end_ms += 600;
        }
        let c = layout(&w, &Layout { max_line: 30, ..Layout::default() });
        assert_eq!(texts(&c), vec!["티어리스트와 메타 분석 / 빠르게 보도록 하겠습니다".to_string()]);
    }

    #[test]
    fn timing_leads_lingers_and_never_blinks() {
        let o = Layout::default();
        // a long line said fast, then silence: it stays up to be read (12 letters/s) but not
        // more than `max_linger_ms` after the speech
        let c = layout(&words(&[("이번", 1000, 1200), ("시즌은", 1200, 1500), ("독트린이", 1500, 1800), ("사기입니다.", 1800, 2200)]), &o);
        assert_eq!(c[0].start_ms, 1000 - o.lead_ms);
        let need = 13 * 1000 / 12;
        assert_eq!(c[0].end_ms, (c[0].start_ms + need).max(2200 + o.hold_ms));
        let c = layout(&words(&[("가나다라마바사아자차카타파하가나다라마바사아자차카타파하.", 0, 300)]), &o);
        assert_eq!(c[0].end_ms, 300 + o.max_linger_ms);
        // a short gap between two captions is closed, a long one is kept
        let c = layout(&words(&[("하나.", 0, 900), ("둘.", 1500, 2400), ("셋.", 6000, 6800)]), &o);
        assert_eq!(c[0].end_ms, c[1].start_ms, "{c:?}");
        assert!(c[1].end_ms < c[2].start_ms);
        assert!(c.windows(2).all(|p| p[0].end_ms <= p[1].start_ms && p[0].start_ms < p[0].end_ms));
        // a one-word caption still gets the minimum when there is room
        let c = layout(&words(&[("네.", 0, 200)]), &o);
        assert_eq!(c[0].end_ms - c[0].start_ms, o.min_cue_ms);
    }

    #[test]
    fn old_settings_without_timing_fields_still_load() {
        let o: Layout = serde_json::from_str(r#"{"max_line":30,"max_lines":2,"max_cue_ms":7000,"pause_ms":700,"min_cue_ms":700,"hold_ms":250}"#).unwrap();
        assert_eq!(o.max_line, 30);
        assert_eq!(o.reading_cps, Layout::default().reading_cps);
        let hostile =
            Layout { lead_ms: i64::MIN, hold_ms: i64::MAX, max_linger_ms: i64::MIN, chain_ms: i64::MAX, min_cue_ms: i64::MIN, reading_cps: u32::MAX, ..o };
        let c = layout(&seq("가 나 다 라", 10), &hostile);
        assert!(c.windows(2).all(|p| p[0].end_ms <= p[1].start_ms));
    }

    #[test]
    fn srt_round_trip() {
        let caps = vec![Caption { start_ms: 1000, end_ms: 2500, lines: vec!["가나".into(), "다라".into()] }];
        let s = to_srt(&caps);
        assert_eq!(s, "1\n00:00:01,000 --> 00:00:02,500\n가나\n다라\n\n");
        assert_eq!(from_srt(&s).unwrap(), caps);
    }
}

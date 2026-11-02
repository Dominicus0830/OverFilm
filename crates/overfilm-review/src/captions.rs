//! Captions that need judgment: where to break a sentence, and what was really said when the
//! recognition is unsure or the glossary correction changed it.
//!
//! For each doubtful caption ([`doubts`]) the reviewer gets the caption as it is, the words as
//! recognised before the glossary correction (pure STT) and after it, the unsure words, and the
//! captions before and after with their own pure recognition ([`question`]). It answers keep, a
//! corrected text (line break included), or a split into several captions ([`parse`]); the
//! split's times come from the word times ([`split_times`]).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::Question;
use crate::text::{bare, cap, clock, display_width};

/// Words below this recogniser confidence are "unsure" (the OverFilmSubs threshold).
pub const UNSURE: f32 = 0.7;

/// One recognised word of a caption.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapWord {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f32,
}

/// A caption shown as context.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Neighbor {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// Pure recognition of the same stretch, when known and different.
    pub raw: Option<String>,
}

/// Caption layout limits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// Line width in half-cells (a Korean letter is 2): 32 = 16 letters.
    pub line_width: u32,
    pub max_lines: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { line_width: 32, max_lines: 2 }
    }
}

/// One caption to ask about.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptionQuestion {
    /// The editor's id for it (the caption id).
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// The caption as it is, lines separated by `\n`.
    pub text: String,
    /// Pure recognition (before the glossary correction), when known.
    pub raw: Option<String>,
    /// The recognised words after correction, joined.
    pub corrected: String,
    pub unsure: Vec<String>,
    pub doubts: Vec<String>,
    pub before: Vec<Neighbor>,
    pub after: Vec<Neighbor>,
}

/// The reviewer's call on a caption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptionDecision {
    Keep,
    /// Replace the text (`text`).
    Fix,
    /// Make several captions (`parts`).
    Split,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptionAnswer {
    pub id: String,
    pub decision: CaptionDecision,
    /// Fix: the new text (lines by `\n`).
    pub text: String,
    /// Split: the new captions in order.
    pub parts: Vec<String>,
    pub reason: String,
}

/// Why a caption needs judgment; (doubts, unsure words). `next` is the caption after it.
pub fn doubts(text: &str, words: &[CapWord], raw: Option<&str>, corrected: &str, next: Option<&str>, lim: &Limits) -> (Vec<String>, Vec<String>) {
    let mut why = Vec::new();
    let unsure: Vec<String> = words.iter().filter(|w| w.confidence < UNSURE).map(|w| w.text.clone()).take(20).collect();
    if !unsure.is_empty() {
        why.push(format!("인식이 자신 없는 단어: {}", unsure.join(", ")));
    }
    if let Some(r) = raw
        && bare(r) != bare(corrected)
    {
        why.push("단어집 교정으로 인식 결과가 바뀜".into());
    }
    // "two" said in English: a numbered name (오버워치 2, 메인 딜러 2) or "two" before a word
    // (투 서브 딜러); the correction step decides by rule, the context may say otherwise
    let spoken_two = |t: &str| t.split_whitespace().any(|w| w == "투");
    let made_two = raw.is_some_and(|r| r.split_whitespace().any(|w| w.starts_with('투'))) && text.contains('2');
    if spoken_two(text) || raw.is_some_and(spoken_two) || made_two {
        why.push("영어로 읽은 숫자 '투'(two): 앞뒤 문맥으로 2로 쓸지 투로 둘지 판단".into());
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() > lim.max_lines as usize {
        why.push(format!("{}줄 (최대 {}줄)", lines.len(), lim.max_lines));
    }
    if lines.iter().any(|l| display_width(l.trim()) > lim.line_width) {
        why.push("줄이 너무 김".into());
    }
    if lines.len() >= 2 && lines.iter().any(|l| bare(l).chars().count() <= 2) {
        why.push("한 줄이 너무 짧아 줄바꿈이 어색함".into());
    }
    let ends = text.trim_end().ends_with(['.', '?', '!', '~', '…']);
    if !ends && next.is_some_and(|n| bare(n).chars().count() <= 3) {
        why.push("다음 자막이 아주 짧음 (문장이 어색하게 끊겼을 수 있음)".into());
    }
    (why, unsure)
}

/// The text of `raw` that lines up with the characters `from..to` of `corrected` (char indices),
/// by an edit-distance alignment of the two; `None` for very long texts.
pub fn raw_span(raw: &str, corrected: &str, from: usize, to: usize) -> Option<String> {
    let r: Vec<char> = raw.chars().collect();
    let c: Vec<char> = corrected.chars().collect();
    if r.len() > 2000 || c.len() > 2000 || from >= to || to > c.len() {
        return None;
    }
    let (n, m) = (c.len(), r.len());
    let w = m + 1;
    let mut d = vec![0u32; (n + 1) * w];
    for j in 0..=m {
        d[j] = j as u32;
    }
    for i in 1..=n {
        d[i * w] = i as u32;
        for j in 1..=m {
            let sub = d[(i - 1) * w + j - 1] + u32::from(c.get(i - 1) != r.get(j - 1));
            d[i * w + j] = sub.min(d[(i - 1) * w + j] + 1).min(d[i * w + j - 1] + 1);
        }
    }
    // map[i]: raw position aligned with corrected position i
    let mut map = vec![0usize; n + 1];
    let (mut i, mut j) = (n, m);
    map[n] = m;
    while i > 0 || j > 0 {
        let here = d[i * w + j];
        if i > 0 && j > 0 && here == d[(i - 1) * w + j - 1] + u32::from(c.get(i - 1) != r.get(j - 1)) {
            i -= 1;
            j -= 1;
        } else if i > 0 && here == d[(i - 1) * w + j] + 1 {
            i -= 1;
        } else {
            j = j.saturating_sub(1);
        }
        if let Some(slot) = map.get_mut(i) {
            *slot = j;
        }
    }
    let (a, b) = (*map.get(from)?, *map.get(to)?);
    let s: String = r.get(a.min(b)..b.max(a))?.iter().collect();
    Some(s.trim().to_string())
}

/// Recognition before the correction for a stretch of a stored chunk (see the engine).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawChunk {
    /// Index of the chunk's first word in the item's transcript, and how many words it has.
    pub first_word: usize,
    pub words: usize,
    /// Pure recognition and the corrected text the words were made from.
    pub raw: String,
    pub text: String,
}

impl RawChunk {
    /// Pure recognition of the transcript words `from..to` (indices in the item's transcript),
    /// given the texts of this chunk's words; `None` when the words no longer match the chunk
    /// (edited transcript) or lie outside it.
    pub fn raw_for(&self, from: usize, to: usize, chunk_words: &[&str]) -> Option<String> {
        let (a, b) = (from.checked_sub(self.first_word)?, to.checked_sub(self.first_word)?);
        if a >= b || b > self.words || chunk_words.len() != self.words {
            return None;
        }
        // letters and digits of the text line up with the words' own
        let joined: String = chunk_words.iter().map(|w| bare(w)).collect();
        if joined != bare(&self.text) {
            return None;
        }
        let pos: Vec<usize> = self.text.chars().enumerate().filter(|(_, c)| c.is_alphanumeric()).map(|(i, _)| i).collect();
        let before: usize = chunk_words.get(..a)?.iter().map(|w| bare(w).chars().count()).sum();
        let inside: usize = chunk_words.get(a..b)?.iter().map(|w| bare(w).chars().count()).sum();
        if inside == 0 {
            return None;
        }
        let from_c = *pos.get(before)?;
        let to_c = pos.get(before + inside - 1)? + 1;
        if self.raw == self.text {
            return Some(self.text.chars().skip(from_c).take(to_c - from_c).collect());
        }
        raw_span(&self.raw, &self.text, from_c, to_c)
    }
}

/// The system prompt.
pub fn system(lim: &Limits) -> String {
    format!(
        "당신은 오버워치 영상의 한국어 자막을 검토하는 사람입니다. 음성 인식(STT)이 만든 자막 중 판단이 필요한 것만 보냅니다. \
각 자막마다 '순수 인식'(단어집 교정 전), '교정본'(오버워치 단어집으로 영웅·스킬·맵·선수 이름을 고친 것), 현재 자막, \
그리고 앞뒤 자막(앞뒤 소리의 인식 결과)이 주어집니다. 앞뒤 문맥으로 실제로 한 말을 판단하세요.\n\
다음 중 하나를 고르세요.\n\
- keep: 지금 자막이 맞다.\n\
- fix: text에 고친 자막 전체를 쓴다. 줄바꿈은 \\n, 최대 {}줄, 한 줄은 한글 {}자 안팎. 의미 단위로 자연스럽게 끊는다 \
(조사·어미 앞에서 끊지 않는다).\n\
- split: 너무 긴 자막이나 두 문장이 섞인 자막을 parts에 순서대로 나눠 쓴다 (2~4개, 각각 위 줄 규칙을 지킨다).\n\
규칙: 들리지 않은 말을 지어내지 않는다. 고칠 근거(순수 인식, 앞뒤 문맥, 오버워치 용어)가 없으면 keep. \
오버워치 용어는 교정본의 표기를 따른다. 욕설은 초성으로 쓴다(예: ㅅㅂ). '(화자1)' 같은 화자 표시는 그대로 둔다. \
숫자: 영어로 읽은 '투'(two)·'원'(one)이 이름이나 순번 뒤에 오면 숫자로 쓴다(오버워치 2, 메인 딜러 2, 시즌 1). \
뒤의 말을 꾸미는 '두 개'라는 뜻이면 그대로 둔다(투 서브 딜러, 투 히트, 투탱). 단위 앞의 한국어 숫자는 아라비아 숫자로 쓴다(5시즌, 13위, 6개월). \
reason은 한국어 한 문장.",
        lim.max_lines,
        lim.line_width / 2
    )
}

fn neighbor_lines(ns: &[Neighbor]) -> String {
    let mut out = String::new();
    for n in ns {
        out.push_str(&format!("    [{}–{}] {}", clock(n.start_ms), clock(n.end_ms), cap(&n.text.replace('\n', " / "), 200)));
        if let Some(r) = &n.raw {
            out.push_str(&format!("   (순수 인식: {})", cap(r, 200)));
        }
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("    (없음)\n");
    }
    out
}

/// The question for a batch of captions.
pub fn question(cases: &[CaptionQuestion], lim: &Limits) -> Question {
    let mut p = String::from("다음 자막들을 검토해 주세요.\n\n");
    for c in cases {
        p.push_str(&format!("### 자막 id={} [{}–{}]\n", c.id, clock(c.start_ms), clock(c.end_ms)));
        p.push_str(&format!("- 현재 자막:\n    {}\n", cap(&c.text, 400).replace('\n', "\n    ")));
        match &c.raw {
            Some(r) => p.push_str(&format!("- 순수 인식(교정 전): {}\n", cap(r, 400))),
            None => p.push_str("- 순수 인식(교정 전): (기록 없음)\n"),
        }
        p.push_str(&format!("- 교정본: {}\n", cap(&c.corrected, 400)));
        if !c.unsure.is_empty() {
            p.push_str(&format!("- 자신 없는 단어: {}\n", c.unsure.join(", ")));
        }
        p.push_str(&format!("- 판단이 필요한 이유: {}\n", if c.doubts.is_empty() { "(확인 요청)".into() } else { c.doubts.join("; ") }));
        p.push_str("- 앞 자막:\n");
        p.push_str(&neighbor_lines(&c.before));
        p.push_str("- 뒤 자막:\n");
        p.push_str(&neighbor_lines(&c.after));
        p.push('\n');
    }
    Question { system: system(lim), prompt: p, schema: schema() }
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "answers": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "decision": {"type": "string", "enum": ["keep", "fix", "split"]},
                        "text": {"type": "string"},
                        "parts": {"type": "array", "items": {"type": "string"}},
                        "reason": {"type": "string"}
                    },
                    "required": ["id", "decision", "reason"]
                }
            }
        },
        "required": ["answers"]
    })
}

/// Lines trimmed, empty ones dropped; `None` when over the limits (a little slack on width).
fn tidy(text: &str, lim: &Limits) -> Option<String> {
    let lines: Vec<String> = text.replace("\\n", "\n").lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect();
    let slack = lim.line_width.saturating_add(lim.line_width / 4);
    let ok = !lines.is_empty() && lines.len() <= lim.max_lines as usize && lines.iter().all(|l| display_width(l) <= slack);
    ok.then(|| lines.join("\n"))
}

/// Letter edits between two strings (bounded inputs).
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().take(1000).collect();
    let b: Vec<char> = b.chars().take(1000).collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}

/// A rewrite stays close to what was heard: within 40 % of the letters (at least 4) of the
/// caption or of the pure recognition.
fn plausible(new: &str, c: &CaptionQuestion) -> bool {
    let n = bare(new);
    let near = |old: &str| {
        let o = bare(old);
        distance(&n, &o) <= (o.chars().count() * 2 / 5).max(4)
    };
    near(&c.text) || c.raw.as_deref().is_some_and(near)
}

/// The answers in `v` for `cases`, checked: unknown ids dropped; a fix or split that breaks the
/// line rules, is empty, or strays far from what was heard is dropped (the caption stays).
pub fn parse(v: &Value, cases: &[CaptionQuestion], lim: &Limits) -> Vec<CaptionAnswer> {
    let mut out: Vec<CaptionAnswer> = Vec::new();
    for a in v["answers"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().take(cases.len().saturating_mul(2)) {
        let Some(id) = a["id"].as_str() else { continue };
        let Some(c) = cases.iter().find(|c| c.id == id) else { continue };
        if out.iter().any(|o| o.id == id) {
            continue;
        }
        let Some(decision) = a["decision"].as_str().and_then(|d| serde_json::from_value::<CaptionDecision>(json!(d)).ok()) else { continue };
        let reason = cap(a["reason"].as_str().unwrap_or_default().trim(), 300);
        let mut ans = CaptionAnswer { id: id.into(), decision, text: String::new(), parts: Vec::new(), reason };
        match decision {
            CaptionDecision::Keep => {}
            CaptionDecision::Fix => {
                let Some(t) = a["text"].as_str().and_then(|t| tidy(t, lim)) else { continue };
                if !plausible(&t, c) {
                    continue;
                }
                if t == c.text {
                    ans.decision = CaptionDecision::Keep;
                } else {
                    ans.text = t;
                }
            }
            CaptionDecision::Split => {
                let parts: Option<Vec<String>> =
                    a["parts"].as_array().map(|p| p.iter().take(5).map(|x| x.as_str().and_then(|t| tidy(t, lim))).collect()).unwrap_or(None);
                let Some(parts) = parts.filter(|p| (2..=4).contains(&p.len())) else { continue };
                if !plausible(&parts.join(" "), c) {
                    continue;
                }
                ans.parts = parts;
            }
        }
        out.push(ans);
    }
    out
}

/// Times of the `parts` a caption `start..end` is split into: each part ends at the word where
/// its share of the letters runs out (the next starts at the following word), or by letter share
/// when the words don't cover the caption.
pub fn split_times(start: i64, end: i64, words: &[CapWord], parts: &[String]) -> Vec<(i64, i64)> {
    let lens: Vec<usize> = parts.iter().map(|p| bare(p).chars().count().max(1)).collect();
    let total: usize = lens.iter().sum::<usize>().max(1);
    let wl: Vec<usize> = words.iter().map(|w| bare(&w.text).chars().count()).collect();
    let wtotal: usize = wl.iter().sum();
    let span = end.saturating_sub(start).max(0);
    let mut cuts: Vec<i64> = Vec::new();
    let mut acc = 0usize;
    for l in lens.iter().take(lens.len().saturating_sub(1)) {
        acc += l;
        let t = if wtotal > 0 && words.len() >= 2 {
            // the word boundary nearest to this share of the words' letters
            let want = acc * wtotal / total;
            let mut run = 0usize;
            let mut k = 0usize;
            while k + 1 < words.len() && run + wl.get(k).copied().unwrap_or(0) <= want {
                run += wl.get(k).copied().unwrap_or(0);
                k += 1;
            }
            let k = k.max(1).min(words.len() - 1);
            let (a, b) = (words.get(k - 1).map_or(start, |w| w.end_ms), words.get(k).map_or(end, |w| w.start_ms));
            a.saturating_add(b) / 2
        } else {
            start.saturating_add(i64::try_from(acc as u128 * span as u128 / total as u128).unwrap_or(0))
        };
        cuts.push(t.clamp(cuts.last().copied().unwrap_or(start), end));
    }
    let mut out = Vec::with_capacity(parts.len());
    let mut at = start;
    for t in cuts.into_iter().chain(std::iter::once(end)) {
        out.push((at, t.max(at)));
        at = t.max(at);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cw(t: &str, a: i64, b: i64, c: f32) -> CapWord {
        CapWord { text: t.into(), start_ms: a, end_ms: b, confidence: c }
    }

    #[test]
    fn doubts_found() {
        let lim = Limits::default();
        let words = vec![cw("겐지가", 0, 500, 0.4), cw("들어갑니다", 500, 1000, 0.95)];
        let (d, u) = doubts("겐지가 들어갑니다", &words, Some("갠지가 들어갑니다"), "겐지가 들어갑니다", None, &lim);
        assert_eq!(u, vec!["겐지가"]);
        assert_eq!(d.len(), 2, "{d:?}");
        let (d, _) = doubts("가나다라마바사아자차카타파하가나다라\n가", &[], None, "", Some("네"), &lim);
        assert_eq!(d.len(), 3, "{d:?}");
        assert!(doubts("좋습니다.", &[cw("좋습니다.", 0, 1, 0.9)], None, "좋습니다.", Some("네"), &lim).0.is_empty());
        // "two" said in English, still there or written as 2 by the correction
        let two = |t: &str, raw: Option<&str>| doubts(t, &[], raw, t, None, &lim).0.iter().any(|d| d.contains("'투'"));
        assert!(two("지금 투 서브 딜러가", None));
        assert!(two("메인 딜러 2라고", Some("메인 딜러 투라고")));
        assert!(!two("이번 투표는", None));
    }

    #[test]
    fn raw_lines_up() {
        // the correction changed 갠지 to 겐지 and 트레 to 트레이서
        let raw = "자 갠지가 들어가고 트레 따라갑니다";
        let text = "자 겐지가 들어가고 트레이서 따라갑니다";
        let i = text.find("트레이서").unwrap();
        let from = text[..i].chars().count();
        assert_eq!(raw_span(raw, text, from, from + 4).as_deref(), Some("트레"));
        assert_eq!(raw_span(raw, text, 2, 5).as_deref(), Some("갠지가"));
        assert!(raw_span(raw, text, 5, 5).is_none());
        assert!(raw_span(raw, text, 0, 999).is_none());
        let ch = RawChunk { first_word: 10, words: 5, raw: raw.into(), text: text.into() };
        let ws = ["자", "겐지가", "들어가고", "트레이서", "따라갑니다"];
        assert_eq!(ch.raw_for(11, 13, &ws).as_deref(), Some("갠지가 들어가고"));
        assert_eq!(ch.raw_for(13, 15, &ws).as_deref(), Some("트레 따라갑니다"));
        assert!(ch.raw_for(9, 12, &ws).is_none());
        assert!(ch.raw_for(11, 16, &ws).is_none());
        assert!(ch.raw_for(11, 13, &["자", "다른", "말", "이네", "요"]).is_none());
        let same = RawChunk { raw: text.into(), ..ch };
        assert_eq!(same.raw_for(11, 12, &ws).as_deref(), Some("겐지가"));
    }

    fn case() -> CaptionQuestion {
        CaptionQuestion {
            id: "5".into(),
            start_ms: 0,
            end_ms: 4000,
            text: "자 겐지가 들어가고 트레이서\n따라갑니다".into(),
            raw: Some("자 갠지가 들어가고 트레 따라갑니다".into()),
            ..Default::default()
        }
    }

    #[test]
    fn answers_checked() {
        let lim = Limits::default();
        let v = json!({"answers": [
            {"id": "5", "decision": "fix", "text": "자 겐지가 들어가고\\n트레이서 따라갑니다", "reason": "줄바꿈"},
            {"id": "6", "decision": "keep", "reason": "없는 id"},
        ]});
        let a = parse(&v, &[case()], &lim);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].text, "자 겐지가 들어가고\n트레이서 따라갑니다");
        let invented = json!({"answers": [{"id": "5", "decision": "fix", "text": "오늘 저녁은 치킨을 먹으러 갑니다", "reason": ""}]});
        assert!(parse(&invented, &[case()], &lim).is_empty());
        let three = json!({"answers": [{"id": "5", "decision": "fix", "text": "자 겐지가\n들어가고 트레이서\n따라갑니다", "reason": ""}]});
        assert!(parse(&three, &[case()], &lim).is_empty());
        let split = json!({"answers": [{"id": "5", "decision": "split", "parts": ["자 겐지가 들어가고", "트레이서 따라갑니다"], "reason": "두 문장"}]});
        assert_eq!(parse(&split, &[case()], &lim)[0].parts.len(), 2);
        let one = json!({"answers": [{"id": "5", "decision": "split", "parts": ["자 겐지가 들어가고 트레이서 따라갑니다"], "reason": ""}]});
        assert!(parse(&one, &[case()], &lim).is_empty());
        let same = json!({"answers": [{"id": "5", "decision": "fix", "text": "자 겐지가 들어가고 트레이서\n따라갑니다", "reason": ""}]});
        assert_eq!(parse(&same, &[case()], &lim)[0].decision, CaptionDecision::Keep);
    }

    #[test]
    fn split_follows_words() {
        let words = vec![
            cw("자", 0, 300, 1.0),
            cw("겐지가", 400, 900, 1.0),
            cw("들어가고", 1000, 1600, 1.0),
            cw("트레이서", 2400, 3000, 1.0),
            cw("따라갑니다", 3100, 3900, 1.0),
        ];
        let parts = vec!["자 겐지가 들어가고".to_string(), "트레이서 따라갑니다".to_string()];
        assert_eq!(split_times(0, 4000, &words, &parts), vec![(0, 2000), (2000, 4000)]);
        // no words: by letters
        assert_eq!(split_times(0, 1000, &[], &["가나다".into(), "라".into()]), vec![(0, 750), (750, 1000)]);
        let t = split_times(i64::MAX - 10, i64::MAX, &[], &["가".into(), "나".into(), "다".into()]);
        assert_eq!(t.len(), 3);
        assert!(t.iter().all(|(a, b)| a <= b));
    }
}

#[cfg(test)]
mod real {
    use super::*;
    use crate::{ClaudeCode, Reviewer, Settings};

    #[test]
    #[ignore = "runs the installed Claude Code with the user's subscription"]
    fn real_claude_code_reviews_a_caption() {
        let lim = Limits::default();
        let case = CaptionQuestion {
            id: "12".into(),
            start_ms: 61_000,
            end_ms: 64_000,
            text: "자 여기서 트레이서가 펄스\n폭탄".into(),
            raw: Some("자 여기서 트레가 펄스 폭탄".into()),
            corrected: "자 여기서 트레이서가 펄스 폭탄".into(),
            unsure: vec!["트레이서가".into()],
            doubts: vec!["인식이 자신 없는 단어: 트레이서가".into(), "한 줄이 너무 짧아 줄바꿈이 어색함".into()],
            before: vec![Neighbor { start_ms: 58_000, end_ms: 61_000, text: "오른쪽으로 돌아서 들어가는 트레이서".into(), raw: None }],
            after: vec![Neighbor { start_ms: 64_000, end_ms: 66_000, text: "붙였습니다 라인하르트 잡아냅니다".into(), raw: None }],
        };
        let cc = ClaudeCode::new(&Settings { enabled: true, ..Settings::default() }).unwrap();
        let v = cc.ask(&question(std::slice::from_ref(&case), &lim), &std::sync::atomic::AtomicBool::new(false)).unwrap();
        println!("{v}");
        let a = parse(&v, &[case], &lim);
        println!("{a:?}");
        assert_eq!(a.len(), 1);
    }
}

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

/// Words below this confidence are worth a look too: the recogniser is often sure of a wrong
/// word that sounds alike (measured on the aichi set: most misses were between 0.7 and 0.9).
pub const CHECK: f32 = 0.9;

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
    let check: Vec<&str> = words.iter().filter(|w| (UNSURE..CHECK).contains(&w.confidence)).map(|w| w.text.as_str()).take(20).collect();
    if !check.is_empty() {
        why.push(format!("조금 덜 확실한 단어 (비슷한 소리의 다른 말일 수 있음): {}", check.join(", ")));
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

/// One caption with what is known about it, before it is chosen to ask about.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaptionIn {
    /// id, times, text, pure recognition and corrected text (doubts and context left empty).
    pub q: CaptionQuestion,
    pub words: Vec<CapWord>,
    /// Real words that are often a misrecognition of a glossary term ([`suspects`]).
    pub suspects: Vec<String>,
}

/// The words of `text` that are a glossary's review-only misrecognition (a `misheard` form marked
/// `@검토`: a real word too, so the correction step leaves it), as `경제하는 → 경쟁?`.
pub fn suspects(terms: &[Term], text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for t in terms {
        for m in t.misheard.iter().filter_map(|m| m.strip_suffix("@검토")) {
            let f = bare(m);
            if f.is_empty() {
                continue;
            }
            for w in text.split_whitespace().filter(|w| bare(w).starts_with(&f)) {
                let s = format!("{w} → {}?", t.aliases.first().unwrap_or(&t.ko));
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// The captions of a track worth asking about (all of them with `all`), in order, each with
/// its doubts and the two captions on either side; `skip` leaves some out (already reviewed).
pub fn select(infos: &[CaptionIn], lim: &Limits, all: bool, skip: impl Fn(&CaptionIn) -> bool, max: usize) -> Vec<(usize, CaptionQuestion)> {
    let neighbor = |i: usize| {
        infos.get(i).map(|x| Neighbor {
            start_ms: x.q.start_ms,
            end_ms: x.q.end_ms,
            text: x.q.text.clone(),
            raw: x.q.raw.clone().filter(|r| bare(r) != bare(&x.q.corrected)),
        })
    };
    let mut out = Vec::new();
    for (i, info) in infos.iter().enumerate() {
        if skip(info) {
            continue;
        }
        let next = infos.get(i + 1).map(|x| x.q.text.as_str());
        let (mut doubts, unsure) = doubts(&info.q.text, &info.words, info.q.raw.as_deref(), &info.q.corrected, next, lim);
        if !info.suspects.is_empty() {
            doubts.push(format!("실제 낱말이지만 오버워치 영상에선 오인식이 잦은 단어: {}", info.suspects.join(", ")));
        }
        if doubts.is_empty() && !all {
            continue;
        }
        let mut c = info.q.clone();
        c.doubts = doubts;
        c.unsure = unsure;
        c.before = (i.saturating_sub(2)..i).filter_map(&neighbor).collect();
        c.after = (i + 1..i + 3).filter_map(&neighbor).collect();
        out.push((i, c));
        if out.len() >= max {
            break;
        }
    }
    out
}

/// The system prompt.
pub fn system(lim: &Limits) -> String {
    format!(
        "당신은 오버워치 영상의 한국어 자막을 검토하는 사람입니다. 음성 인식(STT)이 만든 자막을 보냅니다. \
각 자막마다 '순수 인식'(단어집 교정 전), '교정본'(오버워치 단어집으로 영웅·스킬·맵·선수 이름을 고친 것), 현재 자막, \
그리고 앞뒤 자막(앞뒤 소리의 인식 결과)이 주어집니다. 앞뒤 문맥과 오버워치 지식으로 실제로 한 말을 판단하세요.\n\
다음 중 하나를 고르세요.\n\
- keep: 지금 자막이 맞다.\n\
- fix: text에 고친 자막 전체를 쓴다. 줄바꿈은 \\n, 최대 {}줄, 한 줄은 한글 {}자 안팎. 의미 단위로 자연스럽게 끊는다 \
(조사·어미 앞에서 끊지 않는다).\n\
- split: 너무 긴 자막이나 두 문장이 섞인 자막을 parts에 순서대로 나눠 쓴다 (2~4개, 각각 위 줄 규칙을 지킨다).\n\
오인식 고치기: 인식기는 자신 있게 틀리기도 합니다. 발음이 비슷한 다른 말로 적어 문장이 말이 안 되거나, 없는 단어가 되거나, \
문맥과 맞지 않는 단어(예: 탱커 영웅을 딜러라고 부름, 힐러 자리에 킬러)가 있으면, 발음이 비슷하고 문맥에 맞는 말로 고친다 \
(예: 궁 개이지→궁 게이지, 뒤로 빠저야→뒤로 빠져야). 띄어쓰기·맞춤법 오류도 고친다. \
오버워치 용어·은어(투섭딜·투 서브 딜러=서브 딜러 둘, 투 히트, 밴률·픽률·승률, 한타, 궁, 냥이=제트팩 캣 등)와 \
아래 '정식 표기' 목록의 이름을 떠올려 보고, 소리가 비슷한 용어가 있으면 그 표기로 쓴다. \
고칠 말을 고를 때: 한국어는 받침이 다음 글자로 넘어가 들리므로(짚어→[지퍼], 읽어→[일거]) \
먼저 소리 나는 대로 읽었을 때 똑같거나 거의 같은 말을 찾고, 그중 문맥에 맞는 것을 쓴다. 뜻만 통하고 소리가 다른 말(지퍼→지켜봐)은 쓰지 않는다. \
영웅 이름으로 바꾸기 전에 숫자·순위(일이삼위=1, 2, 3위, 이위=2위)나 흔한 낱말일 가능성을 먼저 따진다. \
인식기는 빠르게 이어 읽은 숫자를 엉뚱한 낱말로 적는다: 순위·점수를 말하는 문맥에서 없는 단어가 숫자를 이어 읽은 소리\
(일이삼위[이리사뮈], 사오위[사오위], 십이삼위)와 비슷하면 숫자로 쓴다(키리섬이가 모두 힐러죠→1, 2, 3위가 모두 힐러죠). \
앞뒤 자막에 이미 나온 말(경쟁전, 영웅 이름, 순위)과 소리가 비슷하면 그 말일 가능성이 높다(경쟁전 이야기 뒤의 경제하는→경쟁하는). \
영웅 목록의 역할은 지금 게임 기준일 뿐이다. 영웅 옆의 역할이 목록과 다르면 먼저 화자가 역할 변경·과거·가정·비교를 말하는지 본다 \
('둠피스트는 딜러로 다시 바뀌어야 해', '딜러 시절 둠피스트', '딜러처럼 쓰는'): 그런 말이면 화자 말 그대로 둔다. \
그런 말이 없고 영웅을 그냥 소개·분류하는 자리(탱커 삼국밥 중 마지막, 고티어 힐러들)에서만 인식 오류로 보고 목록의 역할로 고친다. \
없는 단어가 영웅 이름과 소리가 거의 같으면(자리하네요→자리야네요) 영웅 이름으로 고친다. \
고칠 근거가 소리와 문맥 어디에도 없으면 keep. 발음과 동떨어진 말을 지어내지 않는다. \
오버워치 용어는 교정본·정식 표기 목록의 표기를 따른다. 욕설은 초성으로 쓴다(예: ㅅㅂ). '(화자1)' 같은 화자 표시는 그대로 둔다. \
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

/// A glossary term as the reviewer is shown it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Term {
    /// The spelling captions use.
    pub ko: String,
    pub en: String,
    /// What it is: "heroes", "players", "maps"… (the glossary table).
    pub kind: String,
    /// Other names people say for it, and forms the recogniser is known to write for it.
    pub aliases: Vec<String>,
    pub misheard: Vec<String>,
    /// What it means or its role (`돌격-개시자`, `서브 딜러 둘`).
    pub note: String,
}

/// A glossary form without its context mark (`양이@영웅` → `양이`).
fn plain_form(f: &str) -> &str {
    f.split('@').next().unwrap_or(f).trim()
}

/// The terms of `terms` that the `texts` mention, by their spelling, another name or a known
/// misrecognition (spacing ignored; two-letter forms count only as whole words), at most `max`,
/// one line each: `겐지 (Genji, 영웅; 다른 이름: …; 잘못 인식되는 꼴: …)`.
pub fn relevant_terms(terms: &[Term], texts: &[&str], max: usize) -> Vec<String> {
    let bare_texts: Vec<String> = texts.iter().map(|t| bare(t)).collect();
    let words: std::collections::HashSet<String> = texts.iter().flat_map(|t| t.split_whitespace().map(bare)).collect();
    let hit = |form: &str| {
        let b = bare(plain_form(form));
        match b.chars().count() {
            0 | 1 => false,
            2 => words.iter().any(|w| w.starts_with(&b)),
            _ => bare_texts.iter().any(|t| t.contains(&b)),
        }
    };
    let mut out = Vec::new();
    for t in terms {
        if out.len() >= max {
            break;
        }
        if !(hit(&t.ko) || t.aliases.iter().any(|a| hit(a)) || t.misheard.iter().any(|m| hit(m))) {
            continue;
        }
        let mut line = t.ko.clone();
        let mut info = Vec::new();
        if !t.en.is_empty() {
            info.push(t.en.clone());
        }
        if !t.kind.is_empty() {
            info.push(t.kind.clone());
        }
        if !t.note.is_empty() {
            info.push(cap(&t.note, 60));
        }
        if !t.aliases.is_empty() {
            info.push(format!("다른 이름: {}", t.aliases.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
        }
        if !t.misheard.is_empty() {
            info.push(format!("잘못 인식되는 꼴: {}", t.misheard.iter().take(6).map(|m| plain_form(m)).collect::<Vec<_>>().join(", ")));
        }
        if !info.is_empty() {
            line.push_str(&format!(" ({})", info.join("; ")));
        }
        out.push(line);
    }
    out
}

/// The role a hero's note names first (`돌격-투사` → tank), in the words casters use.
fn role(note: &str) -> Option<&'static str> {
    [("돌격", "탱커"), ("공격", "딜러"), ("지원", "힐러")].into_iter().filter_map(|(k, r)| note.find(k).map(|i| (i, r))).min_by_key(|(i, _)| *i).map(|(_, r)| r)
}

/// Every hero with their role (`자리야 탱커`), so the reviewer can tell a hero name from a
/// common word that sounds like one and check that a role said next to a hero fits.
pub fn roster(terms: &[Term]) -> Vec<String> {
    terms
        .iter()
        .filter(|t| t.kind == "heroes")
        .map(|t| match role(&t.note) {
            Some(r) => format!("{} {r}", t.ko),
            None => t.ko.clone(),
        })
        .collect()
}

/// What the reviewer should know about the whole video.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Background {
    /// "일반 게임 영상" or "대회 해설 중계".
    pub video: String,
    /// What the screen showed around these captions: the match, the players, the heroes in play.
    pub on_screen: Vec<String>,
    /// Glossary terms the captions mention ([`relevant_terms`]).
    pub terms: Vec<String>,
    /// Every hero with their role ([`roster`]).
    pub roster: Vec<String>,
}

impl Background {
    fn text(&self) -> String {
        let mut p = String::new();
        if !self.video.is_empty() {
            p.push_str(&format!("영상 종류: {}\n", self.video));
        }
        if !self.on_screen.is_empty() {
            p.push_str("화면에서 읽은 경기 정보 (이 이름들이 나올 가능성이 높음):\n");
            for l in self.on_screen.iter().take(20) {
                p.push_str(&format!("  - {}\n", cap(l, 300)));
            }
        }
        if !self.roster.is_empty() {
            let list: Vec<&str> = self.roster.iter().take(80).map(String::as_str).collect();
            p.push_str(&format!(
                "오버워치 영웅 전체와 지금 게임의 역할 (탱커=돌격, 딜러=공격, 힐러=지원; 영상이 역할 변경을 말하면 화자 말을 따름): {}\n",
                list.join(", ")
            ));
        }
        if !self.terms.is_empty() {
            p.push_str("이 자막들에 나온 오버워치 용어의 정식 표기 (자막은 이 표기를 씀):\n");
            for l in self.terms.iter().take(60) {
                p.push_str(&format!("  - {}\n", cap(l, 200)));
            }
        }
        if !p.is_empty() {
            p.push('\n');
        }
        p
    }
}

/// The question for a batch of captions.
pub fn question(cases: &[CaptionQuestion], lim: &Limits) -> Question {
    question_with(cases, lim, &Background::default())
}

/// The question for a batch of captions, with what is known about the video.
pub fn question_with(cases: &[CaptionQuestion], lim: &Limits, bg: &Background) -> Question {
    let mut p = bg.text();
    p.push_str("다음 자막들을 검토해 주세요.\n\n");
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
        p.push_str(&format!(
            "- 판단이 필요한 이유: {}\n",
            if c.doubts.is_empty() { "(문맥 확인: 말이 안 되거나 문맥과 안 맞는 단어가 없는지)".into() } else { c.doubts.join("; ") }
        ));
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
        // confident but not quite: asked about too
        let (d, u) = doubts("이 영웅은 지퍼야겠죠.", &[cw("지퍼야겠죠.", 0, 1, 0.8)], None, "이 영웅은 지퍼야겠죠.", None, &lim);
        assert!(u.is_empty());
        assert!(d.iter().any(|d| d.contains("지퍼야겠죠")), "{d:?}");
    }

    fn info(id: &str, text: &str, raw: &str, conf: f32) -> CaptionIn {
        let q = CaptionQuestion { id: id.into(), text: text.into(), corrected: text.into(), raw: Some(raw.into()), ..CaptionQuestion::default() };
        CaptionIn { q, words: vec![cw(text, 0, 1, conf)], suspects: Vec::new() }
    }

    #[test]
    fn selects_doubtful_with_neighbours() {
        let lim = Limits::default();
        let infos =
            vec![info("1", "하나.", "하나", 1.0), info("2", "둘.", "둘", 1.0), info("3", "겐지가 간다.", "갠지가 간다", 1.0), info("4", "넷.", "넷", 1.0)];
        let got = select(&infos, &lim, false, |_| false, 10);
        assert_eq!(got.len(), 1);
        let (i, c) = &got[0];
        assert_eq!(*i, 2);
        assert_eq!(c.before.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["하나.", "둘."]);
        assert_eq!(c.after.len(), 1);
        assert!(c.after[0].raw.is_none(), "same as the caption: not repeated");
        assert_eq!(select(&infos, &lim, true, |_| false, 10).len(), 4);
        assert_eq!(select(&infos, &lim, true, |x| x.q.id == "1", 2).len(), 2);
        assert!(select(&[], &lim, true, |_| false, 10).is_empty());
    }

    fn term(ko: &str, misheard: &[&str], note: &str) -> Term {
        Term {
            ko: ko.into(),
            en: String::new(),
            kind: "heroes".into(),
            aliases: Vec::new(),
            misheard: misheard.iter().map(|s| s.to_string()).collect(),
            note: note.into(),
        }
    }

    #[test]
    fn terms_the_captions_mention() {
        let terms = vec![term("둠피스트", &[], "돌격-개시자"), term("냥이", &["양이@영웅"], ""), term("겐지", &[], ""), term("밴률", &["벤율"], "")];
        let got = relevant_terms(&terms, &["둠피스트 딜러가", "양이 캐서디 연계", "벤율이 높죠"], 10);
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(got[0].starts_with("둠피스트 (heroes; 돌격-개시자"), "{got:?}");
        assert!(got[1].contains("잘못 인식되는 꼴: 양이") && !got[1].contains('@'), "{got:?}");
        // two-letter forms only as whole words (no 양이 inside 고양이)
        assert!(relevant_terms(&terms, &["고양이가"], 10).is_empty());
        assert_eq!(relevant_terms(&terms, &["둠피스트 겐지 벤율"], 1).len(), 1);
    }

    #[test]
    fn review_only_misrecognitions() {
        let t = Term { ko: "경쟁전".into(), aliases: vec!["경쟁".into()], misheard: vec!["경제@검토".into()], ..Term::default() };
        assert_eq!(suspects(std::slice::from_ref(&t), "영상 보고 경제하는 걸"), ["경제하는 → 경쟁?"]);
        assert!(suspects(std::slice::from_ref(&t), "경쟁하는 걸").is_empty());
        let lim = Limits::default();
        let mut i = info("1", "영상 보고 경제하는 걸.", "영상 보고 경제하는 걸", 1.0);
        assert!(select(std::slice::from_ref(&i), &lim, false, |_| false, 5).is_empty());
        i.suspects = suspects(&[t], &i.q.text);
        let got = select(&[i], &lim, false, |_| false, 5);
        assert!(got[0].1.doubts.iter().any(|d| d.contains("경제하는 → 경쟁?")), "{:?}", got[0].1.doubts);
    }

    #[test]
    fn heroes_with_roles() {
        let terms = vec![
            term("둠피스트", &[], "돌격-개시자"),
            term("솜브라", &[], "2026-10 5시즌부터 지원 (전에는 공격-수색가)"),
            term("시온", &[], ""),
            Term { kind: "maps".into(), ..term("일리오스", &[], "") },
        ];
        assert_eq!(roster(&terms), ["둠피스트 탱커", "솜브라 힐러", "시온"]);
        let bg = Background { roster: roster(&terms), ..Background::default() };
        assert!(bg.text().contains("둠피스트 탱커, 솜브라 힐러"), "{}", bg.text());
    }

    #[test]
    fn background_goes_first() {
        let lim = Limits::default();
        let c = CaptionQuestion { id: "7".into(), text: "둠피스트 딜러".into(), ..CaptionQuestion::default() };
        let bg = Background {
            video: "대회 해설".into(),
            on_screen: vec!["OverFilm 선수 — 로제: 태종".into()],
            terms: vec!["둠피스트 (돌격)".into()],
            roster: Vec::new(),
        };
        let q = question_with(std::slice::from_ref(&c), &lim, &bg);
        let (v, s, t, case) = (q.prompt.find("영상 종류"), q.prompt.find("로제: 태종"), q.prompt.find("둠피스트 (돌격)"), q.prompt.find("### 자막 id=7"));
        assert!(v < s && s < t && t < case && v.is_some(), "{}", q.prompt);
        assert!(!question(&[c], &lim).prompt.contains("영상 종류"));
        assert!(q.system.contains("자신 있게 틀리기도"));
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

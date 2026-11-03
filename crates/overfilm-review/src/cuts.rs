//! Ambiguous cuts: is this "OverFilm 컷" range really dead time, and if part of it must stay,
//! exactly which part.
//!
//! The editor finds the ranges worth a second look with [`doubts`] (speech or game voice at an
//! edge or inside, a highlight next to it), sends each with the words heard around it and what
//! the screen showed ([`question`]), and turns the answer into the ranges still to cut
//! ([`CutAnswer::cut_ranges`]). Nothing is cut here: the editor only changes its cut markers.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cli::Question;
use crate::text::{cap, clock};

/// How close speech may come to an edge before the cut counts as doubtful (ms).
pub const EDGE_MS: i64 = 1500;
/// Words around the range sent as context (ms each side).
pub const CONTEXT_MS: i64 = 30_000;
/// Shortest piece of a range left to cut or kept (ms); smaller pieces are folded away.
pub const MIN_PIECE_MS: i64 = 500;

/// A word heard on the sequence.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
    /// In-game voice (hero lines, announcer).
    pub game: bool,
}

/// What the screen showed at a moment (from the screen templates).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenAt {
    pub at_ms: i64,
    /// "대기 화면", "라운드 준비", "퍼즈", "경기 화면", "알 수 없음".
    pub screen: String,
}

/// One cut range to ask about.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CutQuestion {
    /// The editor's id for it (the marker id).
    pub id: String,
    /// What the rules took it for: "대기", "퍼즈", "무음".
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// Why it is doubtful ([`doubts`]).
    pub doubts: Vec<String>,
    /// Words from [`CONTEXT_MS`] before to after.
    pub words: Vec<Word>,
    pub screens: Vec<ScreenAt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CutDecision {
    /// Cut the whole range.
    Cut,
    /// Cut none of it.
    Keep,
    /// Keep the `keep` parts, cut the rest.
    Partial,
}

/// The reviewer's answer for one range, checked against it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CutAnswer {
    pub id: String,
    pub decision: CutDecision,
    /// Parts to keep (inside the range, sorted, not overlapping); empty unless partial.
    pub keep: Vec<(i64, i64)>,
    pub reason: String,
}

/// Why the cut `start..end` of `kind` deserves a second look, from the words around it and the
/// highlight ranges; empty when it is clear.
pub fn doubts(start: i64, end: i64, words: &[Word], highlights: &[(i64, i64)]) -> Vec<String> {
    let mut why = Vec::new();
    let speech = |w: &&Word| !w.game;
    let near = |a: i64, b: i64| words.iter().filter(speech).any(|w| w.start_ms < b && a < w.end_ms);
    if near(start.saturating_sub(EDGE_MS), start.saturating_add(EDGE_MS)) {
        why.push(format!("시작 경계({}) 가까이에 말이 있음", clock(start)));
    }
    if near(end.saturating_sub(EDGE_MS), end.saturating_add(EDGE_MS)) {
        why.push(format!("끝 경계({}) 가까이에 말이 있음", clock(end)));
    }
    let inside = words.iter().filter(speech).filter(|w| w.start_ms >= start && w.end_ms <= end).count();
    if inside > 0 {
        why.push(format!("구간 안에 말 {inside}단어"));
    }
    let game = words.iter().filter(|w| w.game && w.start_ms < end && start < w.end_ms).count();
    if game > 0 {
        why.push(format!("구간 안에 게임 음성 {game}단어 (궁극기 대사 등)"));
    }
    if highlights.iter().any(|&(a, b)| a < end.saturating_add(5000) && start.saturating_sub(5000) < b) {
        why.push("하이라이트·교전 후보와 겹치거나 5초 안에 있음".into());
    }
    why
}

/// The words of `all` from [`CONTEXT_MS`] before `start` to after `end` (at most 400).
pub fn context(all: &[Word], start: i64, end: i64) -> Vec<Word> {
    let (a, b) = (start.saturating_sub(CONTEXT_MS), end.saturating_add(CONTEXT_MS));
    all.iter().filter(|w| w.end_ms > a && w.start_ms < b).take(400).cloned().collect()
}

/// Words as lines of one speaker, broken at pauses over 0.7 s; `[mm:ss.s–mm:ss.s] 화자: 말`.
fn lines(words: &[Word], start: i64, end: i64) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < words.len() {
        let Some(first) = words.get(i) else { break };
        let mut j = i + 1;
        while let (Some(prev), Some(w)) = (words.get(j - 1), words.get(j)) {
            if w.speaker != first.speaker || w.game != first.game || w.start_ms.saturating_sub(prev.end_ms) > 700 || j - i >= 30 {
                break;
            }
            j += 1;
        }
        let run = words.get(i..j).unwrap_or(&[]);
        let last = run.last().map_or(first.end_ms, |w| w.end_ms);
        let place = if last <= start {
            "앞"
        } else if first.start_ms >= end {
            "뒤"
        } else {
            "안"
        };
        let who = if first.game { "게임 음성".to_string() } else { first.speaker.clone().unwrap_or_else(|| "말".into()) };
        let text: Vec<&str> = run.iter().map(|w| w.text.as_str()).collect();
        out.push_str(&format!("  [{}–{}] ({place}) {who}: {}\n", clock(first.start_ms), clock(last), cap(&text.join(" "), 300)));
        i = j;
    }
    if out.is_empty() {
        out.push_str("  (들리는 말 없음)\n");
    }
    out
}

/// The system prompt: what the reviewer decides and the user's rules.
pub fn system(rules: &str) -> String {
    format!(
        "당신은 오버워치 영상 편집자를 돕는 검토자입니다. 편집 프로그램(OverFilm)이 자동으로 자르려는 구간 중 \
판단이 애매한 것만 보냅니다. 각 구간마다 다음 중 하나를 고르세요.\n\
- cut: 구간 전체를 자른다 (정말 쓸모없는 대기·퍼즈·무음).\n\
- keep: 하나도 자르지 않는다.\n\
- partial: 일부만 남긴다. keep에 남길 부분을 밀리초(ms) 정수로 정확히 적는다. 반드시 구간 안이어야 하고, \
말이 잘리지 않게 말 앞뒤로 0.3초 정도 여유를 둔다.\n\
판단 기준: 시청자가 들어야 할 말(해설, 반응, 농담, 설명)이 잘리면 안 된다. 문장 중간에서 끊기면 안 된다. \
게임 음성(궁극기 대사)이나 하이라이트 직전·직후 분위기는 살린다. 확신이 없으면 자르지 않는 쪽(keep/partial)을 고른다.\n\
사용자 규칙: {rules}\n\
reason은 한국어 한두 문장으로 쓴다."
    )
}

/// The user's standing cut rules (2026-11-01): waiting goes up to 10 s before the start, pauses go
/// whole, nothing else of the match is cut; silences only when nothing happens.
pub const DEFAULT_RULES: &str = "대회 영상의 경기 대기 시간·라운드 준비 시간은 라운드 시작 10초 전까지만 자르고, 퍼즈는 자연스럽게 자른다. \
무음 구간은 정말 아무 일도 없을 때만 자른다. 그 밖의 경기 장면과 해설은 절대 자르지 않는다.";

/// The question for a batch of ranges.
pub fn question(cases: &[CutQuestion], rules: &str) -> Question {
    let mut p = String::from("다음 구간들을 판단해 주세요. 시간은 시퀀스 시간이며 ms 값은 정수 밀리초입니다.\n\n");
    for c in cases {
        p.push_str(&format!(
            "### 구간 id={}\n- 종류: {}\n- 범위: {} – {} (start_ms={}, end_ms={}, 길이 {:.1}초)\n- 애매한 이유: {}\n",
            c.id,
            c.kind,
            clock(c.start_ms),
            clock(c.end_ms),
            c.start_ms,
            c.end_ms,
            c.end_ms.saturating_sub(c.start_ms) as f64 / 1000.0,
            if c.doubts.is_empty() { "(없음, 확인 요청)".to_string() } else { c.doubts.join("; ") }
        ));
        if !c.screens.is_empty() {
            let s: Vec<String> = c.screens.iter().map(|s| format!("{} {}", clock(s.at_ms), s.screen)).collect();
            p.push_str(&format!("- 화면: {}\n", s.join(", ")));
        }
        p.push_str("- 주변에서 들리는 말 (앞/안/뒤 = 구간 앞, 구간 안, 구간 뒤):\n");
        p.push_str(&lines(&c.words, c.start_ms, c.end_ms));
        p.push('\n');
    }
    Question { system: system(rules), prompt: p, schema: schema() }
}

/// JSON schema of the answer.
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
                        "decision": {"type": "string", "enum": ["cut", "keep", "partial"]},
                        "keep": {"type": "array", "items": {"type": "object",
                            "properties": {"start_ms": {"type": "integer"}, "end_ms": {"type": "integer"}},
                            "required": ["start_ms", "end_ms"]}},
                        "reason": {"type": "string"}
                    },
                    "required": ["id", "decision", "reason"]
                }
            }
        },
        "required": ["answers"]
    })
}

/// The answers in `v` for `cases`, checked: unknown ids dropped, keep parts clipped into their
/// range and merged; a partial keeping nothing is a cut, one keeping (almost) all is a keep.
pub fn parse(v: &Value, cases: &[CutQuestion]) -> Vec<CutAnswer> {
    let mut out: Vec<CutAnswer> = Vec::new();
    for a in v["answers"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().take(cases.len().saturating_mul(2)) {
        let Some(id) = a["id"].as_str() else { continue };
        let Some(c) = cases.iter().find(|c| c.id == id) else { continue };
        if out.iter().any(|o| o.id == id) {
            continue;
        }
        let Some(decision) = a["decision"].as_str().and_then(|d| serde_json::from_value::<CutDecision>(json!(d)).ok()) else { continue };
        let reason = cap(a["reason"].as_str().unwrap_or_default().trim(), 300);
        let mut keep: Vec<(i64, i64)> = a["keep"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .take(32)
            .filter_map(|k| Some((k["start_ms"].as_i64()?.clamp(c.start_ms, c.end_ms), k["end_ms"].as_i64()?.clamp(c.start_ms, c.end_ms))))
            .filter(|(a, b)| b > a)
            .collect();
        keep.sort_unstable();
        let mut merged: Vec<(i64, i64)> = Vec::new();
        for (a, b) in keep {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        let mut ans = CutAnswer { id: id.to_string(), decision, keep: if decision == CutDecision::Partial { merged } else { Vec::new() }, reason };
        if decision == CutDecision::Partial {
            let pieces = ans.cut_ranges(c.start_ms, c.end_ms);
            if ans.keep.is_empty() {
                ans.decision = CutDecision::Cut;
            } else if pieces.is_empty() {
                ans.decision = CutDecision::Keep;
                ans.keep.clear();
            }
        }
        out.push(ans);
    }
    out
}

impl CutAnswer {
    /// The parts of `start..end` still to cut: all of it, none, or what lies between the kept
    /// parts, without pieces under [`MIN_PIECE_MS`].
    pub fn cut_ranges(&self, start: i64, end: i64) -> Vec<(i64, i64)> {
        match self.decision {
            CutDecision::Cut => vec![(start, end)],
            CutDecision::Keep => Vec::new(),
            CutDecision::Partial => {
                let mut out = Vec::new();
                let mut at = start;
                for &(a, b) in &self.keep {
                    if a.saturating_sub(at) >= MIN_PIECE_MS {
                        out.push((at, a));
                    }
                    at = at.max(b);
                }
                if end.saturating_sub(at) >= MIN_PIECE_MS {
                    out.push((at, end));
                }
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, a: i64, b: i64) -> Word {
        Word { text: text.into(), start_ms: a, end_ms: b, speaker: Some("화자1".into()), game: false }
    }

    #[test]
    fn doubts_found() {
        let words = vec![w("자", 9000, 9800), w("갑니다", 30_500, 31_000), Word { game: true, ..w("류승룡", 20_000, 21_000) }];
        let d = doubts(10_000, 30_000, &words, &[]);
        assert_eq!(d.len(), 3, "{d:?}");
        assert!(doubts(100_000, 120_000, &words, &[]).is_empty());
        assert_eq!(doubts(100_000, 120_000, &words, &[(124_000, 130_000)]).len(), 1);
    }

    fn case() -> CutQuestion {
        CutQuestion { id: "7".into(), kind: "무음".into(), start_ms: 10_000, end_ms: 20_000, ..Default::default() }
    }

    #[test]
    fn partial_answers_checked() {
        let v = json!({"answers": [
            {"id": "7", "decision": "partial", "keep": [{"start_ms": 15_000, "end_ms": 16_000}, {"start_ms": 5_000, "end_ms": 10_200}, {"start_ms": 15_500, "end_ms": 17_000}], "reason": "말"},
            {"id": "7", "decision": "cut", "reason": "중복"},
            {"id": "99", "decision": "cut", "reason": "없는 id"},
            {"id": "8", "decision": "maybe", "reason": "x"}
        ]});
        let a = parse(&v, &[case()]);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].keep, vec![(10_000, 10_200), (15_000, 17_000)]);
        // 10.0–10.2 kept, 10.2–15 cut, 15–17 kept, 17–20 cut
        assert_eq!(a[0].cut_ranges(10_000, 20_000), vec![(10_200, 15_000), (17_000, 20_000)]);
    }

    #[test]
    fn degenerate_partials() {
        let none = json!({"answers": [{"id": "7", "decision": "partial", "keep": [{"start_ms": 30_000, "end_ms": 40_000}], "reason": ""}]});
        assert_eq!(parse(&none, &[case()])[0].decision, CutDecision::Cut);
        let all = json!({"answers": [{"id": "7", "decision": "partial", "keep": [{"start_ms": 10_100, "end_ms": 19_800}], "reason": ""}]});
        let a = &parse(&all, &[case()])[0];
        assert_eq!(a.decision, CutDecision::Keep);
        assert!(a.cut_ranges(10_000, 20_000).is_empty());
        let huge = json!({"answers": [{"id": "7", "decision": "partial", "keep": [{"start_ms": i64::MIN, "end_ms": i64::MAX}], "reason": ""}]});
        assert_eq!(parse(&huge, &[case()])[0].decision, CutDecision::Keep);
        assert!(parse(&json!({"answers": "x"}), &[case()]).is_empty());
        assert!(parse(&json!(null), &[case()]).is_empty());
    }

    #[test]
    fn prompt_has_the_context() {
        let mut c = case();
        c.words = vec![w("이제", 8000, 8400), w("시작합니다", 8400, 9000), Word { game: true, ..w("석양이", 12_000, 13_000) }, w("와", 21_000, 21_500)];
        c.screens = vec![ScreenAt { at_ms: 12_000, screen: "경기 화면".into() }];
        let q = question(&[c], DEFAULT_RULES);
        assert!(q.prompt.contains("id=7") && q.prompt.contains("(앞) 화자1: 이제 시작합니다"), "{}", q.prompt);
        assert!(q.prompt.contains("(안) 게임 음성: 석양이") && q.prompt.contains("(뒤) 화자1: 와"));
        assert!(q.prompt.contains("0:12.0 경기 화면"));
        assert!(q.system.contains("10초 전"));
        assert_eq!(q.schema["required"][0], "answers");
    }
}

/// The real Claude Code on this machine (`cargo test -p overfilm-review -- --ignored`): it must be
/// installed and signed in.
#[cfg(test)]
mod real {
    use super::*;
    use crate::{ClaudeCode, Reviewer, Settings};

    #[test]
    #[ignore = "runs the installed Claude Code with the user's subscription"]
    fn real_claude_code_answers_a_cut() {
        let w = |t: &str, a: i64, b: i64| Word { text: t.into(), start_ms: a, end_ms: b, speaker: Some("해설1".into()), game: false };
        let case = CutQuestion {
            id: "1".into(),
            kind: "무음".into(),
            start_ms: 61_000,
            end_ms: 66_000,
            doubts: vec!["끝 경계(1:06.0) 가까이에 말이 있음".into()],
            words: vec![
                w("자", 58_000, 58_300),
                w("이제", 58_400, 58_800),
                w("한타", 59_000, 59_500),
                w("들어갑니다", 59_600, 60_600),
                w("와", 65_700, 66_000),
                w("나노", 66_000, 66_400),
                w("들어갔어요", 66_400, 67_200),
            ],
            screens: vec![ScreenAt { at_ms: 63_000, screen: "경기 화면".into() }],
        };
        let cc = ClaudeCode::new(&Settings { enabled: true, ..Settings::default() }).unwrap();
        println!("{:?}", cc.status());
        let q = question(std::slice::from_ref(&case), DEFAULT_RULES);
        let v = cc.ask(&q, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        println!("{v:#}");
        let a = parse(&v, std::slice::from_ref(&case));
        assert_eq!(a.len(), 1, "{v}");
        println!("{:?} → cut {:?}", a[0], a[0].cut_ranges(case.start_ms, case.end_ms));
    }
}

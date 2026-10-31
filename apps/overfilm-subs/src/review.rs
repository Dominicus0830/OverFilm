//! Caption review: problems a person (or an assistant over MCP) should look at, and the edits that
//! fix them. Everything works on SRT files, so captions from any tool can be reviewed.

use overfilm_captions::{Caption, Layout, width};
use overfilm_glossary::Corrector;
use serde::Serialize;

/// Reading speed above which a caption is flagged (Hangul syllables and letters per second; the
/// usual Korean limit is 12–14).
pub const MAX_CPS: u32 = 15;
/// Captions shorter than this are hard to read (ms).
pub const MIN_MS: i64 = 700;

/// One finding.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Issue {
    /// 1-based caption number (as in the SRT).
    pub index: usize,
    pub kind: &'static str,
    pub message: String,
    /// Suggested text, when there is an obvious fix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Letters and digits (what the reader actually reads).
pub fn letters(s: &str) -> u32 {
    u32::try_from(s.chars().filter(|c| c.is_alphanumeric()).count()).unwrap_or(u32::MAX)
}

/// Letters per second, rounded down; 0 for an empty caption.
pub fn cps(c: &Caption) -> u32 {
    let ms = (c.end_ms - c.start_ms).max(1);
    u32::try_from(i64::from(letters(&c.text())).saturating_mul(1000) / ms).unwrap_or(u32::MAX)
}

fn layout_issues(i: usize, c: &Caption, o: &Layout, out: &mut Vec<Issue>) {
    let max_lines = usize::try_from(o.max_lines).unwrap_or(2);
    if c.lines.len() > max_lines {
        out.push(Issue { index: i, kind: "lines", message: format!("{}줄입니다 (최대 {max_lines}줄)", c.lines.len()), suggestion: None });
    }
    for l in &c.lines {
        if width(l) > o.max_line {
            out.push(Issue {
                index: i,
                kind: "width",
                message: format!("한 줄이 {}자로 깁니다 (최대 {}자): {l}", width(l).div_ceil(2), o.max_line / 2),
                suggestion: None,
            });
        }
    }
    if c.text().trim().is_empty() {
        out.push(Issue { index: i, kind: "empty", message: "빈 자막입니다".into(), suggestion: None });
    }
}

fn timing_issues(i: usize, c: &Caption, prev: Option<&Caption>, out: &mut Vec<Issue>) {
    let dur = c.end_ms - c.start_ms;
    if dur < MIN_MS {
        out.push(Issue { index: i, kind: "short", message: format!("{dur}ms만 보입니다 (최소 {MIN_MS}ms)"), suggestion: None });
    }
    let speed = cps(c);
    if speed > MAX_CPS {
        out.push(Issue { index: i, kind: "fast", message: format!("초당 {speed}자로 읽기 빠릅니다 (권장 {MAX_CPS}자 이하)"), suggestion: None });
    }
    if let Some(p) = prev {
        if c.start_ms < p.end_ms {
            out.push(Issue { index: i, kind: "overlap", message: format!("앞 자막과 {}ms 겹칩니다", p.end_ms - c.start_ms), suggestion: None });
        }
        if p.text().trim() == c.text().trim() && !c.text().trim().is_empty() {
            out.push(Issue {
                index: i, kind: "repeat", message: "앞 자막과 같은 내용입니다 (인식 반복일 수 있음)".into(), suggestion: None
            });
        }
    }
}

/// Everything worth a look in `caps`.
pub fn review(caps: &[Caption], o: &Layout, corrector: &Corrector) -> Vec<Issue> {
    let mut out = Vec::new();
    for (k, c) in caps.iter().enumerate() {
        let i = k + 1;
        layout_issues(i, c, o, &mut out);
        timing_issues(i, c, k.checked_sub(1).and_then(|p| caps.get(p)), &mut out);
        let fixed = corrector.apply(&c.text());
        if fixed.replaced > 0 {
            let list: Vec<String> = fixed.changes.iter().map(|(a, b)| format!("{a} → {b}")).collect();
            out.push(Issue { index: i, kind: "glossary", message: format!("단어집과 다릅니다: {}", list.join(", ")), suggestion: Some(fixed.text) });
        }
    }
    out
}

/// A gap between captions (no speech captioned).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Silence {
    /// Caption numbers before and after (0 = start of the video).
    pub after: usize,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Gaps of at least `min_ms` between captions (and before the first).
pub fn silences(caps: &[Caption], min_ms: i64) -> Vec<Silence> {
    let mut out = Vec::new();
    let mut prev_end = 0i64;
    for (k, c) in caps.iter().enumerate() {
        if c.start_ms - prev_end >= min_ms.max(1) {
            out.push(Silence { after: k, start_ms: prev_end, end_ms: c.start_ms });
        }
        prev_end = prev_end.max(c.end_ms);
    }
    out
}

/// `HH:MM:SS,mmm`.
pub fn clock(ms: i64) -> String {
    let ms = ms.max(0);
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Parse `HH:MM:SS,mmm`, `MM:SS.mmm`, `SS.mmm` or plain milliseconds.
pub fn parse_clock(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(ms) = s.parse::<i64>() {
        return Some(ms);
    }
    let (hms, frac) = s.rsplit_once([',', '.']).unwrap_or((s, "0"));
    let mut total = 0i64;
    for part in hms.split(':') {
        total = total.checked_mul(60)?.checked_add(part.trim().parse::<i64>().ok().filter(|v| *v >= 0)?)?;
    }
    let f: String = frac.chars().take(3).collect();
    let f_ms = f.parse::<i64>().ok()? * 10i64.pow(3 - u32::try_from(f.len()).ok()?.min(3));
    total.checked_mul(1000)?.checked_add(f_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(s: i64, e: i64, t: &str) -> Caption {
        Caption { start_ms: s, end_ms: e, lines: t.lines().map(String::from).collect() }
    }

    #[test]
    fn finds_problems() {
        let g = crate::pack::glossary(None, true).unwrap();
        let c = crate::pack::corrector(&g);
        let caps = vec![
            cap(0, 2000, "괜찮은 자막입니다"),
            cap(1500, 1900, "겹치고 짧고 아주아주아주 빠르게 지나가는 자막"),
            cap(3000, 5000, "씨발 이건 뭐야"),
            cap(5000, 7000, "씨발 이건 뭐야"),
            cap(8000, 9000, "한 줄\n두 줄\n세 줄"),
        ];
        let kinds: Vec<(usize, &str)> = review(&caps, &Layout::default(), &c).iter().map(|i| (i.index, i.kind)).collect();
        assert!(!kinds.iter().any(|k| k.0 == 1), "{kinds:?}");
        for k in [(2, "short"), (2, "fast"), (2, "overlap"), (3, "glossary"), (4, "repeat"), (5, "lines")] {
            assert!(kinds.contains(&k), "{k:?} missing in {kinds:?}");
        }
    }

    #[test]
    fn silences_and_clock() {
        let caps = vec![cap(3000, 4000, "a"), cap(4100, 5000, "b"), cap(9000, 9500, "c")];
        let s = silences(&caps, 2000);
        assert_eq!(s, vec![Silence { after: 0, start_ms: 0, end_ms: 3000 }, Silence { after: 2, start_ms: 5000, end_ms: 9000 }]);
        assert_eq!(clock(3_723_004), "01:02:03,004");
        assert_eq!(parse_clock("01:02:03,004"), Some(3_723_004));
        assert_eq!(parse_clock("2:03.5"), Some(123_500));
        assert_eq!(parse_clock("1500"), Some(1500));
        assert_eq!(parse_clock("99999999999999:99:99"), None);
        assert_eq!(parse_clock("a:b"), None);
    }
}

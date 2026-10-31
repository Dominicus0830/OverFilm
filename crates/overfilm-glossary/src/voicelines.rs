//! In-game voice lines (hero ultimates, the announcer) found in a transcript by text, so captions
//! can leave game audio out. The table is `packs/overwatch/ko/voicelines.tsv`
//! (`hero kind text lang note`).
//!
//! A run of consecutive words matches a line when, compared on Hangul syllables, letters and
//! digits only (spacing and punctuation ignored), its edit distance is at most a fifth of the
//! line's length. Lines shorter than [`MIN_SYLLABLES`] are too easy to say for real ("간다",
//! "승리") and only count as supporting evidence ([`Match::short`]).

/// Lines shorter than this (in compared characters) are weak evidence on their own.
pub const MIN_SYLLABLES: usize = 5;
/// Most lines kept from a table.
pub const MAX_LINES: usize = 5_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub hero: String,
    pub kind: String,
    pub text: String,
    norm: Vec<char>,
}

/// A match: words `[start, end)` say `line` with `score` (1 = exact).
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub score: f32,
    /// The line is shorter than [`MIN_SYLLABLES`].
    pub short: bool,
}

#[derive(Clone, Debug, Default)]
pub struct VoiceLines {
    pub lines: Vec<Line>,
}

fn norm(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        if let Some(c0) = cur.first_mut() {
            *c0 = i + 1;
        }
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let del = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let ins = cur.get(j).copied().unwrap_or(0) + 1;
            if let Some(c) = cur.get_mut(j + 1) {
                *c = sub.min(del).min(ins);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.get(b.len()).copied().unwrap_or(0)
}

impl VoiceLines {
    /// Parse a table (header `hero kind text …`; `#` comments and blank lines skipped).
    pub fn parse(tsv: &str) -> Self {
        let mut lines = Vec::new();
        let mut header = true;
        for row in tsv.lines() {
            let row = row.trim_end_matches('\r');
            if row.trim().is_empty() || row.starts_with('#') {
                continue;
            }
            if header {
                header = false;
                if row.starts_with("hero\t") {
                    continue;
                }
            }
            let mut cols = row.split('\t');
            let (Some(hero), Some(kind), Some(text)) = (cols.next(), cols.next(), cols.next()) else { continue };
            let n = norm(text);
            if n.len() < 2 || n.len() > 60 || lines.len() >= MAX_LINES {
                continue;
            }
            lines.push(Line { hero: hero.trim().into(), kind: kind.trim().into(), text: text.trim().into(), norm: n });
        }
        Self { lines }
    }

    /// Non-overlapping matches in `words`, best first where they compete.
    pub fn find(&self, words: &[&str]) -> Vec<Match> {
        let w: Vec<Vec<char>> = words.iter().map(|w| norm(w)).collect();
        let longest = self.lines.iter().map(|l| l.norm.len()).max().unwrap_or(0);
        let mut found: Vec<Match> = Vec::new();
        for i in 0..w.len() {
            let mut span: Vec<char> = Vec::new();
            for j in i..w.len() {
                if let Some(x) = w.get(j) {
                    span.extend(x);
                }
                if span.len() > longest + longest / 4 + 1 {
                    break;
                }
                // a span must start and end with real characters
                if span.is_empty() || w.get(i).is_some_and(Vec::is_empty) || w.get(j).is_some_and(Vec::is_empty) {
                    continue;
                }
                for (li, l) in self.lines.iter().enumerate() {
                    let (a, b) = (span.len(), l.norm.len());
                    let allowed = b / 5;
                    if a.abs_diff(b) > allowed {
                        continue;
                    }
                    let d = edit_distance(&span, &l.norm);
                    if d <= allowed {
                        found.push(Match { start: i, end: j + 1, line: li, score: 1.0 - d as f32 / b.max(1) as f32, short: b < MIN_SYLLABLES });
                    }
                }
            }
        }
        // keep the best (score, then length) of overlapping matches
        found.sort_by(|a, b| b.score.total_cmp(&a.score).then((b.end - b.start).cmp(&(a.end - a.start))));
        let mut keep: Vec<Match> = Vec::new();
        for m in found {
            if keep.iter().all(|k| m.end <= k.start || m.start >= k.end) {
                keep.push(m);
            }
        }
        keep.sort_by_key(|m| m.start);
        keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str =
        "hero\tkind\ttext\tlang\tnote\n# c\n캐서디\tult_enemy\t석양이 진다\tko\t\n겐지\tult_enemy\t류진노 켄오 쿠라에\tja\t\n아나운서\tannouncer\t승리\tko\t\n";

    #[test]
    fn finds_lines_across_spacing_and_small_errors() {
        let v = VoiceLines::parse(T);
        assert_eq!(v.lines.len(), 3);
        let words = ["와", "석양이", "진다.", "아", "류진노켄오", "쿠라애", "승리!"];
        let m = v.find(&words);
        assert_eq!(m.len(), 3, "{m:?}");
        assert_eq!((m[0].start, m[0].end, m[0].line), (1, 3, 0));
        assert_eq!(m[0].score, 1.0);
        assert_eq!((m[1].start, m[1].end, m[1].line), (4, 6, 1));
        assert!(m[1].score < 1.0 && !m[1].short);
        assert!(m[2].short);
    }

    #[test]
    fn unrelated_speech_and_hostile_input() {
        let v = VoiceLines::parse(T);
        assert!(v.find(&["석양", "이쪽으로", "진입해요"]).is_empty());
        assert!(v.find(&[]).is_empty());
        assert!(VoiceLines::parse("").lines.is_empty());
        assert!(VoiceLines::parse("a\tb").lines.is_empty());
        let long = "가".repeat(10_000);
        assert!(v.find(&[&long, "!!", ""]).is_empty());
    }
}

//! In-game voice lines (hero ultimates, the announcer) found in a transcript by text, so captions
//! can leave game audio out. The table is `packs/overwatch/ko/voicelines.tsv`
//! (`hero kind text lang note`).
//!
//! A run of consecutive words matches a line when, compared on Hangul syllables, letters and
//! digits only (spacing and punctuation ignored), its edit distance is at most a fifth of the
//! line's length. Lines shorter than [`MIN_SYLLABLES`] are too easy to say for real ("간다",
//! "승리") and only count as supporting evidence ([`Match::short`]).

use std::collections::{HashMap, HashSet};

/// Lines shorter than this (in compared characters) are weak evidence on their own.
pub const MIN_SYLLABLES: usize = 5;
/// Most lines kept from a table.
pub const MAX_LINES: usize = 60_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub hero: String,
    pub kind: String,
    pub text: String,
    norm: Vec<char>,
}

impl Line {
    /// Compared characters (letters and digits).
    pub fn len(&self) -> usize {
        self.norm.len()
    }

    pub fn is_empty(&self) -> bool {
        self.norm.is_empty()
    }
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

/// Distinct neighbouring character pairs.
fn pairs_of(s: &[char]) -> HashSet<(char, char)> {
    s.windows(2).filter_map(|p| Some((*p.first()?, *p.get(1)?))).collect()
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
    ///
    /// Only lines that share enough character pairs with the text after a word are compared: an
    /// edit removes at most two of a line's distinct pairs, so a line within `allowed` edits keeps
    /// all but `2 × allowed` of them. The answer is the same as comparing every line.
    pub fn find(&self, words: &[&str]) -> Vec<Match> {
        let w: Vec<Vec<char>> = words.iter().map(|w| norm(w)).collect();
        let longest = self.lines.iter().map(|l| l.norm.len()).max().unwrap_or(0);
        let reach = longest + longest / 4 + 1;
        let mut index: HashMap<(char, char), Vec<usize>> = HashMap::new();
        let mut need: Vec<usize> = Vec::with_capacity(self.lines.len());
        for (li, l) in self.lines.iter().enumerate() {
            let pairs = pairs_of(&l.norm);
            need.push(pairs.len().saturating_sub(2 * (l.norm.len() / 5)).max(1));
            for p in pairs {
                index.entry(p).or_default().push(li);
            }
        }
        let mut shared = vec![0usize; self.lines.len()];
        let mut found: Vec<Match> = Vec::new();
        for i in 0..w.len() {
            // a span must start with real characters
            if w.get(i).is_none_or(Vec::is_empty) {
                continue;
            }
            let mut text: Vec<char> = Vec::new();
            for x in w.get(i..).unwrap_or_default() {
                if text.len() > reach {
                    break;
                }
                text.extend(x);
            }
            let mut touched: Vec<usize> = Vec::new();
            for p in pairs_of(&text) {
                for &li in index.get(&p).map(Vec::as_slice).unwrap_or_default() {
                    if let Some(c) = shared.get_mut(li) {
                        if *c == 0 {
                            touched.push(li);
                        }
                        *c += 1;
                    }
                }
            }
            // in table order, so ties break as when every line is compared
            let mut cands: Vec<usize> =
                touched.iter().copied().filter(|&li| shared.get(li).copied().unwrap_or(0) >= need.get(li).copied().unwrap_or(1)).collect();
            cands.sort_unstable();
            for &li in &touched {
                if let Some(c) = shared.get_mut(li) {
                    *c = 0;
                }
            }
            if cands.is_empty() {
                continue;
            }
            let mut span: Vec<char> = Vec::new();
            for j in i..w.len() {
                if let Some(x) = w.get(j) {
                    span.extend(x);
                }
                if span.len() > reach {
                    break;
                }
                // and end with real characters
                if w.get(j).is_some_and(Vec::is_empty) {
                    continue;
                }
                for &li in &cands {
                    let Some(l) = self.lines.get(li) else { continue };
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
        keep_best(found)
    }
}

/// The best (score, then length) of overlapping matches, in text order.
fn keep_best(mut found: Vec<Match>) -> Vec<Match> {
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

    /// Every line compared at every span: what `find` must agree with.
    fn brute(v: &VoiceLines, words: &[&str]) -> Vec<Match> {
        let w: Vec<Vec<char>> = words.iter().map(|w| norm(w)).collect();
        let mut out = Vec::new();
        for i in 0..w.len() {
            let mut span = Vec::new();
            for j in i..w.len() {
                span.extend(&w[j]);
                if w[i].is_empty() || w[j].is_empty() {
                    continue;
                }
                for (li, l) in v.lines.iter().enumerate() {
                    let (a, b) = (span.len(), l.norm.len());
                    let d = edit_distance(&span, &l.norm);
                    if a.abs_diff(b) <= b / 5 && d <= b / 5 {
                        out.push(Match { start: i, end: j + 1, line: li, score: 1.0 - d as f32 / b as f32, short: b < MIN_SYLLABLES });
                    }
                }
            }
        }
        keep_best(out)
    }

    #[test]
    fn the_pair_filter_misses_nothing() {
        let syl: Vec<char> = "가나다라마바사아자차카타".chars().collect();
        let mut seed = 7u64;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        let mut tsv = String::from("hero\tkind\ttext\n");
        let mut lines = Vec::new();
        for _ in 0..60 {
            let n = 2 + next(12);
            let l: String = (0..n).map(|_| syl[next(syl.len())]).collect();
            tsv.push_str(&format!("h\tk\t{l}\n"));
            lines.push(l);
        }
        let v = VoiceLines::parse(&tsv);
        let mut total = 0;
        for _ in 0..40 {
            // text made of lines with a few edits, cut into words
            let mut text: Vec<char> = Vec::new();
            for _ in 0..4 {
                let mut l: Vec<char> = lines[next(lines.len())].chars().collect();
                for _ in 0..next(3) {
                    let at = next(l.len());
                    match next(3) {
                        0 => l[at] = syl[next(syl.len())],
                        1 => drop(l.remove(at)),
                        _ => l.insert(at, syl[next(syl.len())]),
                    }
                }
                text.extend(l);
            }
            let mut words: Vec<String> = Vec::new();
            let mut k = 0;
            while k < text.len() {
                let n = 1 + next(4);
                words.push(text[k..(k + n).min(text.len())].iter().collect());
                k += n;
            }
            let refs: Vec<&str> = words.iter().map(String::as_str).collect();
            let got = v.find(&refs);
            assert_eq!(got, brute(&v, &refs));
            total += got.len();
        }
        assert!(total > 40, "{total}");
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

//! Game-term accuracy: how many of the reference's terms the hypothesis spells right.
//!
//! A term list is plain UTF-8 text, one term per line: the canonical spelling, then optionally
//! other accepted spellings after `|` (`디바 | D.Va`). Blank lines and lines starting with `#`
//! are ignored. The glossary format of milestone M2 will export this list.
//!
//! Matching runs on normalized text with word breaks removed (Korean spacing varies: `솔저 76`
//! and `솔저76` are the same term) and takes the longest spelling at each position, so
//! `위도우메이커` counts once as itself and not also as `메이`. Scores compare counts per term
//! (bag of terms), not positions: a term the hypothesis says as often as the reference counts as
//! fully recognized wherever it appears.

use std::collections::HashMap;

use serde::Serialize;

use crate::normalize::normalize;

/// The most terms a list may hold.
pub const MAX_TERMS: usize = 20_000;
/// The longest spelling, in characters.
pub const MAX_SPELLING: usize = 64;

/// One term and its accepted spellings (normalized, without word breaks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    /// The canonical spelling as written in the list.
    pub name: String,
    spellings: Vec<Vec<char>>,
}

/// A parsed term list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TermList {
    pub terms: Vec<Term>,
}

/// A problem in a term list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TermError {
    #[error("line {0}: a spelling normalizes to nothing (only punctuation?)")]
    Empty(usize),
    #[error("line {0}: spelling longer than {MAX_SPELLING} characters")]
    TooLong(usize),
    #[error("more than {MAX_TERMS} terms")]
    TooMany,
}

impl TermList {
    /// Parse a term list (see the module docs).
    pub fn parse(text: &str) -> Result<TermList, TermError> {
        let mut terms = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if terms.len() >= MAX_TERMS {
                return Err(TermError::TooMany);
            }
            let line_no = i.saturating_add(1);
            let mut spellings = Vec::new();
            for part in line.split('|') {
                let s: Vec<char> = normalize(part).chars();
                if s.is_empty() {
                    return Err(TermError::Empty(line_no));
                }
                if s.len() > MAX_SPELLING {
                    return Err(TermError::TooLong(line_no));
                }
                spellings.push(s);
            }
            let name = line.split('|').next().unwrap_or(line).trim().to_string();
            terms.push(Term { name, spellings });
        }
        Ok(TermList { terms })
    }

    /// How many times each term occurs in `text` (indexed like [`TermList::terms`]).
    pub fn count(&self, text: &str) -> Vec<usize> {
        let mut counts = vec![0usize; self.terms.len()];
        // Spellings by first character, longest first; equal lengths keep list order, so the
        // first term listed wins.
        let mut by_first: HashMap<char, Vec<(&[char], usize)>> = HashMap::new();
        for (t, term) in self.terms.iter().enumerate() {
            for s in &term.spellings {
                if let Some(&c) = s.first() {
                    by_first.entry(c).or_default().push((s.as_slice(), t));
                }
            }
        }
        for v in by_first.values_mut() {
            v.sort_by_key(|s| std::cmp::Reverse(s.0.len()));
        }
        let hay = normalize(text).chars();
        let mut pos = 0;
        while let Some(c) = hay.get(pos) {
            let rest = hay.get(pos..).unwrap_or_default();
            let found = by_first.get(c).and_then(|v| v.iter().find(|(s, _)| rest.starts_with(s)));
            match found {
                Some((s, t)) => {
                    if let Some(c) = counts.get_mut(*t) {
                        *c = c.saturating_add(1);
                    }
                    pos = pos.saturating_add(s.len().max(1));
                }
                None => pos += 1,
            }
        }
        counts
    }

    /// Score `hyp` against `reference`.
    pub fn score(&self, reference: &str, hyp: &str) -> TermScore {
        let r = self.count(reference);
        let h = self.count(hyp);
        let mut out = TermScore::default();
        for (term, (&rc, &hc)) in self.terms.iter().zip(r.iter().zip(h.iter())) {
            let matched = rc.min(hc);
            out.reference = out.reference.saturating_add(rc);
            out.hypothesis = out.hypothesis.saturating_add(hc);
            out.matched = out.matched.saturating_add(matched);
            if rc != hc {
                out.mismatched.push(TermMiss { term: term.name.clone(), reference: rc, hypothesis: hc });
            }
        }
        out
    }
}

/// A term the hypothesis says a different number of times than the reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TermMiss {
    pub term: String,
    pub reference: usize,
    pub hypothesis: usize,
}

/// Term counts of one comparison.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TermScore {
    /// Term occurrences in the reference.
    pub reference: usize,
    /// Term occurrences in the hypothesis.
    pub hypothesis: usize,
    /// Occurrences found in both (per term, the smaller count).
    pub matched: usize,
    /// Terms whose counts differ, in list order.
    pub mismatched: Vec<TermMiss>,
}

impl TermScore {
    /// Term accuracy (recall): matched ÷ reference occurrences; 1 when the reference has none.
    pub fn accuracy(&self) -> f64 {
        ratio(self.matched, self.reference)
    }
    /// Precision: matched ÷ hypothesis occurrences; 1 when the hypothesis has none.
    pub fn precision(&self) -> f64 {
        ratio(self.matched, self.hypothesis)
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 { 1.0 } else { a as f64 / b as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "# heroes\n위도우메이커\n메이\n디바 | D.Va\n솔저: 76\n\n";

    #[test]
    fn parse_skips_comments_and_keeps_names() {
        let l = TermList::parse(LIST).unwrap();
        let names: Vec<&str> = l.terms.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["위도우메이커", "메이", "디바", "솔저: 76"]);
    }

    #[test]
    fn longest_match_wins() {
        let l = TermList::parse(LIST).unwrap();
        assert_eq!(l.count("위도우메이커가 메이를 잡았다"), [1, 1, 0, 0]);
    }

    #[test]
    fn spacing_punctuation_and_variants() {
        let l = TermList::parse(LIST).unwrap();
        assert_eq!(l.count("솔저76, 솔저 : 76! D.va 디바"), [0, 0, 2, 2]);
    }

    #[test]
    fn score_counts_and_misses() {
        let l = TermList::parse(LIST).unwrap();
        let s = l.score("메이 메이 디바", "메이 데이 디바 디바");
        assert_eq!((s.reference, s.hypothesis, s.matched), (3, 3, 2));
        assert_eq!(s.mismatched.len(), 2);
        assert!((s.accuracy() - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn bad_lists() {
        assert_eq!(TermList::parse("ok\n!!!\n"), Err(TermError::Empty(2)));
        assert_eq!(TermList::parse("a | \n"), Err(TermError::Empty(1)));
        assert_eq!(TermList::parse(&"가".repeat(MAX_SPELLING + 1)), Err(TermError::TooLong(1)));
    }

    #[test]
    fn empty_list_scores_one() {
        let s = TermList::default().score("겐지", "");
        assert_eq!(s.accuracy(), 1.0);
    }
}

//! Transcript correction with a glossary: known misrecognitions become the glossary spelling and
//! profanity becomes its initial consonants.
//!
//! - Every `misheard` form of an entry is replaced by its `ko`. A misrecognition is matched only at
//!   the start of a word (start of text, or after a character that is not a letter or digit), so
//!   Korean particles may follow (`로도그가` → `로드호그가`) but a form inside another word is left
//!   alone.
//! - For entries flagged `mask` (profanity) the forms are replaced anywhere, also inside a word
//!   (`씨발아` → `ㅅㅂ아`).
//! - At each position the longest form wins (`개씨발` → `ㄱㅅㅂ` before `씨발` → `ㅅㅂ`), so overlapping
//!   entries behave predictably. Replacements are not rescanned.
//! - With [`Corrector::fuzzy`], a word that sounds almost like a name is corrected too: spellings
//!   of three syllables or more from the name tables (heroes, abilities, maps, modes, teams,
//!   players) are compared jamo by jamo, silent `ㅇ` ignored, so `제냐타` = `젠야타` and
//!   `둔피스트` (one jamo off) → `둠피스트` (a final consonant counts as the same consonant
//!   starting the next syllable). Three- and four-syllable names allow one jamo edit,
//!   longer ones two. A word that already is a spelling in any table is never changed (common
//!   words that sound like a name, such as `나이스` for `나이프`, are listed in `slang.tsv` for that), and a difference
//!   only in a last syllable that is a particle (`자리에` vs `자리야`) is not a match.

use std::collections::HashMap;

use crate::Glossary;

#[derive(Clone, Debug)]
struct Rule {
    from: Vec<char>,
    to: String,
    anywhere: bool,
}

/// Tables whose spellings the fuzzy matcher corrects toward.
const FUZZY_CATEGORIES: [&str; 6] = ["heroes", "abilities", "maps", "modes", "teams", "players"];
/// Particles that may end a word in place of a name's last syllable.
const PARTICLES: [char; 12] = ['에', '의', '가', '이', '은', '는', '을', '를', '도', '로', '와', '과'];

#[derive(Clone, Debug)]
struct Name {
    spelling: String,
    syllables: usize,
    jamo: Vec<u16>,
}

/// Replacement rules built from a glossary (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct Corrector {
    /// Rules by first character, longest `from` first.
    by_first: HashMap<char, Vec<Rule>>,
    /// Names for fuzzy matching; empty unless [`Corrector::fuzzy`] was called.
    names: Vec<Name>,
    /// Every Hangul spelling in the name tables, which fuzzy matching leaves alone.
    known: std::collections::HashSet<String>,
}

fn is_syllable(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

/// Jamo of Hangul syllables as small codes (initial, medial, final); a silent initial `ㅇ` is
/// dropped. `None` if a character is not a syllable.
fn jamo(chars: &[char]) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(chars.len().saturating_mul(3));
    for &c in chars {
        if !is_syllable(c) {
            return None;
        }
        let s = u32::from(c) - 0xAC00;
        let (initial, medial, fin) = (s / (21 * 28), (s / 28) % 21, s % 28);
        if initial != 11 {
            out.push(u16::try_from(initial).ok()?);
        }
        out.push(u16::try_from(100 + medial).ok()?);
        // A final consonant compares equal to the same consonant as the next initial (젠야 = 제냐).
        let as_initial = match fin {
            0 => None,
            1 => Some(0),
            2 => Some(1),
            4 => Some(2),
            7 => Some(3),
            8 => Some(5),
            16 => Some(6),
            17 => Some(7),
            19 => Some(9),
            20 => Some(10),
            22 => Some(12),
            23 => Some(14),
            24 => Some(15),
            25 => Some(16),
            26 => Some(17),
            27 => Some(18),
            f => Some(200 + f),
        };
        if let Some(code) = as_initial {
            out.push(u16::try_from(code).ok()?);
        }
    }
    Some(out)
}

/// Levenshtein distance of two short sequences.
fn distance(a: &[u16], b: &[u16]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        if let Some(c0) = cur.first_mut() {
            *c0 = i + 1;
        }
        for (j, y) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(usize::MAX).saturating_add(usize::from(x != y));
            let del = prev.get(j + 1).copied().unwrap_or(usize::MAX).saturating_add(1);
            let ins = cur.get(j).copied().unwrap_or(usize::MAX).saturating_add(1);
            if let Some(c) = cur.get_mut(j + 1) {
                *c = sub.min(del).min(ins);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.last().copied().unwrap_or(0)
}

/// What [`Corrector::apply`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Correction {
    pub text: String,
    /// Replacements made.
    pub replaced: usize,
    /// Each replacement, `(from, to)`, in order.
    pub changes: Vec<(String, String)>,
}

impl Corrector {
    /// Build the rules of `glossary`. Forms equal to their replacement are skipped.
    pub fn new(glossary: &Glossary) -> Corrector {
        let mut by_first: HashMap<char, Vec<Rule>> = HashMap::new();
        for e in glossary.entries.iter().filter(|e| !e.ko.is_empty()) {
            let anywhere = e.has_flag("mask");
            for m in e.misheard.iter().filter(|m| **m != e.ko) {
                let from: Vec<char> = m.chars().collect();
                if let Some(&c) = from.first() {
                    by_first.entry(c).or_default().push(Rule { from, to: e.ko.clone(), anywhere });
                }
            }
        }
        for rules in by_first.values_mut() {
            rules.sort_by_key(|r| std::cmp::Reverse(r.from.len()));
        }
        Corrector { by_first, ..Corrector::default() }
    }

    /// Also correct near-misses of names (see the module docs).
    pub fn fuzzy(mut self, glossary: &Glossary) -> Corrector {
        for e in &glossary.entries {
            let name_table = FUZZY_CATEGORIES.contains(&e.category.as_str());
            for s in std::iter::once(&e.ko).chain(&e.aliases).chain(&e.old) {
                let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
                let Some(j) = jamo(&chars) else { continue };
                // Every spelling of every table is left alone, and so is each word of a multi-word
                // spelling (`라이프` of `라이프 위버`).
                self.known.insert(chars.iter().collect());
                if s.contains(' ') {
                    self.known.extend(s.split_whitespace().map(String::from));
                }
                if name_table && chars.len() >= 3 && !e.has_flag("noeval") && !e.has_flag("verify") {
                    self.names.push(Name { spelling: s.clone(), syllables: chars.len(), jamo: j });
                }
            }
        }
        self
    }

    /// The best fuzzy match for the word starting at `rest`: (characters consumed, spelling).
    fn fuzzy_match(&self, rest: &[char]) -> Option<(usize, &str)> {
        let word = rest.iter().take_while(|c| is_syllable(**c)).count();
        // A known spelling at this position (with or without particles) is left alone.
        if (1..=word).any(|n| self.known.contains(&rest.iter().take(n).collect::<String>())) {
            return None;
        }
        let mut best: Option<(usize, usize, &Name)> = None;
        for name in &self.names {
            let allowed = if name.syllables >= 5 { 2 } else { 1 };
            // Shorter windows only for long names: `자리` is a word, not a clipped `자리야`.
            let shortest = if name.syllables >= 4 { name.syllables - 1 } else { name.syllables };
            for n in shortest..=name.syllables + 1 {
                let Some(window) = rest.get(..n).filter(|_| n >= 2 && n <= word) else { continue };
                let Some(j) = jamo(window) else { continue };
                let d = distance(&j, &name.jamo);
                if d > allowed {
                    continue;
                }
                // `자리에` is 자리 + particle, not a misheard 자리야.
                let same_head =
                    window.len() == name.syllables && name.spelling.chars().filter(|c| !c.is_whitespace()).take(n - 1).eq(window.iter().take(n - 1).copied());
                if same_head && window.last().is_some_and(|c| PARTICLES.contains(c)) {
                    continue;
                }
                if best.is_none_or(|(bd, bn, _)| d < bd || (d == bd && n.abs_diff(name.syllables) < bn.abs_diff(name.syllables))) {
                    best = Some((d, n, name));
                }
            }
        }
        best.map(|(_, n, name)| (n, name.spelling.as_str()))
    }

    /// Whether there is nothing to replace.
    pub fn is_empty(&self) -> bool {
        self.by_first.is_empty() && self.names.is_empty()
    }

    /// Correct `text`.
    pub fn apply(&self, text: &str) -> Correction {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Correction { text: String::with_capacity(text.len()), ..Correction::default() };
        let mut pos = 0;
        while let Some(&c) = chars.get(pos) {
            let word_start = pos == 0 || chars.get(pos - 1).is_none_or(|p| !p.is_alphanumeric());
            let rest = chars.get(pos..).unwrap_or_default();
            let rule = self.by_first.get(&c).and_then(|rules| rules.iter().find(|r| (r.anywhere || word_start) && rest.starts_with(&r.from)));
            let hit = rule.map(|r| (r.from.len(), r.to.as_str())).or_else(|| if word_start && !self.names.is_empty() { self.fuzzy_match(rest) } else { None });
            match hit {
                Some((len, to)) => {
                    out.text.push_str(to);
                    out.replaced = out.replaced.saturating_add(1);
                    out.changes.push((rest.iter().take(len).collect(), to.to_string()));
                    pos = pos.saturating_add(len.max(1));
                }
                None => {
                    out.text.push(c);
                    pos += 1;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corrector(rows: &str) -> Corrector {
        let mut g = Glossary::default();
        g.add_table("t", &format!("id\tko\ten\taliases\told\tmisheard\tflags\tnote\n{rows}")).unwrap();
        Corrector::new(&g)
    }

    #[test]
    fn misheard_at_word_start_with_particles() {
        let c = corrector("roadhog\t로드호그\t\t\t\t로도그; 로도 그\n");
        let r = c.apply("로도그가 갈고리, 로도 그 궁. 빨로도그");
        assert_eq!(r.text, "로드호그가 갈고리, 로드호그 궁. 빨로도그");
        assert_eq!(r.replaced, 2);
    }

    #[test]
    fn profanity_masked_anywhere_longest_first() {
        let c = corrector("s\tㅅㅂ\t\t\t\t씨발; 시발\tmask\ngs\tㄱㅅㅂ\t\t\t\t개씨발\tmask\n");
        assert_eq!(c.apply("아씨발 개씨발 시발아").text, "아ㅅㅂ ㄱㅅㅂ ㅅㅂ아");
    }

    fn fuzzy(rows: &str) -> Corrector {
        let mut g = Glossary::default();
        g.add_table("heroes", &format!("id\tko\ten\taliases\told\tmisheard\tflags\tnote\n{rows}")).unwrap();
        Corrector::new(&g).fuzzy(&g)
    }

    #[test]
    fn fuzzy_fixes_near_misses_only() {
        let c = fuzzy("lw\t라이프 위버\nknife\t나이프\nzen\t젠야타\nzarya\t자리야\ndoom\t둠피스트\ntorb\t토르비욘\nmercy\t메르시\n");
        assert_eq!(c.apply("제냐타 둔피스트를 토르비온입니다 자리아처럼").text, "젠야타 둠피스트를 토르비욘입니다 자리야처럼");
        // Ordinary words, particles and correct names stay.
        assert_eq!(c.apply("자리에 앉아 자리가 젠야타가 메르시").text, "자리에 앉아 자리가 젠야타가 메르시");
        assert_eq!(c.apply("라이프 위버").text, "라이프 위버");
        assert_eq!(c.apply("가나다라마바").replaced, 0);
    }

    #[test]
    fn jamo_and_distance() {
        assert_eq!(jamo(&['젠', '야']), jamo(&['제', '냐']));
        assert_ne!(jamo(&['강']), jamo(&['가']));
        assert_eq!(jamo(&['a']), None);
        assert_eq!(distance(&[1, 2, 3], &[1, 3]), 1);
        assert_eq!(distance(&[], &[1, 2]), 2);
    }

    #[test]
    fn nothing_to_do() {
        let c = Corrector::new(&Glossary::default());
        assert!(c.is_empty());
        assert_eq!(c.apply("겐지 용검").text, "겐지 용검");
        assert_eq!(c.apply("").replaced, 0);
    }
}

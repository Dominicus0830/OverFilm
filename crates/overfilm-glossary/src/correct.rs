//! Transcript correction with a glossary: known misrecognitions become the glossary spelling and
//! profanity becomes its initial consonants.
//!
//! - Every `misheard` form of an entry is replaced by its `ko`. A misrecognition is matched only at
//!   the start of a word (start of text, or after a character that is not a letter or digit), so
//!   Korean particles may follow (`로도그가` → `로드호그가`) but a form inside another word is left
//!   alone.
//! - A form ending in `@픽` is a sound-alike that is also an ordinary word (`캐슬`), so it is replaced
//!   only when a word about picking a hero follows (`캐슬 쓰게 되면` → `캣 쓰게 되면`, `캐슬을 픽하면`;
//!   one particle may come between), and `캐슬` elsewhere is left alone.
//! - A form ending in `@영웅` is a sound-alike of a hero nickname that is also an ordinary word
//!   (`양이` for `냥이`): it is replaced only next to a hero name, the word before or after it
//!   (`솜브라와 양이 캐서디 연계` → `냥이`), so `양이 많다` stays.
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
//! - With [`Corrector::apply_preferring`], names known to be in play (the teams and players on
//!   screen, the heroes being played; see [`Preferred`]) win a tie with other names and three
//!   syllables or more allow one more jamo edit (`홈런보` → `홈런볼` would anyway; `우텐` is two
//!   syllables and still needs `선수` after it). It is a soft preference: every other name is
//!   corrected exactly as without it, since a hero nobody plays can still be talked about.

use std::collections::HashMap;

use crate::Glossary;

#[derive(Clone, Debug)]
struct Rule {
    from: Vec<char>,
    to: String,
    anywhere: bool,
    /// Only before a hero-pick word (`@픽` forms).
    pick: bool,
    /// Only next to a hero name (`@영웅` forms).
    hero: bool,
}

/// Marks a `misheard` form that is replaced only next to a hero name.
const HERO_MARK: &str = "@영웅";
/// Particles a hero name may carry before the word it is next to (`솜브라와 냥이`).
const HERO_PARTICLES: [&str; 7] = ["이랑", "하고", "와", "과", "랑", "에", "의"];

/// Whether the word after `rest`'s first `len` characters, or the word before `before`, is a hero
/// name (with a particle).
fn next_to_hero(before: &[char], rest: &[char], len: usize, heroes: &[Vec<char>]) -> bool {
    let is_hero = |w: &[char]| {
        heroes.iter().any(|h| {
            w.starts_with(h)
                && w.get(h.len()..).is_some_and(|tail| tail.is_empty() || HERO_PARTICLES.iter().any(|p| tail.iter().copied().eq(p.chars())) || tail.len() == 1)
        })
    };
    // the next word, spaced or run together (`양이캐서디`)
    let after = rest.get(len..).unwrap_or_default();
    let next: Vec<char> = after.iter().copied().skip_while(|c| c.is_whitespace()).take_while(|c| c.is_alphanumeric()).collect();
    if is_hero(&next) {
        return true;
    }
    // the previous word
    let prev: Vec<char> = {
        let mut w: Vec<char> = before.iter().rev().skip_while(|c| c.is_whitespace()).take_while(|c| c.is_alphanumeric()).copied().collect();
        w.reverse();
        w
    };
    before.last().is_some_and(|c| c.is_whitespace()) && is_hero(&prev)
}

/// Marks a `misheard` form that is replaced only before a hero-pick word.
const PICK_MARK: &str = "@픽";
/// Starts of the words that follow a hero being picked or played (`쓰게`, `픽하면`, `꺼냈`).
const PICK_WORDS: [&str; 16] = ["쓰", "써", "썼", "쓸", "픽", "고르", "골라", "골랐", "꺼내", "꺼냈", "잡", "하면", "했", "플레이", "나오", "나왔"];
/// Particles that may come between the hero and the pick word (`캣을 쓰면`).
const PICK_PARTICLES: [char; 9] = ['을', '를', '이', '가', '도', '만', '은', '는', '랑'];

/// Whether `rest` (just after a name) is followed by a hero-pick word.
fn before_pick(rest: &[char]) -> bool {
    let mut i = usize::from(rest.first().is_some_and(|c| PICK_PARTICLES.contains(c)));
    while rest.get(i).is_some_and(|c| c.is_whitespace()) {
        i += 1;
    }
    let word: String = rest.iter().skip(i).take(4).collect();
    PICK_WORDS.iter().any(|w| word.starts_with(w))
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
    /// A player's name: matched more loosely right before `선수`, `감독`, `코치` (see below).
    person: bool,
    /// [`jamo_with`] loose, for that looser match.
    loose: Vec<u16>,
    /// In play (see [`Preferred`]).
    preferred: bool,
}

impl Name {
    fn new(spelling: &str, person: bool, preferred: bool) -> Option<Name> {
        let chars: Vec<char> = spelling.chars().filter(|c| !c.is_whitespace()).collect();
        let jamo = jamo(&chars)?;
        Some(Name { spelling: spelling.to_string(), syllables: chars.len(), jamo, person, loose: jamo_with(&chars, true).unwrap_or_default(), preferred })
    }
}

/// Names known to be in play, preferred by [`Corrector::apply_preferring`]: (spelling, is a
/// person). Spellings of fewer than two syllables or with letters other than Hangul are left out.
#[derive(Clone, Debug, Default)]
pub struct Preferred {
    names: Vec<Name>,
}

impl Preferred {
    pub fn new<'a>(names: impl IntoIterator<Item = (&'a str, bool)>) -> Preferred {
        let mut out: Vec<Name> = Vec::new();
        for (s, person) in names {
            if out.iter().any(|n| n.spelling == s) {
                continue;
            }
            if let Some(n) = Name::new(s, person, true).filter(|n| n.syllables >= 2) {
                out.push(n);
            }
        }
        Preferred { names: out }
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Words after a name that say it is a person (`베키 선수`, `러쉬 감독님`).
const TITLES: [&str; 4] = ["선수", "감독", "코치", "단장"];

/// Whether a title follows the word of `word` syllables at the start of `rest`.
fn before_title(rest: &[char], word: usize) -> bool {
    let after: String = rest.iter().skip(word).skip_while(|c| c.is_whitespace()).take(2).collect();
    TITLES.iter().any(|t| after.starts_with(t))
}

/// Replacement rules built from a glossary (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct Corrector {
    /// Rules by first character, longest `from` first.
    by_first: HashMap<char, Vec<Rule>>,
    /// Names for fuzzy matching; empty unless [`Corrector::fuzzy`] was called.
    names: Vec<Name>,
    /// Hero names and nicknames without spaces, for `@영웅` forms.
    heroes: Vec<Vec<char>>,
    /// Every Hangul spelling in the name tables, which fuzzy matching leaves alone.
    known: std::collections::HashSet<String>,
    /// Words of multi-word spellings (`제트팩` of `제트팩 캣`): left alone only as a whole word
    /// (with a particle), so `제트팩켓` can still become `제트팩 캣`.
    parts: std::collections::HashSet<String>,
    /// Also apply [`crate::ko::tidy`] (Korean spelling and spacing).
    korean: bool,
    /// Also apply [`crate::digits::write_digits`] (spoken numbers before counters as digits).
    digits: bool,
}

fn is_syllable(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

/// Jamo of Hangul syllables as small codes (initial, medial, final); a silent initial `ㅇ` is
/// dropped. `None` if a character is not a syllable.
fn jamo(chars: &[char]) -> Option<Vec<u16>> {
    jamo_with(chars, false)
}

/// [`jamo`], or with `loose` closer to how names sound when said quickly: a final consonant
/// followed by the same initial counts once (`백기` ≈ `배기`) and `ㅔ`/`ㅐ` (`ㅖ`/`ㅒ`, `ㅞ`/`ㅙ`/`ㅚ`)
/// are one vowel. Only used for player names right before a title, where a name is expected; with
/// it everywhere, `골라라` would be one edit from `고릴라`.
fn jamo_with(chars: &[char], loose: bool) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(chars.len().saturating_mul(3));
    for &c in chars {
        if !is_syllable(c) {
            return None;
        }
        let s = u32::from(c) - 0xAC00;
        let (initial, medial, fin) = (s / (21 * 28), (s / 28) % 21, s % 28);
        if initial != 11 && !(loose && out.last() == Some(&u16::try_from(initial).ok()?)) {
            out.push(u16::try_from(initial).ok()?);
        }
        let medial = match medial {
            5 if loose => 1,
            7 if loose => 3,
            15 | 11 if loose => 10,
            m => m,
        };
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
            let anywhere = e.has_flag("mask") || e.has_flag("inword");
            for m in e.misheard.iter().filter(|m| **m != e.ko) {
                let (form, pick, hero) = match (m.strip_suffix(PICK_MARK), m.strip_suffix(HERO_MARK)) {
                    (Some(f), _) => (f.trim(), true, false),
                    (_, Some(f)) => (f.trim(), false, true),
                    _ => (m.as_str(), false, false),
                };
                let from: Vec<char> = form.chars().collect();
                if let Some(&c) = from.first() {
                    by_first.entry(c).or_default().push(Rule { from, to: e.ko.clone(), anywhere, pick, hero });
                }
            }
        }
        let mut heroes: Vec<Vec<char>> = glossary
            .entries
            .iter()
            .filter(|e| e.category == "heroes")
            .flat_map(|e| std::iter::once(&e.ko).chain(&e.aliases))
            .map(|s| s.chars().filter(|c| !c.is_whitespace()).collect::<Vec<char>>())
            .filter(|h| h.len() >= 2)
            .collect();
        heroes.sort_by_key(|h| std::cmp::Reverse(h.len()));
        for rules in by_first.values_mut() {
            rules.sort_by_key(|r| std::cmp::Reverse(r.from.len()));
        }
        Corrector { by_first, heroes, ..Corrector::default() }
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
                    self.parts.extend(s.split_whitespace().filter(|p| !self.known.contains(*p)).map(String::from));
                }
                let person = e.category == "players";
                if name_table && (chars.len() >= 3 || (person && chars.len() == 2)) && !e.has_flag("noeval") && !e.has_flag("verify") {
                    let loose = jamo_with(&chars, true).unwrap_or_default();
                    self.names.push(Name { spelling: s.clone(), syllables: chars.len(), jamo: j, person, loose, preferred: false });
                }
            }
        }
        self
    }

    /// The best fuzzy match for the word starting at `rest`: (characters consumed, spelling).
    fn fuzzy_match<'a>(&'a self, rest: &[char], preferred: &'a Preferred) -> Option<(usize, &'a str)> {
        let word = rest.iter().take_while(|c| is_syllable(**c)).count();
        // A known spelling at this position (with or without particles) is left alone.
        if (1..=word).any(|n| self.known.contains(&rest.iter().take(n).collect::<String>())) {
            return None;
        }
        let part_alone = |n: usize| n == word || (n + 1 == word && rest.get(n).is_some_and(|c| PARTICLES.contains(c)));
        if (1..=word).any(|n| part_alone(n) && self.parts.contains(&rest.iter().take(n).collect::<String>())) {
            return None;
        }
        let titled = before_title(rest, word);
        let mut best: Option<(usize, usize, &Name)> = None;
        for name in self.names.iter().chain(&preferred.names) {
            // Two-syllable player names only right before a title (`백기 선수` → `베키 선수`), where a
            // title also allows one more edit for longer names (`트라우드 선수`).
            if name.syllables < 3 && !(titled && name.person) {
                continue;
            }
            let base = if name.syllables >= 5 { 2 } else { 1 };
            let allowed = if titled && name.person && name.syllables >= 3 { base + 1 } else { base };
            // Shorter windows only for long names: `자리` is a word, not a clipped `자리야`.
            let shortest = if name.syllables >= 4 { name.syllables - 1 } else { name.syllables };
            for n in shortest..=name.syllables + 1 {
                let Some(window) = rest.get(..n).filter(|_| n >= 2 && n <= word) else { continue };
                let loose = titled && name.person;
                let Some(j) = jamo_with(window, loose) else { continue };
                let d = distance(&j, if loose { &name.loose } else { &name.jamo });
                // a name in play allows one more edit, but only over its own length (`그러니까요`
                // is no clipped `아이러니`)
                let allowed = if name.preferred && !titled && name.syllables >= 3 && n == name.syllables { allowed + 1 } else { allowed };
                if d > allowed {
                    continue;
                }
                // `자리에` is 자리 + particle, not a misheard 자리야.
                let same_head =
                    window.len() == name.syllables && name.spelling.chars().filter(|c| !c.is_whitespace()).take(n - 1).eq(window.iter().take(n - 1).copied());
                if same_head && window.last().is_some_and(|c| PARTICLES.contains(c)) {
                    continue;
                }
                let better = |(bd, bn, b): (usize, usize, &Name)| {
                    let (a, z) = (n.abs_diff(name.syllables), bn.abs_diff(name.syllables));
                    d < bd || (d == bd && (a < z || (a == z && name.preferred && !b.preferred)))
                };
                if best.is_none_or(better) {
                    best = Some((d, n, name));
                }
            }
        }
        best.map(|(_, n, name)| (n, name.spelling.as_str()))
    }

    /// Also fix Korean spelling and spacing slips after the glossary rules (see [`crate::ko`]).
    pub fn korean(mut self) -> Corrector {
        self.korean = true;
        self
    }

    /// Also write spoken numbers before counters as digits (`오 시즌` → `5시즌`, see
    /// [`crate::digits`]).
    pub fn digits(mut self, on: bool) -> Corrector {
        self.digits = on;
        self
    }

    /// Whether there is nothing to replace.
    pub fn is_empty(&self) -> bool {
        self.by_first.is_empty() && self.names.is_empty() && !self.korean && !self.digits
    }

    /// Correct `text`.
    pub fn apply(&self, text: &str) -> Correction {
        self.apply_preferring(text, &Preferred::default())
    }

    /// Correct `text`, preferring the names in play (see the module docs).
    pub fn apply_preferring(&self, text: &str, preferred: &Preferred) -> Correction {
        let mut out = self.apply_glossary(text, preferred);
        if self.korean {
            let t = crate::ko::tidy(&out.text);
            out.replaced = out.replaced.saturating_add(t.changes.len());
            out.changes.extend(t.changes);
            out.text = t.text;
        }
        if self.digits {
            let t = crate::digits::write_digits(&out.text);
            out.replaced = out.replaced.saturating_add(t.changes.len());
            out.changes.extend(t.changes);
            out.text = t.text;
        }
        out
    }

    fn apply_glossary(&self, text: &str, preferred: &Preferred) -> Correction {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Correction { text: String::with_capacity(text.len()), ..Correction::default() };
        let mut pos = 0;
        while let Some(&c) = chars.get(pos) {
            let word_start = pos == 0 || chars.get(pos - 1).is_none_or(|p| !p.is_alphanumeric());
            let rest = chars.get(pos..).unwrap_or_default();
            let rule = self.by_first.get(&c).and_then(|rules| {
                rules.iter().find(|r| {
                    (r.anywhere || word_start)
                        && rest.starts_with(&r.from)
                        && (!r.pick || before_pick(rest.get(r.from.len()..).unwrap_or_default()))
                        && (!r.hero || next_to_hero(chars.get(..pos).unwrap_or_default(), rest, r.from.len(), &self.heroes))
                })
            });
            let hit = rule
                .map(|r| (r.from.len(), r.to.as_str()))
                .or_else(|| if word_start && !(self.names.is_empty() && preferred.is_empty()) { self.fuzzy_match(rest, preferred) } else { None });
            let hit = hit.filter(|(len, to)| rest.iter().take(*len).copied().ne(to.chars()));
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
    fn hero_forms_only_next_to_a_hero() {
        let mut g = Glossary::default();
        g.add_table("heroes", "id\tko\ten\taliases\told\tmisheard\tflags\tnote\ncassidy\t캐서디\t\t\t\t\t\t\nsombra\t솜브라\t\t\t\t\t\t\n").unwrap();
        g.add_table("spelling", "id\tko\ten\taliases\told\tmisheard\tflags\tnote\nnyangi\t냥이\t\t\t\t양이@영웅\tnoeval\t\n").unwrap();
        let c = Corrector::new(&g);
        // aichi tier list: "솜브라와 냥이 캐서디 연계"
        assert_eq!(c.apply("솜브라와 양이 캐서디 연계").text, "솜브라와 냥이 캐서디 연계");
        assert_eq!(c.apply("양이 캐서디랑").text, "냥이 캐서디랑");
        // the recogniser ran them together in that video
        assert_eq!(c.apply("양이캐서디 연계가").text, "냥이캐서디 연계가");
        assert_eq!(c.apply("솜브라랑 양이가").text, "솜브라랑 냥이가");
        for keep in ["양이 많다", "음식 양이 적어", "양이", "캐서디양이"] {
            assert_eq!(c.apply(keep).text, keep, "{keep}");
        }
    }

    #[test]
    fn pick_forms_only_before_pick_words() {
        let c = corrector("cat\t캣\t\t\t\t캐슬@픽; 캐스 @픽\ndmon\t디몬\t\t\t\t딜몬\n");
        assert_eq!(c.apply("보통 뭐 캐슬 쓰게 되면은 딜몬도 좀 쓰고").text, "보통 뭐 캣 쓰게 되면은 디몬도 좀 쓰고");
        assert_eq!(c.apply("캐슬을 픽하면 캐스 꺼냈다").text, "캣을 픽하면 캣 꺼냈다");
        assert_eq!(c.apply("모래 캐슬 만들기, 캐슬이 무너졌다").text, "모래 캐슬 만들기, 캐슬이 무너졌다");
        assert_eq!(c.apply("캐슬").text, "캐슬");
    }

    #[test]
    fn titles_loosen_player_names() {
        let mut g = Glossary::default();
        g.add_table("players", "id\tko\ten\taliases\told\tmisheard\tflags\tnote\nbecky\t베키\nskewed\t스큐드\nproud\t프라우드\n").unwrap();
        let c = Corrector::new(&g).fuzzy(&g);
        assert_eq!(c.apply("백기 선수의 벤데타 트라우스 선수").text, "베키 선수의 벤데타 프라우드 선수");
        // Without a title, short and far names are left alone, and so are words a sound-alike edit
        // away only when said quickly (`골라라` / `고릴라`).
        assert_eq!(c.apply("백기를 들었다 트라우스 클라우드").text, "백기를 들었다 트라우스 클라우드");
        let mut h = Glossary::default();
        h.add_table("heroes", "id\tko\ten\taliases\told\tmisheard\tflags\tnote\nwinston\t윈스턴\t\t고릴라\n").unwrap();
        assert_eq!(Corrector::new(&h).fuzzy(&h).apply("아무를 골라라").text, "아무를 골라라");
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
        assert_eq!(c.apply("라이프가 좋다").text, "라이프가 좋다");
        let c = fuzzy("jc\t제트팩 캣\nlw\t라이프 위버\n");
        assert_eq!(c.apply("제트팩켓 제트팩켓이").text, "제트팩 캣 제트팩 캣이");
        assert_eq!(c.apply("제트팩 캣 제트팩이 라이프 위버").replaced, 0);
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

#[cfg(test)]
mod preferred_tests {
    use super::*;

    fn glossary() -> Glossary {
        let mut g = Glossary::default();
        g.add_table("heroes", "id\tko\ten\taliases\told\tmisheard\tflags\tnote\ntracer\t트레이서\nmoira\t모이라\nmauga\t마우가\n").unwrap();
        g.add_table("players", "id\tko\ten\taliases\told\tmisheard\tflags\tnote\nhomerunball\t홈런볼\nwutian\t우톈\t\t\t\t\tverify\nproud\t프라우드\n")
            .unwrap();
        g.add_table("teams", "id\tko\ten\taliases\told\tmisheard\tflags\tnote\no2\t오투 블래스트\n").unwrap();
        g
    }

    #[test]
    fn names_in_play_win_ties_and_allow_one_more_edit() {
        let g = glossary();
        let c = Corrector::new(&g).fuzzy(&g);
        // two edits from 홈런볼: left alone without knowing who plays
        assert_eq!(c.apply("홈넌보 들어가").text, "홈넌보 들어가");
        let p = Preferred::new([("홈런볼", true), ("우톈", true), ("트레이서", false)]);
        assert_eq!(c.apply_preferring("홈넌보 들어가", &p).text, "홈런볼 들어가");
        // the commentary's own words stay: a clipped or stretched window gets no extra edit
        assert_eq!(c.apply_preferring("그러니까요 그러니 아이러니까", &Preferred::new([("아이러니", true)])).text, "그러니까요 그러니 아이러니까");
        // a two-syllable player still needs a title, even in play (and even flagged `verify`)
        assert_eq!(c.apply_preferring("우텐이 잘하네", &p).text, "우텐이 잘하네");
        assert_eq!(c.apply_preferring("우텐 선수", &p).text, "우톈 선수");
        // a hero nobody plays is corrected as usual (soft prior), and known words stay
        assert_eq!(c.apply_preferring("모이아로 바꿔야", &p).text, "모이라로 바꿔야");
        assert_eq!(c.apply_preferring("마우가 트레이서", &p).text, "마우가 트레이서");
    }

    #[test]
    fn preferred_skips_what_it_cannot_spell() {
        let p = Preferred::new([("A1IEN", true), ("감", true), ("", false), ("에일리언", true), ("에일리언", true)]);
        assert_eq!(p.names.len(), 1);
        assert!(Preferred::default().is_empty());
    }
}

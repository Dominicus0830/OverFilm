//! OverFilm glossary and language packs (milestone M2).
//!
//! A glossary is a set of TSV tables, one per category (`heroes.tsv`, `abilities.tsv`,
//! `players.tsv`…), as in the Overwatch Korean pack in `packs/overwatch/ko/glossary/`. Every
//! table has the same columns, named on its first line:
//!
//! | column | content |
//! |---|---|
//! | `id` | stable id shared by every language (`cassidy`, `ana.sleep_dart`) |
//! | `ko` | the spelling captions should use |
//! | `en` | English name (for players and teams, the handle as written) |
//! | `aliases` | other accepted spellings and common short forms, `;`-separated |
//! | `old` | former names still heard (`맥크리` for `캐서디`), `;`-separated |
//! | `misheard` | known recognizer errors to correct into `ko`, `;`-separated |
//! | `flags` | space-separated: `noeval` (too generic to score), `verify` (unconfirmed spelling), `mask` (profanity: captions write `ko`, its initial consonants, for every `misheard` form) |
//! | `note` | free text |
//!
//! Blank lines and lines starting with `#` are ignored; missing trailing columns are empty.
//! [`Glossary::term_list`] exports the term list `overfilm-eval` scores with.
//!
//! [`Corrector`] applies a glossary to a transcript (misrecognitions, profanity masking). Still
//! planned: the `transcript.applyGlossary` command and jamo similarity matching. See `OVERFILM.md`.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod correct;

pub use correct::{Correction, Corrector};

/// The columns every table has, in order.
pub const COLUMNS: [&str; 8] = ["id", "ko", "en", "aliases", "old", "misheard", "flags", "note"];
/// The most entries a glossary may hold.
pub const MAX_ENTRIES: usize = 50_000;
/// The longest field, in characters.
pub const MAX_FIELD: usize = 1_000;

/// One glossary entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// The table it came from (`heroes`, `players`…).
    pub category: String,
    pub id: String,
    pub ko: String,
    pub en: String,
    pub aliases: Vec<String>,
    pub old: Vec<String>,
    pub misheard: Vec<String>,
    pub flags: Vec<String>,
    pub note: String,
}

impl Entry {
    /// Whether `flag` is set.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }
}

/// A problem in a glossary table.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlossaryError {
    #[error("{category}: the first line must be the header `{}`", COLUMNS.join("\\t"))]
    Header { category: String },
    #[error("{category} line {line}: more than {} columns", COLUMNS.len())]
    TooManyColumns { category: String, line: usize },
    #[error("{category} line {line}: empty id")]
    NoId { category: String, line: usize },
    #[error("{category} line {line}: duplicate id `{id}`")]
    DuplicateId { category: String, line: usize, id: String },
    #[error("{category} line {line}: a spelling contains `|`")]
    Pipe { category: String, line: usize },
    #[error("{category} line {line}: field longer than {MAX_FIELD} characters")]
    TooLong { category: String, line: usize },
    #[error("more than {MAX_ENTRIES} entries")]
    TooMany,
}

/// A glossary: entries from one or more tables, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Glossary {
    pub entries: Vec<Entry>,
}

fn list(field: &str) -> Vec<String> {
    field.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

impl Glossary {
    /// Add the table `text` (TSV, see the module docs) under `category`. Ids must be unique
    /// within a category. On error nothing is added.
    pub fn add_table(&mut self, category: &str, text: &str) -> Result<(), GlossaryError> {
        let mut lines = text.lines().enumerate().map(|(i, l)| (i.saturating_add(1), l.trim_start_matches('\u{feff}')));
        let mut lines = lines.by_ref().filter(|(_, l)| !l.trim().is_empty() && !l.starts_with('#'));
        let header_ok = lines.next().is_some_and(|(_, h)| h.trim_end().split('\t').map(str::trim).eq(COLUMNS));
        if !header_ok {
            return Err(GlossaryError::Header { category: category.into() });
        }
        let err_at = |line| (category.to_string(), line);
        let mut added: Vec<Entry> = Vec::new();
        for (line, l) in lines {
            if self.entries.len().saturating_add(added.len()) >= MAX_ENTRIES {
                return Err(GlossaryError::TooMany);
            }
            let fields: Vec<&str> = l.split('\t').map(str::trim).collect();
            if fields.len() > COLUMNS.len() {
                let (category, line) = err_at(line);
                return Err(GlossaryError::TooManyColumns { category, line });
            }
            if fields.iter().any(|f| f.chars().count() > MAX_FIELD) {
                let (category, line) = err_at(line);
                return Err(GlossaryError::TooLong { category, line });
            }
            let field = |i: usize| fields.get(i).copied().unwrap_or_default();
            let id = field(0);
            if id.is_empty() {
                let (category, line) = err_at(line);
                return Err(GlossaryError::NoId { category, line });
            }
            if added.iter().any(|e| e.id == id) || self.entries.iter().any(|e| e.category == category && e.id == id) {
                let (category, line) = err_at(line);
                return Err(GlossaryError::DuplicateId { category, line, id: id.into() });
            }
            if (1..6).any(|i| field(i).contains('|')) {
                let (category, line) = err_at(line);
                return Err(GlossaryError::Pipe { category, line });
            }
            added.push(Entry {
                category: category.into(),
                id: id.into(),
                ko: field(1).into(),
                en: field(2).into(),
                aliases: list(field(3)),
                old: list(field(4)),
                misheard: list(field(5)),
                flags: field(6).split_whitespace().map(String::from).collect(),
                note: field(7).into(),
            });
        }
        self.entries.extend(added);
        Ok(())
    }

    /// The entry with `id` in `category`.
    pub fn get(&self, category: &str, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.category == category && e.id == id)
    }

    /// The term list `overfilm-eval --terms` reads: one line per entry with a Korean spelling and
    /// no `noeval` flag, `ko | aliases | old names`, plus the English handle for `players` and
    /// `teams` (broadcast captions often write handles as-is). Misrecognitions are left out, and
    /// so are one-character spellings (`궁`, `피`), which would match inside ordinary words.
    pub fn term_list(&self) -> String {
        let mut out = String::from("# generated from the glossary (overfilm-glossary)\n");
        for e in self.entries.iter().filter(|e| !e.ko.is_empty() && !e.has_flag("noeval")) {
            let mut spellings = vec![e.ko.as_str()];
            spellings.extend(e.aliases.iter().chain(&e.old).map(String::as_str));
            if matches!(e.category.as_str(), "players" | "teams") && !e.en.is_empty() {
                spellings.push(&e.en);
            }
            spellings.retain(|s| s.chars().filter(|c| c.is_alphanumeric()).count() >= 2);
            if spellings.is_empty() {
                continue;
            }
            out.push_str(&spellings.join(" | "));
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "id\tko\ten\taliases\told\tmisheard\tflags\tnote\n";

    fn table(rows: &str) -> String {
        format!("# comment\n{HEADER}{rows}")
    }

    #[test]
    fn parses_rows_and_lists() {
        let mut g = Glossary::default();
        g.add_table("heroes", &table("cassidy\t캐서디\tCassidy\t\t맥크리; McCree\t\t\t공격\n\nmei\t메이\n")).unwrap();
        let c = g.get("heroes", "cassidy").unwrap();
        assert_eq!(c.old, ["맥크리", "McCree"]);
        assert_eq!(c.note, "공격");
        assert_eq!(g.get("heroes", "mei").unwrap().ko, "메이");
    }

    #[test]
    fn term_list_skips_noeval_and_adds_handles() {
        let mut g = Glossary::default();
        g.add_table("heroes", &table("cassidy\t캐서디\tCassidy\t캐\t맥크리\nx\t\tOnlyEnglish\nlip\t립\tLIP\t\t\t\tnoeval\n")).unwrap();
        g.add_table("players", &table("proud\t프라우드\tProud\t\t\t트라우드\n")).unwrap();
        let t = g.term_list();
        assert!(t.contains("\n캐서디 | 맥크리\n"));
        assert!(t.contains("\n프라우드 | Proud\n"));
        assert!(!t.contains("립") && !t.contains("OnlyEnglish") && !t.contains("트라우드"));
    }

    #[test]
    fn bad_tables() {
        let mut g = Glossary::default();
        assert!(matches!(g.add_table("h", "id\tko\n"), Err(GlossaryError::Header { .. })));
        assert!(matches!(g.add_table("h", ""), Err(GlossaryError::Header { .. })));
        assert!(matches!(g.add_table("h", &table("\tx\n")), Err(GlossaryError::NoId { line: 3, .. })));
        assert!(matches!(g.add_table("h", &table("a\tx\na\ty\n")), Err(GlossaryError::DuplicateId { .. })));
        assert!(matches!(g.add_table("h", &table("a\tx|y\n")), Err(GlossaryError::Pipe { .. })));
        assert!(matches!(g.add_table("h", &table(&format!("a{}\n", "\tx".repeat(8)))), Err(GlossaryError::TooManyColumns { .. })));
        assert!(matches!(g.add_table("h", &table(&format!("a\t{}\n", "가".repeat(MAX_FIELD + 1)))), Err(GlossaryError::TooLong { .. })));
        assert!(g.entries.is_empty());
    }

    /// The shipped Overwatch Korean pack parses and keeps the ids the eval and captions rely on.
    #[test]
    fn overwatch_ko_pack_parses() {
        let tables = [
            ("heroes", include_str!("../../../packs/overwatch/ko/glossary/heroes.tsv")),
            ("abilities", include_str!("../../../packs/overwatch/ko/glossary/abilities.tsv")),
            ("maps", include_str!("../../../packs/overwatch/ko/glossary/maps.tsv")),
            ("modes", include_str!("../../../packs/overwatch/ko/glossary/modes.tsv")),
            ("terms", include_str!("../../../packs/overwatch/ko/glossary/terms.tsv")),
            ("teams", include_str!("../../../packs/overwatch/ko/glossary/teams.tsv")),
            ("players", include_str!("../../../packs/overwatch/ko/glossary/players.tsv")),
            ("slang", include_str!("../../../packs/overwatch/ko/glossary/slang.tsv")),
            ("matchups", include_str!("../../../packs/overwatch/ko/glossary/matchups.tsv")),
            ("profanity", include_str!("../../../packs/overwatch/ko/glossary/profanity.tsv")),
        ];
        let mut g = Glossary::default();
        for (category, text) in tables {
            g.add_table(category, text).unwrap();
        }
        let cassidy = g.get("heroes", "cassidy").unwrap();
        assert_eq!(cassidy.ko, "캐서디");
        assert!(cassidy.old.iter().any(|o| o == "맥크리"));
        assert!(g.entries.iter().filter(|e| e.category == "heroes").count() >= 50);
        assert!(g.get("matchups", "crazy_raccoon-zeta_division").is_some_and(|m| m.ko == "라제전" && m.aliases == ["제라전"]));
        // Profanity is masked to its initial consonants and never scored.
        let p = g.get("profanity", "ssibal").unwrap();
        assert!(p.ko == "ㅅㅂ" && p.has_flag("mask") && p.has_flag("noeval") && p.misheard.iter().any(|m| m == "씨발"));
        assert!(g.get("teams", "cheeseburger").is_some_and(|t| t.aliases.iter().any(|a| a == "처벅")));
        assert!(g.get("teams", "crazy_raccoon").is_some_and(|t| t.aliases.iter().any(|a| a == "CR")));
        // Every ability id starts with a hero id.
        for a in g.entries.iter().filter(|e| e.category == "abilities") {
            let hero = a.id.split('.').next().unwrap();
            assert!(g.get("heroes", hero).is_some(), "ability {} has no hero {hero}", a.id);
        }
    }
}

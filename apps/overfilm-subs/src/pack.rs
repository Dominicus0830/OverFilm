//! The Overwatch Korean pack built into the program: glossary tables and the recognition context
//! (`packs/overwatch/ko/`), so the exe works on its own. A user glossary folder can add tables.

use std::path::Path;

use overfilm_glossary::{Corrector, Glossary};
use serde::{Deserialize, Serialize};

const TABLES: &[(&str, &str)] = &[
    ("abilities", include_str!("../../../packs/overwatch/ko/glossary/abilities.tsv")),
    ("heroes", include_str!("../../../packs/overwatch/ko/glossary/heroes.tsv")),
    ("maps", include_str!("../../../packs/overwatch/ko/glossary/maps.tsv")),
    ("matchups", include_str!("../../../packs/overwatch/ko/glossary/matchups.tsv")),
    ("modes", include_str!("../../../packs/overwatch/ko/glossary/modes.tsv")),
    ("players", include_str!("../../../packs/overwatch/ko/glossary/players.tsv")),
    ("profanity", include_str!("../../../packs/overwatch/ko/glossary/profanity.tsv")),
    ("slang", include_str!("../../../packs/overwatch/ko/glossary/slang.tsv")),
    ("teams", include_str!("../../../packs/overwatch/ko/glossary/teams.tsv")),
    ("terms", include_str!("../../../packs/overwatch/ko/glossary/terms.tsv")),
];

/// Hero and term list given to the recogniser (the setup measured on the M1 test set).
const CONTEXT: &str = include_str!("../../../packs/overwatch/ko/context.txt");

/// Largest user glossary file read.
const MAX_TABLE: u64 = 4 << 20;
/// Longest context text handed to the recogniser (characters); more slows every chunk.
pub const MAX_CONTEXT_CHARS: usize = 2000;

/// What kind of video it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Gameplay, lectures, tier lists: heroes and game terms.
    #[default]
    General,
    /// Tournament broadcasts (OWCS, 옵드컵): also team and player names.
    Commentary,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::General => "일반 게임 영상",
            Mode::Commentary => "대회 해설 (팀·선수 이름 포함)",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "general" | "일반" => Some(Mode::General),
            "commentary" | "해설" => Some(Mode::Commentary),
            _ => None,
        }
    }
}

/// The built-in glossary plus every `*.tsv` in `extra_dir` (category = file name).
pub fn glossary(extra_dir: Option<&Path>, mask_profanity: bool) -> Result<Glossary, String> {
    let mut g = Glossary::default();
    for (cat, text) in TABLES {
        if *cat == "profanity" && !mask_profanity {
            continue;
        }
        g.add_table(cat, text).map_err(|e| format!("내장 단어집 {cat}: {e}"))?;
    }
    if let Some(dir) = extra_dir {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "tsv"))
            .collect();
        files.sort();
        for f in files {
            if std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0) > MAX_TABLE {
                return Err(format!("{}: 파일이 너무 큽니다", f.display()));
            }
            let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            // a user table may reuse a built-in category name: keep it apart
            let cat = format!("user-{}", f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            g.add_table(&cat, &text).map_err(|e| format!("{}: {e}", f.display()))?;
        }
    }
    Ok(g)
}

/// Glossary correction: misrecognitions, profanity initials (when in the glossary), near-miss
/// names.
pub fn corrector(g: &Glossary) -> Corrector {
    Corrector::new(g).fuzzy(g)
}

fn names(g: &Glossary, category: &str) -> Vec<String> {
    g.entries.iter().filter(|e| e.category == category && !e.ko.is_empty() && !e.has_flag("noeval")).map(|e| e.ko.clone()).collect()
}

/// Recognition context for `mode`, plus the user's extra terms (comma or line separated).
pub fn context(g: &Glossary, mode: Mode, extra_terms: &str) -> String {
    let mut s = CONTEXT.trim().to_string();
    if mode == Mode::Commentary {
        s.push_str(&format!(" 팀: {}. 선수: {}.", names(g, "teams").join(", "), names(g, "players").join(", ")));
    }
    let extra: Vec<&str> = extra_terms.split([',', '\n', ';']).map(str::trim).filter(|t| !t.is_empty()).collect();
    if !extra.is_empty() {
        s.push_str(&format!(" 추가 용어: {}.", extra.join(", ")));
    }
    s.chars().take(MAX_CONTEXT_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_pack_loads_and_corrects() {
        let g = glossary(None, true).unwrap();
        let c = corrector(&g);
        assert!(!c.is_empty());
        assert!(c.apply("씨발 진짜").text.starts_with("ㅅㅂ"));
        let g2 = glossary(None, false).unwrap();
        assert!(!corrector(&g2).apply("씨발").text.starts_with("ㅅㅂ"));
    }

    #[test]
    fn contexts() {
        let g = glossary(None, true).unwrap();
        let general = context(&g, Mode::General, "");
        assert!(general.contains("정커퀸") && general.contains("독트린"));
        let comm = context(&g, Mode::Commentary, "새 선수, 다른 팀");
        assert!(comm.contains("팀: ") && comm.contains("추가 용어: 새 선수, 다른 팀"));
        assert!(comm.chars().count() <= MAX_CONTEXT_CHARS);
    }
}

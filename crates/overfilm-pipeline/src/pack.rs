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
    ("netslang", include_str!("../../../packs/overwatch/ko/glossary/netslang.tsv")),
    ("players", include_str!("../../../packs/overwatch/ko/glossary/players.tsv")),
    ("profanity", include_str!("../../../packs/overwatch/ko/glossary/profanity.tsv")),
    ("slang", include_str!("../../../packs/overwatch/ko/glossary/slang.tsv")),
    ("spelling", include_str!("../../../packs/overwatch/ko/glossary/spelling.tsv")),
    ("teams", include_str!("../../../packs/overwatch/ko/glossary/teams.tsv")),
    ("terms", include_str!("../../../packs/overwatch/ko/glossary/terms.tsv")),
];

/// In-game voice lines (hero ultimates, announcer), to keep game audio out of captions.
const VOICELINES: &str = include_str!("../../../packs/overwatch/ko/voicelines.tsv");

/// The built-in voice line table.
pub fn voicelines() -> overfilm_glossary::voicelines::VoiceLines {
    overfilm_glossary::voicelines::VoiceLines::parse(VOICELINES)
}

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
/// names, Korean spelling and spacing, spoken numbers as digits.
pub fn corrector(g: &Glossary) -> Corrector {
    corrector_with(g, true)
}

/// [`corrector`], with spoken numbers turned into digits or left as spoken.
pub fn corrector_with(g: &Glossary, digits: bool) -> Corrector {
    Corrector::new(g).fuzzy(g).korean().digits(digits)
}

fn names(g: &Glossary, category: &str) -> Vec<String> {
    g.entries.iter().filter(|e| e.category == category && !e.ko.is_empty() && !e.has_flag("noeval")).map(|e| e.ko.clone()).collect()
}

/// Recognition context for `mode`, plus the user's extra terms (comma or line separated).
pub fn context(g: &Glossary, mode: Mode, extra_terms: &str) -> String {
    let mut s = CONTEXT.trim().to_string();
    if mode == Mode::Commentary {
        let teams: Vec<String> = g
            .entries
            .iter()
            .filter(|e| e.category == "teams" && !e.ko.is_empty() && !e.has_flag("noeval"))
            .flat_map(|e| std::iter::once(e.ko.clone()).chain(e.aliases.iter().filter(|a| a.chars().any(|c| ('가'..='힣').contains(&c))).cloned()))
            .collect();
        s.push_str(&format!(" 팀: {}. 선수: {}.", teams.join(", "), names(g, "players").join(", ")));
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
    fn spelling_and_numbers() {
        let g = glossary(None, true).unwrap();
        let fixed = corrector(&g).apply("오 시즌 강벤 이거 는 할수있어요 저 티어");
        assert_eq!(fixed.text, "5시즌 강밴 이거는 할 수 있어요 저티어");
        assert_eq!(corrector_with(&g, false).apply("오 시즌 일 위").text, "오 시즌 일 위");
        // the spelling table is not scored as game terms
        assert!(!g.term_list().contains("강밴"));
    }

    #[test]
    fn community_slang() {
        let g = glossary(None, true).unwrap();
        let c = corrector(&g);
        assert_eq!(c.apply("알 잘 딱 깔 센 하게 킹 받네 딜러 차이").text, "알잘딱깔센 하게 킹받네 딜러차이");
        // Slang spellings are known words, so the name fixer leaves them alone.
        for w in ["메모위", "아키우일", "고츠", "볼토시", "네카오", "중꺾마", "레게노", "뱅온", "합방"] {
            assert_eq!(c.apply(w).text, w);
        }
        // Not recognition context: on the M1 set it cost 0.15 points of CER (see the pack README).
        assert!(!context(&g, Mode::General, "").contains("알잘딱깔센"));
        // User report: a multi-speaker clip with the newest heroes.
        assert_eq!(c.apply("보통 뭐 캐슬 쓰게 되면은 딜몬도 좀 쓰고").text, "보통 뭐 캣 쓰게 되면은 디몬도 좀 쓰고");
        assert_eq!(c.apply("모래 캐슬 성").text, "모래 캐슬 성");
        // T1 vs RØDE commentary: team, hero, map and player names.
        assert_eq!(c.apply("티온이 동커퀸 아이언발드 백기 선수").text, "티원이 정커퀸 아이헨발데 베키 선수");
        // Quick speech (the M1 aichi video at 1.35x): a known word running into the rest of a name.
        assert_eq!(c.apply("솜브라 로더그가 사기였던 제트팩킷").text, "솜브라 로드호그가 사기였던 제트팩 캣");
        // aichi tier list: "히트박스 패치로 꿀을 빨고 있었던"
        assert_eq!(c.apply("패치로 꼴을 빨고 있었던").text, "패치로 꿀을 빨고 있었던");
        // user report on the same video: 냥이 (제트팩 캣) heard as 양이, and English "two"
        assert_eq!(c.apply("솜브라와 양이캐서디 연계가").text, "솜브라와 냥이캐서디 연계가");
        assert_eq!(c.apply("밥 양이 많다").text, "밥 양이 많다");
        assert_eq!(c.apply("그냥 메인 딜러 투라고 표기해 봤습니다").text, "그냥 메인 딜러 2라고 표기해 봤습니다");
        assert_eq!(c.apply("저는 지금 투 서브 딜러가").text, "저는 지금 투 서브 딜러가");
    }

    #[test]
    fn voice_lines_load() {
        let v = voicelines();
        assert!(v.lines.len() > 80);
        assert!(!v.find(&["석양이", "진다"]).is_empty());
    }

    #[test]
    fn contexts() {
        let g = glossary(None, true).unwrap();
        let general = context(&g, Mode::General, "");
        assert!(general.contains("정커퀸") && general.contains("독트린"));
        let comm = context(&g, Mode::Commentary, "새 선수, 다른 팀");
        assert!(comm.contains("팀: ") && comm.contains("추가 용어: 새 선수, 다른 팀"));
        assert!(comm.contains("치벅"), "team nicknames are context too");
        assert!(comm.chars().count() <= MAX_CONTEXT_CHARS);
    }
}

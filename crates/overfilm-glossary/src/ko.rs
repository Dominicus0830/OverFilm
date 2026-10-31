//! Korean spelling and spacing tidy-up for recognised speech (맞춤법·띄어쓰기).
//!
//! Speech recognisers write Korean mostly right, but a few slips recur: a particle split off its
//! word (`이거 는`), a dependent noun glued to its verb (`할수있다`, `하는거`, `것같아`), and the
//! common nonstandard spellings of speech (`할께요`, `됬다`, `그럴려면`). Only rules that are
//! almost never wrong are applied: each pattern names the exact syllables it fires on, so an
//! ordinary word that happens to contain the same letters is left alone (`실수 없이`, `볼거리`,
//! `할수록`, `올려`, `알려` stay). Word-level spellings that are specific to a game belong in the
//! glossary (`spelling.tsv`), not here.

/// Particles that are never a word of their own: written apart from the previous word, they are
/// joined to it (`이거 는` → `이거는`, `영웅이다 라고` → `영웅이다라고`). One-syllable particles that are
/// also words (`이`, `가`, `나`, `도`, `만`, `와`, `에`, `의`) are not listed.
const PARTICLES: &[&str] = &[
    "은",
    "는",
    "을",
    "를",
    "으로",
    "으로는",
    "으로도",
    "에서",
    "에서는",
    "에서도",
    "에게",
    "에게는",
    "부터",
    "부터는",
    "서부터",
    "서부터는",
    "까지",
    "까지는",
    "까지도",
    "처럼",
    "이랑",
    "라고",
    "이라고",
    "라고도",
    "이라고도",
    "라는",
    "이라는",
];

/// Syllables ending in ㄹ that are the future/attributive form of a verb (`할`, `갈`…), after which
/// `수`, `때`, `거`, `것`, `게` start a new word. Syllables that also end ordinary nouns (`실`, `말`,
/// `물`, `일`, `발`, `탈`…) are left out on purpose.
const VERB_L: &[char] = &['할', '갈', '볼', '올', '줄', '될', '쓸', '칠', '쏠', '낼', '뺄', '걸', '들', '알', '살', '팔', '놀', '을', '설', '럴'];

/// `X려` written `X+ㄹ려` (`할려고` → `하려고`): the syllable with ㄹ and the one without. `올려`,
/// `알려`, `걸려`, `들려`, `살려`, `갈려` are real words (올리다, 알리다…) and are not listed.
const L_RYEO: &[(char, char)] =
    &[('할', '하'), ('볼', '보'), ('줄', '주'), ('될', '되'), ('칠', '치'), ('쏠', '쏘'), ('낼', '내'), ('뺄', '빼'), ('럴', '러'), ('을', '으')];

/// Syllables after which `것`/`거` (the dependent noun) starts a new word (`하는거` → `하는 거`).
const BEFORE_GEOT: &[char] = &['는', '은', '던', '한', '된', '인', '런', '운', '할', '될', '을', '갈', '볼', '줄', '쓸', '본', '온', '간', '준'];

fn is_syllable(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// What [`tidy`] changed: the new text and each change `(before, after)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tidied {
    pub text: String,
    pub changes: Vec<(String, String)>,
}

/// Join particles written as separate words (see [`PARTICLES`]).
fn join_particles(text: &str, changes: &mut Vec<(String, String)>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c == ' ' && i > 0 && chars.get(i - 1).is_some_and(|p| is_syllable(*p) || p.is_ascii_alphanumeric()) {
            let word_end = chars.get(i + 1..).map_or(0, |r| r.iter().take_while(|c| is_word_char(**c)).count());
            let word: String = chars.iter().skip(i + 1).take(word_end).collect();
            // The particle must end the word (then a space, punctuation or the end of the text).
            let after_ok = chars.get(i + 1 + word_end).is_none_or(|c| !is_word_char(*c));
            if word_end > 0 && after_ok && PARTICLES.contains(&word.as_str()) {
                let prev: String =
                    chars.get(..i).unwrap_or_default().iter().rev().take_while(|c| is_word_char(**c)).collect::<Vec<_>>().into_iter().rev().collect();
                changes.push((format!("{prev} {word}"), format!("{prev}{word}")));
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Spacing of dependent nouns and the common speech misspellings (see the module docs).
fn fix_words(text: &str, changes: &mut Vec<(String, String)>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut out: Vec<char> = Vec::with_capacity(chars.len() + 8);
    let mut i = 0;
    while let Some(c) = at(i) {
        let prev = out.last().copied();
        let next = at(i + 1);
        let next2 = at(i + 2);
        let prev_verb_l = prev.is_some_and(|p| VERB_L.contains(&p));
        let mut note = |from: String, to: String| changes.push((from, to));
        match c {
            // 할수있다 → 할 수 있다 (also 할수 있다, 할 수있다)
            '수' if prev_verb_l && (matches!(next, Some('있' | '없')) || (next == Some(' ') && matches!(next2, Some('있' | '없')))) => {
                note(format!("{}수", prev.unwrap_or(' ')), format!("{} 수", prev.unwrap_or(' ')));
                out.push(' ');
                out.push('수');
                if next != Some(' ') {
                    out.push(' ');
                }
            }
            '수' if prev == Some(' ') && out.len() >= 2 && out.get(out.len() - 2).is_some_and(|p| VERB_L.contains(p)) && matches!(next, Some('있' | '없')) =>
            {
                note("수있".into(), "수 있".into());
                out.push('수');
                out.push(' ');
            }
            // 할때 → 할 때
            '때' if prev_verb_l && next != Some('문') => {
                note(format!("{}때", prev.unwrap_or(' ')), format!("{} 때", prev.unwrap_or(' ')));
                out.push(' ');
                out.push('때');
            }
            // 기때문에 → 기 때문에
            '때' if next == Some('문') && prev.is_some_and(is_syllable) => {
                note("때문".into(), " 때문".into());
                out.push(' ');
                out.push('때');
            }
            // 하는거 / 하는것 → 하는 거 / 하는 것; 할꺼 → 할 거 (but 볼거리, 할거리 stay)
            '것' | '거' | '꺼'
                if prev.is_some_and(|p| BEFORE_GEOT.contains(&p)) && next != Some('리') && !(c == '거' && matches!(next, Some('든' | '나' | '늘'))) =>
            {
                let to = if c == '꺼' { '거' } else { c };
                note(format!("{}{c}", prev.unwrap_or(' ')), format!("{} {to}", prev.unwrap_or(' ')));
                out.push(' ');
                out.push(to);
            }
            // 것같아 / 거같아 → 것 같아
            '같' if matches!(prev, Some('것' | '거')) => {
                note(format!("{}같", prev.unwrap_or(' ')), format!("{} 같", prev.unwrap_or(' ')));
                out.push(' ');
                out.push('같');
            }
            // 하는중 → 하는 중
            '중' if prev == Some('는') => {
                note("는중".into(), "는 중".into());
                out.push(' ');
                out.push('중');
            }
            // 할께요 → 할게요 (선수들께 is the honorific 께)
            '께' if prev_verb_l && prev != Some('들') && next.is_none_or(|n| n == '요' || !is_syllable(n)) => {
                note(format!("{}께", prev.unwrap_or(' ')), format!("{}게", prev.unwrap_or(' ')));
                out.push('게');
            }
            // 할려고 → 하려고, 그럴려면 → 그러려면, 먹을려고 → 먹으려고
            '려' if prev.is_some_and(|p| L_RYEO.iter().any(|(l, _)| *l == p)) && matches!(next, Some('고' | '면' | '는' | '다' | '나')) => {
                if let (Some(p), Some(last)) = (prev, out.last_mut())
                    && let Some((_, plain)) = L_RYEO.iter().find(|(l, _)| *l == p)
                {
                    *last = *plain;
                    note(format!("{p}려"), format!("{plain}려"));
                }
                out.push('려');
            }
            // 됬 → 됐
            '됬' => {
                note("됬".into(), "됐".into());
                out.push('됐');
            }
            // 되요 / 되서 → 돼요 / 돼서 (되 never takes 요 or 서 directly)
            '되' if matches!(next, Some('요' | '서')) && at(i + 2).is_none_or(|n| !is_syllable(n) || n == '도') => {
                note(format!("되{}", next.unwrap_or(' ')), format!("돼{}", next.unwrap_or(' ')));
                out.push('돼');
            }
            // 거에요 / 거애요 → 거예요
            '에' | '애' if matches!(prev, Some('거' | '꺼')) && next == Some('요') => {
                note(format!("{}{c}요", prev.unwrap_or(' ')), "거예요".into());
                if let Some(last) = out.last_mut() {
                    *last = '거';
                }
                out.push('예');
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out.into_iter().collect()
}

/// Tidy `text` (see the module docs). Applying it twice changes nothing more.
pub fn tidy(text: &str) -> Tidied {
    let mut changes = Vec::new();
    let joined = join_particles(text, &mut changes);
    let fixed = fix_words(&joined, &mut changes);
    // Inserted spaces never double up with existing ones.
    let mut text = String::with_capacity(fixed.len());
    for c in fixed.chars() {
        if c == ' ' && text.ends_with(' ') {
            continue;
        }
        text.push(c);
    }
    Tidied { text, changes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> String {
        tidy(s).text
    }

    #[test]
    fn particles_join() {
        assert_eq!(t("이거 는 개념 은 영상 으로 여기 서부터는 좋다."), "이거는 개념은 영상으로 여기서부터는 좋다.");
        assert_eq!(t("영웅이다 라고 했고 B 를"), "영웅이다라고 했고 B를");
        // words that are also particles stay apart, and so does a particle after punctuation
        assert_eq!(t("나 는커녕 이 영웅 가 보자. 는"), "나 는커녕 이 영웅 가 보자. 는");
        assert_eq!(t("은 시작"), "은 시작");
    }

    #[test]
    fn dependent_nouns() {
        assert_eq!(t("할수있다 볼수 없고 갈 수있어 먹을수있는"), "할 수 있다 볼 수 없고 갈 수 있어 먹을 수 있는");
        assert_eq!(t("하는거 같아 그런것같아 할거야 할꺼야"), "하는 거 같아 그런 것 같아 할 거야 할 거야");
        assert_eq!(t("할때 기때문에 하는중이에요"), "할 때 기 때문에 하는 중이에요");
    }

    #[test]
    fn ordinary_words_stay() {
        for s in [
            "실수 없이",
            "실수없이",
            "볼거리",
            "할거리",
            "할수록",
            "올려",
            "알려",
            "걸려",
            "그때",
            "말수가",
            "선수들께",
            "물건",
            "그거든",
            "중요한",
            "되는",
            "되서다",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn misspellings() {
        assert_eq!(t("할께요 갈께 됬다 안되요 되서 그럴려면 할려고 먹을려고 거에요"), "할게요 갈게 됐다 안돼요 돼서 그러려면 하려고 먹으려고 거예요");
    }

    #[test]
    fn idempotent_and_safe_on_odd_input() {
        for s in ["", " ", "수", "께", "려", "되", "할 수있다", "  는", "a 는 b", "거에요", "됬됬"] {
            let once = t(s);
            assert_eq!(t(&once), once, "{s}");
        }
    }

    #[test]
    fn reports_changes() {
        let r = tidy("이거 는 할께");
        assert_eq!(r.changes.len(), 2);
        assert_eq!(r.changes[0], ("이거 는".to_string(), "이거는".to_string()));
    }
}

//! Spoken numbers to digits in captions (`오 시즌` → `5시즌`, `사십칠 퍼센트` → `47%`, `여섯 개` →
//! `6개`), the way Korean video captions write them.
//!
//! Recognisers write numbers as spoken. A number word is only rewritten when a counter follows it
//! (`위`, `시즌`, `퍼센트`, `개`, `명`…), so ordinary words that are also numbers stay (`이 영웅`,
//! `일이 많다`, `네, 맞아요`). On top of that:
//!
//! - Sino-Korean numbers must be well formed (`사십칠`, `십이`, `이천이십육`); runs like `삼사`
//!   (three or four) are left alone.
//! - The single syllables `이` (this) and `사` (buy) are never read as numbers, nor is a number
//!   glued to a counter that would make another word (`십분` = fully, `오점` = a flaw): glued
//!   one-syllable numbers only become digits before counters marked `glued`.
//! - A counter followed by a syllable that makes it another word is not a counter (`일 위험`,
//!   `오 초반`, `세 개발자`).
//! - Native numbers (`두`…`열`) only before `개`, `명`, `킬`, `판`, `마리`; `한` (one) stays a word.
//! - English numbers after a name: `오버워치 투` → `오버워치 2`, `메인 딜러 투라고` → `메인 딜러
//!   2라고`, `시즌 원` → `시즌 1`; `투` before another word means "two" and stays (`투 서브 딜러`).

use crate::ko::Tidied;

/// A counter after a Sino-Korean number: spoken form, written form, whether it may be glued to a
/// one-syllable number, and syllables after it that make it a different word.
struct Counter {
    spoken: &'static str,
    written: &'static str,
    glued: bool,
    not_before: &'static [char],
}

const SINO_COUNTERS: &[Counter] = &[
    Counter { spoken: "퍼센트", written: "%", glued: true, not_before: &[] },
    Counter { spoken: "프로", written: "%", glued: false, not_before: &['게', '선', '필', '팀', '리', '젝', '그', '토', '세', '들', '야', '님'] },
    Counter { spoken: "시즌", written: "시즌", glued: true, not_before: &[] },
    Counter { spoken: "세트", written: "세트", glued: true, not_before: &[] },
    Counter { spoken: "라운드", written: "라운드", glued: true, not_before: &[] },
    Counter { spoken: "연승", written: "연승", glued: true, not_before: &[] },
    Counter { spoken: "연패", written: "연패", glued: true, not_before: &[] },
    Counter { spoken: "레벨", written: "레벨", glued: true, not_before: &['업'] },
    Counter {
        spoken: "위",
        written: "위",
        glued: true,
        not_before: &['험', '치', '협', '기', '해', '반', '원', '성', '에', '로', '아', '층', '상', '선', '쪽'],
    },
    Counter { spoken: "등", written: "등", glued: true, not_before: &['장', '산', '록', '뒤', '대', '불'] },
    Counter { spoken: "년", written: "년", glued: true, not_before: &['생', '차', '전', '후'] },
    Counter { spoken: "킬", written: "킬", glued: true, not_before: &['러', '캠', '로', '각', '피'] },
    Counter { spoken: "초", written: "초", glued: false, not_before: &['반', '기', '보', '록', '콜', '월', '대', '점', '고', '능'] },
    Counter {
        spoken: "분", written: "분", glued: false, not_before: &['명', '석', '위', '들', '야', '노', '배', '해', '께', '량', '산', '류', '열', '쟁']
    },
    Counter { spoken: "점", written: "점", glued: false, not_before: &['령', '수', '유', '프', '검', '심', '차', '점', '만'] },
    Counter { spoken: "승", written: "승", glued: false, not_before: &['리', '률', '부', '자', '패', '격', '천', '급', '산', '인'] },
    Counter { spoken: "패", written: "패", glued: false, not_before: &['치', '배', '스', '턴', '널', '키', '션', '드', '기'] },
    Counter { spoken: "개월", written: "개월", glued: true, not_before: &[] },
    Counter { spoken: "월", written: "월", glued: false, not_before: &['드', '등', '급', '요', '세', '간', '말'] },
];

const NATIVE: &[(&str, u32)] = &[("두", 2), ("세", 3), ("네", 4), ("다섯", 5), ("여섯", 6), ("일곱", 7), ("여덟", 8), ("아홉", 9), ("열", 10)];
const NATIVE_COUNTERS: &[(&str, &[char])] = &[
    ("개", &['골', '인', '발', '념', '선', '막', '편', '최', '성', '척', '조', '방', '입', '그', '판']),
    ("명", &['확', '령', '예', '중', '단', '성', '칭', '언', '작', '절', '대', '분', '백', '함']),
    ("킬", &['러', '캠', '로', '각', '피']),
    ("판", &['정', '단', '매', '결', '도', '때', '사', '넬', '교']),
    ("마리", &[]),
];

fn sino_digit(c: char) -> Option<u64> {
    Some(match c {
        '일' => 1,
        '이' => 2,
        '삼' => 3,
        '사' => 4,
        '오' => 5,
        '육' => 6,
        '칠' => 7,
        '팔' => 8,
        '구' => 9,
        _ => return None,
    })
}

fn sino_unit(c: char) -> Option<u64> {
    Some(match c {
        '십' => 10,
        '백' => 100,
        '천' => 1000,
        _ => return None,
    })
}

/// Value of a well-formed Sino-Korean number (up to 9999 만 9999), or `None`.
fn sino_value(s: &[char]) -> Option<u64> {
    let mut total: u64 = 0;
    let mut group: u64 = 0;
    let mut digit: Option<u64> = None;
    let mut last_unit = u64::MAX;
    let mut seen_man = false;
    for &c in s {
        if let Some(d) = sino_digit(c) {
            if digit.is_some() {
                return None; // 삼사: two digits in a row
            }
            digit = Some(d);
        } else if let Some(u) = sino_unit(c) {
            if u >= last_unit {
                return None; // units must fall: 천 > 백 > 십
            }
            // 일십 / 일백 are not said; a lone unit means one of it
            if digit == Some(1) && u < 1000 {
                return None;
            }
            group = group.checked_add(digit.unwrap_or(1).checked_mul(u)?)?;
            digit = None;
            last_unit = u;
        } else if c == '만' {
            if seen_man {
                return None;
            }
            seen_man = true;
            let g = group.checked_add(digit.unwrap_or(0))?;
            total = g.max(1).checked_mul(10_000)?;
            group = 0;
            digit = None;
            last_unit = u64::MAX;
        } else {
            return None;
        }
    }
    total.checked_add(group)?.checked_add(digit.unwrap_or(0))
}

fn is_number_char(c: char) -> bool {
    sino_digit(c).is_some() || sino_unit(c).is_some() || c == '만'
}

fn starts_with(rest: &[char], s: &str) -> bool {
    let n = s.chars().count();
    rest.get(..n).is_some_and(|w| w.iter().copied().eq(s.chars()))
}

/// The counter starting at `rest`, if the syllable after it does not make another word.
fn counter_at<'a>(rest: &[char], counters: impl Iterator<Item = (&'a str, &'a [char])>) -> Option<&'a str> {
    counters
        .filter(|(spoken, not_before)| starts_with(rest, spoken) && rest.get(spoken.chars().count()).is_none_or(|n| !not_before.contains(n)))
        .map(|(s, _)| s)
        .max_by_key(|s| s.chars().count())
}

/// Rewrite spoken numbers before counters as digits (see the module docs).
pub fn write_digits(text: &str) -> Tidied {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut changes = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let word_start = i == 0 || chars.get(i - 1).is_none_or(|p| !p.is_alphanumeric());
        if word_start && let Some((len, written)) = english_at(&chars, i).or_else(|| number_at(chars.get(i..).unwrap_or_default())) {
            changes.push((chars.iter().skip(i).take(len).collect(), written.clone()));
            out.push_str(&written);
            i += len;
            continue;
        }
        out.push(c);
        i += 1;
    }
    Tidied { text: out, changes }
}

/// Particles after a numbered name (`오버워치 투에서`, `메인 딜러 투라고`).
const NAME_PARTICLES: &[&str] = &[
    "에서",
    "으로",
    "라고",
    "이라고",
    "까지",
    "부터",
    "처럼",
    "보다",
    "이랑",
    "입니다",
    "이에요",
    "예요",
    "랑",
    "는",
    "은",
    "가",
    "이",
    "를",
    "을",
    "에",
    "로",
    "의",
    "도",
    "만",
    "와",
    "과",
    "다",
];

const ENGLISH_NUMBERS: [(&str, u32, bool); 3] = [("투", 2, false), ("원", 1, true), ("쓰리", 3, true)];

/// English numbers said after a name (`오버워치 투` → `오버워치 2`, `메인 딜러 투라고` → `메인 딜러
/// 2라고`, `시즌 원만` → `시즌 1만`): (characters used, written form). `투` is a numbered name only as
/// the last word of the phrase, alone or with a particle; before another word it is "two" and stays
/// (`투 서브 딜러`, `투 히트`), as do words that start with it (`투표`, `투자`). `원` (also won,
/// circle) and `쓰리` only after `시즌`.
fn english_at(chars: &[char], i: usize) -> Option<(usize, String)> {
    let rest = chars.get(i..)?;
    // a word before, then one space
    let before: String = chars.get(..i)?.iter().rev().skip(1).take_while(|c| c.is_alphanumeric()).collect::<Vec<_>>().into_iter().rev().collect();
    // counting out loud (`원 투 쓰리`) stays
    if before.is_empty() || chars.get(i.checked_sub(1)?) != Some(&' ') || ENGLISH_NUMBERS.iter().any(|(w, _, _)| before == *w) {
        return None;
    }
    for (word, n, season_only) in ENGLISH_NUMBERS {
        if !starts_with(rest, word) || (season_only && before != "시즌") {
            continue;
        }
        let p = word.chars().count();
        let tail: String = rest.get(p..).unwrap_or_default().iter().take_while(|c| c.is_alphanumeric()).collect();
        let particle = tail.is_empty() || NAME_PARTICLES.contains(&tail.as_str());
        // another word right after (`투 서브`) means "two"
        let then_word = tail.is_empty() && rest.get(p) == Some(&' ') && rest.get(p + 1).is_some_and(|c| c.is_alphanumeric());
        if particle && !then_word {
            return Some((p, n.to_string()));
        }
    }
    None
}

/// A spoken number and its counter at the start of `rest`: (characters used, written form).
fn number_at(rest: &[char]) -> Option<(usize, String)> {
    // Native numbers before 개 / 명 / 킬 / 판 / 마리.
    for (word, n) in NATIVE {
        if !starts_with(rest, word) {
            continue;
        }
        let mut p = word.chars().count();
        let spaced = rest.get(p) == Some(&' ');
        if spaced {
            p += 1;
        }
        let after = rest.get(p..).unwrap_or_default();
        if let Some(counter) = counter_at(after, NATIVE_COUNTERS.iter().map(|(s, nb)| (*s, *nb))) {
            return Some((p + counter.chars().count(), format!("{n}{counter}")));
        }
    }
    // Sino-Korean numbers before a counter.
    let len = rest.iter().take_while(|c| is_number_char(**c)).count();
    // Try the longest number first, then shorter ones (`오시즌`: 오 + 시즌).
    for n in (1..=len).rev() {
        let num = rest.get(..n)?;
        let Some(value) = sino_value(num) else { continue };
        if n == 1 && matches!(num.first(), Some('이' | '사')) {
            continue;
        }
        let mut p = n;
        let spaced = rest.get(p) == Some(&' ');
        if spaced {
            p += 1;
        }
        let after = rest.get(p..).unwrap_or_default();
        let Some(counter) = SINO_COUNTERS.iter().find(|k| counter_at(after, std::iter::once((k.spoken, k.not_before))).is_some()) else { continue };
        if n == 1 && !spaced && !counter.glued {
            continue;
        }
        let used = p + counter.spoken.chars().count();
        return Some((used, format!("{value}{}", counter.written)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> String {
        write_digits(s).text
    }

    #[test]
    fn values() {
        let v = |s: &str| sino_value(&s.chars().collect::<Vec<_>>());
        assert_eq!(v("사십칠"), Some(47));
        assert_eq!(v("십이"), Some(12));
        assert_eq!(v("십"), Some(10));
        assert_eq!(v("이천이십육"), Some(2026));
        assert_eq!(v("삼만오천"), Some(35_000));
        assert_eq!(v("만"), Some(10_000));
        assert_eq!(v("삼사"), None);
        assert_eq!(v("십백"), None);
        assert_eq!(v("일십"), None);
        assert_eq!(v("만만"), None);
    }

    #[test]
    fn numbers_before_counters() {
        assert_eq!(d("오시즌 티어리스트"), "5시즌 티어리스트");
        assert_eq!(d("전환 중에 일 위입니다"), "전환 중에 1위입니다");
        assert_eq!(d("사십칠 퍼센트로 떨어졌는데"), "47%로 떨어졌는데");
        assert_eq!(d("여섯 개의 전환, 총 세 명"), "6개의 전환, 총 3명");
        assert_eq!(d("이십 점 십육 위는"), "20점 16위는");
        assert_eq!(d("이십육 년도 시즌 일"), "26년도 시즌 일");
        assert_eq!(d("오십삼 프로니까"), "53%니까");
        // months: aichi tier list, "길게 육 개월치를 보면"
        assert_eq!(d("길게 육 개월치를 보면 십이개월"), "길게 6개월치를 보면 12개월");
    }

    #[test]
    fn english_numbers_after_names() {
        // aichi tier list
        assert_eq!(d("그냥 메인 딜러 투라고 표기해 봤습니다."), "그냥 메인 딜러 2라고 표기해 봤습니다.");
        assert_eq!(d("오버워치 투에서 키리코가"), "오버워치 2에서 키리코가");
        assert_eq!(d("26년도 시즌 원만 보다"), "26년도 시즌 1만 보다");
        assert_eq!(d("오버워치 투."), "오버워치 2.");
        // "two" before a word, words starting with 투, sentence starts, 원 elsewhere
        for s in
            ["놓고 투 서브 딜러하는", "지금 투 서브 딜러가", "투 히트 조합", "이번 투표는", "돈을 투자해", "원 플러스 원", "백 원만", "투가 좋아", "시즌 원 투"]
        {
            assert_eq!(d(s), s, "{s}");
        }
    }

    #[test]
    fn words_stay() {
        for s in [
            "이 영웅",
            "이번 시즌",
            "이 시즌",
            "일이 많다",
            "네, 맞아요",
            "한 명",
            "십분 활용",
            "오점",
            "일 위험",
            "오 초반",
            "세 개발자",
            "삼사 위",
            "사위",
            "이 위에",
            "오 프로게이머",
            "일 점령",
            "두개골",
            "일상",
            "세트",
            "위",
            "프로",
        ] {
            assert_eq!(d(s), s, "{s}");
        }
    }

    #[test]
    fn glued_to_previous_word_stays() {
        assert_eq!(d("통일 위"), "통일 위");
        assert_eq!(d("우리팀 오 킬"), "우리팀 5킬");
    }

    #[test]
    fn idempotent_and_odd_input() {
        for s in ["", "오", "십", "만만만", "일 ", "사십칠 퍼센", "5시즌", "47%"] {
            let once = d(s);
            assert_eq!(d(&once), once, "{s}");
        }
    }
}

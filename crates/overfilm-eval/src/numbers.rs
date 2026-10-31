//! Digits to their Korean reading, so `6개` and `여섯 개`, or `1위` and `일 위`, compare equal.
//!
//! Recognizers and caption editors disagree on whether to write a number as digits or as it is
//! spoken; neither is a recognition error. Both sides are therefore turned into the spoken form:
//!
//! - Before a native counter (`개`, `명`, `가지`, `마리`, `살`, `시`, `시간`, `달`, `번째`),
//!   1–99 is read with native numerals (`6개` → `여섯개`, `3명` → `세명`, `20살` → `스무살`).
//! - Otherwise numbers are read in Sino-Korean (`24년` → `이십사년`, `13위` → `십삼위`,
//!   `10000` → `만`); a decimal point reads `점` and the fraction digit by digit (`0.5` → `영점오`).
//! - `%` reads `퍼센트`.
//!
//! `,` between digit groups (`1,000`) is dropped. Numbers longer than [`MAX_DIGITS`] stay as digits.
//! `번` alone is left Sino (`2번` is usually "number two", `이 번`), which mis-reads "two times".

/// The longest integer part spelled out (up to 10^16 - 1).
pub const MAX_DIGITS: usize = 16;

const SINO: [&str; 10] = ["영", "일", "이", "삼", "사", "오", "육", "칠", "팔", "구"];
/// Attributive forms used before a counter (한 개, 두 명, 세 가지, 네 마리).
const NATIVE_ONES_ATTR: [&str; 10] = ["", "한", "두", "세", "네", "다섯", "여섯", "일곱", "여덟", "아홉"];
const NATIVE_TENS: [&str; 10] = ["", "열", "스물", "서른", "마흔", "쉰", "예순", "일흔", "여든", "아흔"];
const COUNTERS: &[&str] = &["번째", "시간", "가지", "마리", "개", "명", "살", "시", "달"];
const GROUPS: [&str; 4] = ["", "만", "억", "조"];

/// Replace every digit run in `text` with its Korean reading (see the module docs).
pub fn spell_numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() * 2);
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c == '%' {
            out.push_str("퍼센트");
            i += 1;
            continue;
        }
        if !c.is_ascii_digit() {
            out.push(c);
            i += 1;
            continue;
        }
        // Integer part, allowing `,` between digits.
        let mut int = String::new();
        while let Some(&d) = chars.get(i) {
            if d.is_ascii_digit() {
                int.push(d);
            } else if d == ',' && chars.get(i + 1).is_some_and(char::is_ascii_digit) && !int.is_empty() {
                // thousands separator
            } else {
                break;
            }
            i += 1;
        }
        let mut frac = String::new();
        if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            i += 1;
            while let Some(&d) = chars.get(i).filter(|d| d.is_ascii_digit()) {
                frac.push(d);
                i += 1;
            }
        }
        // The counter may follow after spaces.
        let mut j = i;
        while chars.get(j).is_some_and(|c| *c == ' ') {
            j += 1;
        }
        let rest: String = chars.iter().skip(j).take(2).collect();
        let native = frac.is_empty() && COUNTERS.iter().any(|c| rest.starts_with(c));
        let reading = if native { native_reading(&int, rest.starts_with("번째")) } else { None };
        match reading.or_else(|| sino_reading(&int)) {
            Some(r) => out.push_str(&r),
            None => out.push_str(&int),
        }
        if !frac.is_empty() {
            out.push('점');
            for d in frac.chars() {
                out.push_str(digit(d).map_or("", |n| SINO.get(n).copied().unwrap_or("")));
            }
        }
    }
    out
}

fn digit(c: char) -> Option<usize> {
    c.to_digit(10).and_then(|d| usize::try_from(d).ok())
}

/// Native reading of 1–99 before a counter; `ordinal` is `번째` (1 reads `첫`).
fn native_reading(int: &str, ordinal: bool) -> Option<String> {
    let n: usize = int.parse().ok()?;
    if n == 0 || n > 99 {
        return None;
    }
    if ordinal && n == 1 {
        return Some("첫".into());
    }
    let (t, o) = (n / 10, n % 10);
    let tens = if t == 2 && o == 0 { "스무" } else { NATIVE_TENS.get(t).copied()? };
    let ones = NATIVE_ONES_ATTR.get(o).copied()?;
    Some(format!("{tens}{ones}"))
}

/// Sino-Korean reading of a non-negative integer of at most [`MAX_DIGITS`] digits.
fn sino_reading(int: &str) -> Option<String> {
    let int = int.trim_start_matches('0');
    if int.is_empty() {
        return Some("영".into());
    }
    if int.len() > MAX_DIGITS {
        return None;
    }
    let digits: Vec<usize> = int.chars().map(digit).collect::<Option<_>>()?;
    let mut out = String::new();
    // Groups of four from the right: (value digits, group index).
    let n = digits.len();
    let groups = n.div_ceil(4);
    for g in (0..groups).rev() {
        let lo = n.saturating_sub((g + 1) * 4);
        let hi = n.saturating_sub(g * 4);
        let part = digits.get(lo..hi)?;
        let mut s = String::new();
        for (k, &d) in part.iter().enumerate() {
            let place = part.len() - 1 - k;
            if d == 0 {
                continue;
            }
            let unit = ["", "십", "백", "천"].get(place).copied()?;
            if !(d == 1 && place > 0) {
                s.push_str(SINO.get(d).copied()?);
            }
            s.push_str(unit);
        }
        if s.is_empty() {
            continue;
        }
        // 10000 reads 만, not 일만.
        if g == 1 && s == "일" {
            s.clear();
        }
        out.push_str(&s);
        out.push_str(GROUPS.get(g).copied()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sino() {
        assert_eq!(spell_numbers("13위"), "십삼위");
        assert_eq!(spell_numbers("24년 7월"), "이십사년 칠월");
        assert_eq!(spell_numbers("1위"), "일위");
        assert_eq!(spell_numbers("10000"), "만");
        assert_eq!(spell_numbers("2,098"), "이천구십팔");
        assert_eq!(spell_numbers("110"), "백십");
        assert_eq!(spell_numbers("0"), "영");
        assert_eq!(spell_numbers("100000001"), "일억일");
    }

    #[test]
    fn native_before_counters() {
        assert_eq!(spell_numbers("6개의 전환"), "여섯개의 전환");
        assert_eq!(spell_numbers("3명으로"), "세명으로");
        assert_eq!(spell_numbers("4개 서버"), "네개 서버");
        assert_eq!(spell_numbers("20살"), "스무살");
        assert_eq!(spell_numbers("21 가지"), "스물한 가지");
        assert_eq!(spell_numbers("1번째"), "첫번째");
        assert_eq!(spell_numbers("100개"), "백개");
    }

    #[test]
    fn percent_and_decimals() {
        assert_eq!(spell_numbers("70%"), "칠십퍼센트");
        assert_eq!(spell_numbers("0.5점"), "영점오점");
        assert_eq!(spell_numbers("v1."), "v일.");
    }

    #[test]
    fn huge_numbers_stay_digits() {
        let s = "1".repeat(MAX_DIGITS + 1);
        assert_eq!(spell_numbers(&s), s);
    }
}

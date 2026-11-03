//! Text normalization shared by every metric.
//!
//! Reference and hypothesis go through the same steps, so formatting differences that a viewer
//! would not call recognition errors do not count: caption markup (`<i>`, `{\an8}`) is dropped,
//! conjoining Hangul jamo are composed into syllables (macOS often writes decomposed Hangul),
//! full-width ASCII becomes half-width, digits become their Korean reading ([`crate::numbers`]:
//! `6개` and `여섯 개` match), letters are lowercased, and every character that is not a letter or
//! digit becomes a word break.

/// Normalized text: the characters that count, in words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Normalized {
    /// Words (Korean eojeol), in order.
    pub words: Vec<String>,
}

impl Normalized {
    /// All characters with word breaks removed (the CER sequence).
    pub fn chars(&self) -> Vec<char> {
        self.words.iter().flat_map(|w| w.chars()).collect()
    }
    /// The text with word breaks removed (the term-matching string).
    pub fn joined(&self) -> String {
        self.words.concat()
    }
}

/// Normalize `text` (see the module docs).
pub fn normalize(text: &str) -> Normalized {
    let stripped = strip_markup(text);
    let composed: String = compose_hangul(&stripped).chars().map(halfwidth).collect();
    let spelled = crate::numbers::spell_numbers(&composed);
    let mut words = Vec::new();
    let mut word = String::new();
    for c in spelled.chars() {
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    Normalized { words }
}

/// Drop `<…>` tags and `{…}` override blocks. An unclosed `<` or `{` is kept as text.
fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(['<', '{']) {
        let (before, from) = rest.split_at(i);
        out.push_str(before);
        let close = if from.starts_with('<') { '>' } else { '}' };
        match from.find(close) {
            Some(j) => {
                out.push(' ');
                rest = from.get(j + close.len_utf8()..).unwrap_or_default();
            }
            None => {
                out.push_str(from);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

/// Compose conjoining jamo (U+1100 block) into precomposed syllables (Unicode §3.12).
fn compose_hangul(text: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(text.len());
    for c in text.chars() {
        let cp = c as u32;
        if let Some(&last) = out.last() {
            let lp = last as u32;
            // L + V -> LV
            if (L_BASE..L_BASE + L_COUNT).contains(&lp)
                && (V_BASE..V_BASE + V_COUNT).contains(&cp)
                && let (Some(slot), Some(syl)) = (out.last_mut(), char::from_u32(S_BASE + ((lp - L_BASE) * V_COUNT + (cp - V_BASE)) * T_COUNT))
            {
                *slot = syl;
                continue;
            }
            // LV + T -> LVT
            if (S_BASE..S_BASE + S_COUNT).contains(&lp)
                && (lp - S_BASE).is_multiple_of(T_COUNT)
                && (T_BASE + 1..T_BASE + T_COUNT).contains(&cp)
                && let (Some(slot), Some(syl)) = (out.last_mut(), char::from_u32(lp + (cp - T_BASE)))
            {
                *slot = syl;
                continue;
            }
        }
        out.push(c);
    }
    out.into_iter().collect()
}

/// Full-width ASCII forms (U+FF01–U+FF5E) and the ideographic space to their ASCII equivalents.
fn halfwidth(c: char) -> char {
    match c as u32 {
        0x3000 => ' ',
        cp @ 0xFF01..=0xFF5E => char::from_u32(cp - 0xFEE0).unwrap_or(c),
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punctuation_and_markup_are_word_breaks() {
        let n = normalize("<i>겐지,</i> 용검!! {\\an8}솔저: 76");
        assert_eq!(n.words, ["겐지", "용검", "솔저", "칠십육"]);
        assert_eq!(n.joined(), "겐지용검솔저칠십육");
    }

    #[test]
    fn decomposed_hangul_matches_precomposed() {
        // "한조" as conjoining jamo: ᄒ ᅡ ᆫ ᄌ ᅩ
        let decomposed = "\u{1112}\u{1161}\u{11AB}\u{110C}\u{1169}";
        assert_eq!(normalize(decomposed), normalize("한조"));
    }

    #[test]
    fn digits_match_their_reading() {
        assert_eq!(normalize("6개의 전환 중에 1위").joined(), normalize("여섯 개의 전환 중에 일 위").joined());
        assert_eq!(normalize("７０%").joined(), "칠십퍼센트");
    }

    #[test]
    fn fullwidth_and_case_fold() {
        assert_eq!(normalize("Ｄ.Ｖａ").words, ["d", "va"]);
    }

    #[test]
    fn unclosed_markup_is_text() {
        assert_eq!(normalize("a < b").words, ["a", "b"]);
        assert_eq!(normalize("{x").words, ["x"]);
    }

    #[test]
    fn empty_and_symbols_only() {
        assert!(normalize("").words.is_empty());
        assert!(normalize("?! … ♪").words.is_empty());
    }
}

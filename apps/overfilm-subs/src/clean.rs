//! Filler clean-up for captions: hesitation sounds (어, 음, 으음…) and stutters (그 그 그게 → 그게,
//! 아니 아니 아니 → 아니) come out of the captions. The transcript keeps every word, so turning the
//! option off and laying out again brings them back.

use overfilm_captions::TimedWord;

/// Words that are only a hesitation sound (punctuation aside).
const FILLERS: &[&str] = &["어", "어어", "어어어", "음", "음음", "으음", "으", "흠", "엄", "에", "에에", "아아", "아아아", "그그"];

/// One-syllable words that are words in their own right even before a word starting with them
/// (`나 나가`, `너 너무`): never taken for a stutter.
const KEEP_SHORT: &[&str] =
    &["나", "너", "내", "네", "난", "넌", "날", "널", "제", "저", "걔", "쟤", "또", "더", "안", "못", "잘", "다", "왜", "뭐", "꼭", "좀"];

/// Longest pause inside a stutter (ms); two words further apart are said on purpose.
const STUTTER_GAP_MS: i64 = 600;

/// The word without the punctuation around it.
fn core(w: &str) -> &str {
    w.trim_matches(|c: char| !c.is_alphanumeric())
}

fn trailing_punct(w: &str) -> &str {
    let keep = w.trim_end_matches(|c: char| !c.is_alphanumeric()).len();
    w.get(keep..).unwrap_or("")
}

pub fn is_filler(w: &str) -> bool {
    FILLERS.contains(&core(w))
}

/// `words` without fillers and stutters. Punctuation a dropped word carried (a sentence end) moves
/// to the word before it, so captions still end there.
pub fn clean(words: &[TimedWord]) -> Vec<TimedWord> {
    let mut out: Vec<TimedWord> = Vec::with_capacity(words.len());
    for (i, w) in words.iter().enumerate() {
        let c = core(&w.text);
        let next = words.get(i + 1);
        let close = |n: &TimedWord| n.start_ms.saturating_sub(w.end_ms) <= STUTTER_GAP_MS;
        let drop = is_filler(&w.text)
            // the same word again right away: keep the last one (it has the punctuation)
            || next.is_some_and(|n| close(n) && !c.is_empty() && core(&n.text) == c && trailing_punct(&w.text).is_empty())
            // a first syllable said before the whole word: 그 그게, 이 이거
            || next.is_some_and(|n| {
                let nc = core(&n.text);
                close(n)
                    && c.chars().count() == 1
                    && nc.chars().count() > 1
                    && nc.starts_with(c)
                    && !KEEP_SHORT.contains(&c)
                    && trailing_punct(&w.text).is_empty()
            });
        if drop {
            let p = trailing_punct(&w.text);
            if let Some(prev) = out.last_mut()
                && !p.is_empty()
                && trailing_punct(&prev.text).is_empty()
            {
                prev.text.push_str(p);
            }
            continue;
        }
        out.push(w.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<TimedWord> {
        text.split_whitespace().enumerate().map(|(i, t)| TimedWord { text: t.into(), start_ms: i as i64 * 300, end_ms: i as i64 * 300 + 250 }).collect()
    }

    fn joined(w: &[TimedWord]) -> String {
        w.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn drops_hesitations_and_stutters() {
        assert_eq!(joined(&clean(&words("어 그러니까 음 이건 으음, 아니 아니 아니 그 그 그게 맞아요"))), "그러니까 이건, 아니 그게 맞아요");
        assert_eq!(joined(&clean(&words("이 이거 보세요 음."))), "이거 보세요.");
    }

    #[test]
    fn keeps_real_words() {
        // pronouns before a word that starts with them, and repeats said apart
        assert_eq!(joined(&clean(&words("나 나가요 너 너무해"))), "나 나가요 너 너무해");
        let mut w = words("가자 가자");
        w[1].start_ms = 2000;
        w[1].end_ms = 2300;
        assert_eq!(joined(&clean(&w)), "가자 가자");
        // a repeat with its own sentence end is two sentences
        assert_eq!(joined(&clean(&words("좋아. 좋아"))), "좋아. 좋아");
        assert_eq!(joined(&clean(&words("에이스 어그로 음식"))), "에이스 어그로 음식");
    }

    #[test]
    fn empty_and_only_fillers() {
        assert!(clean(&[]).is_empty());
        assert!(clean(&words("어 음 으음")).is_empty());
    }
}

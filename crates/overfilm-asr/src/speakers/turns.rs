//! Speaker changes put where the transcript says one turn ends.
//!
//! The recogniser writes one stream of words, and when two people talk at once (or one starts
//! as the other finishes) it writes them turn by turn: the first person's sentence to its end,
//! then the second person's. The forced aligner then squeezes the second person's first words
//! after the first person's last ones, so a speaker change found by time ([`super::label_words`])
//! lands a word or two off: the end of one sentence goes to the next speaker, or the start of the
//! next sentence stays with the first. Here a change that is not at a sentence end moves to the
//! nearest one within [`super::Params::turn_reach`] words, never over a third speaker and never
//! swallowing a whole backchannel ("네" inside someone's sentence).
//!
//! Measured on 12 synthetic Korean conversations with overlapped speech and real transcripts
//! (Zeroth-Korean, 2–4 people): speaker-attributed character errors 19.2 → 16.4 % (plain
//! character errors 14.6 %, so most of the gap from wrong speakers is closed); see `OVERFILM.md`.

use crate::Word;

/// Whether a word ends a sentence (the recogniser's punctuation).
pub fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', '」', '』', '”', '’']).ends_with(['.', '?', '!', '…', '。', '？', '！'])
}

/// Move each speaker change between `words[i - 1]` and `words[i]` that is not at a sentence end
/// to the nearest sentence end within `reach` words, when the words it moves over all belong to
/// the speaker giving them up and both speakers keep a word on their side. Returns how many
/// changes moved. `labels` has one entry per word; `reach` 0 moves nothing.
pub fn settle(words: &[Word], labels: &mut [Option<usize>], reach: usize) -> usize {
    let n = words.len().min(labels.len());
    let mut moved = 0;
    let mut i = 1;
    while i < n {
        let (Some(Some(a)), Some(Some(b))) = (labels.get(i - 1).copied(), labels.get(i).copied()) else {
            i += 1;
            continue;
        };
        let Some(before) = words.get(i - 1) else { break };
        if a == b || ends_sentence(&before.text) {
            i += 1;
            continue;
        }
        // nearest sentence end: a change at j means words[j - 1] ends a sentence
        let ends_at = |j: usize| j >= 1 && j < n && words.get(j - 1).is_some_and(|w| ends_sentence(&w.text));
        // the words moved over all belong to the speaker giving them up, and both speakers keep a
        // word on their side (a backchannel inside someone's sentence stays)
        let left_ok = |j: usize| j >= 1 && labels.get(j - 1..i).is_some_and(|r| r.iter().all(|&l| l == Some(a)));
        let right_ok = |j: usize| labels.get(i..=j).is_some_and(|r| r.iter().all(|&l| l == Some(b)));
        let target = (1..=reach.min(n)).find_map(|d| {
            let l = i.checked_sub(d).filter(|&j| ends_at(j) && left_ok(j));
            let r = i.checked_add(d).filter(|&j| ends_at(j) && right_ok(j));
            l.or(r)
        });
        match target {
            Some(j) if j < i => {
                for l in labels.get_mut(j..i).into_iter().flatten() {
                    *l = Some(b);
                }
                moved += 1;
                i += 1;
            }
            Some(j) => {
                for l in labels.get_mut(i..j).into_iter().flatten() {
                    *l = Some(a);
                }
                moved += 1;
                i = j + 1;
            }
            None => i += 1,
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(t: &[&str]) -> Vec<Word> {
        t.iter().enumerate().map(|(i, s)| Word { text: (*s).into(), start_ms: i as i64 * 300, end_ms: i as i64 * 300 + 250, ..Word::default() }).collect()
    }

    const REACH: usize = 3;

    #[test]
    fn sentence_ends() {
        assert!(ends_sentence("했습니다."));
        assert!(ends_sentence("그래요?"));
        assert!(ends_sentence("와!\""));
        assert!(!ends_sentence("그래서"));
        assert!(!ends_sentence(""));
    }

    #[test]
    fn a_late_change_moves_back_to_the_sentence_end() {
        // A: "보완하기 위한 목적이다." B: "유열씨는 일본…" — by time the change fell after "위한"
        let w = words(&["효과를", "보완하기", "위한", "목적이다.", "유열씨는", "일본"]);
        let mut l = vec![Some(0), Some(0), Some(0), Some(1), Some(1), Some(1)];
        assert_eq!(settle(&w, &mut l, REACH), 1);
        assert_eq!(l, vec![Some(0), Some(0), Some(0), Some(0), Some(1), Some(1)]);
    }

    #[test]
    fn an_early_change_moves_forward() {
        let w = words(&["그녀는", "그", "과정을", "받아들였다.", "평소", "에는"]);
        let mut l = vec![Some(0), Some(0), Some(1), Some(1), Some(1), Some(1)];
        assert_eq!(settle(&w, &mut l, REACH), 1);
        assert_eq!(l, vec![Some(0), Some(0), Some(0), Some(0), Some(1), Some(1)]);
    }

    #[test]
    fn far_sentence_ends_stay() {
        let w = words(&["하나", "둘", "셋", "넷", "다섯.", "여섯"]);
        let mut l = vec![Some(0), Some(1), Some(1), Some(1), Some(1), Some(1)];
        // the sentence end is 4 words away
        assert_eq!(settle(&w, &mut l, REACH), 0);
        assert_eq!(settle(&w, &mut l, 0), 0);
        assert_eq!(l, vec![Some(0), Some(1), Some(1), Some(1), Some(1), Some(1)]);
    }

    #[test]
    fn a_change_already_at_a_sentence_end_stays() {
        let w = words(&["그래요.", "네", "맞아요."]);
        let mut l = vec![Some(0), Some(1), Some(1)];
        assert_eq!(settle(&w, &mut l, REACH), 0);
    }

    #[test]
    fn never_moves_over_a_third_speaker_or_unknown_words() {
        let w = words(&["가.", "나", "다", "라", "마."]);
        let mut l = vec![Some(0), Some(0), Some(2), Some(1), Some(1)];
        settle(&w, &mut l, REACH);
        // the 2 → 1 change has no sentence end it could reach without crossing another speaker
        assert_eq!(l.get(2..), Some(&[Some(2), Some(1), Some(1)][..]));
        let mut l = vec![Some(0), None, Some(0), Some(1), Some(1)];
        settle(&w, &mut l, REACH);
        assert_eq!(l.get(1), Some(&None));
    }

    #[test]
    fn a_backchannel_keeps_its_speaker() {
        // B's "네" inside A's sentence, A's sentence ends right after
        let w = words(&["그래서", "우리가", "네", "갔어요.", "다음"]);
        let mut l = vec![Some(0), Some(0), Some(1), Some(0), Some(0)];
        settle(&w, &mut l, REACH);
        assert_eq!(l.get(2), Some(&Some(1)));
    }

    #[test]
    fn mismatched_lengths_and_empty_input_are_safe() {
        let w = words(&["가.", "나"]);
        let mut l = vec![Some(0)];
        assert_eq!(settle(&w, &mut l, REACH), 0);
        assert_eq!(settle(&[], &mut [], REACH), 0);
        let mut l = vec![Some(0), Some(1)];
        assert_eq!(settle(&w, &mut l, usize::MAX), 0);
    }
}

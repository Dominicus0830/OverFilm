//! Speakers and game voice in a finished transcript: label who spoke each word (화자1, 화자2…),
//! find in-game voice lines (hero ultimates, the announcer) and build captions turn by turn.
//!
//! Game voice is found by text (`packs/overwatch/ko/voicelines.tsv`): a near-exact match of a
//! line of five or more syllables marks its words as game audio. A speaker whose words are mostly
//! voice lines (at least two matches) is a game voice as a whole, and then even short lines and
//! the rest of its words count as game audio.

use overfilm_asr::speakers::{Diarizer, Labels, Params};
use overfilm_asr::{Speaker, Transcript};
use overfilm_captions::{Caption, Layout, TimedWord};
use overfilm_glossary::voicelines::VoiceLines;
use serde::{Deserialize, Serialize};

/// What captions do with in-game voice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameVoice {
    /// Leave it out (the default: captions are for the people speaking).
    #[default]
    Drop,
    /// Keep it, labelled "(게임)".
    Label,
    /// Keep it like any other speech.
    Keep,
}

impl GameVoice {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "drop" | "빼기" => Some(Self::Drop),
            "label" | "표시" => Some(Self::Label),
            "keep" | "그대로" => Some(Self::Keep),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Drop => "자막에서 빼기",
            Self::Label => "(게임)으로 표시",
            Self::Keep => "그대로 두기",
        }
    }
}

/// Label used for game voice turns.
pub const GAME_LABEL: &str = "게임";
/// Match score a long voice line needs on its own.
const SURE: f32 = 0.8;
/// Lines shorter than this (compared characters) must match exactly on their own: "여기까지"
/// said by a caster is one edit from Sojourn's "여기까지다".
const EXACT_BELOW: usize = 8;

fn is_sure(m: &overfilm_glossary::voicelines::Match, lines: &VoiceLines) -> bool {
    let len = lines.lines.get(m.line).map_or(0, |l| l.len());
    !m.short && (m.score >= 1.0 || (len >= EXACT_BELOW && m.score >= SURE))
}

/// Write per-word speakers and the speaker list into `t` (people numbered 화자1, 화자2… by first
/// appearance).
pub fn apply_labels(t: &mut Transcript, labels: &Labels) {
    let mut i = 0;
    for c in &mut t.chunks {
        for w in &mut c.words {
            w.speaker = labels.words.get(i).copied().flatten();
            i += 1;
        }
    }
    t.speakers = labels.speakers.iter().map(|s| Speaker { label: String::new(), seconds: s.seconds, words: s.words, game: false }).collect();
    name_speakers(t);
}

/// Name speakers: people 화자1, 화자2… in order, game voices "게임 음성". Names the user gave
/// (anything not of that form) are kept.
pub fn name_speakers(t: &mut Transcript) {
    let mut n = 0;
    for s in &mut t.speakers {
        let auto = s.label.is_empty() || s.label == "게임 음성" || s.label.strip_prefix("화자").is_some_and(|r| r.parse::<u32>().is_ok());
        if s.game {
            if auto {
                s.label = "게임 음성".into();
            }
        } else {
            n += 1;
            if auto {
                s.label = format!("화자{n}");
            }
        }
    }
}

/// Mark game voice in `t`; returns the number of voice lines found.
pub fn mark_game_voice(t: &mut Transcript, lines: &VoiceLines) -> usize {
    let texts: Vec<String> = t.words().map(|w| w.text.clone()).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let found = lines.find(&refs);
    let mut game = vec![false; texts.len()];
    let mut weak = vec![false; texts.len()];
    for m in &found {
        let sure = is_sure(m, lines);
        for g in game.get_mut(m.start..m.end).into_iter().flatten() {
            *g |= sure;
        }
        for g in weak.get_mut(m.start..m.end).into_iter().flatten() {
            *g = true;
        }
    }
    // speakers made mostly of voice lines are game voices
    let spk: Vec<Option<usize>> = t.words().map(|w| w.speaker).collect();
    for (si, s) in t.speakers.iter_mut().enumerate() {
        let mine: Vec<usize> = (0..spk.len()).filter(|&i| spk.get(i).copied().flatten() == Some(si)).collect();
        let lined = mine.iter().filter(|&&i| weak.get(i).copied().unwrap_or(false)).count();
        let matches = found.iter().filter(|m| spk.get(m.start).copied().flatten() == Some(si)).count();
        s.game = matches >= 2 && lined * 2 >= mine.len();
        if s.game {
            for &i in &mine {
                if let Some(g) = game.get_mut(i) {
                    *g = true;
                }
            }
        }
    }
    let mut i = 0;
    for c in &mut t.chunks {
        for w in &mut c.words {
            w.game = game.get(i).copied().unwrap_or(false);
            i += 1;
        }
    }
    name_speakers(t);
    found.iter().filter(|m| is_sure(m, lines)).count()
}

/// Number of speakers who are people.
pub fn people(t: &Transcript) -> usize {
    t.speakers.iter().filter(|s| !s.game).count()
}

/// Caption turns: consecutive words of one speaker. Labels are given when there are two or more
/// people (or the turn is game voice shown as such); `only` keeps one speaker's words.
pub fn turns(t: &Transcript, game: GameVoice, only: Option<usize>) -> Vec<(Option<String>, Vec<TimedWord>)> {
    let label_people = people(t) > 1 && only.is_none();
    let mut out: Vec<(Option<String>, Vec<TimedWord>)> = Vec::new();
    let mut key: Option<(Option<usize>, bool)> = None;
    for w in t.words() {
        let is_game = w.game && game != GameVoice::Keep;
        if is_game && game == GameVoice::Drop {
            key = None;
            continue;
        }
        if only.is_some() && (w.speaker != only || is_game) {
            key = None;
            continue;
        }
        let k = (w.speaker, is_game);
        if key != Some(k) {
            let label = if is_game {
                Some(GAME_LABEL.to_string())
            } else if label_people {
                w.speaker.and_then(|s| t.speakers.get(s)).map(|s| s.label.clone())
            } else {
                None
            };
            out.push((label, Vec::new()));
            key = Some(k);
        }
        if let Some(last) = out.last_mut() {
            last.1.push(TimedWord { text: w.text.clone(), start_ms: w.start_ms, end_ms: w.end_ms });
        }
    }
    out
}

/// Captions of a transcript.
pub fn captions(t: &Transcript, game: GameVoice, layout: &Layout) -> Vec<Caption> {
    overfilm_captions::layout_turns(&turns(t, game, None), layout)
}

/// One caption list per person (speaker index, label, captions).
pub fn captions_per_speaker(t: &Transcript, layout: &Layout) -> Vec<(usize, String, Vec<Caption>)> {
    t.speakers
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.game)
        .map(|(i, s)| (i, s.label.clone(), overfilm_captions::layout_turns(&turns(t, GameVoice::Drop, Some(i)), layout)))
        .collect()
}

/// Run the speaker network over `audio` and label `t`'s words.
pub fn label(audio: &[f32], t: &mut Transcript, model_dir: &std::path::Path, p: &Params, progress: &mut dyn FnMut(f32) -> bool) -> Result<(), String> {
    let words: Vec<overfilm_asr::Word> = t.words().cloned().collect();
    if words.is_empty() {
        return Ok(());
    }
    let d = Diarizer::load(model_dir).map_err(|e| format!("화자 구별 모델: {e}"))?;
    let labels = d.run(audio, &words, p, progress).map_err(|e| e.to_string())?;
    apply_labels(t, &labels);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use overfilm_asr::speakers::SpeakerStat;
    use overfilm_asr::{ChunkText, Word};

    fn transcript(words: &[(&str, i64, Option<usize>)]) -> Transcript {
        let words: Vec<Word> = words.iter().map(|&(t, s, sp)| Word { text: t.into(), start_ms: s, end_ms: s + 400, speaker: sp, game: false }).collect();
        Transcript { chunks: vec![ChunkText { start_ms: 0, end_ms: 10_000, text: String::new(), words }], ..Transcript::default() }
    }

    #[test]
    fn labels_turns_and_game_voice() {
        let mut t =
            transcript(&[("자", 0, None), ("들어가요", 500, None), ("석양이", 1000, None), ("진다", 1400, None), ("네", 2000, None), ("좋아요", 2400, None)]);
        let labels = Labels {
            words: vec![Some(0), Some(0), Some(2), Some(2), Some(1), Some(1)],
            speakers: (0..3).map(|id| SpeakerStat { id, seconds: 0.8, words: 2 }).collect(),
        };
        apply_labels(&mut t, &labels);
        assert_eq!(t.speakers.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["화자1", "화자2", "화자3"]);
        let lines = VoiceLines::parse("hero\tkind\ttext\n캐서디\tult_enemy\t석양이 진다\n");
        assert_eq!(mark_game_voice(&mut t, &lines), 1);
        // one edit away from a short line is ordinary speech
        let mut near = transcript(&[("여기까지", 0, None)]);
        assert_eq!(mark_game_voice(&mut near, &VoiceLines::parse("hero\tkind\ttext\n소전\tult\t여기까지다\n")), 0);
        assert!(!near.words().any(|w| w.game));
        // one match is not enough to call speaker 3 a game voice, but the words are game audio
        assert!(!t.speakers[2].game);
        assert!(t.words().filter(|w| w.game).count() == 2);
        let lay = Layout::default();
        let caps = captions(&t, GameVoice::Drop, &lay);
        let text: Vec<String> = caps.iter().map(|c| c.lines.join(" ")).collect();
        assert_eq!(text, ["(화자1) 자 들어가요", "(화자2) 네 좋아요"]);
        let shown = captions(&t, GameVoice::Label, &lay);
        assert!(shown.iter().any(|c| c.lines.join(" ") == "(게임) 석양이 진다"));
        let per = captions_per_speaker(&t, &lay);
        assert_eq!(per.len(), 3);
        assert_eq!(per[1].2.len(), 1);
        assert_eq!(per[1].2[0].lines.join(" "), "네 좋아요");
    }

    #[test]
    fn one_person_gets_no_labels_and_renames_survive() {
        let mut t = transcript(&[("하나", 0, Some(0)), ("둘", 500, Some(0))]);
        t.speakers = vec![Speaker { label: "아이치".into(), seconds: 1.0, words: 2, game: false }];
        name_speakers(&mut t);
        assert_eq!(t.speakers[0].label, "아이치");
        let caps = captions(&t, GameVoice::Drop, &Layout::default());
        assert_eq!(caps[0].lines.join(" "), "하나 둘");
        // a transcript without speakers (older files) still lays out
        let plain = transcript(&[("셋", 0, None)]);
        assert_eq!(captions(&plain, GameVoice::Drop, &Layout::default()).len(), 1);
    }

    #[test]
    fn a_speaker_of_voice_lines_is_game_voice() {
        let mut t = transcript(&[
            ("석양이", 0, Some(1)),
            ("진다", 400, Some(1)),
            ("말", 900, Some(0)),
            ("승리", 1500, Some(1)),
            ("석양이", 3000, Some(1)),
            ("진다", 3400, Some(1)),
        ]);
        t.speakers = vec![Speaker::default(), Speaker::default()];
        let lines = VoiceLines::parse("hero\tkind\ttext\n캐서디\tult_enemy\t석양이 진다\n아나운서\tannouncer\t승리\n");
        mark_game_voice(&mut t, &lines);
        assert!(t.speakers[1].game && !t.speakers[0].game);
        assert_eq!(t.speakers[1].label, "게임 음성");
        assert_eq!(t.speakers[0].label, "화자1");
        assert!(t.words().filter(|w| w.speaker == Some(1)).all(|w| w.game));
        assert_eq!(people(&t), 1);
    }
}

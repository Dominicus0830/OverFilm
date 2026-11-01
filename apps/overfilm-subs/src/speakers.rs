//! Speakers and game voice in a finished transcript: label who spoke each word (화자1, 화자2…),
//! find in-game voice lines (hero ultimates, the announcer) and build captions turn by turn.
//!
//! Game voice is found by text (`packs/overwatch/ko/voicelines.tsv`): a near-exact match of a
//! line of five or more syllables marks its words as game audio. A speaker whose words are mostly
//! voice lines (at least two matches) is a game voice as a whole, and then even short lines and
//! the rest of its words count as game audio.

use overfilm_asr::speakers::overlap::OverlapParams;
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

/// Line kind for communication-wheel lines ("방어구가 필요해"): people say them too, so they only
/// count towards a speaker being game voice, never on their own.
const COMM: &str = "comm";

/// Exact, or a long line nearly said.
fn is_close(m: &overfilm_glossary::voicelines::Match, lines: &VoiceLines) -> bool {
    let len = lines.lines.get(m.line).map_or(0, |l| l.len());
    m.score >= 1.0 || (len >= EXACT_BELOW && m.score >= SURE)
}

/// Whether a line alone marks its words as game audio: not a communication-wheel line, and
/// either an ultimate / announcer / skin line or long. Short everyday lines ("아니거든요",
/// "감사합니다") are said by people too and only count towards a speaker.
fn is_sure(m: &overfilm_glossary::voicelines::Match, lines: &VoiceLines) -> bool {
    let Some(l) = lines.lines.get(m.line) else { return false };
    let signature = l.kind.starts_with("ult") || l.kind == "announcer" || l.kind == "other";
    l.kind != COMM && (signature || l.len() >= EXACT_BELOW) && !m.short && is_close(m, lines)
}

/// Write per-word speakers and the speaker list into `t` (people numbered 화자1, 화자2… by first
/// appearance).
pub fn apply_labels(t: &mut Transcript, labels: &Labels) {
    let mut i = 0;
    for c in &mut t.chunks {
        for w in &mut c.words {
            w.speaker = labels.words.get(i).copied().flatten();
            w.also = labels.also.get(i).copied().flatten().filter(|&a| Some(a) != w.speaker);
            i += 1;
        }
    }
    t.speakers = labels
        .speakers
        .iter()
        .map(|s| Speaker { label: s.name.clone().unwrap_or_default(), seconds: s.seconds, words: s.words, game: false, enrolled: s.name.is_some() })
        .collect();
    name_speakers(t);
}

/// Name speakers: unnamed people get the next free 화자N, game voices "게임 음성". Existing names
/// (numbers given earlier, names the user gave, enrolled names) stay, so renaming one speaker
/// never renumbers the others.
pub fn name_speakers(t: &mut Transcript) {
    const GAME: &str = "게임 음성";
    let mut used: Vec<u32> = t.speakers.iter().filter_map(|s| s.label.strip_prefix("화자").and_then(|r| r.parse().ok())).collect();
    for s in &mut t.speakers {
        let numbered = s.label.strip_prefix("화자").is_some_and(|r| r.parse::<u32>().is_ok());
        if s.game {
            if s.label.is_empty() || numbered {
                if let Some(n) = s.label.strip_prefix("화자").and_then(|r| r.parse::<u32>().ok()) {
                    used.retain(|&u| u != n);
                }
                s.label = GAME.into();
            }
        } else if s.label.is_empty() || s.label == GAME {
            let n = (1..).find(|n| !used.contains(n)).unwrap_or(1);
            used.push(n);
            s.label = format!("화자{n}");
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
    // near misses of short lines ("하겠습니다" for "알겠습니다") are ordinary speech
    let found: Vec<_> = found.into_iter().filter(|m| is_close(m, lines)).collect();
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
    let also: Vec<Option<usize>> = t.words().map(|w| w.also).collect();
    for (si, s) in t.speakers.iter_mut().enumerate() {
        let mine: Vec<usize> = (0..spk.len()).filter(|&i| spk.get(i).copied().flatten() == Some(si)).collect();
        let lined = mine.iter().filter(|&&i| weak.get(i).copied().unwrap_or(false)).count();
        let matches = found.iter().filter(|m| spk.get(m.start).copied().flatten() == Some(si)).count();
        s.game = matches >= 2 && lined * 2 >= mine.len();
    }
    let is_game_voice = |s: Option<usize>, t: &Transcript| s.and_then(|s| t.speakers.get(s)).is_some_and(|s| s.game);
    // a person talking over a game voice: words that are no voice line are the person's
    let mut swap = vec![false; spk.len()];
    for i in 0..spk.len() {
        let (s, a) = (spk.get(i).copied().flatten(), also.get(i).copied().flatten());
        if is_game_voice(s, t) {
            if a.is_some() && !is_game_voice(a, t) && !weak.get(i).copied().unwrap_or(false) {
                if let Some(x) = swap.get_mut(i) {
                    *x = true;
                }
            } else if let Some(g) = game.get_mut(i) {
                *g = true;
            }
        }
    }
    let mut i = 0;
    for c in &mut t.chunks {
        for w in &mut c.words {
            w.game = game.get(i).copied().unwrap_or(false);
            if swap.get(i).copied().unwrap_or(false) {
                std::mem::swap(&mut w.speaker, &mut w.also);
            }
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

/// Which speakers' words captions keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    All,
    /// Only voices the user enrolled (e.g. the streamer).
    Enrolled,
    /// One speaker (per-speaker files).
    One(usize),
}

/// Caption turns: consecutive words of one speaker. Labels are given when two or more people are
/// shown (or the turn is game voice shown as such).
pub fn turns(t: &Transcript, game: GameVoice, keep: Keep) -> Vec<(Option<String>, Vec<TimedWord>)> {
    let shown = |s: Option<usize>| match keep {
        Keep::All => true,
        Keep::Enrolled => s.and_then(|s| t.speakers.get(s)).is_some_and(|s| s.enrolled && !s.game),
        Keep::One(i) => s == Some(i),
    };
    let shown_people = t.speakers.iter().enumerate().filter(|(i, s)| !s.game && shown(Some(*i))).count();
    let label_people = shown_people > 1 && !matches!(keep, Keep::One(_));
    let only = matches!(keep, Keep::One(_) | Keep::Enrolled);
    let mut out: Vec<(Option<String>, Vec<TimedWord>)> = Vec::new();
    let mut key: Option<(Option<usize>, Option<usize>, bool)> = None;
    let name = |s: Option<usize>| s.and_then(|s| t.speakers.get(s)).filter(|s| !s.game).map(|s| s.label.clone());
    let overlaps = steady_overlaps(t);
    for (w, &w_also) in t.words().zip(&overlaps) {
        let is_game = w.game && game != GameVoice::Keep;
        if is_game && game == GameVoice::Drop {
            key = None;
            continue;
        }
        // a word said over someone else's is kept for either of the two
        let also = w_also.filter(|&a| shown(Some(a)) && name(Some(a)).is_some());
        if !(shown(w.speaker) || also.is_some()) || (only && is_game) {
            key = None;
            continue;
        }
        let k = (w.speaker, if label_people { also } else { None }, is_game);
        if key != Some(k) {
            let label = if is_game {
                Some(GAME_LABEL.to_string())
            } else if label_people {
                match (name(w.speaker), name(also)) {
                    (Some(a), Some(b)) => Some(format!("{a}·{b}")),
                    (a, b) => a.or(b),
                }
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

/// Overlaps shorter than this (or of one word) do not change captions: a blip of cross-talk
/// would only break a turn apart.
pub const MIN_OVERLAP_MS: i64 = 1000;

/// Each word's second speaker where it is part of a steady overlap (at least two words and
/// [`MIN_OVERLAP_MS`] with the same pair of speakers).
pub fn steady_overlaps(t: &Transcript) -> Vec<Option<usize>> {
    let words: Vec<&overfilm_asr::Word> = t.words().collect();
    let mut out = vec![None; words.len()];
    let mut i = 0;
    while i < words.len() {
        let key = words.get(i).map(|w| (w.speaker, w.also));
        let mut j = i + 1;
        while j < words.len() && words.get(j).map(|w| (w.speaker, w.also)) == key {
            j += 1;
        }
        let span = match (words.get(i), words.get(j - 1)) {
            (Some(a), Some(b)) => b.end_ms.saturating_sub(a.start_ms),
            _ => 0,
        };
        if let Some((_, Some(a))) = key
            && j - i >= 2
            && span >= MIN_OVERLAP_MS
        {
            out.iter_mut().take(j).skip(i).for_each(|x| *x = Some(a));
        }
        i = j;
    }
    out
}

/// Captions of a transcript.
pub fn captions(t: &Transcript, game: GameVoice, layout: &Layout) -> Vec<Caption> {
    captions_keep(t, game, Keep::All, layout)
}

/// Captions of the speakers `keep` selects.
pub fn captions_keep(t: &Transcript, game: GameVoice, keep: Keep, layout: &Layout) -> Vec<Caption> {
    overfilm_captions::layout_turns(&turns(t, game, keep), layout)
}

/// One caption list per person (speaker index, label, captions).
pub fn captions_per_speaker(t: &Transcript, layout: &Layout) -> Vec<(usize, String, Vec<Caption>)> {
    t.speakers
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.game)
        .map(|(i, s)| (i, s.label.clone(), overfilm_captions::layout_turns(&turns(t, GameVoice::Drop, Keep::One(i)), layout)))
        .collect()
}

/// Run the speaker network over `audio` (on the GPU when `gpu` and one matches the CPU) and
/// label `t`'s words. Returns where the network ran.
pub fn label(
    audio: &[f32],
    t: &mut Transcript,
    model_dir: &std::path::Path,
    p: &Params,
    gpu: bool,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<String, String> {
    let words: Vec<overfilm_asr::Word> = t.words().cloned().collect();
    if words.is_empty() {
        return Ok(String::new());
    }
    let mut d = Diarizer::load(model_dir).map_err(|e| format!("화자 구별 모델: {e}"))?;
    if gpu {
        d.use_gpu();
    }
    // with the overlap model (models/<segment id> next to the speaker model) the overlapped
    // frames stay out of the voices and each word learns who else talks; without it, or if that
    // fails, plain labels
    let seg = &overfilm_asr::catalogue::SEGMENT;
    let net = model_dir.parent().filter(|m| seg.installed(m)).and_then(|m| overfilm_asr::speakers::seg::SegNet::load(&seg.dir(m).join("model.onnx")).ok());
    let with = match &net {
        Some(net) => match d.run_with_overlaps(net, audio, &words, p, &OverlapParams::default(), progress) {
            Ok(l) => Some(l),
            Err(overfilm_asr::AsrError::Cancelled) => return Err("취소됨".into()),
            Err(_) => None,
        },
        None => None,
    };
    let labels = match with {
        Some(l) => l,
        None => d.run(audio, &words, p, progress).map_err(|e| e.to_string())?,
    };
    apply_labels(t, &labels);
    Ok(d.device.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use overfilm_asr::speakers::SpeakerStat;
    use overfilm_asr::{ChunkText, Word};

    fn transcript(words: &[(&str, i64, Option<usize>)]) -> Transcript {
        let words: Vec<Word> = words.iter().map(|&(t, s, sp)| Word { text: t.into(), start_ms: s, end_ms: s + 400, speaker: sp, ..Word::default() }).collect();
        Transcript { chunks: vec![ChunkText { start_ms: 0, end_ms: 10_000, text: String::new(), words }], ..Transcript::default() }
    }

    #[test]
    fn labels_turns_and_game_voice() {
        let mut t =
            transcript(&[("자", 0, None), ("들어가요", 500, None), ("석양이", 1000, None), ("진다", 1400, None), ("네", 2000, None), ("좋아요", 2400, None)]);
        let labels = Labels {
            words: vec![Some(0), Some(0), Some(2), Some(2), Some(1), Some(1)],
            speakers: (0..3).map(|id| SpeakerStat { id, seconds: 0.8, words: 2, name: (id == 1).then(|| "스트리머".into()) }).collect(),
            ..Labels::default()
        };
        apply_labels(&mut t, &labels);
        assert_eq!(t.speakers.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["화자1", "스트리머", "화자2"]);
        assert!(t.speakers[1].enrolled);
        let lines = VoiceLines::parse("hero\tkind\ttext\n캐서디\tult_enemy\t석양이 진다\n");
        assert_eq!(mark_game_voice(&mut t, &lines), 1);
        // one edit away from a short line is ordinary speech
        let mut near = transcript(&[("여기까지", 0, None)]);
        assert_eq!(mark_game_voice(&mut near, &VoiceLines::parse("hero\tkind\ttext\n소전\tult\t여기까지다\n")), 0);
        assert!(!near.words().any(|w| w.game));
        // a communication-wheel line said by a person stays theirs
        let mut comm = transcript(&[("방어구가", 0, Some(0)), ("필요해요", 500, Some(0))]);
        comm.speakers = vec![Speaker::default()];
        assert_eq!(mark_game_voice(&mut comm, &VoiceLines::parse("hero\tkind\ttext\n키리코\tcomm\t방어구가 필요해요\n")), 0);
        assert!(!comm.words().any(|w| w.game));
        // so is a short everyday line, but not a short ultimate line
        let mut short = transcript(&[("아니거든요", 0, Some(0))]);
        short.speakers = vec![Speaker::default()];
        assert_eq!(mark_game_voice(&mut short, &VoiceLines::parse("hero\tkind\ttext\n시메트라\tmatchtalk\t아니거든요\n")), 0);
        // one match is not enough to call speaker 3 a game voice, but the words are game audio
        assert!(!t.speakers[2].game);
        assert!(t.words().filter(|w| w.game).count() == 2);
        let lay = Layout::default();
        let caps = captions(&t, GameVoice::Drop, &lay);
        let text: Vec<String> = caps.iter().map(|c| c.lines.join(" ")).collect();
        assert_eq!(text, ["(화자1) 자 들어가요", "(스트리머) 네 좋아요"]);
        // only the enrolled voice, without a label
        let mine = captions_keep(&t, GameVoice::Drop, Keep::Enrolled, &lay);
        assert_eq!(mine.iter().map(|c| c.lines.join(" ")).collect::<Vec<_>>(), ["네 좋아요"]);
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
        t.speakers = vec![Speaker { label: "아이치".into(), seconds: 1.0, words: 2, ..Speaker::default() }];
        name_speakers(&mut t);
        assert_eq!(t.speakers[0].label, "아이치");
        let caps = captions(&t, GameVoice::Drop, &Layout::default());
        assert_eq!(caps[0].lines.join(" "), "하나 둘");
        // a transcript without speakers (older files) still lays out
        let plain = transcript(&[("셋", 0, None)]);
        assert_eq!(captions(&plain, GameVoice::Drop, &Layout::default()).len(), 1);
    }

    #[test]
    fn overlapped_words_carry_both_speakers() {
        let mut t = transcript(&[("자", 0, None), ("들어가", 500, None), ("아니", 1000, None), ("잠깐", 1500, None), ("만요", 2000, None), ("네", 3000, None)]);
        let labels = Labels {
            words: vec![Some(0), Some(0), Some(0), Some(0), Some(0), Some(1)],
            speakers: (0..2).map(|id| SpeakerStat { id, seconds: 1.0, words: 3, ..Default::default() }).collect(),
            also: vec![None, None, Some(1), Some(1), Some(1), Some(1)],
        };
        apply_labels(&mut t, &labels);
        // the second speaker never repeats the word's own
        assert_eq!(t.words().map(|w| w.also).collect::<Vec<_>>(), [None, None, Some(1), Some(1), Some(1), None]);
        let text: Vec<String> = captions(&t, GameVoice::Drop, &Layout::default()).iter().map(|c| c.lines.join(" ")).collect();
        assert_eq!(text, ["(화자1) 자 들어가", "(화자1·화자2) 아니 잠깐 만요", "(화자2) 네"]);
        // speaker 2's own file keeps the words said over speaker 1
        let per = captions_per_speaker(&t, &Layout::default());
        assert_eq!(per[1].2.iter().map(|c| c.lines.join(" ")).collect::<Vec<_>>().join(" "), "아니 잠깐 만요 네");
        // a one-word blip of cross-talk changes nothing
        let mut blip = transcript(&[("자", 0, Some(0)), ("들어가", 500, Some(0)), ("네", 3000, Some(1))]);
        blip.chunks[0].words[1].also = Some(1);
        blip.speakers = vec![Speaker::default(), Speaker::default()];
        name_speakers(&mut blip);
        assert_eq!(steady_overlaps(&blip), vec![None; 3]);
        assert_eq!(captions(&blip, GameVoice::Drop, &Layout::default())[0].lines.join(" "), "(화자1) 자 들어가");
    }

    #[test]
    fn a_person_over_a_game_voice_keeps_their_words() {
        let mut t = transcript(&[
            ("석양이", 0, Some(1)),
            ("진다", 400, Some(1)),
            ("말", 900, Some(0)),
            ("석양이", 3000, Some(1)),
            ("진다", 3400, Some(1)),
            ("좋아", 3800, Some(1)),
        ]);
        t.chunks[0].words[5].also = Some(0);
        t.speakers = vec![Speaker::default(), Speaker::default()];
        mark_game_voice(&mut t, &VoiceLines::parse("hero\tkind\ttext\n캐서디\tult_enemy\t석양이 진다\n"));
        assert!(t.speakers[1].game);
        let w = t.words().nth(5).unwrap();
        assert_eq!((w.speaker, w.also, w.game), (Some(0), Some(1), false));
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

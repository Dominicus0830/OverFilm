//! Changes the window makes to a finished result, and the small bits of state behind its screens:
//! video presets, progress stages, speaker renames and merges, a voice sample picked from a
//! speaker's own words, glossary fixes, and the recent-results list.

use std::path::{Path, PathBuf};

use overfilm_asr::Transcript;
use overfilm_captions::Caption;
use serde::{Deserialize, Serialize};

use crate::job::{self, Options};
use crate::pack::Mode;
use crate::speakers::{self, GameVoice, Keep};

/// What kind of video this is. Picking one sets the options that suit it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Playing alone or with friends: people counted automatically.
    #[default]
    General,
    /// A streamer's broadcast: only the enrolled voices get captions.
    Streamer,
    /// Tournament commentary: team and player names, one SRT per caster.
    Commentary,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::General, Preset::Streamer, Preset::Commentary];

    pub fn label(self) -> &'static str {
        match self {
            Preset::General => "일반 게임 영상",
            Preset::Streamer => "스트리머 방송",
            Preset::Commentary => "대회 해설",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Preset::General => "혼자 또는 친구와 플레이. 말하는 사람은 자동으로 셉니다.",
            Preset::Streamer => "등록한 목소리의 말만 자막으로. 게임 소리와 다른 사람은 뺍니다.",
            Preset::Commentary => "팀·선수 이름을 인식에 넣고, 해설자마다 SRT를 따로 저장합니다.",
        }
    }

    /// Set the options this kind of video needs; the rest (line length, masking…) stay.
    pub fn apply(self, o: &mut Options) {
        o.speakers = true;
        o.game_voice = GameVoice::Drop;
        match self {
            Preset::General => {
                o.mode = Mode::General;
                o.per_speaker = false;
                o.only_voices = false;
            }
            Preset::Streamer => {
                o.mode = Mode::General;
                o.per_speaker = false;
                o.only_voices = true;
            }
            Preset::Commentary => {
                o.mode = Mode::Commentary;
                o.per_speaker = true;
                o.only_voices = false;
            }
        }
    }
}

/// The steps of one extraction as the window lists them, with where each starts on the shared
/// progress bar of [`job::run`].
pub const STAGES: [(&str, f32); 5] =
    [("모델 준비", 0.0), ("소리 읽기", 0.05), ("음성 인식", 0.10), ("목소리 구별, 겹친 말", 0.90), ("단어집 교정, 저장", 0.98)];

/// Which stage `progress` (0–1) is in, and how far through it (0–1).
pub fn stage(progress: f32) -> (usize, f32) {
    let p = if progress.is_finite() { progress.clamp(0.0, 1.0) } else { 0.0 };
    let mut i = 0;
    for (k, (_, start)) in STAGES.iter().enumerate() {
        if p >= *start {
            i = k;
        }
    }
    let start = STAGES.get(i).map_or(0.0, |s| s.1);
    let end = STAGES.get(i + 1).map_or(1.0, |s| s.1);
    let within = if end > start { ((p - start) / (end - start)).clamp(0.0, 1.0) } else { 1.0 };
    (i, within)
}

/// A finished result opened in the window: the SRT and, when there is one, the transcript beside
/// it (word times and speakers).
#[derive(Clone, Debug)]
pub struct Opened {
    pub srt: PathBuf,
    pub transcript: Option<Transcript>,
    pub captions: Vec<Caption>,
}

impl Opened {
    pub fn load(srt: &Path) -> Result<Self, String> {
        let captions = job::read_srt(srt)?;
        let tpath = job::transcript_path(srt);
        let transcript = if tpath.is_file() { Some(read_transcript(&tpath)?) } else { None };
        Ok(Self { srt: srt.to_path_buf(), transcript, captions })
    }

    /// Captions again from the transcript (after a speaker change), written over the SRT (a
    /// `.srt.bak` is kept the first time), per-speaker files refreshed when asked, transcript saved.
    pub fn rebuild(&mut self, o: &Options) -> Result<(), String> {
        let Some(t) = &self.transcript else { return Err("단어 시간 파일(.overfilm.json)이 없어 다시 만들 수 없습니다".into()) };
        let keep = if o.only_voices && t.speakers.iter().any(|s| s.enrolled && !s.game) { Keep::Enrolled } else { Keep::All };
        let caps = speakers::captions_keep(t, o.game_voice, keep, &o.layout());
        save_srt(&self.srt, &caps)?;
        self.captions = caps;
        if o.per_speaker {
            self.write_per_speaker(o)?;
        }
        self.save_transcript()
    }

    /// One SRT per person beside the main one (`이름.화자1.srt`…); returns the files written.
    pub fn write_per_speaker(&self, o: &Options) -> Result<Vec<PathBuf>, String> {
        let Some(t) = &self.transcript else { return Ok(Vec::new()) };
        if speakers::people(t) < 2 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for (_, label, caps) in speakers::captions_per_speaker(t, &o.layout()) {
            let p = job::per_speaker_path(&self.srt, &label);
            job::write_srt(&p, &caps)?;
            out.push(p);
        }
        Ok(out)
    }

    fn save_transcript(&self) -> Result<(), String> {
        let Some(t) = &self.transcript else { return Ok(()) };
        let tpath = job::transcript_path(&self.srt);
        let json = serde_json::to_string_pretty(t).map_err(|e| e.to_string())?;
        std::fs::write(&tpath, json).map_err(|e| format!("{}: {e}", tpath.display()))
    }

    /// Put back an earlier state (undo) and save it.
    pub fn restore(&mut self, captions: Vec<Caption>, transcript: Option<Transcript>) -> Result<(), String> {
        save_srt(&self.srt, &captions)?;
        self.captions = captions;
        if transcript.is_some() {
            self.transcript = transcript;
            self.save_transcript()?;
        }
        Ok(())
    }

    /// Replace caption `index` (0-based) with `text` (lines split at `\n`) and save.
    pub fn set_text(&mut self, index: usize, text: &str) -> Result<(), String> {
        let c = self.captions.get_mut(index).ok_or("자막 번호가 없습니다")?;
        let lines: Vec<String> = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        if lines.is_empty() {
            return Err("빈 자막은 저장하지 않습니다".into());
        }
        c.lines = lines;
        save_srt(&self.srt, &self.captions)
    }

    /// Apply the glossary to every caption; returns the number of changes.
    pub fn fix_all(&mut self, corrector: &overfilm_glossary::Corrector) -> Result<usize, String> {
        let mut n = 0;
        for c in &mut self.captions {
            for l in &mut c.lines {
                let f = corrector.apply(l);
                n += f.changes.len();
                *l = f.text;
            }
        }
        if n > 0 {
            save_srt(&self.srt, &self.captions)?;
        }
        Ok(n)
    }
}

fn read_transcript(path: &Path) -> Result<Transcript, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > 256 << 20 {
        return Err(format!("{}: 파일이 너무 큽니다", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write `caps`, keeping the first version as `.srt.bak`.
pub fn save_srt(path: &Path, caps: &[Caption]) -> Result<(), String> {
    let bak = path.with_extension("srt.bak");
    if !bak.exists() && path.exists() {
        std::fs::copy(path, &bak).map_err(|e| format!("백업 실패 {}: {e}", bak.display()))?;
    }
    job::write_srt(path, caps)
}

/// Give speaker `idx` a new name. A name another speaker already has merges the two (the user
/// is saying they are the same person).
pub fn rename_speaker(t: &mut Transcript, idx: usize, name: &str) -> Result<(), String> {
    let name: String = name.trim().chars().take(20).collect();
    if name.is_empty() {
        return Err("이름이 비어 있습니다".into());
    }
    if let Some(other) = t.speakers.iter().enumerate().position(|(k, s)| k != idx && s.label == name) {
        return merge_speakers(t, idx, other);
    }
    let s = t.speakers.get_mut(idx).ok_or("그런 화자가 없습니다")?;
    s.label = name;
    Ok(())
}

/// Speaker `from` is really `into`: move its words over and drop it from the list.
pub fn merge_speakers(t: &mut Transcript, from: usize, into: usize) -> Result<(), String> {
    if from == into {
        return Ok(());
    }
    let n = t.speakers.len();
    if from >= n || into >= n {
        return Err("그런 화자가 없습니다".into());
    }
    let (secs, words) = t.speakers.get(from).map(|s| (s.seconds, s.words)).unwrap_or_default();
    if let Some(s) = t.speakers.get_mut(into) {
        s.seconds += secs;
        s.words = s.words.saturating_add(words);
    }
    let shift = |i: usize| if i > from { i - 1 } else { i };
    let into_after = shift(into);
    for c in &mut t.chunks {
        for w in &mut c.words {
            w.speaker = w.speaker.map(|s| if s == from { into_after } else { shift(s) });
            w.also = w.also.map(|s| if s == from { into_after } else { shift(s) }).filter(|&a| Some(a) != w.speaker);
        }
    }
    t.speakers.remove(from);
    Ok(())
}

/// Mark speaker `idx` as game voice (hero lines) or as a person again.
pub fn set_game(t: &mut Transcript, idx: usize, game: bool) -> Result<(), String> {
    let s = t.speakers.get_mut(idx).ok_or("그런 화자가 없습니다")?;
    s.game = game;
    if !game && s.label == "게임 음성" {
        s.label.clear();
    }
    for c in &mut t.chunks {
        for w in c.words.iter_mut().filter(|w| w.speaker == Some(idx)) {
            w.game = game;
        }
    }
    speakers::name_speakers(t);
    Ok(())
}

/// Shortest and longest voice sample to enrol, and the pause that ends a stretch of talk.
const SAMPLE_MIN_MS: i64 = 5_000;
const SAMPLE_MAX_MS: i64 = 30_000;
const SAMPLE_GAP_MS: i64 = 1_500;

/// The longest stretch where only speaker `idx` talks (no one over them, no game voice, no long
/// pause), at most 30 s: a clean sample to enrol their voice from. `None` under 5 s.
pub fn solo_span(t: &Transcript, idx: usize) -> Option<(u64, u64)> {
    let mut best: Option<(i64, i64)> = None;
    let mut run: Option<(i64, i64)> = None;
    let consider = |r: Option<(i64, i64)>, best: &mut Option<(i64, i64)>| {
        if let Some((a, b)) = r {
            let b = b.min(a.saturating_add(SAMPLE_MAX_MS));
            if best.is_none_or(|(x, y)| b - a > y - x) {
                *best = Some((a, b));
            }
        }
    };
    for w in t.words() {
        let clean = w.speaker == Some(idx) && !w.game && w.also.is_none() && w.end_ms >= w.start_ms;
        match (clean, run) {
            (true, Some((a, b))) if w.start_ms.saturating_sub(b) <= SAMPLE_GAP_MS => run = Some((a, w.end_ms.max(b))),
            (true, _) => {
                consider(run, &mut best);
                run = Some((w.start_ms, w.end_ms));
            }
            (false, _) => {
                consider(run, &mut best);
                run = None;
            }
        }
    }
    consider(run, &mut best);
    let (a, b) = best.filter(|(a, b)| b - a >= SAMPLE_MIN_MS)?;
    Some((u64::try_from(a).ok()?, u64::try_from(b).ok()?))
}

/// The speaker each caption belongs to, read from the `(이름)` the captions start a turn with;
/// captions without one continue the turn before. The text is returned without the label.
pub fn caption_speakers(caps: &[Caption]) -> Vec<(Option<String>, String)> {
    let mut current: Option<String> = None;
    caps.iter()
        .map(|c| {
            let text = c.text();
            let trimmed = text.trim_start();
            if let Some(rest) = trimmed.strip_prefix('(')
                && let Some(end) = rest.find(')')
                && end > 0
                && end <= 60
            {
                let (label, after) = rest.split_at(end);
                current = Some(label.to_string());
                return (current.clone(), after.trim_start_matches(')').trim_start().to_string());
            }
            (current.clone(), text)
        })
        .collect()
}

/// One entry of the recent list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    pub input: PathBuf,
    pub srt: PathBuf,
    #[serde(default)]
    pub preset: Preset,
    pub captions: usize,
    /// People found (0 when speakers were not told apart).
    #[serde(default)]
    pub people: usize,
    /// Review findings still open when last seen.
    #[serde(default)]
    pub open_issues: usize,
}

pub const RECENT_MAX: usize = 20;

/// Where the window keeps its own files (recent list, preferences): next to the models folder.
pub fn data_dir() -> PathBuf {
    job::models_dir().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

pub fn load_recent(path: &Path) -> Vec<Recent> {
    let Ok(meta) = std::fs::metadata(path) else { return Vec::new() };
    if meta.len() > 1 << 20 {
        return Vec::new();
    }
    let mut list: Vec<Recent> = std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    list.truncate(RECENT_MAX);
    list
}

/// Put `r` first (replacing an older entry for the same SRT) and save.
pub fn push_recent(path: &Path, list: &mut Vec<Recent>, r: Recent) -> Result<(), String> {
    list.retain(|x| x.srt != r.srt);
    list.insert(0, r);
    save_recent(path, list)
}

/// Write the recent list (at most [`RECENT_MAX`] entries).
pub fn save_recent(path: &Path, list: &mut Vec<Recent>) -> Result<(), String> {
    list.truncate(RECENT_MAX);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use overfilm_asr::{ChunkText, Speaker, Word};

    fn w(text: &str, start: i64, end: i64, sp: usize) -> Word {
        Word { text: text.into(), start_ms: start, end_ms: end, speaker: Some(sp), game: false, also: None }
    }

    fn transcript(words: Vec<Word>, labels: &[&str]) -> Transcript {
        Transcript {
            language: "Korean".into(),
            device: "CPU".into(),
            chunks: vec![ChunkText { start_ms: 0, end_ms: 60_000, text: String::new(), words }],
            speakers: labels.iter().map(|l| Speaker { label: (*l).into(), seconds: 1.0, words: 1, game: false, enrolled: false }).collect(),
        }
    }

    #[test]
    fn presets_set_what_the_video_needs() {
        let mut o = Options::new(PathBuf::from("a.mp4"));
        o.line_chars = 18;
        Preset::Commentary.apply(&mut o);
        assert_eq!((o.mode, o.per_speaker, o.only_voices), (Mode::Commentary, true, false));
        Preset::Streamer.apply(&mut o);
        assert_eq!((o.mode, o.per_speaker, o.only_voices), (Mode::General, false, true));
        Preset::General.apply(&mut o);
        assert!(!o.only_voices && o.speakers);
        assert_eq!(o.line_chars, 18, "presets leave the caption shape alone");
    }

    #[test]
    fn stages_follow_the_progress_bar() {
        assert_eq!(stage(0.0).0, 0);
        assert_eq!(stage(0.07).0, 1);
        let (i, f) = stage(0.5);
        assert_eq!(i, 2);
        assert!((f - 0.5).abs() < 1e-3);
        assert_eq!(stage(0.95).0, 3);
        assert_eq!(stage(1.0), (4, 1.0));
        assert_eq!(stage(f32::NAN).0, 0);
        assert_eq!(stage(7.0).0, 4);
    }

    #[test]
    fn merging_moves_words_and_renumbers() {
        let mut t = transcript(vec![w("가", 0, 500, 0), w("나", 600, 900, 1), w("다", 1000, 1500, 2)], &["화자1", "화자2", "화자3"]);
        if let Some(x) = t.chunks.get_mut(0).and_then(|c| c.words.get_mut(0)) {
            x.also = Some(1);
        }
        merge_speakers(&mut t, 1, 0).unwrap();
        let sp: Vec<_> = t.words().map(|w| w.speaker).collect();
        assert_eq!(sp, vec![Some(0), Some(0), Some(1)]);
        assert_eq!(t.words().next().unwrap().also, None, "talking over yourself is not overlap");
        assert_eq!(t.speakers.len(), 2);
        assert_eq!(t.speakers[1].label, "화자3");
        assert_eq!(t.speakers[0].words, 2);
        assert!(merge_speakers(&mut t, 5, 0).is_err());
    }

    #[test]
    fn renaming_to_a_taken_name_merges() {
        let mut t = transcript(vec![w("가", 0, 500, 0), w("나", 600, 900, 1)], &["아이치", "화자2"]);
        rename_speaker(&mut t, 1, " 아이치 ").unwrap();
        assert_eq!(t.speakers.len(), 1);
        assert!(t.words().all(|w| w.speaker == Some(0)));
        assert!(rename_speaker(&mut t, 0, "  ").is_err());
        rename_speaker(&mut t, 0, "도현").unwrap();
        assert_eq!(t.speakers[0].label, "도현");
    }

    #[test]
    fn game_voice_toggles_back_to_a_person() {
        let mut t = transcript(vec![w("석양이", 0, 500, 1)], &["화자1", "화자2"]);
        set_game(&mut t, 1, true).unwrap();
        assert_eq!(t.speakers[1].label, "게임 음성");
        assert!(t.words().all(|w| w.game));
        set_game(&mut t, 1, false).unwrap();
        assert!(t.speakers[1].label.starts_with("화자"));
        assert!(t.words().all(|w| !w.game));
    }

    #[test]
    fn solo_span_skips_overlap_and_caps_at_thirty_seconds() {
        let mut words = Vec::new();
        // speaker 0 talks 0–8 s, someone talks over at 8 s, then speaker 0 alone 10–50 s
        for k in 0..16 {
            words.push(w("말", k * 500, k * 500 + 400, 0));
        }
        let mut over = w("겹", 8_000, 8_400, 0);
        over.also = Some(1);
        words.push(over);
        for k in 0..80 {
            words.push(w("말", 10_000 + k * 500, 10_000 + k * 500 + 400, 0));
        }
        let t = transcript(words, &["화자1", "화자2"]);
        assert_eq!(solo_span(&t, 0), Some((10_000, 40_000)));
        assert_eq!(solo_span(&t, 1), None);
        assert_eq!(solo_span(&t, 9), None);
    }

    #[test]
    fn caption_labels_carry_over() {
        let caps = vec![
            Caption { start_ms: 0, end_ms: 500, lines: vec!["(화자1) 키라고?".into()] },
            Caption { start_ms: 500, end_ms: 900, lines: vec!["말도 안 돼.".into()] },
            Caption { start_ms: 900, end_ms: 1500, lines: vec!["(화자1·화자2) 안 들었네.".into()] },
            Caption { start_ms: 1500, end_ms: 1900, lines: vec!["(웃음".into()] },
        ];
        let s = caption_speakers(&caps);
        assert_eq!(s[0], (Some("화자1".into()), "키라고?".into()));
        assert_eq!(s[1].0.as_deref(), Some("화자1"));
        assert_eq!(s[2], (Some("화자1·화자2".into()), "안 들었네.".into()));
        assert_eq!(s[3].1, "(웃음");
    }

    #[test]
    fn edits_keep_a_backup_and_rebuild_from_the_transcript() {
        let d = tempfile::tempdir().unwrap();
        let srt = d.path().join("a.srt");
        let t = transcript(vec![w("안녕하세요", 0, 800, 0), w("반갑습니다", 2000, 2800, 1)], &["화자1", "화자2"]);
        std::fs::write(job::transcript_path(&srt), serde_json::to_string(&t).unwrap()).unwrap();
        job::write_srt(&srt, &[Caption { start_ms: 0, end_ms: 800, lines: vec!["안녕하세요".into()] }]).unwrap();
        let mut r = Opened::load(&srt).unwrap();
        r.set_text(0, "안녕\n하세요").unwrap();
        assert!(srt.with_extension("srt.bak").exists());
        assert_eq!(job::read_srt(&srt).unwrap()[0].lines, vec!["안녕", "하세요"]);
        assert!(r.set_text(0, " \n ").is_err());
        assert!(r.set_text(9, "x").is_err());
        let tr = r.transcript.as_mut().unwrap();
        rename_speaker(tr, 1, "아이치").unwrap();
        let mut o = Options::new(PathBuf::from("a.mp4"));
        o.per_speaker = true;
        r.rebuild(&o).unwrap();
        assert!(r.captions.iter().any(|c| c.text().contains("(아이치)")), "{:?}", r.captions);
        assert!(job::per_speaker_path(&srt, "아이치").exists());
        let back = Opened::load(&srt).unwrap();
        assert_eq!(back.transcript.unwrap().speakers[1].label, "아이치");
        r.restore(vec![Caption { start_ms: 0, end_ms: 800, lines: vec!["처음".into()] }], Some(t.clone())).unwrap();
        let back = Opened::load(&srt).unwrap();
        assert_eq!(back.captions[0].lines, vec!["처음"]);
        assert_eq!(back.transcript.unwrap().speakers[1].label, "화자2");
    }

    #[test]
    fn rebuild_without_transcript_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let srt = d.path().join("b.srt");
        job::write_srt(&srt, &[Caption { start_ms: 0, end_ms: 800, lines: vec!["가".into()] }]).unwrap();
        let mut r = Opened::load(&srt).unwrap();
        assert!(r.rebuild(&Options::new(PathBuf::new())).is_err());
    }

    #[test]
    fn recent_list_is_capped_and_deduplicated() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub").join("recent.json");
        let mut list = Vec::new();
        for k in 0..25 {
            let r =
                Recent { input: format!("{k}.mp4").into(), srt: format!("{k}.srt").into(), preset: Preset::General, captions: k, people: 1, open_issues: 0 };
            push_recent(&p, &mut list, r).unwrap();
        }
        let again = Recent { input: "3.mp4".into(), srt: "3.srt".into(), preset: Preset::Streamer, captions: 3, people: 1, open_issues: 2 };
        push_recent(&p, &mut list, again.clone()).unwrap();
        let back = load_recent(&p);
        assert_eq!(back.len(), RECENT_MAX);
        assert_eq!(back[0], again);
        assert_eq!(back.iter().filter(|r| r.srt.as_path() == Path::new("3.srt")).count(), 1);
        std::fs::write(&p, "not json").unwrap();
        assert!(load_recent(&p).is_empty());
    }
}

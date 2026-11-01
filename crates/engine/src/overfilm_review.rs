//! OverFilm: hand the calls the editor is unsure of to Claude Code (see `OVERFILM.md`, crate
//! `overfilm-review`). Off until the user turns it on; runs the local `claude` command line with
//! their Claude subscription (never an API key); nothing is cut here: the answers change cut
//! markers and captions, in one undo step per review.
//!
//! - `overfilm.review.settings {enabled?, auto?, cliPath?, model?, timeoutSeconds?, batch?,
//!   maxItems?}` reads and changes the settings (`claude-review.json` in the OverFilm data folder,
//!   shared with OverFilmSubs). `auto`: review on its own after Auto Cut / Suggestions mark cuts
//!   and after OverFilm Captions places captions.
//! - `overfilm.review.status {cliPath?}` finds `claude` (at `cliPath` when given, not saved), its
//!   version and whether it is signed in.
//! - `overfilm.review.cuts {markers?:[id], all=false, again=false, rules?, wait=false}` asks about
//!   the "OverFilm 컷" markers the rules are unsure of (speech at an edge or inside, game voice
//!   inside, a highlight or team fight next to it; `all`: every one; `markers`: these) with the words heard 30 s
//!   around and what the screen showed. Per marker: cut (kept, the reason added), keep (renamed
//!   "OverFilm 유지 (Claude)", which Apply OverFilm Cut Markers leaves alone), or partial (the
//!   marker replaced by cut markers for the parts to cut and keep markers for the parts to keep).
//! - `overfilm.review.captions {track?, captions?:[id], all=false, again=false, lineChars=16,
//!   maxLines=2, wait=false}` asks about the captions that need judgment (unsure words, a glossary
//!   correction, awkward breaks) with the pure recognition, the corrected text and the captions
//!   around them; fixes and splits are applied, keeps left alone.
//!
//! Reviews run as background jobs (Jobs panel; stop cancels the running `claude`). A marker or
//! caption changed while its review ran is left as it is. The pure recognition is kept in memory
//! and beside a saved project (`<project>.overfilm-raw.json`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use filmcraft_project::{Caption, ClipId, ItemId, Label, Marker, MarkerId, MarkerKind, TrackId};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_review::captions::{self as caps, CapWord, CaptionAnswer, CaptionDecision, CaptionQuestion, Limits, Neighbor, RawChunk};
use overfilm_review::cuts::{self, CutAnswer, CutDecision, CutQuestion, ScreenAt};
use overfilm_review::{ReviewError, Reviewer, Settings};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, str_p, u64_p};
use crate::{EngineError, Result, Session};

/// Name of the markers of ranges Claude said to keep (not an "OverFilm 컷" marker, so Apply
/// OverFilm Cut Markers leaves them).
pub const MARK_KEEP: &str = "OverFilm 유지 (Claude)";
/// Written in the comment of every marker a review decided.
const NOTE: &str = "Claude 검토:";
const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;

/// Review state kept on the session.
#[derive(Default)]
pub struct ReviewState {
    /// Settings file (None: the OverFilm data folder; tests point it elsewhere).
    pub settings_path: Option<PathBuf>,
    /// Answers questions instead of Claude Code (tests, other hosts).
    pub reviewer: Option<Arc<dyn Reviewer>>,
    /// Pure recognition per media item.
    pub raw: BTreeMap<ItemId, Vec<RawChunk>>,
    pub jobs: Vec<PendingReview>,
    /// Captions already reviewed (asked again only with `again`).
    pub reviewed: std::collections::BTreeSet<u64>,
}

/// What a running review asked.
pub enum Asked {
    Cuts(Vec<CutQuestion>),
    Captions { track: TrackId, cases: Vec<CaptionQuestion>, words: BTreeMap<String, Vec<CapWord>>, texts: BTreeMap<String, String> },
}

enum Answers {
    Cuts(Vec<CutAnswer>),
    Captions(Vec<CaptionAnswer>),
}

/// A running review; [`poll`] applies its answers.
pub struct PendingReview {
    pub job: u64,
    pub seq: ItemId,
    pub asked: Asked,
    answers: Arc<Mutex<Option<(Answers, Vec<String>)>>>,
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.review.settings",
            label: "Claude Code Review Settings (OverFilm)…",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"enabled":bool?,"auto":bool?,"cliPath":str?,"model":str?,"timeoutSeconds":10..1800?,"batch":1..20?,"maxItems":1..500?}"#,
            enabled: crate::commands::always,
            run: settings_cmd,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.review.status",
            label: "Check Claude Code (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"cliPath":str?}"#,
            enabled: crate::commands::always,
            run: status_cmd,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.review.cuts",
            label: "Ask Claude Code About Unclear Cuts (OverFilm)",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"markers":[id]?,"all":bool=false,"again":bool=false,"rules":str?,"wait":bool=false}"#,
            enabled: can_review_cuts,
            run: review_cuts,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.review.captions",
            label: "Ask Claude Code About Unclear Captions (OverFilm)",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"track":id?,"captions":[id]?,"all":bool=false,"again":bool=false,"lineChars":n=16,"maxLines":1|2=2,"wait":bool=false}"#,
            enabled: can_review_captions,
            run: review_captions,
            journal: true,
        },
    ]
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn ms(t: Tick) -> i64 {
    t.0 / TICKS_PER_MS
}

fn tick(ms: i64) -> Tick {
    Tick(ms.max(0).saturating_mul(TICKS_PER_MS))
}

fn settings_path(s: &Session) -> Option<PathBuf> {
    match &s.overfilm_review.settings_path {
        Some(p) => Some(p.clone()),
        // tests never read the user's own switch
        None if cfg!(test) => None,
        None => Some(overfilm_pipeline::data_dir().join(overfilm_review::settings::FILE)),
    }
}

/// The user's settings (review off when there are none).
pub fn settings(s: &Session) -> Settings {
    settings_path(s).map(|p| Settings::load(&p)).unwrap_or_default()
}

fn review_on(s: &Session) -> std::result::Result<(), String> {
    crate::commands::has_seq(s)?;
    if settings(s).enabled { Ok(()) } else { Err(ReviewError::Disabled.to_string()) }
}

fn cut_marker(m: &Marker) -> bool {
    m.name.starts_with(crate::overfilm_autocut::MARK) && m.duration > Tick::ZERO
}

fn can_review_cuts(s: &Session) -> std::result::Result<(), String> {
    review_on(s)?;
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if q.markers.iter().any(cut_marker) { Ok(()) } else { Err("no OverFilm cut markers on this sequence; run Auto Cut or Suggestions first".into()) }
}

fn can_review_captions(s: &Session) -> std::result::Result<(), String> {
    review_on(s)?;
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if q.caption_tracks.iter().any(|t| !t.captions.is_empty()) { Ok(()) } else { Err("the sequence has no captions; run OverFilm Captions first".into()) }
}

fn settings_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.review.settings";
    let mut st = settings(s);
    let before = st.clone();
    if let Some(b) = bool_p(p, "enabled") {
        st.enabled = b;
    }
    if let Some(b) = bool_p(p, "auto") {
        st.auto = b;
    }
    if let Some(t) = str_p(p, "cliPath") {
        st.cli_path = t.to_string();
    }
    if let Some(t) = str_p(p, "model") {
        st.model = t.to_string();
    }
    if let Some(n) = u64_p(p, "timeoutSeconds") {
        st.timeout_secs = n;
    }
    if let Some(n) = u64_p(p, "batch") {
        st.batch = usize::try_from(n).unwrap_or(usize::MAX);
    }
    if let Some(n) = u64_p(p, "maxItems") {
        st.max_items = usize::try_from(n).unwrap_or(usize::MAX);
    }
    let st = st.clamped();
    if st != before {
        let path = settings_path(s).ok_or_else(|| bad(CMD, "no settings folder"))?;
        st.save(&path).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    }
    Ok(serde_json::to_value(&st).unwrap_or_default())
}

fn status_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let mut st = settings(s);
    if let Some(t) = str_p(p, "cliPath") {
        // checking a path being typed in the settings dialog, not saved
        st.cli_path = t.to_string();
        st = st.clamped();
    }
    let status = match overfilm_review::ClaudeCode::new(&st) {
        Ok(cc) => cc.status(),
        Err(e) => overfilm_review::Status { message: e.to_string(), ..Default::default() },
    };
    Ok(json!({"settings": st, "status": status}))
}

fn reviewer(s: &Session, st: &Settings) -> std::result::Result<Arc<dyn Reviewer>, ReviewError> {
    if !st.enabled {
        return Err(ReviewError::Disabled);
    }
    match &s.overfilm_review.reviewer {
        Some(r) => Ok(r.clone()),
        None => Ok(Arc::new(overfilm_review::ClaudeCode::new(st)?)),
    }
}

fn ids_p(p: &Value, k: &str) -> Option<Vec<u64>> {
    p.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).take(1000).collect())
}

// ---------------------------------------------------------------------------------------------
// cuts

fn screen_name(sc: overfilm_vision::Screen) -> &'static str {
    match sc {
        overfilm_vision::Screen::Standby => "대기 화면",
        overfilm_vision::Screen::Setup => "라운드 준비",
        overfilm_vision::Screen::Pause => "퍼즈",
        overfilm_vision::Screen::Live => "경기 화면",
    }
}

/// What the screen showed around a cut, when the project has screen templates.
fn screens(s: &Session, set: Option<&overfilm_vision::TemplateSet>, start: i64, end: i64) -> Vec<ScreenAt> {
    let Some(set) = set else { return Vec::new() };
    let mid = start.saturating_add(end.saturating_sub(start) / 2);
    [start.saturating_sub(3000), start.saturating_add(1000), mid, end.saturating_sub(1000), end.saturating_add(3000)]
        .into_iter()
        .filter(|t| *t >= 0)
        .filter_map(|t| {
            let f = crate::overfilm_autocut::frame_at(s, None, tick(t)).ok()?;
            let screen = match set.best(&f) {
                Some((sc, _, _)) => screen_name(sc),
                None if set.live(&f) => screen_name(overfilm_vision::Screen::Live),
                None => "알 수 없음",
            };
            Some(ScreenAt { at_ms: t, screen: screen.into() })
        })
        .collect()
}

fn sequence_words(s: &Session) -> Vec<cuts::Word> {
    crate::transcript::sequence_words(s)
        .into_iter()
        .map(|w| cuts::Word {
            game: w.speaker.as_deref() == Some(crate::overfilm_transcribe::GAME_SPEAKER),
            text: w.text,
            start_ms: ms(w.start),
            end_ms: ms(w.end),
            speaker: w.speaker,
        })
        .collect()
}

/// The cut markers to ask about and what to send for each.
fn cut_questions(s: &Session, p: &Value, max: usize) -> Result<Vec<CutQuestion>> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let chosen = ids_p(p, "markers");
    let all = bool_p(p, "all").unwrap_or(false) || chosen.is_some();
    let again = bool_p(p, "again").unwrap_or(false) || chosen.is_some();
    let words = sequence_words(s);
    let highlights: Vec<(i64, i64)> = q
        .markers
        .iter()
        .filter(|m| m.name.starts_with(crate::overfilm_assist::MARK_HIGHLIGHT) || m.name.starts_with(crate::overfilm_kills::MARK_FIGHT))
        .map(|m| (ms(m.start), ms(m.start + m.duration)))
        .collect();
    let set = crate::overfilm_autocut::templates_path(s, p, "overfilm.review.cuts")
        .ok()
        .and_then(|t| crate::overfilm_autocut::load_set(&t).ok())
        .filter(|set| !set.templates.is_empty());
    let mut out = Vec::new();
    for m in q.markers.iter().filter(|m| cut_marker(m)) {
        if chosen.as_ref().is_some_and(|c| !c.contains(&m.id.0)) || (!again && m.comment.contains(NOTE)) {
            continue;
        }
        let (a, b) = (ms(m.start), ms(m.start + m.duration));
        let doubts = cuts::doubts(a, b, &words, &highlights);
        if doubts.is_empty() && !all {
            continue;
        }
        let kind = m.name.split(':').nth(1).map(str::trim).filter(|k| !k.is_empty()).unwrap_or("컷").to_string();
        out.push(CutQuestion { id: m.id.0.to_string(), kind, start_ms: a, end_ms: b, doubts, words: cuts::context(&words, a, b), screens: Vec::new() });
        if out.len() >= max {
            break;
        }
    }
    for c in &mut out {
        c.screens = screens(s, set.as_ref(), c.start_ms, c.end_ms);
    }
    Ok(out)
}

fn review_cuts(s: &mut Session, p: &Value) -> Result<Value> {
    let st = settings(s);
    let r = reviewer(s, &st).map_err(|e| EngineError::Other(e.to_string()))?;
    let cases = cut_questions(s, p, st.max_items)?;
    if cases.is_empty() {
        return Ok(json!({"asked": 0, "message": "애매한 컷이 없습니다 (all=true로 전부 물어볼 수 있습니다)"}));
    }
    let rules = str_p(p, "rules").filter(|r| !r.trim().is_empty()).map(|r| r.chars().take(2000).collect()).unwrap_or_else(|| cuts::DEFAULT_RULES.to_string());
    let n = cases.len();
    let batches: Vec<Vec<CutQuestion>> = cases.chunks(st.batch.max(1)).map(<[CutQuestion]>::to_vec).collect();
    let work = move |stop: &std::sync::atomic::AtomicBool, step: &mut dyn FnMut(usize)| {
        let mut answers = Vec::new();
        let mut errors = Vec::new();
        for (i, b) in batches.iter().enumerate() {
            step(i);
            match r.ask(&cuts::question(b, &rules), stop) {
                Ok(v) => answers.extend(cuts::parse(&v, b)),
                Err(e) => {
                    let stop_all = !matches!(e, ReviewError::BadAnswer(_) | ReviewError::Failed(_) | ReviewError::Timeout(_));
                    errors.push(e.to_string());
                    if stop_all {
                        break;
                    }
                }
            }
        }
        (Answers::Cuts(answers), errors)
    };
    start(s, p, Asked::Cuts(cases), n, st.batch, work)
}

/// Apply cut answers to the markers (one undo step); what was decided.
fn apply_cuts(s: &mut Session, cases: &[CutQuestion], answers: Vec<CutAnswer>) -> Result<Value> {
    let mut report = Vec::new();
    if answers.is_empty() {
        return Ok(json!({"decided": report}));
    }
    s.edit("Claude Code Cut Review", |pr, st| {
        let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
        let mut new_ids: Vec<u64> = (0..answers.len().saturating_mul(8)).map(|_| pr.alloc_id()).collect();
        let q = pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        for a in &answers {
            let Some(c) = cases.iter().find(|c| c.id == a.id) else { continue };
            let Some(i) = q.markers.iter().position(|m| m.id.0.to_string() == a.id) else { continue };
            let Some(m) = q.markers.get(i).cloned() else { continue };
            if ms(m.start) != c.start_ms || ms(m.start + m.duration) != c.end_ms || !m.name.starts_with(crate::overfilm_autocut::MARK) {
                // moved or renamed while the review ran: the user's change wins
                continue;
            }
            let note = |what: &str| format!("{} · {NOTE} {what} – {}", m.comment, a.reason);
            report.push(json!({"marker": m.id.0, "decision": a.decision, "keep": a.keep, "reason": a.reason,
                "from": overfilm_review::clock(c.start_ms), "to": overfilm_review::clock(c.end_ms)}));
            match a.decision {
                CutDecision::Cut => {
                    if let Some(mm) = q.markers.get_mut(i) {
                        mm.comment = note("자르기");
                    }
                }
                CutDecision::Keep => {
                    if let Some(mm) = q.markers.get_mut(i) {
                        mm.comment = format!("원래 {} · {}", m.name, note("유지"));
                        mm.name = MARK_KEEP.into();
                        mm.color = Label::Blue;
                    }
                }
                CutDecision::Partial => {
                    q.markers.remove(i);
                    for (x, y) in a.cut_ranges(c.start_ms, c.end_ms) {
                        let Some(id) = new_ids.pop() else { break };
                        q.markers
                            .push(Marker { id: MarkerId(id), start: tick(x), duration: tick(y) - tick(x), comment: note("일부만 자르기"), ..m.clone() });
                    }
                    for &(x, y) in &a.keep {
                        let Some(id) = new_ids.pop() else { break };
                        q.markers.push(Marker {
                            id: MarkerId(id),
                            start: tick(x),
                            duration: tick(y) - tick(x),
                            name: MARK_KEEP.into(),
                            comment: format!("원래 {} · {}", m.name, note("이 부분 유지")),
                            kind: MarkerKind::Comment,
                            color: Label::Blue,
                        });
                    }
                }
            }
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    Ok(json!({"decided": report}))
}

// ---------------------------------------------------------------------------------------------
// captions

/// The pure recognition of a recognised item: one entry per chunk, in transcript-word order (as
/// `overfilm_transcribe::to_project` lays the words out).
#[cfg(feature = "overfilm-qwen")]
pub(crate) fn raw_chunks(rec: &overfilm_pipeline::recognise::Recognised) -> Vec<RawChunk> {
    let mut first = 0usize;
    let mut out = Vec::with_capacity(rec.transcript.chunks.len());
    for (i, c) in rec.transcript.chunks.iter().enumerate() {
        let raw = rec.raw_texts.get(i).cloned().unwrap_or_default();
        out.push(RawChunk { first_word: first, words: c.words.len(), raw, text: c.text.clone() });
        first = first.saturating_add(c.words.len());
    }
    out
}

fn raw_file(s: &Session) -> Option<String> {
    let proj = s.path.as_deref()?;
    Some(format!("{}.overfilm-raw.json", proj.strip_suffix(".fcproj").unwrap_or(proj)))
}

/// Keep the pure recognition of freshly recognised items (and beside a saved project).
pub fn store_raw(s: &mut Session, raw: Vec<(ItemId, Vec<RawChunk>)>) {
    if raw.is_empty() {
        return;
    }
    load_raw(s);
    for (item, chunks) in raw {
        s.overfilm_review.raw.insert(item, chunks);
    }
    if let Some(path) = raw_file(s) {
        let map: BTreeMap<String, &Vec<RawChunk>> = s.overfilm_review.raw.iter().map(|(k, v)| (k.0.to_string(), v)).collect();
        if let Ok(text) = serde_json::to_string(&map)
            && let Err(e) = std::fs::write(&path, text)
        {
            log::warn!("{path}: {e}");
        }
    }
}

/// Read the saved pure recognition of items not in memory.
fn load_raw(s: &mut Session) {
    let Some(path) = raw_file(s) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<RawChunk>>>(&text) else { return };
    for (k, v) in map {
        if let Ok(id) = k.parse::<u64>() {
            s.overfilm_review.raw.entry(ItemId(id)).or_insert(v);
        }
    }
}

/// Pure recognition of a run of sequence words (all of one item), or None when unknown.
fn raw_of(s: &Session, words: &[&filmcraft_edit::transcript::SeqWord]) -> Option<String> {
    let first = words.first()?;
    let chunks = s.overfilm_review.raw.get(&first.item)?;
    let t = s.project.transcripts.get(&first.item)?;
    let mut parts = Vec::new();
    for ch in chunks {
        let lo = ch.first_word;
        let hi = lo.saturating_add(ch.words);
        let inside: Vec<usize> = words.iter().filter(|w| w.item == first.item && (lo..hi).contains(&w.index)).map(|w| w.index).collect();
        let (Some(&a), Some(&b)) = (inside.iter().min(), inside.iter().max()) else { continue };
        let texts: Vec<&str> = t.words.get(lo..hi)?.iter().map(|w| w.text.as_str()).collect();
        parts.push(ch.raw_for(a, b + 1, &texts)?);
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

struct CaptionInfo {
    q: CaptionQuestion,
    words: Vec<CapWord>,
}

/// One caption's words, corrected text and pure recognition.
fn caption_info(s: &Session, all: &[filmcraft_edit::transcript::SeqWord], c: &Caption) -> CaptionInfo {
    let (a, b) = (c.start, c.start + c.duration);
    let ws: Vec<&filmcraft_edit::transcript::SeqWord> = all
        .iter()
        .filter(|w| {
            let mid = Tick(w.start.0.saturating_add(w.end.0.saturating_sub(w.start.0) / 2));
            mid >= a && mid < b && w.speaker.as_deref() != Some(crate::overfilm_transcribe::GAME_SPEAKER)
        })
        .collect();
    let corrected = ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    let raw = raw_of(s, &ws);
    let words = ws.iter().map(|w| CapWord { text: w.text.clone(), start_ms: ms(w.start), end_ms: ms(w.end), confidence: w.confidence }).collect();
    let q = CaptionQuestion { id: c.id.0.to_string(), start_ms: ms(a), end_ms: ms(b), text: c.text.clone(), raw, corrected, ..Default::default() };
    CaptionInfo { q, words }
}

fn limits_p(p: &Value) -> Limits {
    let chars = u64_p(p, "lineChars").map(|n| n.clamp(4, 60) as u32).unwrap_or(16);
    let lines = u64_p(p, "maxLines").map(|n| n.clamp(1, 2) as u32).unwrap_or(2);
    Limits { line_width: chars * 2, max_lines: lines }
}

fn caption_track(s: &Session, p: &Value) -> Result<TrackId> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if let Some(id) = u64_p(p, "track") {
        return q.caption_tracks.iter().find(|t| t.id.0 == id).map(|t| t.id).ok_or_else(|| bad("overfilm.review.captions", format!("no caption track {id}")));
    }
    q.caption_tracks
        .iter()
        .find(|t| t.name == crate::overfilm_transcribe::CAPTION_TRACK)
        .or_else(|| q.caption_tracks.iter().find(|t| !t.captions.is_empty()))
        .map(|t| t.id)
        .ok_or_else(|| bad("overfilm.review.captions", "the sequence has no captions"))
}

type CaptionCases = (Vec<CaptionQuestion>, BTreeMap<String, Vec<CapWord>>, BTreeMap<String, String>);

fn caption_questions(s: &Session, p: &Value, track: TrackId, lim: &Limits, max: usize) -> Result<CaptionCases> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let t = q.caption_tracks.iter().find(|t| t.id == track).ok_or(EngineError::NoSequence)?;
    let chosen = ids_p(p, "captions");
    let all_flag = bool_p(p, "all").unwrap_or(false) || chosen.is_some();
    let words = crate::transcript::sequence_words(s);
    let infos: Vec<CaptionInfo> = t.captions.iter().map(|c| caption_info(s, &words, c)).collect();
    let neighbor = |i: usize| {
        infos.get(i).map(|x| Neighbor {
            start_ms: x.q.start_ms,
            end_ms: x.q.end_ms,
            text: x.q.text.clone(),
            raw: x.q.raw.clone().filter(|r| overfilm_review::bare(r) != overfilm_review::bare(&x.q.corrected)),
        })
    };
    let again = bool_p(p, "again").unwrap_or(false) || chosen.is_some();
    let (mut cases, mut wmap, mut texts) = (Vec::new(), BTreeMap::new(), BTreeMap::new());
    for (i, info) in infos.iter().enumerate() {
        if chosen.as_ref().is_some_and(|c| !c.iter().any(|id| id.to_string() == info.q.id))
            || (!again && info.q.id.parse::<u64>().is_ok_and(|id| s.overfilm_review.reviewed.contains(&id)))
        {
            continue;
        }
        let next = infos.get(i + 1).map(|x| x.q.text.as_str());
        let (doubts, unsure) = caps::doubts(&info.q.text, &info.words, info.q.raw.as_deref(), &info.q.corrected, next, lim);
        if doubts.is_empty() && !all_flag {
            continue;
        }
        let mut c = info.q.clone();
        c.doubts = doubts;
        c.unsure = unsure;
        c.before = (i.saturating_sub(2)..i).filter_map(&neighbor).collect();
        c.after = (i + 1..i + 3).filter_map(&neighbor).collect();
        wmap.insert(c.id.clone(), info.words.clone());
        texts.insert(c.id.clone(), c.text.clone());
        cases.push(c);
        if cases.len() >= max {
            break;
        }
    }
    Ok((cases, wmap, texts))
}

fn review_captions(s: &mut Session, p: &Value) -> Result<Value> {
    let st = settings(s);
    let r = reviewer(s, &st).map_err(|e| EngineError::Other(e.to_string()))?;
    load_raw(s);
    let track = caption_track(s, p)?;
    let lim = limits_p(p);
    let (cases, words, texts) = caption_questions(s, p, track, &lim, st.max_items)?;
    if cases.is_empty() {
        return Ok(json!({"asked": 0, "message": "판단이 필요한 자막이 없습니다 (all=true로 전부 물어볼 수 있습니다)"}));
    }
    let n = cases.len();
    let batches: Vec<Vec<CaptionQuestion>> = cases.chunks(st.batch.max(1)).map(<[CaptionQuestion]>::to_vec).collect();
    let work = move |stop: &std::sync::atomic::AtomicBool, step: &mut dyn FnMut(usize)| {
        let mut answers = Vec::new();
        let mut errors = Vec::new();
        for (i, b) in batches.iter().enumerate() {
            step(i);
            match r.ask(&caps::question(b, &lim), stop) {
                Ok(v) => answers.extend(caps::parse(&v, b, &lim)),
                Err(e) => {
                    let stop_all = !matches!(e, ReviewError::BadAnswer(_) | ReviewError::Failed(_) | ReviewError::Timeout(_));
                    errors.push(e.to_string());
                    if stop_all {
                        break;
                    }
                }
            }
        }
        (Answers::Captions(answers), errors)
    };
    start(s, p, Asked::Captions { track, cases, words, texts }, n, st.batch, work)
}

/// The speaker label a caption starts with (`(화자1) `), kept when a fix drops it.
fn label_of(text: &str) -> Option<&str> {
    let end = text.find(") ")?;
    (text.starts_with('(') && end < 40).then(|| text.get(..end + 2)).flatten()
}

fn with_label(old: &str, new: &str) -> String {
    match label_of(old) {
        Some(l) if !new.starts_with(l.trim_end()) => format!("{l}{new}"),
        _ => new.to_string(),
    }
}

fn apply_captions(s: &mut Session, track: TrackId, asked: &Asked, answers: Vec<CaptionAnswer>) -> Result<Value> {
    let Asked::Captions { words, texts, cases, .. } = asked else { return Ok(json!({})) };
    let mut report = Vec::new();
    let mut reviewed = Vec::new();
    if answers.is_empty() {
        return Ok(json!({"decided": report}));
    }
    s.edit("Claude Code Caption Review", |pr, st| {
        let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
        let mut ids: Vec<u64> = (0..answers.len().saturating_mul(4)).map(|_| pr.alloc_id()).collect();
        let q = pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        let t = q.caption_tracks.iter_mut().find(|t| t.id == track).ok_or(EngineError::NoSequence)?;
        for a in &answers {
            let Some(i) = t.captions.iter().position(|c| c.id.0.to_string() == a.id) else { continue };
            let Some(c) = t.captions.get(i).cloned() else { continue };
            if texts.get(&a.id) != Some(&c.text) || !cases.iter().any(|x| x.id == a.id && x.start_ms == ms(c.start)) {
                continue; // edited while the review ran
            }
            reviewed.push(c.id.0);
            report.push(json!({"caption": c.id.0, "decision": a.decision, "before": c.text, "text": a.text, "parts": a.parts, "reason": a.reason}));
            match a.decision {
                CaptionDecision::Keep => {}
                CaptionDecision::Fix => {
                    if let Some(cc) = t.captions.get_mut(i) {
                        cc.text = with_label(&c.text, &a.text);
                    }
                }
                CaptionDecision::Split => {
                    let w = words.get(&a.id).map(Vec::as_slice).unwrap_or(&[]);
                    let times = caps::split_times(ms(c.start), ms(c.start + c.duration), w, &a.parts);
                    let mut new = Vec::new();
                    for (k, (part, (x, y))) in a.parts.iter().zip(times).enumerate() {
                        let (x, y) = if k == 0 { (c.start, tick(y)) } else { (tick(x), tick(y)) };
                        let y = if k + 1 == a.parts.len() { c.start + c.duration } else { y };
                        if y <= x {
                            continue;
                        }
                        let id = if k == 0 { c.id } else { ClipId(ids.pop().unwrap_or_default()) };
                        if id.0 == 0 {
                            continue;
                        }
                        new.push(Caption {
                            id,
                            start: x,
                            duration: y - x,
                            text: if k == 0 { with_label(&c.text, part) } else { part.clone() },
                            cue_id: if k == 0 { c.cue_id.clone() } else { None },
                            ..c.clone()
                        });
                    }
                    if !new.is_empty() {
                        t.captions.splice(i..=i, new);
                    }
                }
            }
        }
        t.captions.sort_by_key(|c| c.start);
        Ok(())
    })?;
    s.overfilm_review.reviewed.extend(reviewed);
    Ok(json!({"decided": report}))
}

// ---------------------------------------------------------------------------------------------
// jobs

type Work = dyn FnOnce(&std::sync::atomic::AtomicBool, &mut dyn FnMut(usize)) -> (Answers, Vec<String>) + Send;

fn start(
    s: &mut Session,
    p: &Value,
    asked: Asked,
    n: usize,
    batch: usize,
    work: impl FnOnce(&std::sync::atomic::AtomicBool, &mut dyn FnMut(usize)) -> (Answers, Vec<String>) + Send + 'static,
) -> Result<Value> {
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let busy = s.overfilm_review.jobs.iter().any(|j| j.seq == seq && std::mem::discriminant(&j.asked) == std::mem::discriminant(&asked));
    if busy {
        return Err(EngineError::Other("Claude Code is already reviewing this sequence".into()));
    }
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let label = match asked {
        Asked::Cuts(_) => "Claude Code: 애매한 컷 검토",
        Asked::Captions { .. } => "Claude Code: 자막 검토",
    };
    let job = crate::Job { id, label: label.into(), progress: Default::default(), result: Default::default() };
    let calls = n.div_ceil(batch.max(1));
    job.progress.total.store(calls as u64, std::sync::atomic::Ordering::Relaxed);
    let answers: Arc<Mutex<Option<(Answers, Vec<String>)>>> = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), answers.clone());
    let work: Box<Work> = Box::new(work);
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let mut step = |i: usize| {
            prog.done.store(i as u64, Ordering::Relaxed);
            *lock(&prog.status) = format!("Claude Code에 묻는 중 {}/{calls} ({n}개)", i + 1);
        };
        let (a, errors) = work(&prog.cancel, &mut step);
        let secs = t0.elapsed().as_secs_f64();
        let got = match &a {
            Answers::Cuts(v) => v.len(),
            Answers::Captions(v) => v.len(),
        };
        prog.done.store(calls as u64, Ordering::Relaxed);
        let r = if got == 0 && !errors.is_empty() {
            let e = errors.join("; ");
            *lock(&prog.status) = e.clone();
            Err(e)
        } else {
            *lock(&prog.status) =
                format!("답 {got}개 ({secs:.0}초){}", if errors.is_empty() { String::new() } else { format!(", 실패: {}", errors.join("; ")) });
            Ok(filmcraft_export::Report { path: String::new(), frames: 0, seconds: secs, bytes: 0, render_fps: 0.0, extra_files: Vec::new() })
        };
        *lock(&out) = Some((a, errors));
        *lock(&res) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.overfilm_review.jobs.push(PendingReview { job: id, seq, asked, answers });
    let mut out = json!({"job": id, "asked": n});
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        if let Some(r) = poll_one(s, id) {
            let v = r?;
            if let Value::Object(m) = v {
                for (k, v) in m {
                    out[k] = v;
                }
            }
        }
    } else {
        std::thread::Builder::new().name("overfilm-review".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    Ok(out)
}

/// Apply finished reviews. Called once per UI frame.
pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    let ids: Vec<u64> = s
        .overfilm_review
        .jobs
        .iter()
        .filter(|pj| s.jobs.iter().find(|j| j.id == pj.job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed)))
        .map(|pj| pj.job)
        .collect();
    for id in ids {
        match poll_one(s, id) {
            Some(Ok(v)) => {
                let n = v["decided"].as_array().map_or(0, Vec::len);
                let failed = v["errors"].as_array().map_or(0, Vec::len);
                s.toast(format!(
                    "Claude Code 검토: {n}개 판단을 반영했습니다{}",
                    if failed > 0 { format!(" (실패 {failed}번, 작업 창 참고)") } else { String::new() }
                ));
            }
            Some(Err(e)) => s.error_toast("overfilm.review", format!("Claude Code 검토: {e}")),
            None => {}
        }
    }
}

fn poll_one(s: &mut Session, job: u64) -> Option<Result<Value>> {
    use std::sync::atomic::Ordering;
    let i = s.overfilm_review.jobs.iter().position(|j| j.job == job)?;
    let pj = s.overfilm_review.jobs.remove(i);
    let cancelled = s.jobs.iter().find(|j| j.id == job).is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
    let (answers, errors) = lock(&pj.answers).take().filter(|_| !cancelled)?;
    if s.state.active_sequence != Some(pj.seq) {
        return Some(Err(EngineError::Other("the sequence is no longer open; nothing was changed".into())));
    }
    let got = match &answers {
        Answers::Cuts(v) => v.len(),
        Answers::Captions(v) => v.len(),
    };
    if got == 0 && !errors.is_empty() {
        return Some(Err(EngineError::Other(errors.join("; "))));
    }
    let r = match (answers, &pj.asked) {
        (Answers::Cuts(a), Asked::Cuts(cases)) => apply_cuts(s, cases, a),
        (Answers::Captions(a), Asked::Captions { track, .. }) => apply_captions(s, *track, &pj.asked, a),
        _ => Ok(json!({})),
    };
    Some(r.map(|mut v| {
        v["errors"] = json!(errors);
        v
    }))
}

/// After cut markers were made: review them when the user asked for that ([`Settings::auto`]).
pub fn after_cut_marks(s: &mut Session) {
    let st = settings(s);
    if !(st.enabled && st.auto) || can_review_cuts(s).is_err() {
        return;
    }
    match review_cuts(s, &json!({})) {
        Ok(v) if v["asked"].as_u64().unwrap_or(0) > 0 => s.toast(format!("Claude Code에 애매한 컷 {}개를 맡겼습니다", v["asked"])),
        Ok(_) => {}
        Err(e) => s.error_toast("overfilm.review.cuts", format!("Claude Code 검토: {e}")),
    }
}

/// After OverFilm Captions placed captions: review them when the user asked for that.
pub fn after_captions(s: &mut Session) {
    let st = settings(s);
    if !(st.enabled && st.auto) || can_review_captions(s).is_err() {
        return;
    }
    match review_captions(s, &json!({})) {
        Ok(v) if v["asked"].as_u64().unwrap_or(0) > 0 => s.toast(format!("Claude Code에 자막 {}개 판단을 맡겼습니다", v["asked"])),
        Ok(_) => {}
        Err(e) => s.error_toast("overfilm.review.captions", format!("Claude Code 검토: {e}")),
    }
}

#[cfg(test)]
#[path = "overfilm_review_tests.rs"]
mod tests;

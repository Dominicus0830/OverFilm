//! OverFilm editing assistants (see `OVERFILM.md`): suggestions put on the sequence as markers,
//! for the editor to check before anything is cut. The analysis is `overfilm_ai`.
//!
//! - `overfilm.assist.suggest {silence=true, highlights=true, count=10, seconds=20,
//!   thresholdDb=-45, minSilence=1, templates?, wait=false}` mixes the sequence's sound in a
//!   background job, then marks
//!   - quiet stretches (no speech, no game sound, never over a transcript word) as
//!     "OverFilm 컷: 무음" range markers, which `overfilm.autoCut.apply` removes with the rest of the
//!     OverFilm cut markers;
//!   - the best moments as "OverFilm 하이라이트 N" range markers (green), with why in the comment:
//!     the casters or the room got louder than the minute around it, hype words in the transcript,
//!     hero ultimates in the game voice. Waiting time already marked or cut by Auto Cut is skipped,
//!     and when the screen-template file has a game-HUD (`live`) template, a moment is kept only
//!     if the game is on screen at its peak (bumpers and replays between rounds are loud too).
//!     Deaths marked by `overfilm.kills.scan` score too ("처치 N" in the comment).
//!
//! Running it again replaces the earlier suggestions.

use std::sync::{Arc, Mutex};

use filmcraft_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_ai::{HighlightParams, SilenceParams};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, f64_p, has_seq, u64_p};
use crate::{EngineError, Result, Session};

/// Name of the silence markers (an OverFilm cut marker: Apply OverFilm Cut Markers removes them).
pub const MARK_SILENCE: &str = "OverFilm 컷: 무음";
/// Name prefix of the highlight markers.
pub const MARK_HIGHLIGHT: &str = "OverFilm 하이라이트";
const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "overfilm.assist.suggest",
        label: "Suggest Silences and Highlights (OverFilm)",
        menu: &["Sequence", "OverFilm"],
        shortcut: None,
        params: r#"{"silence":bool=true,"highlights":bool=true,"count":n=10,"seconds":n=20,"thresholdDb":dB=-45,"minSilence":seconds=1,"templates":path?,"wait":bool=false}"#,
        enabled: has_seq,
        run: suggest,
        journal: true,
    }]
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What to suggest once the sound is measured.
#[derive(Clone, Debug)]
pub struct AssistOptions {
    pub silence: Option<SilenceParams>,
    pub highlights: Option<HighlightParams>,
    /// Screen-template file to check the game is on screen (None: not checked).
    pub templates: Option<String>,
}

/// A running measurement; [`poll`] makes the suggestions.
pub struct PendingAssist {
    pub job: u64,
    pub seq: ItemId,
    pub options: AssistOptions,
    pub levels: Arc<Mutex<Option<Vec<f32>>>>,
}

fn suggest(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.assist.suggest";
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let silence = bool_p(p, "silence").unwrap_or(true).then(|| SilenceParams {
        threshold_db: f64_p(p, "thresholdDb").filter(|v| v.is_finite()).map(|v| v.clamp(-90.0, -10.0) as f32).unwrap_or(-45.0),
        min_ms: f64_p(p, "minSilence").filter(|v| v.is_finite()).map(|v| (v.clamp(0.2, 60.0) * 1000.0) as i64).unwrap_or(1000),
        ..SilenceParams::default()
    });
    let highlights = bool_p(p, "highlights").unwrap_or(true).then(|| HighlightParams {
        count: u64_p(p, "count").map(|n| n.clamp(1, 100) as usize).unwrap_or(10),
        // the learnt style's moment length, else 20 s
        length_ms: f64_p(p, "seconds")
            .filter(|v| v.is_finite())
            .map(|v| (v.clamp(5.0, 120.0) * 1000.0) as i64)
            .or_else(|| crate::overfilm_style::saved(s, p).and_then(|st| st.highlight_length_ms()))
            .unwrap_or(20_000),
        ..HighlightParams::default()
    });
    if silence.is_none() && highlights.is_none() {
        return Err(bad(CMD, "nothing to suggest (silence and highlights are both off)"));
    }
    if s.overfilm_assist_jobs.iter().any(|j| j.seq == seq) {
        return Err(EngineError::Other("OverFilm suggestions are already being made for this sequence".into()));
    }
    let templates = crate::overfilm_autocut::templates_path(s, p, CMD).ok().filter(|t| std::path::Path::new(t).is_file());
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let dur = q.duration();
    if dur <= Tick::ZERO {
        return Err(bad(CMD, "the sequence is empty"));
    }
    let sr = q.settings.sample_rate.max(8000);
    let total = usize::try_from(dur.to_units_floor(i64::from(sr)).max(0)).unwrap_or(0).min(overfilm_ai::MAX_BLOCKS * (sr as usize / 10));
    let project = s.project.clone();
    let provider = s.media.full_res_provider(project.clone(), s.services.clone());
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label: "OverFilm Suggestions".into(), progress: Default::default(), result: Default::default() };
    job.progress.total.store((total / sr as usize).max(1) as u64, std::sync::atomic::Ordering::Relaxed);
    let levels: Arc<Mutex<Option<Vec<f32>>>> = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), levels.clone());
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let mut lv = Vec::with_capacity(total / (sr as usize / 10).max(1) + 1);
        let mut err = None;
        if let Some(q) = project.sequence(seq) {
            let mut pos = 0usize;
            while pos < total {
                if prog.cancel.load(Ordering::Relaxed) {
                    err = Some("stopped".to_string());
                    break;
                }
                let n = (sr as usize).min(total - pos);
                let buf = filmcraft_render::audio::mix_sequence(&project, q, pos as i64, n, &provider);
                let mono = filmcraft_speech::downmix(&buf.channels);
                lv.extend(overfilm_ai::levels(&mono, sr));
                pos += n;
                prog.done.store((pos / sr as usize) as u64, Ordering::Relaxed);
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        let r = match err {
            Some(e) => {
                *lock(&prog.status) = "Stopped: nothing was changed".into();
                Err(e)
            }
            None => {
                *lock(&prog.status) = format!("sound measured in {secs:.0}s");
                *lock(&out) = Some(lv);
                Ok(filmcraft_export::Report { path: String::new(), frames: 0, seconds: secs, bytes: 0, render_fps: 0.0, extra_files: Vec::new() })
            }
        };
        *lock(&res) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.overfilm_assist_jobs.push(PendingAssist { job: id, seq, options: AssistOptions { silence, highlights, templates }, levels });
    let mut out = json!({"job": id});
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        if let Some(r) = poll_one(s, id) {
            out = r?;
            out["job"] = json!(id);
        }
    } else {
        std::thread::Builder::new().name("overfilm-assist".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    Ok(out)
}

/// Make the suggestions of finished measurements. Called once per UI frame.
pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    let ids: Vec<u64> = s
        .overfilm_assist_jobs
        .iter()
        .filter(|pj| s.jobs.iter().find(|j| j.id == pj.job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed)))
        .map(|pj| pj.job)
        .collect();
    for id in ids {
        match poll_one(s, id) {
            Some(Ok(v)) => s.toast(format!(
                "OverFilm: {} silence(s), {} highlight(s) marked; check them on the timeline",
                v["silences"].as_array().map_or(0, Vec::len),
                v["highlights"].as_array().map_or(0, Vec::len)
            )),
            Some(Err(e)) => s.error_toast("overfilm.assist.suggest", format!("OverFilm suggestions: {e}")),
            None => {}
        }
    }
}

fn poll_one(s: &mut Session, job: u64) -> Option<Result<Value>> {
    use std::sync::atomic::Ordering;
    let i = s.overfilm_assist_jobs.iter().position(|j| j.job == job)?;
    let pj = s.overfilm_assist_jobs.remove(i);
    let cancelled = s.jobs.iter().find(|j| j.id == job).is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
    let levels = lock(&pj.levels).take().filter(|_| !cancelled)?;
    if s.state.active_sequence != Some(pj.seq) {
        return Some(Err(EngineError::Other("the sequence is no longer open; nothing was marked".into())));
    }
    Some(make(s, &levels, &pj.options))
}

fn ms(t: Tick) -> i64 {
    t.0 / TICKS_PER_MS
}

fn tick(ms: i64) -> Tick {
    Tick(ms.max(0).saturating_mul(TICKS_PER_MS))
}

fn clock(ms: i64) -> String {
    overfilm_vision::plan::clock(tick(ms))
}

/// Mark the suggestions of measured `levels` on the active sequence (one undo step).
pub fn make(s: &mut Session, levels: &[f32], o: &AssistOptions) -> Result<Value> {
    let words: Vec<overfilm_ai::Word> = crate::transcript::sequence_words(s)
        .iter()
        .map(|w| overfilm_ai::Word {
            text: w.text.clone(),
            start_ms: ms(w.start),
            end_ms: ms(w.end),
            game: w.speaker.as_deref() == Some(crate::overfilm_transcribe::GAME_SPEAKER),
        })
        .collect();
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    // waiting already marked by Auto Cut: no highlight there, no second cut marker
    let waits: Vec<(i64, i64)> = q
        .markers
        .iter()
        .filter(|m| m.name.starts_with(crate::overfilm_autocut::MARK) && m.name != MARK_SILENCE)
        .map(|m| (ms(m.start), ms(m.start + m.duration)))
        .collect();
    let in_wait = |a: i64, b: i64| waits.iter().any(|&(x, y)| a < y && x < b);
    let silences: Vec<overfilm_ai::Span> = o
        .silence
        .as_ref()
        .map(|p| overfilm_ai::silences(levels, &words, p).into_iter().filter(|s| !in_wait(s.start_ms, s.end_ms)).collect())
        .unwrap_or_default();
    let set = o
        .templates
        .as_deref()
        .and_then(|t| crate::overfilm_autocut::load_set(t).ok())
        .filter(|set| set.templates.iter().any(|t| t.screen == overfilm_vision::Screen::Live));
    let mut highlights = Vec::new();
    if let Some(p) = &o.highlights {
        // ask for more than needed: some fall in waiting time or off the game
        let wide = HighlightParams { count: p.count.saturating_mul(3), ..p.clone() };
        let kills = crate::overfilm_kills::kill_times_ms(s);
        for h in overfilm_ai::highlights_with_kills(levels, &words, &kills, &wide) {
            if highlights.len() >= p.count {
                break;
            }
            if in_wait(h.span.start_ms, h.span.end_ms) {
                continue;
            }
            if let Some(set) = &set {
                // the game on screen in at least two of the peak and 2 s either side: a bumper
                // between rounds is loud too, and the HUD may still show for a second of it
                let on = [0, -2000, 2000]
                    .iter()
                    .filter_map(|d| crate::overfilm_autocut::frame_at(s, None, tick(h.peak_ms + d)).ok())
                    .filter(|f| set.live(f))
                    .count();
                if on < 2 {
                    continue;
                }
            }
            highlights.push(h);
        }
        highlights.sort_by_key(|h| h.span.start_ms);
    }
    let report = json!({
        "silences": silences.iter().map(|x| json!({"from": clock(x.start_ms), "to": clock(x.end_ms), "start": tick(x.start_ms).0, "end": tick(x.end_ms).0})).collect::<Vec<_>>(),
        "highlights": highlights.iter().map(|h| json!({"from": clock(h.span.start_ms), "to": clock(h.span.end_ms), "start": tick(h.span.start_ms).0, "end": tick(h.span.end_ms).0,
            "score": h.score, "louderDb": h.louder_db, "words": h.words, "gameWords": h.game_words, "kills": h.kills})).collect::<Vec<_>>(),
        "checkedOnScreen": set.is_some(),
    });
    let do_silence = o.silence.is_some();
    let do_high = o.highlights.is_some();
    s.edit("OverFilm Suggestions", move |pr, st| {
        let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
        let ids: Vec<u64> = (0..silences.len() + highlights.len()).map(|_| pr.alloc_id()).collect();
        let mut ids = ids.into_iter();
        let q = pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        q.markers.retain(|m| !(do_silence && m.name == MARK_SILENCE) && !(do_high && m.name.starts_with(MARK_HIGHLIGHT)));
        for x in &silences {
            let Some(id) = ids.next() else { break };
            q.markers.push(Marker {
                id: MarkerId(id),
                start: tick(x.start_ms),
                duration: tick(x.end_ms) - tick(x.start_ms),
                name: MARK_SILENCE.into(),
                comment: format!("{} – {} 조용함", clock(x.start_ms), clock(x.end_ms)),
                kind: MarkerKind::Comment,
                color: Label::Brown,
                url: String::new(),
                author: String::new(),
                custom_color: None,
            });
        }
        for (n, h) in highlights.iter().enumerate() {
            let Some(id) = ids.next() else { break };
            let mut why = vec![format!("점수 {:.0}", h.score)];
            if h.louder_db >= 3.0 {
                why.push(format!("소리 +{:.0} dB", h.louder_db));
            }
            if !h.words.is_empty() {
                why.push(h.words.join(", "));
            }
            if h.kills > 0 {
                why.push(format!("처치 {}", h.kills));
            }
            if h.game_words > 0 {
                why.push(format!("게임 음성 {}단어", h.game_words));
            }
            q.markers.push(Marker {
                id: MarkerId(id),
                start: tick(h.span.start_ms),
                duration: tick(h.span.end_ms) - tick(h.span.start_ms),
                name: format!("{MARK_HIGHLIGHT} {}", n + 1),
                comment: why.join(" · "),
                kind: MarkerKind::Comment,
                color: Label::Green,
                url: String::new(),
                author: String::new(),
                custom_color: None,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    crate::overfilm_review::after_cut_marks(s);
    Ok(report)
}

#[cfg(test)]
#[path = "overfilm_assist_tests.rs"]
mod tests;

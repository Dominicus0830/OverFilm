//! OverFilm: automatic cuts for Overwatch esports broadcasts (see `OVERFILM.md`).
//!
//! The recognition and the cut rules live in `overfilm-vision`; this module only feeds it frames
//! and applies its plan:
//!
//! - `overfilm.screenTemplate.learn {screen, name?, rects:[[x,y,w,h]], ignore?:[[x,y,w,h]], time?,
//!   clip?, templates?, threshold?}` learns a template from the video frame at `time` (default:
//!   the playhead), rectangles in frame-relative units (0..1). The set is a JSON file: `templates`,
//!   or `<project>.overfilm-screens.json` beside a saved project.
//! - `overfilm.screenTemplate.list {templates?}` lists the set.
//! - `overfilm.autoCut {clips?, templates?, fps=2, keepBefore=10, frozenPauses=true,
//!   mode="mark"|"cut", dissolveFrames?, wait=false}` analyses the clips (the selected video clips,
//!   or every clip on the lowest video track) in a background job. `mark` puts a range marker
//!   ("OverFilm 컷 …") on the sequence for each planned cut, for review; `cut` removes them at
//!   once.
//! - `overfilm.autoCut.apply {dissolveFrames?}` removes the ranges of the "OverFilm 컷" markers
//!   still on the sequence (after the user deleted or moved some) in one undo step, with a
//!   dissolve and an audio crossfade at each pause join.

use std::sync::{Arc, Mutex};

use filmcraft_project::{ClipId, EffectKind, ItemId, Label, Marker, MarkerId, MarkerKind, Sequence, TrackKind, Transition, TransitionId};
use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};
use overfilm_vision::{Analyzer, Cut, CutParams, CutReason, Luma, NRect, Screen, Template, TemplateSet};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, clips_p, f64_p, has_seq, str_p};
use crate::{EngineError, Result, Session};

/// Marker names of planned cuts start with this.
pub const MARK: &str = "OverFilm 컷";
const MARK_WAIT: &str = "OverFilm 컷: 대기";
const MARK_PAUSE: &str = "OverFilm 컷: 퍼즈";
/// Analysis frame width.
const WIDTH: usize = 1280;
/// Most samples one run decodes (10 hours at 4 per second).
const MAX_SAMPLES: u64 = 144_000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.autoCut",
            label: "Auto Cut Waiting and Pauses (OverFilm)…",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"clips":[id]?,"templates":path?,"fps":0.5..8=2,"keepBefore":seconds=10,"frozenPauses":bool=true,"mode":"mark"|"cut"="mark","dissolveFrames":n?,"wait":bool=false}"#,
            enabled: has_seq,
            run: auto_cut,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.autoCut.apply",
            label: "Apply OverFilm Cut Markers",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"dissolveFrames":n?}"#,
            enabled: has_marks,
            run: apply_marks,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.screenTemplate.learn",
            label: "Learn Screen Template (OverFilm)…",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"screen":"standby"|"setup"|"pause"|"live","name":str?,"rects":[[x,y,w,h]],"ignore":[[x,y,w,h]]?,"time":ticks?,"clip":id?,"templates":path?,"threshold":0..1?}"#,
            enabled: has_seq,
            run: learn,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.screenTemplate.list",
            label: "List Screen Templates (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"templates":path?}"#,
            enabled: crate::commands::always,
            run: list,
            journal: false,
        },
    ]
}

fn has_marks(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if q.markers.iter().any(|m| m.name.starts_with(MARK)) { Ok(()) } else { Err("no OverFilm cut markers on this sequence; run Auto Cut first".into()) }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The template file: `templates`, or one beside the saved project.
fn templates_path(s: &Session, p: &Value, cmd: &str) -> Result<String> {
    if let Some(t) = str_p(p, "templates").filter(|t| !t.trim().is_empty()) {
        return Ok(t.to_string());
    }
    let proj = s.path.as_deref().ok_or_else(|| bad(cmd, "save the project first, or give `templates` (a .json path)"))?;
    let stem = proj.strip_suffix(".fcproj").unwrap_or(proj);
    Ok(format!("{stem}.overfilm-screens.json"))
}

fn load_set(path: &str) -> Result<TemplateSet> {
    match std::fs::read_to_string(path) {
        Ok(text) => TemplateSet::from_json(&text).map_err(|e| EngineError::Other(format!("{path}: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TemplateSet::default()),
        Err(e) => Err(EngineError::Other(format!("{path}: {e}"))),
    }
}

fn rects_p(p: &Value, k: &str, cmd: &str) -> Result<Vec<NRect>> {
    let Some(v) = p.get(k).filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let arr = v.as_array().ok_or_else(|| bad(cmd, format!("`{k}` is a list of [x, y, w, h] in 0..1")))?;
    arr.iter()
        .take(16)
        .map(|r| {
            let n: Vec<f32> = r.as_array().map(|a| a.iter().filter_map(Value::as_f64).map(|f| f as f32).collect()).unwrap_or_default();
            match n.as_slice() {
                [x, y, w, h] => NRect::new(*x, *y, *w, *h).clamped().ok_or_else(|| bad(cmd, format!("`{k}`: {r} is empty or outside the frame"))),
                _ => Err(bad(cmd, format!("`{k}`: {r} is not [x, y, w, h]"))),
            }
        })
        .collect()
}

/// The frame at sequence time `t` of `clip` (or of the topmost video clip there), as analysis luma.
fn frame_at(s: &Session, clip: Option<ClipId>, t: Tick) -> Result<Luma> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let it = match clip {
        Some(c) => q.find_item(c).map(|(_, it)| it).ok_or_else(|| EngineError::Other(format!("no clip {}", c.0)))?,
        None => q
            .tracks(TrackKind::Video)
            .iter()
            .rev()
            .filter(|tr| tr.enabled)
            .find_map(|tr| tr.items.iter().find(|it| it.start <= t && t < it.end()))
            .ok_or_else(|| EngineError::Other("no video clip at that time".into()))?,
    };
    let src = s.source(it.item).ok_or_else(|| EngineError::Other("the clip has no media".into()))?;
    let v = src.info().video.clone().ok_or_else(|| EngineError::Other("the clip has no video".into()))?;
    let scale = (WIDTH as f64 / v.width.max(1) as f64).min(1.0) as f32;
    let f = src
        .video_frame(filmcraft_media::FrameRequest { time: it.source_time_at(t.clamp(it.start, it.end())), scale })
        .map_err(|e| EngineError::Other(format!("can't decode the frame: {e}")))?;
    Luma::from_rgba(&f.to_rgba8(), f.width as usize, f.height as usize, WIDTH).ok_or_else(|| EngineError::Other("empty frame".into()))
}

fn learn(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.screenTemplate.learn";
    let screen = str_p(p, "screen").and_then(Screen::parse).ok_or_else(|| bad(CMD, "`screen` is standby, setup, pause or live"))?;
    let rects = rects_p(p, "rects", CMD)?;
    if rects.is_empty() {
        return Err(bad(CMD, "give `rects`: the parts of the frame that always look the same on this screen"));
    }
    let ignore = rects_p(p, "ignore", CMD)?;
    let path = templates_path(s, p, CMD)?;
    let t = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let frame = frame_at(s, crate::commands::clip_p(p, "clip"), t)?;
    let name = str_p(p, "name").map(str::to_string).unwrap_or_else(|| format!("{} {}", screen.as_str(), overfilm_vision::plan::clock(t)));
    let mut tpl = Template::learn(&name, screen, &frame, &rects, &ignore).map_err(|e| bad(CMD, e.to_string()))?;
    if let Some(th) = f64_p(p, "threshold").filter(|v| v.is_finite()) {
        tpl.threshold = th.clamp(0.1, 1.0) as f32;
    }
    let score = tpl.score(&frame);
    let mut set = load_set(&path)?;
    set.insert(tpl);
    std::fs::write(&path, set.to_json()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"templates": path, "name": name, "screen": screen.as_str(), "count": set.templates.len(), "selfScore": score}))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let path = templates_path(s, p, "overfilm.screenTemplate.list")?;
    let set = load_set(&path)?;
    Ok(json!({
        "templates": path,
        "list": set.templates.iter().map(|t| json!({"name": t.name, "screen": t.screen.as_str(), "parts": t.parts.len(), "threshold": t.threshold})).collect::<Vec<_>>(),
    }))
}

/// One clip to analyse.
struct Work {
    name: String,
    src: filmcraft_media::SharedSource,
    scale: f32,
    /// (sequence time, media time) of each sample.
    times: Vec<(Tick, Tick)>,
}

/// A running analysis; its plan is applied by [`poll`] when it finishes.
pub struct PendingAutoCut {
    pub job: u64,
    pub seq: ItemId,
    pub cut_now: bool,
    pub dissolve_frames: Option<i64>,
    pub result: Arc<Mutex<Option<Vec<Cut>>>>,
}

fn auto_cut(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.autoCut";
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let path = templates_path(s, p, CMD)?;
    let set = load_set(&path)?;
    if set.templates.iter().all(|t| !matches!(t.screen, Screen::Standby | Screen::Setup | Screen::Pause)) {
        return Err(bad(
            CMD,
            format!("{path} has no break-screen, setup or pause template yet; learn one first (overfilm.screenTemplate.learn on a frame of that screen)"),
        ));
    }
    let fps = f64_p(p, "fps").filter(|f| f.is_finite()).unwrap_or(2.0).clamp(0.5, 8.0);
    let mut params = CutParams { frozen_needs_live: set.templates.iter().any(|t| t.screen == Screen::Live), ..CutParams::default() };
    if let Some(k) = f64_p(p, "keepBefore").filter(|v| v.is_finite()) {
        params.keep_before = Tick::from_seconds_f64(k.clamp(0.0, 600.0));
    }
    if let Some(f) = bool_p(p, "frozenPauses") {
        params.frozen_pauses = f;
    }
    let cut_now = match str_p(p, "mode").unwrap_or("mark") {
        "mark" => false,
        "cut" => true,
        m => return Err(bad(CMD, format!("mode is mark or cut, not {m}"))),
    };
    let dissolve_frames = p.get("dissolveFrames").and_then(Value::as_i64).map(|f| f.clamp(0, 120));
    if s.overfilm_jobs.iter().any(|j| j.seq == seq_id) {
        return Err(EngineError::Other("Auto Cut is already running on this sequence".into()));
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let step = Tick::from_seconds_f64(1.0 / fps).max(Tick(1));
    // the chosen clips, or the lowest video track's
    let mut chosen: Vec<ClipId> =
        clips_p(s, p).into_iter().filter(|c| q.find_item(*c).is_some_and(|(t, _)| q.track(t).is_some_and(|t| t.kind == TrackKind::Video))).collect();
    if chosen.is_empty() {
        chosen = q.tracks(TrackKind::Video).first().map(|t| t.items.iter().map(|i| i.id).collect()).unwrap_or_default();
    }
    let mut work = Vec::new();
    let mut total = 0u64;
    let mut last_end = Tick::MIN;
    let mut items: Vec<_> = chosen.iter().filter_map(|c| q.find_item(*c).map(|(_, it)| it.clone())).collect();
    items.sort_by_key(|it| it.start);
    for it in items {
        // overlapping clips (two tracks) would put samples out of order: the earlier one wins
        let start = it.start.max(last_end);
        if start >= it.end() {
            continue;
        }
        let Some(src) = s.source(it.item) else { continue };
        let Some(v) = src.info().video.clone() else { continue };
        let mut times = Vec::new();
        let mut t = start;
        while t < it.end() && total < MAX_SAMPLES {
            times.push((t, it.source_time_at(t)));
            total += 1;
            t += step;
        }
        last_end = it.end();
        work.push(Work { name: it.name.clone(), src, scale: (WIDTH as f64 / v.width.max(1) as f64).min(1.0) as f32, times });
    }
    if work.is_empty() {
        return Err(EngineError::Other("no video clip with media to analyse (select clips, or put the broadcast on V1)".into()));
    }
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label: "OverFilm Auto Cut".into(), progress: Default::default(), result: Default::default() };
    job.progress.total.store(total, std::sync::atomic::Ordering::Relaxed);
    let result: Arc<Mutex<Option<Vec<Cut>>>> = Arc::default();
    let report: Arc<Mutex<Value>> = Arc::default();
    let (prog, res, out, rep) = (job.progress.clone(), job.result.clone(), result.clone(), report.clone());
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let mut an = Analyzer::new(set);
        let mut done = 0u64;
        let mut err = None;
        'clips: for w in &work {
            for (st, mt) in &w.times {
                if prog.cancel.load(Ordering::Relaxed) {
                    err = Some("stopped".to_string());
                    break 'clips;
                }
                // a frame that can't be decoded is skipped: no sample, no cut there
                if let Ok(f) = w.src.video_frame(filmcraft_media::FrameRequest { time: *mt, scale: w.scale })
                    && let Some(l) = Luma::from_rgba(&f.to_rgba8(), f.width as usize, f.height as usize, WIDTH)
                {
                    an.push(*st, l);
                }
                done += 1;
                prog.done.store(done, Ordering::Relaxed);
                if done.is_multiple_of(32) {
                    *lock(&prog.status) = format!("{}: {}", w.name, overfilm_vision::plan::clock(*st));
                }
            }
        }
        let samples = an.into_samples();
        let segs = overfilm_vision::segments(&samples, &params);
        let cuts = overfilm_vision::plan_cuts(&segs, &params);
        let removed = overfilm_vision::plan::removed(&cuts);
        let secs = t0.elapsed().as_secs_f64();
        *lock(&rep) = json!({
            "samples": samples.len(),
            "segments": segs.iter().map(|g| json!({"screen": g.screen.as_str(), "start": g.range.start.0, "end": g.range.end().0, "how": g.how})).collect::<Vec<_>>(),
            "cuts": cuts.iter().map(cut_json).collect::<Vec<_>>(),
            "removedSeconds": removed.0 as f64 / TICKS_PER_SECOND as f64,
        });
        *lock(&prog.status) = match &err {
            Some(e) if e == "stopped" => "Stopped: nothing was changed".into(),
            Some(e) => e.clone(),
            None => format!("{} cut(s), {} removed ({done} frames in {secs:.0}s)", cuts.len(), overfilm_vision::plan::clock(removed)),
        };
        let r = match err {
            Some(e) => Err(e),
            None => {
                *lock(&out) = Some(cuts);
                Ok(filmcraft_export::Report {
                    path: String::new(),
                    frames: done,
                    seconds: secs,
                    bytes: 0,
                    render_fps: done as f64 / secs.max(1e-6),
                    extra_files: Vec::new(),
                })
            }
        };
        *lock(&res) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.overfilm_jobs.push(PendingAutoCut { job: id, seq: seq_id, cut_now, dissolve_frames, result });
    let mut out = json!({"job": id, "frames": total, "templates": path});
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        if let Value::Object(m) = lock(&report).clone() {
            for (k, v) in m {
                out[k] = v;
            }
        }
        poll(s);
    } else {
        std::thread::Builder::new().name("overfilm-autocut".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    if let Some(Err(e)) = s.jobs.iter().find(|j| j.id == id).and_then(|j| lock(&j.result).clone()) {
        return Err(EngineError::Other(e));
    }
    Ok(out)
}

fn cut_json(c: &Cut) -> Value {
    json!({"start": c.range.start.0, "end": c.range.end().0, "reason": c.reason, "dissolve": c.dissolve,
           "from": overfilm_vision::plan::clock(c.range.start), "to": overfilm_vision::plan::clock(c.range.end())})
}

/// Apply finished analyses. Called once per UI frame and after synchronous runs.
pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    let mut i = 0;
    while i < s.overfilm_jobs.len() {
        let job = s.jobs.iter().find(|j| j.id == s.overfilm_jobs[i].job);
        let finished = job.is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        let cancelled = job.is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        if !finished {
            i += 1;
            continue;
        }
        let pj = s.overfilm_jobs.remove(i);
        let Some(cuts) = lock(&pj.result).take().filter(|_| !cancelled) else { continue };
        if s.state.active_sequence != Some(pj.seq) {
            s.error_toast("overfilm.autoCut", "Auto Cut finished on a sequence that is no longer open; nothing was changed");
            continue;
        }
        let r = if pj.cut_now { remove(s, &cuts, pj.dissolve_frames).map(|_| ()) } else { mark(s, &cuts).map(|_| ()) };
        if let Err(e) = r {
            s.error_toast("overfilm.autoCut", format!("Auto Cut: {e}"));
        }
    }
}

/// Put one range marker per planned cut on the sequence (replacing earlier ones).
fn mark(s: &mut Session, cuts: &[Cut]) -> Result<usize> {
    let n = cuts.len();
    let cuts = cuts.to_vec();
    s.edit("OverFilm Auto Cut Markers", move |pr, st| {
        let seq_id = st.active_sequence.ok_or(EngineError::NoSequence)?;
        let ids: Vec<u64> = cuts.iter().map(|_| pr.alloc_id()).collect();
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        q.markers.retain(|m| !m.name.starts_with(MARK));
        for (c, id) in cuts.iter().zip(ids) {
            let (name, color) = match c.reason {
                CutReason::Waiting => (MARK_WAIT, Label::Rose),
                CutReason::Pause => (MARK_PAUSE, Label::Mango),
            };
            q.markers.push(Marker {
                id: MarkerId(id),
                start: c.range.start,
                duration: c.range.duration,
                name: name.into(),
                comment: format!(
                    "{} – {}{}",
                    overfilm_vision::plan::clock(c.range.start),
                    overfilm_vision::plan::clock(c.range.end()),
                    if c.dissolve { " (디졸브)" } else { "" }
                ),
                kind: MarkerKind::Comment,
                color,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    s.toast(format!("OverFilm Auto Cut: {n} cut(s) marked; check them, then Sequence ▸ OverFilm ▸ Apply OverFilm Cut Markers"));
    Ok(n)
}

fn apply_marks(s: &mut Session, p: &Value) -> Result<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let cuts: Vec<Cut> = q
        .markers
        .iter()
        .filter(|m| m.name.starts_with(MARK) && m.duration > Tick::ZERO)
        .map(|m| Cut {
            range: TimeRange::new(m.start, m.duration),
            reason: if m.name.starts_with(MARK_PAUSE) { CutReason::Pause } else { CutReason::Waiting },
            dissolve: m.name.starts_with(MARK_PAUSE) || m.comment.contains("디졸브"),
        })
        .collect();
    if cuts.is_empty() {
        return Err(EngineError::Other("no OverFilm cut markers with a duration on this sequence".into()));
    }
    let frames = p.get("dissolveFrames").and_then(Value::as_i64).map(|f| f.clamp(0, 120));
    remove(s, &cuts, frames)
}

/// Ripple-delete the cut ranges on every unlocked track in one undo step, then dissolve the joins
/// of the cuts that ask for it.
fn remove(s: &mut Session, cuts: &[Cut], dissolve_frames: Option<i64>) -> Result<Value> {
    let rate = s.sequence_rate();
    let vdur = match dissolve_frames {
        Some(f) => rate.tick_of(f),
        None => s.prefs.timeline.video_transition_duration(rate),
    };
    let adur = match dissolve_frames {
        Some(f) => rate.tick_of(f),
        None => s.prefs.timeline.audio_transition_duration(rate),
    };
    let mut ranges: Vec<Cut> = cuts.iter().filter(|c| c.range.duration > Tick::ZERO).cloned().collect();
    ranges.sort_by_key(|c| c.range.start);
    let n = ranges.len();
    let total = s.edit_sequence("OverFilm Auto Cut", move |q, ctx, _| {
        let mut removed = Tick::ZERO;
        let mut joins = Vec::new();
        // latest first so earlier ranges keep their times
        for c in ranges.iter().rev() {
            let r = TimeRange::from_bounds(rate.snap(c.range.start), rate.snap(c.range.end()));
            if r.duration <= Tick::ZERO {
                continue;
            }
            filmcraft_edit::transcript::ripple_delete_ranges(q, vec![r], ctx);
            crate::sequence_tools::ripple_markers(&mut q.markers, r.end(), -r.duration);
            q.markers.retain(|m| !(m.name.starts_with(MARK) && m.start >= r.start && m.start < r.end()));
            removed += r.duration;
            // join times move left by every earlier-in-time cut, which is applied after this one
            for j in joins.iter_mut() {
                *j -= r.duration;
            }
            if c.dissolve {
                joins.push(r.start);
            }
        }
        q.markers.retain(|m| !m.name.starts_with(MARK));
        for t in joins {
            if vdur > Tick::ZERO {
                dissolve_at(q, t, TrackKind::Video, vdur, ctx);
            }
            if adur > Tick::ZERO {
                dissolve_at(q, t, TrackKind::Audio, adur, ctx);
            }
        }
        Ok(removed)
    })?;
    s.toast(format!("OverFilm Auto Cut: {n} cut(s), {} removed", overfilm_vision::plan::clock(total)));
    Ok(json!({"cuts": n, "removedSeconds": total.0 as f64 / TICKS_PER_SECOND as f64}))
}

/// A centred cross dissolve (video) or constant-power crossfade (audio) on every unlocked track
/// with two clips meeting at `t`. A join without media to overlap keeps its plain cut.
fn dissolve_at(q: &mut Sequence, t: Tick, kind: TrackKind, dur: Tick, ctx: &mut filmcraft_edit::EditCtx) {
    let (name, ek) = match kind {
        TrackKind::Video => ("cross_dissolve", EffectKind::VideoTransition),
        _ => ("constant_power", EffectKind::AudioTransition),
    };
    let Some(def) = filmcraft_project::vtransition::find_transition(name, ek) else { return };
    let half = dur.mul_ratio(1, 2);
    let pairs: Vec<_> = q
        .tracks(kind)
        .iter()
        .filter(|tr| !tr.locked)
        .filter_map(|tr| {
            let from = tr.items.iter().find(|it| it.end() == t)?;
            let to = tr.items.iter().find(|it| it.start == t)?;
            // both sides need `half` of media beyond the join, and must be long enough
            if from.duration < dur || to.duration < dur {
                return None;
            }
            Some((tr.id, from.id, to.id))
        })
        .collect();
    for (track, from, to) in pairs {
        let tr = Transition {
            id: TransitionId(0),
            effect: def.instance(),
            start: t - half,
            duration: dur,
            from: Some(from),
            to: Some(to),
            align: Default::default(),
            reverse: false,
        };
        let _ = filmcraft_edit::add_transition(q, track, tr, ctx);
    }
}

#[cfg(test)]
mod tests;

//! OverFilm: the whole edit in one go (see `OVERFILM.md`): Auto Cut, an optional Claude Code
//! review of the unclear cuts, the cuts applied, the kill scan (kills, ultimates, teams, players
//! and heroes off the HUD, so the captions can use the names), OverFilm Captions, an optional
//! Claude Code review of the captions and an optional render. Each step is the command the menus
//! run; this module only starts them one after another and waits for each background job.
//!
//! - `overfilm.flow.run {autoCut=true, templates?, keepBefore=10, fps=2, frozenPauses=true,
//!   reviewCuts=false, applyCuts=true, captions="new"|"reuse"|"off", audioTrack?, mode, speakers,
//!   numSpeakers?, extraTerms?, gpu, gameVoice, lineChars=16, maxLines=2, cleanFillers,
//!   reviewCaptions=false, captionStyle?, kills=true, killFps=2, render=false, renderFormat="h264",
//!   renderPath?, renderSplit="match"|"one", burnCaptions=true, captionSidecar?, wait=false}`: `captions` "new" recognises the speech
//!   (`overfilm.captions.generate`), "reuse" lays the transcript the sequence already has out again
//!   (`overfilm.captions.fromTranscript`, after the cuts: no second recognition). `applyCuts`
//!   false leaves the cut markers for the user to check (the captions then follow the uncut
//!   sequence). `captionStyle` names a caption style (`overfilm.captions.presets`): after the
//!   captions (and their review) it runs `overfilm.captions.stylize` with it, so the finished
//!   captions get the look (and a render shows the styled captions). Left out (or "keep") it keeps
//!   the style the sequence already has (its "OverFilm 자막 · <style>" track), so new captions never
//!   leave an old styled track showing the old words; "none" (or "") takes that track away and
//!   leaves plain captions. `kills` runs `overfilm.kills.scan` after the cuts and before the captions when the
//!   screen templates have name plates or a kill feed (else the step is left out); it is optional,
//!   so a failed scan is noted and the run goes on. `render` exports the finished sequence with
//!   `file.exportMedia` (`renderFormat` is any of its formats, burnt-in captions by default), or
//!   with `overfilm.premiere.export` when `renderFormat` is "premiere"; `renderPath` is a folder or
//!   a file (default: beside the project). `renderSplit` "match" (the default) writes one file per
//!   match when the sequence has several "OverFilm 경기" markers (different teams, from the kill
//!   scan), named `<date>-<team 1>vs<team 2>.<ext>`, each from its marker to the next one, so no
//!   part of the sequence is left out; a single match is one file under that name, and no match
//!   marker keeps the sequence's name. "one" writes the whole sequence as one file. The first step
//!   starts at once, so its errors come back here; a later step that
//!   fails stops the run (a review that fails is noted and the run goes on: it is optional).
//! - `overfilm.flow.status` returns the steps, each `pending`, `running` (with the job's progress
//!   and status), `done`, `skipped` or `failed` with a message.
//! - `overfilm.flow.stop` stops the running job and the steps after it.
//! - `overfilm.panel.settings {settings?, reset=false, file?}`: the OverFilm panel's saved choices
//!   (`overfilm-panel.json` in the OverFilm data folder), so the panel opens with them next time.
//!   `settings` (an object) is saved, merged over what is there; `reset` forgets them all. It
//!   answers `{settings, openAtStart}` (`openAtStart`: open the panel when the editor starts,
//!   on unless the user turned it off).
//!
//! While a run is going, the reviews that Claude Code review's "on its own" setting would start
//! after Auto Cut and OverFilm Captions are left to the run (it asks only when the user ticked
//! that review).

use filmcraft_project::ItemId;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq, str_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "overfilm.flow.run";

/// One step of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    AutoCut,
    ReviewCuts,
    ApplyCuts,
    Kills,
    Captions,
    ReviewCaptions,
    Style,
    Render,
}

impl StepKind {
    pub fn key(self) -> &'static str {
        match self {
            StepKind::AutoCut => "autoCut",
            StepKind::ReviewCuts => "reviewCuts",
            StepKind::ApplyCuts => "applyCuts",
            StepKind::Kills => "kills",
            StepKind::Captions => "captions",
            StepKind::ReviewCaptions => "reviewCaptions",
            StepKind::Style => "captionStyle",
            StepKind::Render => "render",
        }
    }
    /// Whether a failure only skips this step (the reviews, the kill scan and the caption style
    /// are optional: the captions are still made without the names, or plain).
    fn optional(self) -> bool {
        matches!(self, StepKind::ReviewCuts | StepKind::ReviewCaptions | StepKind::Kills | StepKind::Style)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepState {
    Pending,
    Running,
    Done,
    Skipped,
    Failed,
}

impl StepState {
    fn key(&self) -> &'static str {
        match self {
            StepState::Pending => "pending",
            StepState::Running => "running",
            StepState::Done => "done",
            StepState::Skipped => "skipped",
            StepState::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Step {
    pub kind: StepKind,
    pub state: StepState,
    pub message: String,
    /// The background job while it runs.
    pub job: Option<u64>,
    started: Option<web_time::Instant>,
    seconds: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowState {
    Running,
    Done,
    Failed,
    Stopped,
}

impl FlowState {
    fn key(self) -> &'static str {
        match self {
            FlowState::Running => "running",
            FlowState::Done => "done",
            FlowState::Failed => "failed",
            FlowState::Stopped => "stopped",
        }
    }
}

/// A run: its steps and the parameters each step's command gets.
#[derive(Clone, Debug)]
pub struct Flow {
    pub seq: ItemId,
    pub steps: Vec<Step>,
    pub state: FlowState,
    pub message: String,
    auto_cut: Value,
    captions: Value,
    reuse: bool,
    review_captions: Value,
    /// The caption style to put on the captions (`None`: take an earlier styled track away).
    style: Option<String>,
    kills: Value,
    render: Value,
    /// The render step's exports still to start (one per match), and the files written.
    render_queue: Vec<Value>,
    rendered: Vec<String>,
    wait: bool,
    started: web_time::Instant,
}

impl Flow {
    pub fn running(&self) -> bool {
        self.state == FlowState::Running
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: CMD,
            label: "Cut and Caption in One Go (OverFilm)…",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"autoCut":bool=true,"templates":path?,"keepBefore":seconds=10,"fps":0.5..8=2,"frozenPauses":bool=true,"reviewCuts":bool=false,"applyCuts":bool=true,"captions":"new"|"reuse"|"off"="new","audioTrack":n?,"mode":"general"|"commentary"="general","speakers":bool=true,"numSpeakers":n?,"extraTerms":str?,"gpu":bool=true,"gameVoice":"drop"|"label"|"keep"="drop","lineChars":n=16,"maxLines":1|2=2,"leadMs":n=120,"holdMs":n=400,"readingCps":n=12,"cleanFillers":bool=true,"reviewCaptions":bool=false,"captionStyle":str?,"kills":bool=true,"killFps":0.5..8=2,"render":bool=false,"renderFormat":"h264|hevc|av1|prores|dnxhr|mxf-op1a|gif|wav|aac|flac|premiere"="h264","renderPath":path?,"renderSplit":"match"|"one"="match","burnCaptions":bool=true,"captionSidecar":"srt|vtt"?,"wait":bool=false}"#,
            enabled: can_run,
            run,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.flow.status",
            label: "OverFilm One Go Status",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: crate::commands::always,
            run: status_cmd,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.panel.settings",
            label: "OverFilm Panel Settings",
            menu: &[],
            shortcut: None,
            params: r#"{"settings":{key:value}?,"reset":bool=false,"file":path?}"#,
            enabled: crate::commands::always,
            run: panel_settings_cmd,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.flow.stop",
            label: "Stop OverFilm One Go",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: is_running,
            run: stop_cmd,
            journal: false,
        },
    ]
}

fn can_run(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if s.overfilm_flow.as_ref().is_some_and(Flow::running) { Err("OverFilm One Go is already running".into()) } else { Ok(()) }
}

fn is_running(s: &Session) -> std::result::Result<(), String> {
    if s.overfilm_flow.as_ref().is_some_and(Flow::running) { Ok(()) } else { Err("OverFilm One Go is not running".into()) }
}

/// Whether a run is going (Claude Code review's "on its own" leaves the reviews to it).
pub fn active(s: &Session) -> bool {
    s.overfilm_flow.as_ref().is_some_and(Flow::running)
}

/// The keys copied from the run's parameters into a step's command.
fn pick(p: &Value, keys: &[&str]) -> Value {
    let mut out = serde_json::Map::new();
    for k in keys {
        if let Some(v) = p.get(*k).filter(|v| !v.is_null()) {
            out.insert((*k).to_string(), v.clone());
        }
    }
    Value::Object(out)
}

fn command_ready(s: &Session, id: &str) -> std::result::Result<(), String> {
    match crate::commands::find(id) {
        Some(c) => (c.enabled)(s),
        None => Err(format!("{id} is not in this build")),
    }
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let auto_cut = bool_p(p, "autoCut").unwrap_or(true);
    let review_cuts = bool_p(p, "reviewCuts").unwrap_or(false);
    let apply = bool_p(p, "applyCuts").unwrap_or(true);
    let captions = str_p(p, "captions").unwrap_or("new");
    let review_captions = bool_p(p, "reviewCaptions").unwrap_or(false);
    let kills = bool_p(p, "kills").unwrap_or(true) && kill_scan_ready(s, p);
    let render = bool_p(p, "render").unwrap_or(false);
    let render_format = str_p(p, "renderFormat").unwrap_or("h264");
    if render && render_format != "premiere" && filmcraft_export::Format::from_name(render_format).is_none() {
        return Err(bad(CMD, format!("renderFormat `{render_format}` is not an export format")));
    }
    if !matches!(str_p(p, "renderSplit").unwrap_or("match"), "match" | "one") {
        return Err(bad(CMD, "renderSplit is match (one file per match) or one (one file)"));
    }
    if !matches!(captions, "new" | "reuse" | "off") {
        return Err(bad(CMD, format!("captions is new, reuse or off, not {captions}")));
    }
    // cut markers already there (an earlier Auto Cut, Suggestions) are reviewed and applied too
    let marked = command_ready(s, "overfilm.autoCut.apply").is_ok();
    let mut kinds = Vec::new();
    if auto_cut {
        kinds.push(StepKind::AutoCut);
    }
    let review_cuts = review_cuts && (auto_cut || marked);
    if review_cuts {
        kinds.push(StepKind::ReviewCuts);
    }
    if apply && (auto_cut || marked) {
        kinds.push(StepKind::ApplyCuts);
    }
    if kills {
        kinds.push(StepKind::Kills);
    }
    if captions != "off" {
        kinds.push(StepKind::Captions);
        if review_captions {
            kinds.push(StepKind::ReviewCaptions);
        }
    }
    // the caption style: the one asked for, else the one the sequence has ("none": plain)
    let style = match str_p(p, "captionStyle").map(str::trim) {
        None | Some("keep") => crate::overfilm_captions::current_style(s),
        Some("" | "none") => None,
        Some(name) => {
            if !crate::overfilm_captions::preset_exists(s, name) {
                return Err(bad(CMD, format!("no caption style named {name}")));
            }
            Some(name.to_string())
        }
    };
    let styled_now = crate::overfilm_captions::current_style(s).is_some();
    // new captions get the style; a style asked for also goes on captions the sequence has
    let has_captions = s.active_sequence().is_some_and(|q| q.caption_tracks.first().is_some_and(|t| !t.captions.is_empty()));
    let restyle = if captions != "off" {
        style.is_some() || styled_now
    } else {
        style.is_some() && str_p(p, "captionStyle").is_some_and(|v| !matches!(v.trim(), "keep")) && has_captions
    };
    if restyle {
        kinds.push(StepKind::Style);
    }
    if render {
        kinds.push(StepKind::Render);
    }
    if kinds.is_empty() {
        return Err(bad(CMD, "nothing to do: turn on Auto Cut, the kill scan, the captions or the render"));
    }
    // what can be known now is checked now, not an hour in
    if (review_cuts || (review_captions && captions != "off")) && !crate::overfilm_review::settings(s).enabled {
        return Err(bad(CMD, "Claude Code review is off: turn it on in Claude Code Review Settings first, or untick the reviews"));
    }
    match captions {
        "new" => command_ready(s, "overfilm.captions.generate").map_err(|e| bad(CMD, format!("captions: {e}")))?,
        "reuse" if crate::transcript::sequence_words(s).is_empty() => {
            return Err(bad(CMD, "the sequence has no transcript to reuse yet: choose new captions"));
        }
        _ => {}
    }
    let wait = bool_p(p, "wait").unwrap_or(false);
    let mut cut_p = pick(p, &["templates", "keepBefore", "fps", "frozenPauses", "dissolveFrames"]);
    cut_p["mode"] = json!("mark");
    if auto_cut {
        // templates are read now so a missing one is said at once
        let path = crate::overfilm_autocut::templates_path(s, &cut_p, CMD)?;
        crate::overfilm_autocut::load_set(&path)?;
    }
    let cap_p = if captions == "reuse" {
        pick(p, &["gameVoice", "lineChars", "maxLines", "leadMs", "holdMs", "readingCps", "cleanFillers"])
    } else {
        pick(
            p,
            &[
                "items",
                "audioTrack",
                "mode",
                "speakers",
                "numSpeakers",
                "extraTerms",
                "gpu",
                "gameVoice",
                "lineChars",
                "maxLines",
                "leadMs",
                "holdMs",
                "readingCps",
                "cleanFillers",
            ],
        )
    };
    let rev_p = pick(p, &["lineChars", "maxLines", "mode"]);
    let mut kill_p = pick(p, &["templates"]);
    if let Some(fps) = p.get("killFps").filter(|v| !v.is_null()) {
        kill_p["fps"] = fps.clone();
    }
    let render_p = render_params(s, p, render_format);
    let steps =
        kinds.into_iter().map(|kind| Step { kind, state: StepState::Pending, message: String::new(), job: None, started: None, seconds: None }).collect();
    s.overfilm_flow = Some(Flow {
        seq,
        steps,
        state: FlowState::Running,
        message: String::new(),
        auto_cut: cut_p,
        captions: cap_p,
        reuse: captions == "reuse",
        review_captions: rev_p,
        style,
        kills: kill_p,
        render: render_p,
        render_queue: Vec::new(),
        rendered: Vec::new(),
        wait,
        started: web_time::Instant::now(),
    });
    // the first step starts here: its errors are the caller's
    if let Err(e) = drive(s) {
        s.overfilm_flow = None;
        return Err(e);
    }
    Ok(status(s))
}

fn status_cmd(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(status(s))
}

/// The OverFilm panel's settings file.
const PANEL_FILE: &str = "overfilm-panel.json";
/// A settings file bigger than this is not read (it is a few hundred bytes).
const PANEL_FILE_MAX: u64 = 256 * 1024;
/// Keys not kept: the names being typed (saved on their own when the user presses Save).
const PANEL_TRANSIENT: &[&str] = &["names"];

/// The saved panel settings (an empty object when there are none or the file is unreadable).
fn panel_load(path: &std::path::Path) -> serde_json::Map<String, Value> {
    let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= PANEL_FILE_MAX);
    let text = if small { std::fs::read_to_string(path).unwrap_or_default() } else { String::new() };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    }
}

fn panel_settings_cmd(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.panel.settings";
    let path = match str_p(p, "file").filter(|f| !f.trim().is_empty()) {
        Some(f) => std::path::PathBuf::from(f),
        None => std::path::PathBuf::from(crate::overfilm_autocut::user_file(PANEL_FILE)),
    };
    let reset = bool_p(p, "reset").unwrap_or(false);
    let mut saved = if reset { serde_json::Map::new() } else { panel_load(&path) };
    let given = match p.get("settings") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => Some(m),
        Some(_) => return Err(bad(CMD, "settings must be an object")),
    };
    if let Some(m) = given {
        for (k, v) in m {
            if !PANEL_TRANSIENT.contains(&k.as_str()) {
                saved.insert(k.clone(), v.clone());
            }
        }
    }
    if reset || given.is_some() {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| EngineError::Other(format!("{}: {e}", dir.display())))?;
        }
        let text = serde_json::to_string_pretty(&Value::Object(saved.clone())).map_err(|e| EngineError::Other(e.to_string()))?;
        std::fs::write(&path, text).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    }
    let open = saved.get("openAtStart").and_then(Value::as_bool).unwrap_or(true);
    Ok(json!({"settings": saved, "openAtStart": open}))
}

fn stop_cmd(s: &mut Session, _: &Value) -> Result<Value> {
    use std::sync::atomic::Ordering;
    let Some(f) = s.overfilm_flow.as_mut() else { return Ok(status(s)) };
    let job = f.steps.iter().find_map(|st| st.job);
    f.state = FlowState::Stopped;
    f.message = "멈췄습니다".into();
    for st in &mut f.steps {
        if matches!(st.state, StepState::Pending | StepState::Running) {
            st.state = StepState::Skipped;
            st.message = "멈춤".into();
            st.job = None;
        }
    }
    if let Some(j) = job.and_then(|id| s.jobs.iter().find(|j| j.id == id)) {
        j.progress.cancel.store(true, Ordering::Relaxed);
    }
    Ok(status(s))
}

/// The run as JSON (`{}` with `state: "none"` when nothing ran yet).
pub fn status(s: &Session) -> Value {
    use std::sync::atomic::Ordering;
    let Some(f) = s.overfilm_flow.as_ref() else { return json!({"state": "none", "steps": []}) };
    let steps: Vec<Value> = f
        .steps
        .iter()
        .map(|st| {
            let mut v = json!({"step": st.kind.key(), "state": st.state.key(), "message": st.message});
            if let Some(secs) = st.seconds {
                v["seconds"] = json!((secs * 10.0).round() / 10.0);
            }
            if let Some(j) = st.job.and_then(|id| s.jobs.iter().find(|j| j.id == id)) {
                v["job"] = json!(j.id);
                v["progress"] = json!(j.progress.fraction().clamp(0.0, 1.0));
                v["status"] = json!(job_status(j));
                v["etaSeconds"] = json!(j.progress.eta().map(|d| d.as_secs_f64()));
                v["done"] = json!(j.progress.done.load(Ordering::Relaxed));
            }
            v
        })
        .collect();
    json!({"state": f.state.key(), "message": f.message, "sequence": f.seq.0, "steps": steps, "seconds": (f.started.elapsed().as_secs_f64() * 10.0).round() / 10.0})
}

/// Move a run on. Called once per UI frame (after the OverFilm jobs applied their results).
pub fn poll(s: &mut Session) {
    if !active(s) {
        return;
    }
    if let Err(e) = drive(s) {
        fail(s, &e.to_string());
    }
}

enum JobState {
    Running,
    Done(String),
    Failed(String),
    Stopped,
}

/// A step's job: still running until its module applied the result.
fn job_state(s: &Session, kind: StepKind, id: u64) -> JobState {
    use std::sync::atomic::Ordering;
    let pending = match kind {
        StepKind::AutoCut => s.overfilm_jobs.iter().any(|j| j.job == id),
        StepKind::Captions => s.overfilm_caption_jobs.iter().any(|j| j.job == id),
        StepKind::ReviewCuts | StepKind::ReviewCaptions => s.overfilm_review.jobs.iter().any(|j| j.job == id),
        StepKind::Kills => s.overfilm_kill_jobs.iter().any(|j| j.job == id),
        StepKind::ApplyCuts | StepKind::Style | StepKind::Render => false,
    };
    let Some(job) = s.jobs.iter().find(|j| j.id == id) else {
        return if pending { JobState::Running } else { JobState::Done(String::new()) };
    };
    if job.progress.cancel.load(Ordering::Relaxed) {
        return if pending { JobState::Running } else { JobState::Stopped };
    }
    if pending || !job.progress.finished.load(Ordering::Relaxed) {
        return JobState::Running;
    }
    let status = job_status(job);
    match job.result.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        Some(Err(e)) => JobState::Failed(e),
        _ => JobState::Done(status),
    }
}

/// A job's status line.
fn job_status(j: &crate::Job) -> String {
    j.progress.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn fail(s: &mut Session, why: &str) {
    let Some(f) = s.overfilm_flow.as_mut() else { return };
    f.state = FlowState::Failed;
    f.message = why.to_string();
    for st in &mut f.steps {
        if st.state == StepState::Running {
            st.state = StepState::Failed;
            st.message = why.to_string();
            st.job = None;
        }
    }
    s.error_toast(CMD, format!("OverFilm 한 번에: {why}"));
}

/// Finish the current step and start the next ones until one runs in the background.
fn drive(s: &mut Session) -> Result<()> {
    loop {
        let Some(f) = s.overfilm_flow.as_ref() else { return Ok(()) };
        if !f.running() {
            return Ok(());
        }
        let Some(i) = f.steps.iter().position(|st| matches!(st.state, StepState::Pending | StepState::Running)) else {
            let secs = f.started.elapsed().as_secs_f64();
            if let Some(f) = s.overfilm_flow.as_mut() {
                f.state = FlowState::Done;
                f.message = format!("끝났습니다 ({})", clock(secs));
            }
            s.toast(format!("OverFilm 한 번에: 끝났습니다 ({})", clock(secs)));
            return Ok(());
        };
        let Some(step) = f.steps.get(i).cloned() else { return Ok(()) };
        if step.state == StepState::Running {
            let Some(id) = step.job else {
                set(s, i, StepState::Done, String::new());
                continue;
            };
            match job_state(s, step.kind, id) {
                JobState::Running => return Ok(()),
                JobState::Done(_) if step.kind == StepKind::Render => render_next(s, i)?,
                JobState::Done(m) => set(s, i, StepState::Done, m),
                JobState::Stopped => {
                    let _ = stop_cmd(s, &json!({}));
                    return Ok(());
                }
                JobState::Failed(e) if step.kind.optional() => set(s, i, StepState::Failed, e),
                JobState::Failed(e) => return Err(EngineError::Other(e)),
            }
            continue;
        }
        start(s, i, step.kind)?;
    }
}

fn set(s: &mut Session, i: usize, state: StepState, message: String) {
    let Some(st) = s.overfilm_flow.as_mut().and_then(|f| f.steps.get_mut(i)) else { return };
    st.state = state;
    st.message = message;
    st.job = None;
    st.seconds = st.started.map(|t| t.elapsed().as_secs_f64());
}

fn clock(secs: f64) -> String {
    let t = secs.max(0.0).round() as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}

/// Start step `i`: a background job (its id kept), or done at once.
fn start(s: &mut Session, i: usize, kind: StepKind) -> Result<()> {
    let Some(f) = s.overfilm_flow.as_ref() else { return Ok(()) };
    if s.state.active_sequence != Some(f.seq) {
        return Err(EngineError::Other("the sequence it was started on is no longer open; stopped".into()));
    }
    let wait = f.wait;
    let (cut_p, cap_p, rev_p, reuse) = (f.auto_cut.clone(), f.captions.clone(), f.review_captions.clone(), f.reuse);
    let (kill_p, render_p, style) = (f.kills.clone(), f.render.clone(), f.style.clone());
    if let Some(st) = s.overfilm_flow.as_mut().and_then(|f| f.steps.get_mut(i)) {
        st.state = StepState::Running;
        st.started = Some(web_time::Instant::now());
    }
    let with_wait = |mut v: Value| {
        if wait {
            v["wait"] = json!(true);
        }
        v
    };
    let out = match kind {
        StepKind::AutoCut => s.execute("overfilm.autoCut", with_wait(cut_p))?,
        StepKind::ReviewCuts => {
            if command_ready(s, "overfilm.review.cuts").is_err() {
                set(s, i, StepState::Skipped, "검토할 컷이 없습니다".into());
                return Ok(());
            }
            match s.execute("overfilm.review.cuts", with_wait(json!({}))) {
                Ok(v) => v,
                Err(e) => {
                    set(s, i, StepState::Failed, e.to_string());
                    return Ok(());
                }
            }
        }
        StepKind::ApplyCuts => {
            if command_ready(s, "overfilm.autoCut.apply").is_err() {
                set(s, i, StepState::Skipped, "자를 곳이 없습니다".into());
                return Ok(());
            }
            let v = s.execute("overfilm.autoCut.apply", json!({}))?;
            set(s, i, StepState::Done, format!("{}곳을 잘라냈습니다", v["cuts"].as_u64().unwrap_or(0)));
            return Ok(());
        }
        StepKind::Kills => match s.execute("overfilm.kills.scan", with_wait(kill_p)) {
            Ok(v) => v,
            Err(e) => {
                set(s, i, StepState::Failed, e.to_string());
                return Ok(());
            }
        },
        StepKind::Render => {
            let queue = render_exports(s, &render_p);
            if let Some(f) = s.overfilm_flow.as_mut() {
                f.render_queue = queue;
                f.rendered.clear();
            }
            render_next(s, i)?;
            return Ok(());
        }
        StepKind::Captions if reuse => {
            let v = s.execute("overfilm.captions.fromTranscript", cap_p)?;
            set(s, i, StepState::Done, format!("자막 {}개", v["captions"].as_u64().unwrap_or(0)));
            return Ok(());
        }
        StepKind::Captions => s.execute("overfilm.captions.generate", with_wait(cap_p))?,
        StepKind::Style => {
            let r = match &style {
                Some(name) => s
                    .execute("overfilm.captions.stylize", json!({"preset": name}))
                    .map(|v| format!("\"{name}\" 스타일로 자막 {}개", v["styled"].as_u64().unwrap_or(0))),
                None => crate::overfilm_captions::remove_styled(s).map(|n| if n > 0 { "스타일 없이 기본 자막으로".to_string() } else { String::new() }),
            };
            match r {
                Ok(m) => set(s, i, StepState::Done, m),
                Err(e) => set(s, i, StepState::Failed, e.to_string()),
            }
            return Ok(());
        }
        StepKind::ReviewCaptions => {
            if command_ready(s, "overfilm.review.captions").is_err() {
                set(s, i, StepState::Skipped, "검토할 자막이 없습니다".into());
                return Ok(());
            }
            match s.execute("overfilm.review.captions", with_wait(rev_p)) {
                Ok(v) => v,
                Err(e) => {
                    set(s, i, StepState::Failed, e.to_string());
                    return Ok(());
                }
            }
        }
    };
    match out["job"].as_u64() {
        Some(id) => {
            if let Some(st) = s.overfilm_flow.as_mut().and_then(|f| f.steps.get_mut(i)) {
                st.job = Some(id);
            }
        }
        // nothing to ask (a review with no unclear item)
        None => set(s, i, StepState::Done, out["message"].as_str().unwrap_or_default().to_string()),
    }
    Ok(())
}

/// Start the render step's next export, or finish the step when none is left (all of them at once
/// when the run waits).
fn render_next(s: &mut Session, i: usize) -> Result<()> {
    loop {
        let Some(f) = s.overfilm_flow.as_mut() else { return Ok(()) };
        let wait = f.wait;
        if f.render_queue.is_empty() {
            let files = f.rendered.clone();
            let message = match files.as_slice() {
                [] => String::new(),
                [one] => format!("{one} 로 내보냈습니다"),
                many => format!("{}개 파일로 내보냈습니다: {}", many.len(), many.join(", ")),
            };
            set(s, i, StepState::Done, message);
            return Ok(());
        }
        let mut next = f.render_queue.remove(0);
        if wait {
            next["wait"] = json!(true);
        }
        let premiere = next["format"] == "premiere";
        let v = if premiere {
            if let Some(m) = next.as_object_mut() {
                m.remove("format");
            }
            s.execute("overfilm.premiere.export", next)?
        } else {
            s.execute("file.exportMedia", next)?
        };
        let file = if premiere { v["xml"].as_str() } else { v["path"].as_str() }.unwrap_or_default().to_string();
        let Some(f) = s.overfilm_flow.as_mut() else { return Ok(()) };
        let left = f.render_queue.len();
        f.rendered.push(file.clone());
        let n = f.rendered.len();
        if let Some(st) = f.steps.get_mut(i) {
            st.message = if n + left > 1 { format!("{n}/{} {file} 로 내보내는 중", n + left) } else { format!("{file} 로 내보내는 중") };
            match v["job"].as_u64() {
                Some(id) if !wait => {
                    st.job = Some(id);
                    return Ok(());
                }
                _ => st.job = None,
            }
        }
    }
}

/// A match on the sequence: from its "OverFilm 경기" marker to the next one (the first from the
/// start, the last to the end), and its teams.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MatchSpan {
    pub start: filmcraft_time::Tick,
    pub end: filmcraft_time::Tick,
    pub teams: Vec<String>,
}

/// The teams of an "OverFilm 경기: 로제 vs 오투 블래스트" marker (or "OverFilm 경기 2: …"); `None`
/// for another marker. Unknown teams ("?") are left out.
fn match_teams(name: &str) -> Option<Vec<String>> {
    let rest = name.strip_prefix(crate::overfilm_lineup::MARK_MATCH)?;
    let (head, rest) = rest.split_once(": ")?;
    if !head.trim().chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(rest.split(" vs ").map(str::trim).filter(|t| !t.is_empty() && *t != "?").map(String::from).collect())
}

/// The matches on the active sequence, in order, covering it end to end (nothing is left out
/// between two matches); empty when it has no match marker.
pub(crate) fn match_spans(s: &Session) -> Vec<MatchSpan> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let end = q.duration();
    let mut marks: Vec<(filmcraft_time::Tick, Vec<String>)> =
        q.markers.iter().filter_map(|m| match_teams(&m.name).map(|t| (m.start.clamp(filmcraft_time::Tick::ZERO, end), t))).collect();
    marks.sort_by_key(|(at, _)| *at);
    // two markers at the same time are one match (a re-read): keep the one with more teams
    marks.dedup_by(|b, a| {
        if a.0 == b.0 {
            if b.1.len() > a.1.len() {
                a.1 = std::mem::take(&mut b.1);
            }
            true
        } else {
            false
        }
    });
    let n = marks.len();
    marks
        .iter()
        .enumerate()
        .map(|(k, (at, teams))| MatchSpan {
            start: if k == 0 { filmcraft_time::Tick::ZERO } else { *at },
            end: marks.get(k + 1).map_or(end, |(next, _)| *next),
            teams: teams.clone(),
        })
        .filter(|m| m.end > m.start || n == 1)
        .collect()
}

/// Today's date on the user's clock (`2026-11-03`), from the app's local UTC offset (UTC without
/// it, as in tests).
fn today(s: &Session) -> String {
    let unix = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let offset = s.persistence.as_ref().map_or(0, |p| (p.local_offset)(unix));
    chrono::DateTime::from_timestamp(unix.saturating_add(i64::from(offset)), 0).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// A match's file name: `<date>-<team 1>vs<team 2>` (`<date>-경기<n>` when no team was read).
fn match_file_name(date: &str, m: &MatchSpan, n: usize) -> String {
    let teams = if m.teams.is_empty() { format!("경기{n}") } else { m.teams.join("vs") };
    crate::export_tools::file_safe(&format!("{date}-{teams}"))
}

/// The render step's exports: one per match ("match", the default) or the whole sequence ("one",
/// the Premiere hand-over, or a sequence with no match marker).
fn render_exports(s: &Session, base: &Value) -> Vec<Value> {
    let split = base["split"].as_str() != Some("one");
    let mut base = base.clone();
    if let Some(m) = base.as_object_mut() {
        m.remove("split");
    }
    if base["format"] == "premiere" || !split {
        return vec![base];
    }
    let spans = match_spans(s);
    if spans.is_empty() {
        return vec![base];
    }
    // the folder: the one given, the folder of a file given, else beside the project
    let given = base["path"].as_str().unwrap_or_default().to_string();
    let dir = if given.is_empty() {
        crate::export_tools::default_export_dir(s)
    } else if given.ends_with(['/', '\\']) || std::path::Path::new(&given).is_dir() {
        std::path::PathBuf::from(&given)
    } else {
        std::path::Path::new(&given).parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| crate::export_tools::default_export_dir(s))
    };
    let date = today(s);
    let mut names: Vec<String> = Vec::new();
    let single = spans.len() == 1;
    spans
        .iter()
        .enumerate()
        .map(|(k, m)| {
            let mut name = match_file_name(&date, m, k + 1);
            // the same teams twice in a day: a number keeps both files
            let mut extra = 2;
            let first = name.clone();
            while names.contains(&name) {
                name = format!("{first}_{extra}");
                extra += 1;
            }
            names.push(name.clone());
            let mut v = base.clone();
            v["path"] = json!(dir.join(&name).to_string_lossy());
            if !single {
                v["range"] = json!("custom");
                v["startSeconds"] = json!(m.start.seconds());
                v["endSeconds"] = json!(m.end.seconds());
            }
            v
        })
        .collect()
}

/// Whether the kill scan can run with the run's screen templates (name plates or a kill feed
/// learnt); a missing or plain template file leaves the step out instead of failing the run.
fn kill_scan_ready(s: &Session, p: &Value) -> bool {
    let Ok(path) = crate::overfilm_autocut::templates_path(s, &pick(p, &["templates"]), CMD) else { return false };
    crate::overfilm_autocut::load_set(&path).is_ok_and(|set| !set.bars.is_empty() || set.killfeed.is_some())
}

/// The render step's command parameters: `file.exportMedia`'s, or `overfilm.premiere.export`'s
/// (`format: "premiere"` marks those). The path is the user's folder or file, else beside the
/// project.
fn render_params(s: &Session, p: &Value, format: &str) -> Value {
    let path = str_p(p, "renderPath").filter(|x| !x.trim().is_empty()).map(str::to_string);
    if format == "premiere" {
        let dir = path.unwrap_or_else(|| crate::export_tools::default_export_dir(s).to_string_lossy().into_owned());
        return json!({"format": "premiere", "dir": dir});
    }
    let mut v = pick(p, &["captionSidecar"]);
    v["format"] = json!(format);
    v["split"] = json!(str_p(p, "renderSplit").unwrap_or("match"));
    v["path"] = json!(path.unwrap_or_else(|| crate::export_tools::default_export_dir(s).to_string_lossy().into_owned()));
    v["burnCaptions"] = json!(bool_p(p, "burnCaptions").unwrap_or(true));
    v
}

#[cfg(test)]
#[path = "overfilm_flow_tests.rs"]
mod tests;

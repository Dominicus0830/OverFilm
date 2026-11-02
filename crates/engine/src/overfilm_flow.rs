//! OverFilm: the whole edit in one go (see `OVERFILM.md`): Auto Cut, an optional Claude Code
//! review of the unclear cuts, the cuts applied, OverFilm Captions, an optional Claude Code
//! review of the captions. Each step is the command the menus run; this module only starts them
//! one after another and waits for each background job.
//!
//! - `overfilm.flow.run {autoCut=true, templates?, keepBefore=10, fps=2, frozenPauses=true,
//!   reviewCuts=false, applyCuts=true, captions="new"|"reuse"|"off", audioTrack?, mode, speakers,
//!   numSpeakers?, extraTerms?, gpu, gameVoice, lineChars=16, maxLines=2, cleanFillers,
//!   reviewCaptions=false, wait=false}`: `captions` "new" recognises the speech
//!   (`overfilm.captions.generate`), "reuse" lays the transcript the sequence already has out again
//!   (`overfilm.captions.fromTranscript`, after the cuts: no second recognition). `applyCuts`
//!   false leaves the cut markers for the user to check (the captions then follow the uncut
//!   sequence). The first step starts at once, so its errors come back here; a later step that
//!   fails stops the run (a review that fails is noted and the run goes on: it is optional).
//! - `overfilm.flow.status` returns the steps, each `pending`, `running` (with the job's progress
//!   and status), `done`, `skipped` or `failed` with a message.
//! - `overfilm.flow.stop` stops the running job and the steps after it.
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
    Captions,
    ReviewCaptions,
}

impl StepKind {
    pub fn key(self) -> &'static str {
        match self {
            StepKind::AutoCut => "autoCut",
            StepKind::ReviewCuts => "reviewCuts",
            StepKind::ApplyCuts => "applyCuts",
            StepKind::Captions => "captions",
            StepKind::ReviewCaptions => "reviewCaptions",
        }
    }
    /// Whether a failure only skips this step (the reviews are optional).
    fn optional(self) -> bool {
        matches!(self, StepKind::ReviewCuts | StepKind::ReviewCaptions)
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
            params: r#"{"autoCut":bool=true,"templates":path?,"keepBefore":seconds=10,"fps":0.5..8=2,"frozenPauses":bool=true,"reviewCuts":bool=false,"applyCuts":bool=true,"captions":"new"|"reuse"|"off"="new","audioTrack":n?,"mode":"general"|"commentary"="general","speakers":bool=true,"numSpeakers":n?,"extraTerms":str?,"gpu":bool=true,"gameVoice":"drop"|"label"|"keep"="drop","lineChars":n=16,"maxLines":1|2=2,"cleanFillers":bool=false,"reviewCaptions":bool=false,"wait":bool=false}"#,
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
    if captions != "off" {
        kinds.push(StepKind::Captions);
        if review_captions {
            kinds.push(StepKind::ReviewCaptions);
        }
    }
    if kinds.is_empty() {
        return Err(bad(CMD, "nothing to do: turn on Auto Cut or the captions"));
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
        pick(p, &["gameVoice", "lineChars", "maxLines", "cleanFillers"])
    } else {
        pick(p, &["items", "audioTrack", "mode", "speakers", "numSpeakers", "extraTerms", "gpu", "gameVoice", "lineChars", "maxLines", "cleanFillers"])
    };
    let rev_p = pick(p, &["lineChars", "maxLines"]);
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
        StepKind::ApplyCuts => false,
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
        StepKind::Captions if reuse => {
            let v = s.execute("overfilm.captions.fromTranscript", cap_p)?;
            set(s, i, StepState::Done, format!("자막 {}개", v["captions"].as_u64().unwrap_or(0)));
            return Ok(());
        }
        StepKind::Captions => s.execute("overfilm.captions.generate", with_wait(cap_p))?,
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

#[cfg(test)]
#[path = "overfilm_flow_tests.rs"]
mod tests;

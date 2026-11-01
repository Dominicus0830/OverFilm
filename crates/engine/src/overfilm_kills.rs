//! OverFilm deaths and team fights read off the broadcast HUD (see `OVERFILM.md`).
//!
//! - `overfilm.playerBar.learn {team: 1|2, rect: [x, y, w, h], players=5, time?, clip?,
//!   templates?}` learns where one team's name plates are, on a frame where the whole team is
//!   alive (the playhead's by default), into the screen-template file.
//! - `overfilm.kills.scan {fps=2, clips?, templates?, fightGap=10, minKills=3, wait=false}` reads
//!   the plates a few times a second in a background job (only while the game HUD (`live`)
//!   template matches, when there is one) and marks each death as a point marker
//!   "OverFilm 처치: N팀 M번" and each run of at least `minKills` deaths no more than `fightGap`
//!   seconds apart as a range marker "OverFilm 교전 N" (5 s before the first death to 3 s after
//!   the last). Running it again replaces them. `overfilm.assist.suggest` scores the deaths in
//!   its highlights.

use std::sync::{Arc, Mutex};

use filmcraft_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_vision::{Death, DeathTracker, Fight, Luma, PlayerBar};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, f64_p, has_seq, u64_p};
use crate::overfilm_autocut::{WIDTH, frame_at, load_set, sample_plan, templates_path};
use crate::{EngineError, Result, Session};

/// Name prefix of the death markers.
pub const MARK_KILL: &str = "OverFilm 처치";
/// Name prefix of the team-fight markers.
pub const MARK_FIGHT: &str = "OverFilm 교전";
const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.playerBar.learn",
            label: "Learn Player Bar (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"team":1|2,"rect":[x,y,w,h],"players":n=5,"time":ticks?,"clip":id?,"templates":path?}"#,
            enabled: has_seq,
            run: learn,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.kills.scan",
            label: "Find Kills and Team Fights (OverFilm)",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"fps":0.5..8=2,"clips":[id]?,"templates":path?,"fightGap":seconds=10,"minKills":n=3,"wait":bool=false}"#,
            enabled: has_seq,
            run: scan,
            journal: true,
        },
    ]
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn learn(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.playerBar.learn";
    let team = u64_p(p, "team").filter(|t| (1..=2).contains(t)).ok_or_else(|| bad(CMD, "`team` is 1 or 2"))? as u8;
    let r: Vec<f32> = p.get("rect").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|f| f as f32).collect()).unwrap_or_default();
    let [x, y, w, h] = r.as_slice() else { return Err(bad(CMD, "`rect` is [x, y, w, h] in 0..1 around the team's name plates")) };
    let players = u64_p(p, "players").unwrap_or(5).clamp(1, overfilm_vision::kills::MAX_PLAYERS as u64) as u8;
    let path = templates_path(s, p, CMD)?;
    let t = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let frame = frame_at(s, crate::commands::clip_p(p, "clip"), t)?;
    let bar = PlayerBar::learn(&frame, team, overfilm_vision::NRect::new(*x, *y, *w, *h), players).map_err(|e| bad(CMD, e.to_string()))?;
    let dead = bar.dead(&frame);
    let mut set = load_set(&path)?;
    set.insert_bar(bar);
    std::fs::write(&path, set.to_json()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"templates": path, "team": team, "players": players, "bars": set.bars.len(), "readDead": dead}))
}

/// A running scan; [`poll`] marks its deaths.
pub struct PendingKills {
    pub job: u64,
    pub seq: ItemId,
    pub gap_ms: i64,
    pub min_kills: usize,
    pub deaths: Arc<Mutex<Option<Vec<Death>>>>,
}

fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.kills.scan";
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let path = templates_path(s, p, CMD)?;
    let set = load_set(&path)?;
    if set.bars.is_empty() {
        return Err(bad(
            CMD,
            format!("{path} has no player bars yet; learn each team's name plates first (overfilm.playerBar.learn on a frame where everyone is alive)"),
        ));
    }
    if s.overfilm_kill_jobs.iter().any(|j| j.seq == seq) {
        return Err(EngineError::Other("the kill scan is already running on this sequence".into()));
    }
    let fps = f64_p(p, "fps").filter(|f| f.is_finite()).unwrap_or(2.0).clamp(0.5, 8.0);
    let gap_ms = f64_p(p, "fightGap").filter(|v| v.is_finite()).map(|v| (v.clamp(1.0, 120.0) * 1000.0) as i64).unwrap_or(10_000);
    let min_kills = u64_p(p, "minKills").map(|n| n.clamp(1, 20) as usize).unwrap_or(3);
    let (work, total) = sample_plan(s, p, fps)?;
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label: "OverFilm Kills".into(), progress: Default::default(), result: Default::default() };
    job.progress.total.store(total, std::sync::atomic::Ordering::Relaxed);
    let deaths: Arc<Mutex<Option<Vec<Death>>>> = Arc::default();
    let (prog, res, out) = (job.progress.clone(), job.result.clone(), deaths.clone());
    let has_live = set.templates.iter().any(|t| t.screen == overfilm_vision::Screen::Live);
    // a plate must read the other way for half a second (at least two samples) to count
    let confirm = (fps / 2.0).ceil().max(2.0) as u8;
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        // nobody respawns in under 8 s
        let mut tracker = DeathTracker::new(set.bars.clone(), confirm, 8_000);
        let mut done = 0u64;
        let mut stopped = false;
        'clips: for w in &work {
            for (st, mt) in &w.times {
                if prog.cancel.load(Ordering::Relaxed) {
                    stopped = true;
                    break 'clips;
                }
                if let Ok(f) = w.src.video_frame(filmcraft_media::FrameRequest { time: *mt, scale: w.scale })
                    && let Some(l) = Luma::from_rgba(&f.to_rgba8(), f.width as usize, f.height as usize, WIDTH)
                {
                    let live = !has_live || set.live(&l);
                    tracker.push(st.0 / TICKS_PER_MS, &l, live);
                }
                done += 1;
                prog.done.store(done, Ordering::Relaxed);
                if done.is_multiple_of(32) {
                    *lock(&prog.status) = format!("{}: {}", w.name, overfilm_vision::plan::clock(*st));
                }
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        let d = tracker.into_deaths();
        *lock(&prog.status) = if stopped { "Stopped: nothing was changed".into() } else { format!("{} death(s) ({done} frames in {secs:.0}s)", d.len()) };
        let r = if stopped {
            Err("stopped".to_string())
        } else {
            *lock(&out) = Some(d);
            Ok(filmcraft_export::Report {
                path: String::new(),
                frames: done,
                seconds: secs,
                bytes: 0,
                render_fps: done as f64 / secs.max(1e-6),
                extra_files: Vec::new(),
            })
        };
        *lock(&res) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.overfilm_kill_jobs.push(PendingKills { job: id, seq, gap_ms, min_kills, deaths: deaths.clone() });
    let mut out = json!({"job": id, "frames": total, "templates": path});
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        let found = lock(&deaths).clone();
        poll(s);
        if let Some(d) = found {
            let f = overfilm_vision::kills::fights(&d, gap_ms, min_kills);
            out["deaths"] = json!(d.len());
            out["fights"] = f.iter().map(fight_json).collect();
        }
    } else {
        std::thread::Builder::new().name("overfilm-kills".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    if let Some(Err(e)) = s.jobs.iter().find(|j| j.id == id).and_then(|j| lock(&j.result).clone()) {
        return Err(EngineError::Other(e));
    }
    Ok(out)
}

fn clock(ms: i64) -> String {
    overfilm_vision::plan::clock(Tick(ms.saturating_mul(TICKS_PER_MS)))
}

fn fight_json(f: &Fight) -> Value {
    json!({"from": clock(f.start), "to": clock(f.end), "kills": f.kills(), "team1Dead": f.dead[0], "team2Dead": f.dead[1]})
}

pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    let mut i = 0;
    while i < s.overfilm_kill_jobs.len() {
        let job = s.jobs.iter().find(|j| j.id == s.overfilm_kill_jobs[i].job);
        let finished = job.is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        let cancelled = job.is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        if !finished {
            i += 1;
            continue;
        }
        let pj = s.overfilm_kill_jobs.remove(i);
        let Some(deaths) = lock(&pj.deaths).take().filter(|_| !cancelled) else { continue };
        if s.state.active_sequence != Some(pj.seq) {
            s.error_toast("overfilm.kills.scan", "the kill scan finished on a sequence that is no longer open; nothing was changed");
            continue;
        }
        if let Err(e) = mark(s, &deaths, pj.gap_ms, pj.min_kills) {
            s.error_toast("overfilm.kills.scan", format!("Kills: {e}"));
        }
    }
}

/// Mark `deaths` and their fights on the active sequence, replacing earlier ones (one undo step).
pub fn mark(s: &mut Session, deaths: &[Death], gap_ms: i64, min_kills: usize) -> Result<usize> {
    let fights = overfilm_vision::kills::fights(deaths, gap_ms, min_kills);
    let deaths = deaths.to_vec();
    let end = s.active_sequence().ok_or(EngineError::NoSequence)?.duration();
    let t = |ms: i64| Tick(ms.saturating_mul(TICKS_PER_MS)).clamp(Tick::ZERO, end);
    let n = fights.len();
    s.edit("OverFilm Kills", move |pr, st| {
        let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
        let ids: Vec<u64> = (0..deaths.len() + fights.len()).map(|_| pr.alloc_id()).collect();
        let mut ids = ids.into_iter();
        let q = pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        q.markers.retain(|m| !m.name.starts_with(MARK_KILL) && !m.name.starts_with(MARK_FIGHT));
        for d in &deaths {
            let Some(id) = ids.next() else { break };
            q.markers.push(Marker {
                id: MarkerId(id),
                start: t(d.at),
                duration: Tick::ZERO,
                name: format!("{MARK_KILL}: {}팀 {}번", d.team, d.player),
                comment: String::new(),
                kind: MarkerKind::Comment,
                color: if d.team == 1 { Label::Cerulean } else { Label::Rose },
            });
        }
        for (k, f) in fights.iter().enumerate() {
            let Some(id) = ids.next() else { break };
            let (a, b) = (t(f.start.saturating_sub(5_000)), t(f.end.saturating_add(3_000)));
            q.markers.push(Marker {
                id: MarkerId(id),
                start: a,
                duration: b - a,
                name: format!("{MARK_FIGHT} {}", k + 1),
                comment: format!("{}킬 (1팀 {}명, 2팀 {}명 처치됨)", f.kills(), f.dead[0], f.dead[1]),
                kind: MarkerKind::Comment,
                color: Label::Mango,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    s.toast(format!("OverFilm: {} death(s), {n} team fight(s) marked", deaths_len(s)));
    Ok(n)
}

fn deaths_len(s: &Session) -> usize {
    s.active_sequence().map(|q| q.markers.iter().filter(|m| m.name.starts_with(MARK_KILL)).count()).unwrap_or(0)
}

/// Times (ms) of the death markers on the active sequence, for the highlights.
pub(crate) fn kill_times_ms(s: &Session) -> Vec<i64> {
    s.active_sequence().map(|q| q.markers.iter().filter(|m| m.name.starts_with(MARK_KILL)).map(|m| m.start.0 / TICKS_PER_MS).collect()).unwrap_or_default()
}

#[cfg(test)]
#[path = "overfilm_kills_tests.rs"]
mod tests;

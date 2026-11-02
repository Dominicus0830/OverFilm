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
//! - `overfilm.ultIcon.learn {team: 1|2, rect: [x, y, w, h], player=1, time?, clip?, templates?}`
//!   learns the team's ready-ultimate icon (the white circle that replaces the charge number on
//!   a name plate) from player `player`'s plate on a frame where that ultimate is ready.
//! - With icons learnt, the same scan also follows the ultimates (one more pass over frames it
//!   decodes anyway; the deaths it finds are the same): each use is a point marker
//!   "OverFilm 궁극기: N팀 M번" and each death of the other team no more than `ultWindow`
//!   (6) seconds after a use a point marker "OverFilm 궁극기 처치: N팀 M번 ← N팀 M번 궁극기".
//!   Without a kill feed to read, such a kill is the ultimate's by timing, not certainly.
//! - `overfilm.killFeed.learn {rect: [x, y, w, h], icon: [x, y, w, h], time?, clip?, templates?}`:
//!   on broadcasts that show the game's kill feed, `rect` around the feed and `icon` round one
//!   ultimate kill's icon (the white disc before the arrow). The scan then takes the ultimate
//!   kills from the feed instead ("OverFilm 궁극기 처치: N팀 M번 (킬피드)"), each matched to the
//!   death on the plates and to the killer's ultimate when it can be. Without `icon` (no
//!   ultimate kill on that frame) `rect` is enough: the ultimate icons are then read on each row.
//! - With a kill feed learnt, the scan also reads **who killed whom** off it: each row's team
//!   colours, the players' names (as pictures of the text: the same name seen again is the same
//!   player) and their heroes' pictures, and the ultimate icon. Each kill is a point marker
//!   "OverFilm 킬피드: 선수 1 (영웅 1) → 선수 2 (영웅 3)" ("(궁극기)" when made with one). Players
//!   and heroes are numbered as they first appear until named.
//! - `overfilm.killFeed.list {}`: the kills on the active sequence (kill feed markers, else the
//!   deaths on the name plates) with their times, and the players and heroes of the last scan.
//! - `overfilm.killFeed.name {kind: "player"|"hero", id, name, templates?}`: name player or hero
//!   `id` (as numbered in the list); the name is kept in the template file, so later scans know
//!   that name or picture again, and the kill feed markers are written again with it.

use std::sync::{Arc, Mutex};

use filmcraft_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_vision::{
    Death, DeathTracker, FeedReader, FeedScan, FeedTracker, Fight, KillFeed, Luma, PlayerBar, Rgb, Side, UltIcon, UltTracker, UltUse, feedrows, ults,
};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, f64_p, has_seq, u64_p};
use crate::overfilm_autocut::{WIDTH, frame_at, load_set, rgba_at, sample_plan, templates_path};
use crate::{EngineError, Result, Session};

/// Name prefix of the death markers.
pub const MARK_KILL: &str = "OverFilm 처치";
/// Name prefix of the team-fight markers.
pub const MARK_FIGHT: &str = "OverFilm 교전";
/// Name prefix of the ultimate markers (the ultimate-kill markers start with it too).
pub const MARK_ULT: &str = "OverFilm 궁극기";
/// Name prefix of the ultimate-kill markers.
pub const MARK_ULT_KILL: &str = "OverFilm 궁극기 처치";
/// Name prefix of the who-killed-whom markers read off the kill feed.
pub const MARK_FEED: &str = "OverFilm 킬피드";
/// Frames are decoded at most this wide while the kill feed is read (its names are small).
const FEED_WIDTH: usize = 1920;
/// The kill feed's crop is read at most this wide.
const FEED_CROP: usize = 1024;
/// A use whose own player is found dead this soon after (ms) was the plate turning dark.
const ULT_DEATH_GUARD: i64 = 1_500;
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
            label: "Find Kills, Ultimates and Team Fights (OverFilm)",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"fps":0.5..8=2,"clips":[id]?,"templates":path?,"fightGap":seconds=10,"minKills":n=3,"ultWindow":seconds=6,"wait":bool=false}"#,
            enabled: has_seq,
            run: scan,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.ultIcon.learn",
            label: "Learn Ultimate Icon (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"team":1|2,"rect":[x,y,w,h],"player":n=1,"time":ticks?,"clip":id?,"templates":path?}"#,
            enabled: has_seq,
            run: learn_ult,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.killFeed.learn",
            label: "Learn Kill Feed (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"rect":[x,y,w,h],"icon":[x,y,w,h]?,"time":ticks?,"clip":id?,"templates":path?}"#,
            enabled: has_seq,
            run: learn_feed,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.killFeed.list",
            label: "List Kills (OverFilm)",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_seq,
            run: list_kills,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.killFeed.name",
            label: "Name Player or Hero (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"kind":"player"|"hero","id":n,"name":str,"templates":path?}"#,
            enabled: has_seq,
            run: name_feed,
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
    let mut bar = bar;
    // the ultimate icon sits on the first plate, wherever the bar was drawn: keep it
    bar.ult = set.bars.iter().find(|b| b.team == team).and_then(|b| b.ult.clone());
    set.insert_bar(bar);
    crate::overfilm_autocut::write_file(&path, &set.to_json())?;
    Ok(json!({"templates": path, "team": team, "players": players, "bars": set.bars.len(), "readDead": dead}))
}

fn learn_ult(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.ultIcon.learn";
    let team = u64_p(p, "team").filter(|t| (1..=2).contains(t)).ok_or_else(|| bad(CMD, "`team` is 1 or 2"))? as u8;
    let r: Vec<f32> = p.get("rect").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|f| f as f32).collect()).unwrap_or_default();
    let [x, y, w, h] = r.as_slice() else { return Err(bad(CMD, "`rect` is [x, y, w, h] in 0..1 around one ready ultimate icon")) };
    let player = u64_p(p, "player").unwrap_or(1).clamp(1, overfilm_vision::kills::MAX_PLAYERS as u64) as u8;
    let path = templates_path(s, p, CMD)?;
    let mut set = load_set(&path)?;
    let Some(bar) = set.bars.iter_mut().find(|b| b.team == team) else {
        return Err(bad(CMD, format!("{path} has no player bar for team {team} yet; learn its name plates first (overfilm.playerBar.learn)")));
    };
    let t = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let frame = frame_at(s, crate::commands::clip_p(p, "clip"), t)?;
    let icon = UltIcon::learn(&frame, bar, overfilm_vision::NRect::new(*x, *y, *w, *h), player).map_err(|e| bad(CMD, e.to_string()))?;
    bar.ult = Some(icon);
    let ready = bar.ult_ready(&frame).unwrap_or_default();
    crate::overfilm_autocut::write_file(&path, &set.to_json())?;
    Ok(json!({"templates": path, "team": team, "player": player, "readReady": ready}))
}

fn rect4(p: &Value, key: &str) -> Option<overfilm_vision::NRect> {
    let r: Vec<f32> = p.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|f| f as f32).collect()).unwrap_or_default();
    match r.as_slice() {
        [x, y, w, h] => Some(overfilm_vision::NRect::new(*x, *y, *w, *h)),
        _ => None,
    }
}

fn learn_feed(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.killFeed.learn";
    let rect = rect4(p, "rect").ok_or_else(|| bad(CMD, "`rect` is [x, y, w, h] in 0..1 around the kill feed"))?;
    let path = templates_path(s, p, CMD)?;
    let t = p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead());
    let (rgba, fw, fh) = rgba_at(s, crate::commands::clip_p(p, "clip"), t, FEED_WIDTH)?;
    let frame = Luma::from_rgba(&rgba, fw, fh, WIDTH).ok_or_else(|| EngineError::Other("empty frame".into()))?;
    let crop = Rgb::crop_rgba(&rgba, fw, fh, rect, FEED_CROP).ok_or_else(|| bad(CMD, "`rect` is empty or outside the frame"))?;
    let rows = feedrows::read_rows(&crop, None);
    let mut feed = match rect4(p, "icon") {
        Some(icon) => KillFeed::learn(&frame, rect, icon).map_err(|e| bad(CMD, e.to_string()))?,
        None => {
            if rows.is_empty() {
                return Err(bad(CMD, "no kill feed row in the rectangle: draw it around the kill feed on a frame where it shows kills"));
            }
            KillFeed { rect: rect.clamped().ok_or_else(|| bad(CMD, "`rect` is empty or outside the frame"))?, radius: 0.0, row: 0.0 }
        }
    };
    feed.row = feedrows::row_height(&crop).unwrap_or(0.0);
    let found = feed.icons(&frame).len();
    let mut set = load_set(&path)?;
    set.killfeed = Some(feed);
    crate::overfilm_autocut::write_file(&path, &set.to_json())?;
    Ok(json!({"templates": path, "icons": found, "rows": rows.len()}))
}

/// The last kill feed reading of a sequence, for naming its players and heroes.
pub struct FeedState {
    pub seq: ItemId,
    pub scan: FeedScan,
}

/// A running scan; [`poll`] marks its deaths (and ultimates, when the bars have icons).
pub struct PendingKills {
    pub job: u64,
    pub seq: ItemId,
    pub gap_ms: i64,
    pub min_kills: usize,
    pub deaths: Arc<Mutex<Option<Vec<Death>>>>,
    /// Ultimate kill window (ms).
    pub ult_window_ms: i64,
    /// `None` without ultimate icons: the ultimate markers are left alone.
    pub ults: Arc<Mutex<Option<Vec<UltUse>>>>,
    /// `None` without a kill feed: ultimate kills come from timing.
    pub feed: Arc<Mutex<Option<Vec<i64>>>>,
    /// Who killed whom, read off the kill feed.
    pub feed_scan: Arc<Mutex<Option<FeedScan>>>,
}

fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.kills.scan";
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let path = templates_path(s, p, CMD)?;
    let set = load_set(&path)?;
    if set.bars.is_empty() && set.killfeed.is_none() {
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
    let ult_window_ms = f64_p(p, "ultWindow").filter(|v| v.is_finite()).map(|v| (v.clamp(1.0, 30.0) * 1000.0) as i64).unwrap_or(6_000);
    let (work, total) = sample_plan(s, p, fps)?;
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label: "OverFilm Kills".into(), progress: Default::default(), result: Default::default() };
    job.progress.total.store(total, std::sync::atomic::Ordering::Relaxed);
    let deaths: Arc<Mutex<Option<Vec<Death>>>> = Arc::default();
    let ults: Arc<Mutex<Option<Vec<UltUse>>>> = Arc::default();
    let feed: Arc<Mutex<Option<Vec<i64>>>> = Arc::default();
    let feed_scan: Arc<Mutex<Option<FeedScan>>> = Arc::default();
    let (prog, res, out, ult_out, feed_out, scan_out) =
        (job.progress.clone(), job.result.clone(), deaths.clone(), ults.clone(), feed.clone(), feed_scan.clone());
    let has_live = set.templates.iter().any(|t| t.screen == overfilm_vision::Screen::Live);
    // a plate must read the other way for half a second (at least two samples) to count
    let confirm = (fps / 2.0).ceil().max(2.0) as u8;
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        // nobody respawns in under 8 s
        let mut tracker = DeathTracker::new(set.bars.clone(), confirm, 8_000);
        let mut ult_tracker = UltTracker::new(set.bars.clone(), confirm);
        // the ultimate icons by their rim when one was learnt, else as each row reads them
        let mut feed_tracker = set.killfeed.clone().filter(|f| f.radius > 0.0).map(|f| FeedTracker::new(f, confirm));
        let mut reader = set.killfeed.as_ref().map(|f| FeedReader::new(set.feed_names.clone(), confirm, (f.row > 0.0).then_some(f.row)));
        let feed_rect = set.killfeed.as_ref().map(|f| f.rect);
        let mut done = 0u64;
        let mut stopped = false;
        'clips: for w in &work {
            for (st, mt) in &w.times {
                if prog.cancel.load(Ordering::Relaxed) {
                    stopped = true;
                    break 'clips;
                }
                // the kill feed's names are small: decode bigger while it is read
                let scale = match (&reader, w.src.info().video.as_ref()) {
                    (Some(_), Some(v)) => ((FEED_WIDTH as f64 / v.width.max(1) as f64).min(1.0) as f32).max(w.scale),
                    _ => w.scale,
                };
                if let Ok(f) = w.src.video_frame(filmcraft_media::FrameRequest { time: *mt, scale })
                    && let rgba = f.to_rgba8()
                    && let Some(l) = Luma::from_rgba(&rgba, f.width as usize, f.height as usize, WIDTH)
                {
                    if let (Some(r), Some(rect)) = (reader.as_mut(), feed_rect)
                        && let Some(crop) = Rgb::crop_rgba(&rgba, f.width as usize, f.height as usize, rect, FEED_CROP)
                    {
                        r.push(st.0 / TICKS_PER_MS, &crop, !has_live || set.live(&l));
                    }
                    let live = !has_live || set.live(&l);
                    tracker.push(st.0 / TICKS_PER_MS, &l, live);
                    ult_tracker.push(st.0 / TICKS_PER_MS, &l, live);
                    if let Some(f) = feed_tracker.as_mut() {
                        f.push(st.0 / TICKS_PER_MS, &l, live);
                    }
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
            let read = reader.map(FeedReader::finish);
            match (feed_tracker, &read) {
                (Some(f), _) => *lock(&feed_out) = Some(f.into_kills()),
                (None, Some(r)) => *lock(&feed_out) = Some(r.ult_times()),
                _ => {}
            }
            *lock(&scan_out) = read;
            if !ult_tracker.is_empty() {
                *lock(&ult_out) = Some(ults::without_deaths(&ult_tracker.into_uses(), &d, ULT_DEATH_GUARD));
            }
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
    s.overfilm_kill_jobs.push(PendingKills {
        job: id,
        seq,
        gap_ms,
        min_kills,
        deaths: deaths.clone(),
        ult_window_ms,
        ults: ults.clone(),
        feed: feed.clone(),
        feed_scan: feed_scan.clone(),
    });
    let mut out = json!({"job": id, "frames": total, "templates": path});
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        let found = lock(&deaths).clone();
        let found_ults = lock(&ults).clone();
        let found_feed = lock(&feed).clone();
        let found_scan = lock(&feed_scan).clone();
        poll(s);
        if let Some(fs) = found_scan {
            out["feedKills"] = fs.events.iter().map(|e| json!({"at": clock(e.at), "kill": fs.line(e), "ult": e.ult})).collect();
        }
        if let Some(d) = found {
            let f = overfilm_vision::kills::fights(&d, gap_ms, min_kills);
            out["deaths"] = json!(d.len());
            out["fights"] = f.iter().map(fight_json).collect();
            if found_ults.is_some() || found_feed.is_some() {
                let u = found_ults.unwrap_or_default();
                out["ultimates"] = json!(u.len());
                out["ultimateKills"] = json!(ult_kill_marks(&u, &d, ult_window_ms, found_feed.as_deref()).len());
            }
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
        let found_ults = lock(&pj.ults).take();
        let found_feed = lock(&pj.feed).take();
        let found_scan = lock(&pj.feed_scan).take();
        if s.state.active_sequence != Some(pj.seq) {
            s.error_toast("overfilm.kills.scan", "the kill scan finished on a sequence that is no longer open; nothing was changed");
            continue;
        }
        if let Err(e) = mark(s, &deaths, pj.gap_ms, pj.min_kills) {
            s.error_toast("overfilm.kills.scan", format!("Kills: {e}"));
        }
        if (found_ults.is_some() || found_feed.is_some())
            && let Err(e) = mark_ults(s, &found_ults.unwrap_or_default(), &deaths, pj.ult_window_ms, found_feed.as_deref())
        {
            s.error_toast("overfilm.kills.scan", format!("Ultimates: {e}"));
        }
        if let Some(fs) = found_scan {
            if let Err(e) = mark_feed(s, &fs) {
                s.error_toast("overfilm.kills.scan", format!("Kill feed: {e}"));
            }
            s.overfilm_feed = Some(FeedState { seq: pj.seq, scan: fs });
        }
    }
}

/// Mark who killed whom (`scan`, read off the kill feed) on the active sequence, replacing
/// earlier such markers (one undo step).
pub fn mark_feed(s: &mut Session, scan: &FeedScan) -> Result<usize> {
    let end = s.active_sequence().ok_or(EngineError::NoSequence)?.duration();
    let t = |ms: i64| Tick(ms.saturating_mul(TICKS_PER_MS)).clamp(Tick::ZERO, end);
    let marks: Vec<(Tick, String, String, Label)> = scan
        .events
        .iter()
        .map(|e| {
            let name = format!("{MARK_FEED}: {}{}", scan.line(e), if e.ult { " (궁극기)" } else { "" });
            let comment = match e.killer {
                Some(k) => format!("{} → {}", k.side.ko(), e.victim.side.ko()),
                None => format!("{} 처치됨", e.victim.side.ko()),
            };
            let color = match e.killer.map(|k| k.side) {
                Some(Side::Blue) => Label::Cerulean,
                Some(Side::Red) => Label::Rose,
                None => Label::Tan,
            };
            (t(e.at), name, comment, color)
        })
        .collect();
    let n = marks.len();
    s.edit("OverFilm Kill Feed", move |pr, st| {
        let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
        let ids: Vec<u64> = (0..marks.len()).map(|_| pr.alloc_id()).collect();
        let q = pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        q.markers.retain(|m| !m.name.starts_with(MARK_FEED));
        for ((start, name, comment, color), id) in marks.into_iter().zip(ids) {
            q.markers.push(Marker { id: MarkerId(id), start, duration: Tick::ZERO, name, comment, kind: MarkerKind::Comment, color });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    s.toast(format!("OverFilm: {n} kill(s) read off the kill feed"));
    Ok(n)
}

fn list_kills(s: &mut Session, _: &Value) -> Result<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let feed: Vec<&Marker> = q.markers.iter().filter(|m| m.name.starts_with(MARK_FEED)).collect();
    let (source, prefix, list) = if feed.is_empty() {
        ("plates", MARK_KILL, q.markers.iter().filter(|m| m.name.starts_with(MARK_KILL)).collect::<Vec<_>>())
    } else {
        ("killfeed", MARK_FEED, feed)
    };
    let kills: Vec<Value> = list
        .iter()
        .map(|m| {
            let text = m.name.strip_prefix(prefix).unwrap_or(&m.name).trim_start_matches(':').trim();
            json!({
                "time": m.start.0,
                "clock": overfilm_vision::plan::clock(m.start),
                "seconds": m.start.0 as f64 / TICKS_PER_SECOND as f64,
                "text": text,
                "comment": m.comment,
                "ult": text.ends_with("(궁극기)"),
            })
        })
        .collect();
    let seq = s.state.active_sequence;
    let (players, heroes) = match s.overfilm_feed.as_ref().filter(|f| Some(f.seq) == seq) {
        Some(f) => (
            f.scan
                .players
                .iter()
                .enumerate()
                .map(|(i, c)| json!({"id": i + 1, "name": f.scan.player(i), "named": c.label.is_some(), "seen": c.seen}))
                .collect(),
            f.scan.heroes.iter().enumerate().map(|(i, c)| json!({"id": i + 1, "name": f.scan.hero(i), "named": c.label.is_some(), "seen": c.seen})).collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };
    Ok(json!({"source": if kills.is_empty() { "none" } else { source }, "kills": kills, "players": players, "heroes": heroes}))
}

fn name_feed(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.killFeed.name";
    let hero = match crate::commands::str_p(p, "kind") {
        Some("player") => false,
        Some("hero") => true,
        _ => return Err(bad(CMD, "`kind` is \"player\" or \"hero\"")),
    };
    let id = u64_p(p, "id").filter(|i| *i >= 1).ok_or_else(|| bad(CMD, "`id` is the number shown in the list (1, 2, …)"))? as usize;
    let name = crate::commands::str_p(p, "name").map(str::trim).unwrap_or_default().to_string();
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let path = templates_path(s, p, CMD)?;
    let Some(state) = s.overfilm_feed.as_mut().filter(|f| f.seq == seq) else {
        return Err(bad(CMD, "read the kill feed first (overfilm.kills.scan with a kill feed learnt)"));
    };
    let list = if hero { &mut state.scan.heroes } else { &mut state.scan.players };
    let Some(c) = list.get_mut(id - 1) else { return Err(bad(CMD, format!("no {} {id} in the last scan", if hero { "hero" } else { "player" }))) };
    c.label = (!name.is_empty()).then(|| name.chars().take(64).collect());
    let sig = c.sig.clone();
    let scan = state.scan.clone();
    let mut set = load_set(&path)?;
    set.feed_names.teach(hero, &name, &sig);
    crate::overfilm_autocut::write_file(&path, &set.to_json())?;
    let kills = scan
        .events
        .iter()
        .filter(|e| {
            let is = |w: &overfilm_vision::Who| if hero { w.hero == Some(id - 1) } else { w.player == id - 1 };
            is(&e.victim) || e.killer.as_ref().is_some_and(is)
        })
        .count();
    mark_feed(s, &scan)?;
    Ok(json!({"templates": path, "kind": if hero { "hero" } else { "player" }, "id": id, "name": name, "kills": kills}))
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

/// One ultimate-kill marker: time, victim, killer's ultimate, read off the kill feed.
type UltKillMark = (i64, Option<Death>, Option<UltUse>, bool);

/// The ultimate kills to mark: from the kill feed when the broadcast shows one (`feed`), else the
/// deaths of the other team no more than `window_ms` after an ultimate.
fn ult_kill_marks(uses: &[UltUse], deaths: &[Death], window_ms: i64, feed: Option<&[i64]>) -> Vec<UltKillMark> {
    match feed {
        Some(f) => ults::feed_kills(f, deaths, uses, window_ms).into_iter().map(|k| (k.at, k.death, k.by, true)).collect(),
        None => ults::ult_kills(uses, deaths, window_ms).into_iter().map(|k| (k.death.at, Some(k.death), Some(k.by), false)).collect(),
    }
}

/// Mark ultimate uses and ultimate kills (see [`ult_kill_marks`]) on the active sequence,
/// replacing earlier ones (one undo step). Returns (uses, ultimate kills).
pub fn mark_ults(s: &mut Session, uses: &[UltUse], deaths: &[Death], window_ms: i64, feed: Option<&[i64]>) -> Result<(usize, usize)> {
    let kills = ult_kill_marks(uses, deaths, window_ms, feed);
    let uses = uses.to_vec();
    let end = s.active_sequence().ok_or(EngineError::NoSequence)?.duration();
    let t = |ms: i64| Tick(ms.saturating_mul(TICKS_PER_MS)).clamp(Tick::ZERO, end);
    let n = (uses.len(), kills.len());
    s.edit("OverFilm Ultimates", move |pr, st| {
        let seq = st.active_sequence.ok_or(EngineError::NoSequence)?;
        let ids: Vec<u64> = (0..uses.len() + kills.len()).map(|_| pr.alloc_id()).collect();
        let mut ids = ids.into_iter();
        let q = pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        q.markers.retain(|m| !m.name.starts_with(MARK_ULT));
        for u in &uses {
            let Some(id) = ids.next() else { break };
            let k = kills.iter().filter(|k| k.2 == Some(*u)).count();
            let how = if feed.is_some() { "" } else { " (킬피드가 없어 시간으로 추정)" };
            q.markers.push(Marker {
                id: MarkerId(id),
                start: t(u.at),
                duration: Tick::ZERO,
                name: format!("{MARK_ULT}: {}팀 {}번", u.team, u.player),
                comment: if k > 0 { format!("{k}킬로 이어짐{how}") } else { String::new() },
                kind: MarkerKind::Comment,
                color: Label::Violet,
            });
        }
        for (at, death, by, from_feed) in &kills {
            let Some(id) = ids.next() else { break };
            let victim = death.map(|d| format!(": {}팀 {}번", d.team, d.player)).unwrap_or_default();
            let killer = by.map(|u| format!(" ← {}팀 {}번 궁극기", u.team, u.player)).unwrap_or_default();
            let name = if *from_feed { format!("{MARK_ULT_KILL}{victim}{killer} (킬피드)") } else { format!("{MARK_ULT_KILL}{victim}{killer}") };
            let comment = match (from_feed, by) {
                (true, _) => "킬피드의 궁극기 아이콘".to_string(),
                (false, Some(u)) => format!("궁극기 {:.1}초 뒤 처치 (킬피드가 없어 시간으로 추정)", at.saturating_sub(u.at) as f64 / 1000.0),
                (false, None) => String::new(),
            };
            q.markers.push(Marker { id: MarkerId(id), start: t(*at), duration: Tick::ZERO, name, comment, kind: MarkerKind::Comment, color: Label::Yellow });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    s.toast(format!("OverFilm: {} ultimate(s), {} ultimate kill(s) marked", n.0, n.1));
    Ok(n)
}

fn deaths_len(s: &Session) -> usize {
    s.active_sequence().map(|q| q.markers.iter().filter(|m| m.name.starts_with(MARK_KILL)).count()).unwrap_or(0)
}

/// Times (ms) of the death markers on the active sequence, for the highlights: the name plates'
/// deaths, or the kill feed's kills on a video without plates.
pub(crate) fn kill_times_ms(s: &Session) -> Vec<i64> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let of = |prefix: &str| q.markers.iter().filter(|m| m.name.starts_with(prefix)).map(|m| m.start.0 / TICKS_PER_MS).collect::<Vec<_>>();
    let plates = of(MARK_KILL);
    if plates.is_empty() { of(MARK_FEED) } else { plates }
}

#[cfg(test)]
#[path = "overfilm_kills_tests.rs"]
mod tests;

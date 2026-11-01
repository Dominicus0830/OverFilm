//! OverFilm highlight reel (see `OVERFILM.md`): the "OverFilm 하이라이트" markers of the active
//! sequence become a new sequence of just those moments.
//!
//! - `overfilm.highlights.reel {name?, dissolve=true, dissolveFrames?, open=true}` copies the
//!   active sequence as "<sequence> 하이라이트" (or `name`), then ripple-deletes everything outside
//!   the highlight markers on every track, captions included, so the clips keep their effects,
//!   keyframes, links and caption styles. The moments stay in time order; overlapping markers join.
//!   With `dissolve`, each join gets a short cross dissolve and crossfade (the default transition
//!   lengths, or `dissolveFrames`) where both sides have media to overlap; with a learnt editing
//!   style and neither given, the style decides (plain cuts, or its usual dissolve length). The source sequence is
//!   not changed; one undo step removes the reel.

use filmcraft_edit::EditCtx;
use filmcraft_project::{ItemKind, TrackKind};
use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, str_p};
use crate::overfilm_assist::MARK_HIGHLIGHT;
use crate::{EngineError, Result, Session};

/// Prefix of the review's "keep" markers (dropped from the reel like the cut markers).
const MARK_KEEP: &str = "OverFilm 유지";

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "overfilm.highlights.reel",
        label: "Make Highlight Reel (OverFilm)",
        menu: &["Sequence", "OverFilm"],
        shortcut: None,
        params: r#"{"name":str?,"dissolve":bool=true,"dissolveFrames":n?,"open":bool=true}"#,
        enabled: has_highlights,
        run: reel,
        journal: true,
    }]
}

fn has_highlights(s: &Session) -> std::result::Result<(), String> {
    match s.active_sequence() {
        None => Err("no sequence".into()),
        Some(q) if q.markers.iter().any(|m| m.name.starts_with(MARK_HIGHLIGHT) && m.duration > Tick::ZERO) => Ok(()),
        Some(_) => Err("no OverFilm highlight markers; run Suggest Silences and Highlights first".into()),
    }
}

/// The highlight ranges of a sequence `dur` long: snapped to frames, clamped, in time order,
/// overlapping or touching ranges joined.
pub(crate) fn moments(q: &filmcraft_project::Sequence) -> Vec<TimeRange> {
    let rate = q.settings.frame_rate;
    let dur = q.duration();
    let mut r: Vec<TimeRange> = q
        .markers
        .iter()
        .filter(|m| m.name.starts_with(MARK_HIGHLIGHT) && m.duration > Tick::ZERO)
        .map(|m| TimeRange::from_bounds(rate.snap(m.start).max(Tick::ZERO), rate.snap(m.start + m.duration).min(dur)))
        .filter(|r| r.duration > Tick::ZERO)
        .collect();
    r.sort_by_key(|r| r.start);
    let mut out: Vec<TimeRange> = Vec::with_capacity(r.len());
    for x in r {
        match out.last_mut() {
            Some(l) if x.start <= l.end() => *l = TimeRange::from_bounds(l.start, l.end().max(x.end())),
            _ => out.push(x),
        }
    }
    out
}

/// The gaps around `keep` in `[0, dur)`.
fn complement(keep: &[TimeRange], dur: Tick) -> Vec<TimeRange> {
    let mut out = Vec::new();
    let mut at = Tick::ZERO;
    for k in keep {
        if k.start > at {
            out.push(TimeRange::from_bounds(at, k.start));
        }
        at = at.max(k.end());
    }
    if dur > at {
        out.push(TimeRange::from_bounds(at, dur));
    }
    out
}

fn reel(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.highlights.reel";
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?.clone();
    let keep = moments(&q);
    if keep.is_empty() {
        return Err(bad(CMD, "no OverFilm highlight markers on this sequence"));
    }
    let drop = complement(&keep, q.duration());
    let rate = q.settings.frame_rate;
    let mut frames = p.get("dissolveFrames").and_then(Value::as_i64).map(|f| f.clamp(0, 120));
    let mut dissolve = bool_p(p, "dissolve").unwrap_or(true);
    // no say in the call: join the moments the way the learnt style joins cuts
    if frames.is_none()
        && bool_p(p, "dissolve").is_none()
        && let Some(j) = crate::overfilm_style::saved(s, p).and_then(|st| st.join_ms())
    {
        dissolve = j > 0;
        frames = Some(rate.frame_at(Tick(j.saturating_mul(TICKS_PER_SECOND / 1000))).clamp(1, 120));
    }
    let vdur = if !dissolve { Tick::ZERO } else { frames.map(|f| rate.tick_of(f)).unwrap_or_else(|| s.prefs.timeline.video_transition_duration(rate)) };
    let adur = if !dissolve { Tick::ZERO } else { frames.map(|f| rate.tick_of(f)).unwrap_or_else(|| s.prefs.timeline.audio_transition_duration(rate)) };
    let src_name = s.project.item(seq_id).map(|i| i.name.clone()).unwrap_or_default();
    let name = match str_p(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.chars().take(200).collect(),
        None => {
            let base = format!("{src_name} 하이라이트");
            std::iter::once(base.clone())
                .chain((2..1000).map(|k| format!("{base} {k}")))
                .find(|n| !s.project.items.values().any(|i| &i.name == n))
                .unwrap_or(base)
        }
    };
    let media = s.media.clone();
    let n = keep.len();
    let (id, total) = s.edit("OverFilm Highlight Reel", |pr, st| {
        let (label, bin) = (pr.item(seq_id).map(|i| i.label).unwrap_or(filmcraft_project::Label::Forest), pr.root.parent_of(seq_id));
        let mut copy = q.clone();
        copy.mark_in = None;
        copy.mark_out = None;
        copy.work_area = None;
        // ripple delete skips locked tracks: unlock for the cut, lock again after
        let locked: Vec<bool> = copy.all_tracks().map(|t| t.locked).collect();
        for t in copy.all_tracks_mut() {
            t.locked = false;
        }
        // captions follow the cut too
        let captions: Vec<(bool, bool)> = copy.caption_tracks.iter().map(|c| (c.locked, c.sync_lock)).collect();
        for c in copy.caption_tracks.iter_mut() {
            c.locked = false;
            c.sync_lock = true;
        }
        let snapshot = std::sync::Arc::new(pr.clone());
        let snap = snapshot.clone();
        let durations = move |i| crate::media_duration(&snapshot, &media, i);
        let starts = move |i| crate::media_start(&snap, i);
        let mut next = pr.next_id;
        let mut ctx = EditCtx { next_id: &mut next, media_duration: &durations, media_start: &starts, min_duration: rate.frame_duration() };
        let mut joins: Vec<Tick> = Vec::new();
        // latest first so earlier ranges keep their times
        for r in drop.iter().rev() {
            filmcraft_edit::transcript::ripple_delete_ranges(&mut copy, vec![*r], &mut ctx);
            crate::sequence_tools::ripple_markers(&mut copy.markers, r.end(), -r.duration);
            for j in joins.iter_mut() {
                *j -= r.duration;
            }
            // a join only between two moments (not the lead-in or the tail)
            if r.start > Tick::ZERO && r.end() < q.duration() {
                joins.push(r.start);
            }
        }
        // cut and review-keep markers are about the source's cut, not the reel
        copy.markers.retain(|m| !m.name.starts_with(crate::overfilm_autocut::MARK) && !m.name.starts_with(MARK_KEEP));
        for t in joins {
            if vdur > Tick::ZERO {
                crate::overfilm_autocut::dissolve_at(&mut copy, t, TrackKind::Video, vdur, &mut ctx);
            }
            if adur > Tick::ZERO {
                crate::overfilm_autocut::dissolve_at(&mut copy, t, TrackKind::Audio, adur, &mut ctx);
            }
        }
        for (t, l) in copy.all_tracks_mut().zip(locked) {
            t.locked = l;
        }
        for (c, (l, sync)) in copy.caption_tracks.iter_mut().zip(captions) {
            c.locked = l;
            c.sync_lock = sync;
        }
        copy.check().map_err(EngineError::Other)?;
        pr.next_id = next;
        let total = copy.duration();
        let id = pr.add_item(&name, label, ItemKind::Sequence(std::sync::Arc::new(copy)), bin);
        st.project_selection = vec![id];
        Ok((id, total))
    })?;
    if bool_p(p, "open").unwrap_or(true) {
        s.execute("sequence.open", json!({"item": id.0}))?;
    }
    s.toast(format!("OverFilm: highlight reel \"{name}\", {n} moment(s), {}", overfilm_vision::plan::clock(total)));
    Ok(json!({"sequence": id.0, "name": name, "moments": n, "seconds": total.0 as f64 / TICKS_PER_SECOND as f64}))
}

#[cfg(test)]
#[path = "overfilm_reel_tests.rs"]
mod tests;

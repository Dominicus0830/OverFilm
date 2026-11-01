//! OverFilm editing style (see `OVERFILM.md`): learnt from finished sequences (a Premiere Pro edit
//! brought in as FCP XML with File ▸ Import, or one edited here), then followed by the
//! suggestions.
//!
//! - `overfilm.style.learn {sequences: [id]?, style: path?}` reads the sequences selected in the
//!   Project panel (or the open one): shot lengths and cuts per minute, how long one source plays
//!   on, transitions, added effects and zooms (keyframed Motion), how much of the footage is kept,
//!   caption length and lines. The style is saved beside the project (`<project>.overfilm-style.json`)
//!   or at `style`, and a Korean summary comes back.
//! - `overfilm.style.show {style?}` gives the saved style and its summary.
//!
//! With a saved style, `overfilm.assist.suggest` makes highlights as long as the editor lets a
//! moment run (unless `seconds` is given) and `overfilm.highlights.reel` joins the moments the way
//! the editor joins cuts (unless `dissolve` / `dissolveFrames` is given).

use filmcraft_project::{ItemId, ItemKind, Sequence, TrackKind};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_ai::style::{CaptionFact, EditFacts, Style};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, str_p};
use crate::{EngineError, Result, Session};

const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;
/// Most sequences learnt in one go.
const MAX_SEQUENCES: usize = 200;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.style.learn",
            label: "Learn Editing Style (OverFilm, Beta)",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"sequences":[id]?,"style":path?}"#,
            enabled: crate::commands::always,
            run: learn,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.style.show",
            label: "Show Editing Style (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"style":path?}"#,
            enabled: crate::commands::always,
            run: show,
            journal: false,
        },
    ]
}

/// The style file: `style`; else one beside the saved project when it exists; else the user's own.
pub(crate) fn style_path(s: &Session, p: &Value) -> Option<String> {
    if let Some(t) = str_p(p, "style").filter(|t| !t.trim().is_empty()) {
        return Some(t.to_string());
    }
    if let Some(proj) = s.path.as_deref() {
        let beside = format!("{}.overfilm-style.json", proj.strip_suffix(".fcproj").unwrap_or(proj));
        if std::path::Path::new(&beside).is_file() {
            return Some(beside);
        }
    }
    Some(crate::overfilm_autocut::user_file("overfilm-style.json"))
}

/// The saved style, if there is one and it reads.
pub(crate) fn saved(s: &Session, p: &Value) -> Option<Style> {
    let path = style_path(s, p)?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&text).ok().and_then(|v| serde_json::from_value(v.get("style")?.clone()).ok())
}

fn ms(t: Tick) -> i64 {
    t.0 / TICKS_PER_MS
}

/// What `q` shows about the editing (`media_len` gives a source's length, when known).
pub fn facts(q: &Sequence, media_len: &dyn Fn(ItemId) -> Option<Tick>) -> EditFacts {
    let end = q.duration();
    let video = q.tracks(TrackKind::Video);
    // edit points: where any enabled video clip starts or ends
    let mut points: Vec<Tick> = video
        .iter()
        .filter(|t| t.enabled)
        .flat_map(|t| t.items.iter().filter(|i| i.enabled).flat_map(|i| [i.start, i.end()]))
        .filter(|&t| t > Tick::ZERO && t < end)
        .collect();
    points.sort_unstable();
    points.dedup();
    let mut shots = Vec::with_capacity(points.len() + 1);
    let mut at = Tick::ZERO;
    for &p in points.iter().chain(std::iter::once(&end)) {
        if p > at {
            shots.push(ms(p - at));
        }
        at = p;
    }
    // the main footage: the video track with the most clip time
    let main = video.iter().max_by_key(|t| t.items.iter().map(|i| i.duration.0).fold(0i64, i64::saturating_add));
    let mut segments = Vec::new();
    let (mut sources, mut kept) = (0i64, 0i64);
    if let Some(tr) = main {
        let mut items: Vec<_> = tr.items.iter().filter(|i| i.enabled).collect();
        items.sort_by_key(|i| i.start);
        let slack = q.settings.frame_rate.frame_duration().mul_ratio(2, 1);
        let mut cur: Option<(ItemId, Tick, Tick, Tick)> = None; // item, timeline end, source end, length
        for it in &items {
            let src_in = it.source_time_at(it.start);
            let src_out = it.moving_source_time_at(it.end());
            match cur.as_mut() {
                Some(c) if c.0 == it.item && (it.start - c.1).0.abs() <= slack.0 && (src_in - c.2).0.abs() <= slack.0 => {
                    c.1 = it.end();
                    c.2 = src_out;
                    c.3 += it.duration;
                }
                _ => {
                    if let Some(c) = cur.take() {
                        segments.push(ms(c.3));
                    }
                    cur = Some((it.item, it.end(), src_out, it.duration));
                }
            }
        }
        if let Some(c) = cur {
            segments.push(ms(c.3));
        }
        let mut used: Vec<ItemId> = items.iter().map(|i| i.item).collect();
        used.sort_unstable_by_key(|i| i.0);
        used.dedup();
        let lens: Vec<Tick> = used.iter().filter_map(|i| media_len(*i)).filter(|t| *t > Tick::ZERO).collect();
        if lens.len() == used.len() && !used.is_empty() {
            sources = lens.iter().map(|t| ms(*t)).fold(0, i64::saturating_add);
            kept = items.iter().map(|i| ms(i.moving_source_time_at(i.end()) - i.source_time_at(i.start)).abs()).fold(0, i64::saturating_add);
        }
    }
    let transitions =
        video.iter().flat_map(|t| t.transitions.iter()).filter(|t| t.duration > Tick::ZERO).map(|t| (t.effect.effect.clone(), ms(t.duration))).collect();
    let intrinsic = ["motion", "opacity", "time_remap", "volume", "channel_volume", "panner"];
    let mut effects = Vec::new();
    for tr in q.all_tracks() {
        for it in &tr.items {
            for e in &it.effects {
                if e.effect == "motion" && e.params.iter().any(|(k, v)| (k == "scale" || k == "position" || k.starts_with("scale")) && v.is_animated()) {
                    effects.push("zoom".to_string());
                } else if !intrinsic.contains(&e.effect.as_str()) && e.enabled {
                    effects.push(e.effect.clone());
                }
            }
        }
    }
    let captions = q
        .caption_tracks
        .iter()
        .flat_map(|t| t.captions.iter())
        .map(|c| CaptionFact { chars: c.text.chars().filter(|ch| !ch.is_whitespace()).count(), lines: c.text.lines().count().max(1), ms: ms(c.duration) })
        .collect();
    EditFacts {
        length_ms: ms(end),
        shots_ms: shots,
        segments_ms: segments,
        transitions,
        cuts: points.len(),
        effects,
        sources_ms: sources,
        kept_ms: kept,
        captions,
    }
}

fn learn(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.style.learn";
    let path = style_path(s, p).ok_or_else(|| bad(CMD, "save the project first, or give `style` (a .json path)"))?;
    let given: Vec<ItemId> = p.get("sequences").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(ItemId).collect()).unwrap_or_default();
    let mut ids: Vec<ItemId> =
        if given.is_empty() { s.state.project_selection.iter().copied().filter(|i| s.project.sequence(*i).is_some()).collect() } else { given };
    if ids.is_empty() {
        ids.extend(s.state.active_sequence);
    }
    ids.truncate(MAX_SEQUENCES);
    if ids.is_empty() {
        return Err(bad(CMD, "select finished sequences in the Project panel (import a Premiere Pro edit as FCP XML first), or open one"));
    }
    let mut all = Vec::new();
    let mut names = Vec::new();
    for id in &ids {
        let it = s.project.item(*id).ok_or_else(|| bad(CMD, format!("no item {}", id.0)))?;
        let ItemKind::Sequence(q) = &it.kind else { return Err(bad(CMD, format!("{} is not a sequence", it.name))) };
        let media_len = |i: ItemId| crate::media_duration(&s.project, &s.media, i);
        all.push(facts(q, &media_len));
        names.push(it.name.clone());
    }
    let style = overfilm_ai::style::learn(&all);
    let summary = style.summary_ko();
    let doc = json!({"learntFrom": names, "style": style, "summary": summary});
    let text = serde_json::to_string_pretty(&doc).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::overfilm_autocut::write_file(&path, &text)?;
    s.toast(format!("OverFilm: editing style learnt from {} sequence(s)", names.len()));
    Ok(
        json!({"style": path, "learntFrom": names, "summary": summary, "highlightSeconds": style.highlight_length_ms().map(|m| m as f64 / 1000.0), "joinSeconds": style.join_ms().map(|m| m as f64 / 1000.0)}),
    )
}

fn show(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.style.show";
    let path = style_path(s, p).ok_or_else(|| bad(CMD, "save the project first, or give `style` (a .json path)"))?;
    let text = std::fs::read_to_string(&path).map_err(|e| bad(CMD, format!("{path}: {e} (learn a style first)")))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| bad(CMD, format!("{path}: {e}")))?;
    let style: Style = v.get("style").cloned().and_then(|x| serde_json::from_value(x).ok()).ok_or_else(|| bad(CMD, format!("{path}: no style in it")))?;
    Ok(json!({"style": path, "learntFrom": v["learntFrom"], "summary": style.summary_ko(), "detail": style}))
}

#[cfg(test)]
#[path = "overfilm_style_tests.rs"]
mod tests;

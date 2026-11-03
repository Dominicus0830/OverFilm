//! OverFilm: a look at what is being named (see `OVERFILM.md`).
//!
//! `overfilm.names.preview {kind: "plate"|"player"|"hero", id, templates?}`: where to see hero
//! `id` of the name plates (`plate`, numbered as in `overfilm.heroes.list`) or player or hero
//! `id` of the kill feed (as in `overfilm.killFeed.list`): `{time, rect: [x, y, w, h]}`, the
//! sequence time of a frame showing it and the part of the frame around it (the whole plate, or
//! the kill feed row). The panel shows that part of the frame beside the name box
//! ([`preview_image`]) and can move the playhead there.

use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_vision::{NRect, heroes};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, has_seq, str_p, u64_p};
use crate::overfilm_autocut::{load_set, rgba_at, templates_path};
use crate::{EngineError, Result, Session};

const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;
/// Previews are decoded at most this wide (the plates and the feed's names are small).
const DECODE_WIDTH: usize = 1920;
/// A preview is at most this wide.
const MAX_PREVIEW: usize = 480;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "overfilm.names.preview",
        label: "Show What to Name (OverFilm)",
        menu: &[],
        shortcut: None,
        params: r#"{"kind":"plate"|"player"|"hero","id":n,"templates":path?}"#,
        enabled: has_seq,
        run: preview,
        journal: false,
    }]
}

/// The kinds of things named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A hero on the name plates.
    Plate,
    /// A player in the kill feed.
    Player,
    /// A hero in the kill feed.
    Hero,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "plate" => Some(Kind::Plate),
            "player" => Some(Kind::Player),
            "hero" => Some(Kind::Hero),
            _ => None,
        }
    }
}

fn padded(r: NRect, px: f32, py: f32) -> NRect {
    let (x0, y0) = ((r.x - px).max(0.0), (r.y - py).max(0.0));
    let (x1, y1) = ((r.x + r.w + px).min(1.0), (r.y + r.h + py).min(1.0));
    NRect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Where to see `id` (1-based) of `kind`: (sequence time, part of the frame).
pub fn locate(s: &Session, kind: Kind, id: usize, templates: &str) -> Result<(Tick, NRect)> {
    const CMD: &str = "overfilm.names.preview";
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let i = id.checked_sub(1).ok_or_else(|| bad(CMD, "`id` is the number shown in the list (1, 2, …)"))?;
    let set = load_set(templates)?;
    match kind {
        Kind::Plate => {
            let st = s.overfilm_lineup.as_ref().filter(|l| l.seq == seq).ok_or_else(|| bad(CMD, "read the name plates first (overfilm.kills.scan)"))?;
            // a second after it first came on a plate, so the portrait has settled
            let e = st.heroes.events.iter().find(|e| e.to == i).ok_or_else(|| bad(CMD, format!("no hero {id} in the last scan")))?;
            let bar = set.bars.iter().find(|b| b.team == e.team).ok_or_else(|| bad(CMD, format!("{templates} has no player bar for team {}", e.team)))?;
            let k = e.player.saturating_sub(1);
            let pw = bar.rect.w / f32::from(bar.players.max(1));
            let top = heroes::portrait(bar, k).y.min(bar.rect.y);
            let plate = NRect::new(bar.rect.x + pw * f32::from(k), top, pw, (bar.rect.y + bar.rect.h - top).max(0.0));
            Ok((Tick(e.at.saturating_add(1_000).saturating_mul(TICKS_PER_MS)), padded(plate, pw * 0.15, bar.rect.h * 0.6)))
        }
        Kind::Player | Kind::Hero => {
            let f = s.overfilm_feed.as_ref().filter(|f| f.seq == seq).ok_or_else(|| bad(CMD, "read the kill feed first (overfilm.kills.scan)"))?;
            let hero = kind == Kind::Hero;
            let is = |w: &overfilm_vision::Who| if hero { w.hero == Some(i) } else { w.player == i };
            // its latest kill, on the last sample that showed the row
            let e = f
                .scan
                .events
                .iter()
                .rev()
                .find(|e| is(&e.victim) || e.killer.as_ref().is_some_and(is))
                .ok_or_else(|| bad(CMD, format!("no {} {id} in the last scan", if hero { "hero" } else { "player" })))?;
            let feed = set.killfeed.as_ref().ok_or_else(|| bad(CMD, format!("{templates} has no kill feed")))?;
            let row = if feed.row.is_finite() && feed.row > 0.0 { feed.row.min(1.0) } else { 0.12 } * feed.rect.h;
            let y = feed.rect.y + e.y.clamp(0.0, 1.0) * feed.rect.h;
            let band = NRect::new(feed.rect.x, y - row * 0.8, feed.rect.w, row * 1.6);
            Ok((Tick(e.seen.saturating_mul(TICKS_PER_MS)), padded(band, 0.0, 0.0)))
        }
    }
}

/// The part of the frame showing `id` of `kind` (see [`locate`]) as RGBA: (pixels, width, height,
/// sequence time).
pub fn preview_image(s: &Session, kind: Kind, id: usize, templates: &str) -> Result<(Vec<u8>, usize, usize, Tick)> {
    let (t, r) = locate(s, kind, id, templates)?;
    let (rgba, w, h) = rgba_at(s, None, t, DECODE_WIDTH)?;
    let img = overfilm_vision::Rgb::crop_rgba(&rgba, w, h, r, MAX_PREVIEW).ok_or_else(|| EngineError::Other("nothing to show there".into()))?;
    let mut out = Vec::with_capacity(img.px.len() / 3 * 4);
    for c in img.px.chunks_exact(3) {
        out.extend_from_slice(c);
        out.push(255);
    }
    Ok((out, img.width, img.height, t))
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.names.preview";
    let kind = str_p(p, "kind").and_then(Kind::parse).ok_or_else(|| bad(CMD, "`kind` is \"plate\", \"player\" or \"hero\""))?;
    let id = u64_p(p, "id").filter(|i| *i >= 1).ok_or_else(|| bad(CMD, "`id` is the number shown in the list (1, 2, …)"))? as usize;
    let path = templates_path(s, p, CMD)?;
    let (t, r) = locate(s, kind, id, &path)?;
    Ok(json!({"time": t.0, "clock": overfilm_vision::plan::clock(t), "rect": [r.x, r.y, r.w, r.h], "templates": path}))
}

#[cfg(test)]
#[path = "overfilm_names_tests.rs"]
mod tests;

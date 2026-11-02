//! Ultimate kills read off the game's kill feed, on broadcasts that show it.
//!
//! A kill feed row is "killer > victim"; a kill made with an ultimate has the ultimate's icon, a
//! white disc with a white rim and a dark symbol, just before the arrow. Every hero's symbol is
//! different but the round white rim is the same, so a [`KillFeed`] (the feed's rectangle and the
//! icon's radius, learnt from one ultimate kill on a frame of the same broadcast) finds the icons by
//! that rim: a ring of bright samples with darker surroundings. Nothing of the icon itself is
//! stored. A new icon that stays for a few samples is an ultimate kill ([`FeedTracker`]); the row
//! moves up or down as older rows leave, but not sideways, so an icon is followed by its x.

use serde::{Deserialize, Serialize};

use crate::VisionError;
use crate::luma::{Luma, NRect};

/// A sample this bright counts as the white rim.
const BRIGHT: f32 = 170.0;
/// Samples round each ring.
const N: usize = 24;
/// Most icons looked for in one frame (a feed holds about six rows).
const MAX_ICONS: usize = 12;

/// Where a broadcast's kill feed is and how big its ultimate icons are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KillFeed {
    /// Around all of the feed's rows (frame-relative).
    pub rect: NRect,
    /// The icon's radius as a share of the frame height (0: not learnt; the rows' own reading
    /// of the icon is used, see [`crate::feedrows`]).
    pub radius: f32,
    /// One row's height as a share of the rectangle's height (0: not learnt).
    #[serde(default)]
    pub row: f32,
}

/// An icon found on one frame: its centre (frame-relative).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedIcon {
    pub x: f32,
    pub y: f32,
}

impl KillFeed {
    /// Learn from `frame`, where an ultimate kill's icon is inside `icon` (a box drawn round it)
    /// and the feed inside `rect`.
    pub fn learn(frame: &Luma, rect: NRect, icon: NRect) -> Result<KillFeed, VisionError> {
        let rect = rect.clamped().ok_or(VisionError::EmptyRect)?;
        let icon = icon.clamped().ok_or(VisionError::EmptyRect)?;
        let aspect = frame.width as f32 / frame.height.max(1) as f32;
        // the box's shorter side, in frame heights
        let side = (icon.w * aspect).min(icon.h);
        let (cx, cy) = (icon.x + icon.w / 2.0, icon.y + icon.h / 2.0);
        // the drawn box is rarely exact: try radii round half its side; the rim reads bright over
        // a range of them, take the middle of that range
        let tries: Vec<(f32, f32)> = (0..=16).map(|i| side * (0.30 + 0.025 * i as f32)).map(|r| (r, ring_score(frame, cx, cy, r, aspect))).collect();
        let best = tries.iter().map(|t| t.1).fold(0.0f32, f32::max);
        if best <= 0.0 {
            return Err(VisionError::Templates("no round ultimate icon in the box: draw it tight round one white ultimate icon of the kill feed".into()));
        }
        let good: Vec<f32> = tries.iter().filter(|t| t.1 >= best * 0.9).map(|t| t.0).collect();
        let radius = good.get(good.len() / 2).copied().unwrap_or(side / 2.0);
        let feed = KillFeed { rect, radius, row: 0.0 };
        if feed.icons(frame).is_empty() {
            return Err(VisionError::Templates("the ultimate icon is not inside the kill feed's rectangle".into()));
        }
        Ok(feed)
    }

    /// Ultimate icons on `frame`, top to bottom.
    pub fn icons(&self, frame: &Luma) -> Vec<FeedIcon> {
        let (Some(rect), true) = (self.rect.clamped(), self.radius.is_finite() && self.radius > 0.0) else { return Vec::new() };
        let aspect = frame.width as f32 / frame.height.max(1) as f32;
        // at most one pixel apart, and at most 200 steps across (a huge rectangle is not scanned
        // pixel by pixel)
        let r_px = self.radius * frame.height as f32;
        if r_px < 3.0 {
            return Vec::new();
        }
        let step_y = (self.radius / 6.0).max(rect.h / 200.0).max(1.0 / frame.height.max(1) as f32);
        let step_x = step_y / aspect;
        let mut hits: Vec<(f32, FeedIcon)> = Vec::new();
        let mut y = rect.y + self.radius / 2.0;
        while y < rect.y + rect.h - self.radius / 2.0 {
            let mut x = rect.x + self.radius / aspect / 2.0;
            while x < rect.x + rect.w - self.radius / aspect / 2.0 {
                let s = ring_score(frame, x, y, self.radius, aspect);
                if s > 0.0 {
                    hits.push((s, FeedIcon { x, y }));
                }
                x += step_x;
            }
            y += step_y;
        }
        // strongest first, one per icon
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut out: Vec<FeedIcon> = Vec::new();
        for (_, h) in hits {
            if out.len() >= MAX_ICONS {
                break;
            }
            if !out.iter().any(|o| ((o.x - h.x) * aspect).abs() < self.radius * 1.5 && (o.y - h.y).abs() < self.radius * 1.5) {
                out.push(h);
            }
        }
        out.sort_by(|a, b| a.y.total_cmp(&b.y));
        out
    }
}

/// How much `(cx, cy)` looks like the centre of a white-rimmed disc of radius `r` (frame heights):
/// 0 when it does not; otherwise the share of rim directions less the bright share outside. A
/// direction is on the rim when it is bright somewhere between 0.6 and 1.0 of the radius, so a
/// centre a little off still counts (the symbol may cut the rim in a direction or two).
pub(crate) fn ring_score(frame: &Luma, cx: f32, cy: f32, r: f32, aspect: f32) -> f32 {
    const MISS: usize = N / 5;
    let mut missed = 0usize;
    for i in 0..N {
        let a = i as f32 * std::f32::consts::TAU / N as f32;
        let (c, s) = (a.cos() / aspect, a.sin());
        if ![0.8f32, 0.7, 0.9, 0.6, 1.0].iter().any(|k| frame.sample(cx + k * r * c, cy + k * r * s) >= BRIGHT) {
            missed += 1;
            if missed > MISS {
                return 0.0;
            }
        }
    }
    // the rim is round: in nearly every direction the white ends at the same distance (a
    // portrait's hair or a reticle passing through ends all over the place)
    let ends: Vec<f32> = (0..N)
        .filter_map(|i| {
            let a = i as f32 * std::f32::consts::TAU / N as f32;
            let (c, s) = (a.cos() / aspect, a.sin());
            (0..16).rev().map(|j| 0.5 + 0.05 * j as f32).find(|k| frame.sample(cx + k * r * c, cy + k * r * s) >= BRIGHT)
        })
        .collect();
    let mut sorted = ends.clone();
    sorted.sort_by(f32::total_cmp);
    let Some(&mid) = sorted.get(sorted.len() / 2) else { return 0.0 };
    let round = ends.iter().filter(|e| (**e - mid).abs() <= 0.1 + 1e-4).count();
    if !(0.8..=1.2).contains(&mid) || round * 6 < N * 5 {
        return 0.0;
    }
    // a white patch (a portrait's hair, a flash) is bright outside the rim too
    let out = (0..N)
        .filter(|&i| {
            let a = i as f32 * std::f32::consts::TAU / N as f32;
            frame.sample(cx + 1.45 * r * a.cos() / aspect, cy + 1.45 * r * a.sin()) >= BRIGHT
        })
        .count();
    if out * 10 > N * 4 {
        return 0.0;
    }
    round as f32 / N as f32 - out as f32 / N as f32
}

/// Follows the feed's icons; an icon on `confirm` samples in a row that was not there before is an
/// ultimate kill at the time of its first sample.
pub struct FeedTracker {
    feed: KillFeed,
    confirm: u8,
    aspect: f32,
    /// Icons counted, as last seen.
    known: Vec<FeedIcon>,
    /// Icons not yet counted: (icon, samples seen in a row, first time).
    pending: Vec<(FeedIcon, u8, i64)>,
    /// Samples since each known icon was last seen.
    missing: Vec<u8>,
    kills: Vec<i64>,
}

impl FeedTracker {
    pub fn new(feed: KillFeed, confirm: u8) -> Self {
        Self { feed, confirm: confirm.max(1), aspect: 16.0 / 9.0, known: Vec::new(), pending: Vec::new(), missing: Vec::new(), kills: Vec::new() }
    }

    /// The same row moved up or down by up to a row or two, never sideways.
    fn same(&self, a: FeedIcon, b: FeedIcon) -> bool {
        ((a.x - b.x) * self.aspect).abs() < self.feed.radius * 0.6 && (a.y - b.y).abs() < self.feed.radius * 6.0
    }

    /// Feed the sample at `at`. `live`: the game HUD is on screen.
    pub fn push(&mut self, at: i64, frame: &Luma, live: bool) {
        if !live {
            return;
        }
        self.aspect = frame.width as f32 / frame.height.max(1) as f32;
        let icons = self.feed.icons(frame);
        let mut seen_known = vec![false; self.known.len()];
        let mut next_pending: Vec<(FeedIcon, u8, i64)> = Vec::new();
        for ic in icons {
            if let Some(i) =
                (0..self.known.len()).find(|&i| !seen_known.get(i).copied().unwrap_or(true) && self.known.get(i).is_some_and(|k| self.same(*k, ic)))
            {
                if let (Some(s), Some(k)) = (seen_known.get_mut(i), self.known.get_mut(i)) {
                    *s = true;
                    *k = ic;
                }
                continue;
            }
            let (n, first) = self.pending.iter().find(|p| self.same(p.0, ic)).map_or((1, at), |p| (p.1.saturating_add(1), p.2));
            if n >= self.confirm {
                self.kills.push(first);
                self.known.push(ic);
                seen_known.push(true);
                self.missing.push(0);
            } else {
                next_pending.push((ic, n, first));
            }
        }
        self.pending = next_pending;
        // a known icon gone for a second (two samples) has left the feed
        for (m, seen) in self.missing.iter_mut().zip(&seen_known) {
            *m = if *seen { 0 } else { m.saturating_add(1) };
        }
        let keep: Vec<bool> = self.missing.iter().map(|m| *m < 3).collect();
        let mut it = keep.iter();
        self.known.retain(|_| it.next().copied().unwrap_or(false));
        self.missing.retain(|m| *m < 3);
    }

    pub fn into_kills(self) -> Vec<i64> {
        let mut k = self.kills;
        k.sort_unstable();
        k
    }
}

#[cfg(test)]
#[path = "killfeed_tests.rs"]
mod tests;

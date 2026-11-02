//! Ultimates read off the broadcast HUD's player bars.
//!
//! Each name plate has a small box beside the player's picture that shows the ultimate charge as
//! a number ("43%") and, once the ultimate is ready, a round white icon. The icon is the same on
//! every plate of a team, so it is learnt once per bar (an [`UltIcon`]: a gray grid around one
//! ready icon, placed on the bar's first plate) and looked for on every plate.
//!
//! An ultimate is **used** when a plate whose icon read ready stops reading ready while the player
//! is alive (a player who dies keeps the charge: the plate turns dark instead, and the icon comes
//! back on respawn). The broadcast shows no kill feed, so a kill cannot be tied to its killer; a
//! death of the other team within a few seconds of a use is counted as that ultimate's kill
//! ([`ult_kills`]), the closest reading the HUD allows.

use serde::{Deserialize, Serialize};

use crate::VisionError;
use crate::kills::{Death, PlayerBar};
use crate::luma::{Luma, NRect};
use crate::template::Part;

/// Lowest correlation with the learnt icon that reads ready.
const MIN_SCORE: f32 = 0.75;
/// Share of the learnt icon's bright disc a plate must show to read ready (an icon that is
/// fading out, or the different icon some heroes show while their ultimate runs, shows less).
const MIN_FAT: f32 = 0.75;
/// Gray level of the icon's white disc (after a small blur, so thin digits of the charge number
/// do not count).
const BRIGHT: f32 = 180.0;
/// How far the icon may have moved, as a share of the frame width.
const SEARCH: f32 = 0.002;
/// Share of the learnt box's median gray a plate keeps while it is on screen.
const MIN_LEVEL: f32 = 0.6;
/// Nobody charges an ultimate again this fast (ms): a second use sooner is the first one's icon
/// flickering.
const MIN_RECHARGE: i64 = 10_000;

/// The ready-ultimate icon of one team's plates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UltIcon {
    /// The icon's grid, its rectangle placed on the bar's first plate.
    pub part: Part,
    /// Share of the grid that is bright disc on the frame it was learnt from.
    pub fat: f32,
    /// Median gray of the icon's box on that frame: the plate is there while its box stays near
    /// it (a dead plate turns black and the bars leave the screen at the end of a round, both much
    /// darker). 0: not checked.
    #[serde(default)]
    pub level: f32,
}

impl UltIcon {
    /// Learn the icon inside `rect` on `frame`, where it is on `bar`'s plate `player` (1-based)
    /// and that player's ultimate is ready.
    pub fn learn(frame: &Luma, bar: &PlayerBar, rect: NRect, player: u8) -> Result<UltIcon, VisionError> {
        let rect = rect.clamped().ok_or(VisionError::EmptyRect)?;
        let mut part = Part::learn(frame, rect, &[])?;
        part.search = SEARCH;
        let fat = fat_share(&grid(frame, rect, part.w, part.h), part.w, part.h);
        if fat < 0.03 {
            return Err(VisionError::Templates(
                "no bright round icon in the rectangle: draw it around one ready ultimate icon (the white circle), not the charge number".into(),
            ));
        }
        let level = median(&grid(frame, rect, part.w, part.h));
        let k = player.clamp(1, bar.players.max(1)) - 1;
        part.rect.x -= bar.pitch() * k as f32;
        Ok(UltIcon { part, fat, level })
    }

    /// Whether plate `k` (0-based) of `bar` shows the ready icon on `frame`.
    pub fn ready(&self, frame: &Luma, bar: &PlayerBar, k: u8) -> bool {
        let want = if self.fat.is_finite() { self.fat.clamp(0.0, 1.0) } else { 1.0 };
        let (score, fat) = self.read(frame, bar, k);
        fat >= want * MIN_FAT && score >= MIN_SCORE
    }

    /// Whether plate `k`'s icon box still looks like a plate (not black, not gone).
    pub fn present(&self, frame: &Luma, bar: &PlayerBar, k: u8) -> bool {
        if !self.level.is_finite() || self.level <= 0.0 {
            return true;
        }
        let r = self.part.rect;
        let rect = NRect::new(r.x + bar.pitch() * k as f32, r.y, r.w, r.h);
        median(&grid(frame, rect, self.part.w.clamp(1, 128), self.part.h.clamp(1, 128))) >= self.level.min(255.0) * MIN_LEVEL
    }

    /// Plate `k`'s correlation with the icon and its bright-disc share (for tuning).
    pub fn read(&self, frame: &Luma, bar: &PlayerBar, k: u8) -> (f32, f32) {
        let r = self.part.rect;
        let rect = NRect::new(r.x + bar.pitch() * k as f32, r.y, r.w, r.h);
        let (w, h) = (self.part.w.clamp(1, 128), self.part.h.clamp(1, 128));
        let fat = fat_share(&grid(frame, rect, w, h), w, h);
        // the correlation is the dearer half: only worth it when the disc is there
        let want = if self.fat.is_finite() { self.fat.clamp(0.0, 1.0) } else { 1.0 };
        if fat < want * MIN_FAT * 0.5 {
            return (0.0, fat);
        }
        (self.part.score_at(frame, rect), fat)
    }
}

impl PlayerBar {
    /// Width of one plate (frame-relative).
    pub fn pitch(&self) -> f32 {
        self.rect.w / self.players.max(1) as f32
    }

    /// Whether each player's ultimate reads ready on `frame`; `None` without a learnt icon.
    pub fn ult_ready(&self, frame: &Luma) -> Option<Vec<bool>> {
        let icon = self.ult.as_ref()?;
        Some((0..self.players).map(|k| icon.ready(frame, self, k)).collect())
    }
}

fn median(g: &[f32]) -> f32 {
    let mut v: Vec<f32> = g.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// `w`×`h` gray samples across `rect`.
fn grid(frame: &Luma, rect: NRect, w: usize, h: usize) -> Vec<f32> {
    let mut g = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            g.push(frame.sample(rect.x + rect.w * (x as f32 + 0.5) / w as f32, rect.y + rect.h * (y as f32 + 0.5) / h as f32));
        }
    }
    g
}

/// Share of cells of a `w`×`h` grid whose small neighbourhood is bright on average: a filled disc
/// counts, thin strokes (the charge digits) do not.
fn fat_share(g: &[f32], w: usize, h: usize) -> f32 {
    if w == 0 || h == 0 || g.len() < w * h {
        return 0.0;
    }
    let r = (w.min(h) / 12).max(1);
    let mut n = 0usize;
    for y in r..h.saturating_sub(r) {
        for x in r..w.saturating_sub(r) {
            let mut s = 0f32;
            for yy in y - r..=y + r {
                for xx in x - r..=x + r {
                    s += g.get(yy * w + xx).copied().unwrap_or(0.0);
                }
            }
            if s / ((2 * r + 1) * (2 * r + 1)) as f32 >= BRIGHT {
                n += 1;
            }
        }
    }
    n as f32 / (w * h) as f32
}

/// A player used their ultimate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UltUse {
    /// Time of the first sample whose icon no longer read ready.
    pub at: i64,
    pub team: u8,
    pub player: u8,
}

/// Follows the ultimate icons sample by sample, like [`crate::DeathTracker`] follows the plates:
/// an icon must read ready (or not) on `confirm` samples in a row to change state, samples where
/// the HUD is gone are skipped, and a plate that stops reading ready because it turned dark (the
/// player died) is no use.
pub struct UltTracker {
    bars: Vec<PlayerBar>,
    confirm: u8,
    /// Per slot: (reads ready, samples the other way in a row, time of the first of them, the
    /// plate was alive and on screen on it).
    state: Vec<(bool, u8, i64, bool)>,
    /// Per slot: time of the last use.
    last: Vec<Option<i64>>,
    uses: Vec<UltUse>,
}

impl UltTracker {
    /// Only bars with a learnt icon are followed.
    pub fn new(bars: Vec<PlayerBar>, confirm: u8) -> Self {
        let bars: Vec<PlayerBar> = bars.into_iter().filter(|b| b.ult.is_some()).collect();
        let n = bars.iter().map(|b| b.players as usize).sum();
        Self { bars, confirm: confirm.max(1), state: vec![(false, 0, 0, false); n], last: vec![None; n], uses: Vec::new() }
    }

    /// Whether any bar has an icon to follow.
    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }

    /// Feed the sample at `at`. `live`: the game HUD is on screen.
    pub fn push(&mut self, at: i64, frame: &Luma, live: bool) {
        if !live || self.state.is_empty() {
            return;
        }
        let mut read: Vec<(u8, u8, bool, bool)> = Vec::with_capacity(self.state.len());
        for b in &self.bars {
            let dead = b.dead(frame);
            let ready = b.ult_ready(frame).unwrap_or_default();
            for k in 0..b.players {
                let i = k as usize;
                let gone = dead.get(i).copied().unwrap_or(false) || b.ult.as_ref().is_some_and(|u| !u.present(frame, b, k));
                read.push((b.team, k + 1, ready.get(i).copied().unwrap_or(false), gone));
            }
        }
        // the HUD is gone (most plates dark at once): nothing to read
        if read.iter().filter(|r| r.3).count() * 10 > read.len() * 8 {
            return;
        }
        for ((st, last), (team, player, ready, dead)) in self.state.iter_mut().zip(self.last.iter_mut()).zip(read) {
            if ready == st.0 {
                st.1 = 0;
                continue;
            }
            if st.1 == 0 {
                st.2 = at;
                st.3 = !dead;
            }
            st.1 = st.1.saturating_add(1);
            if st.1 >= self.confirm {
                let was_ready = st.0;
                st.0 = ready;
                st.1 = 0;
                if was_ready && !ready && st.3 && !dead && last.is_none_or(|l| st.2.saturating_sub(l) >= MIN_RECHARGE) {
                    self.uses.push(UltUse { at: st.2, team, player });
                    *last = Some(st.2);
                }
            }
        }
    }

    pub fn into_uses(self) -> Vec<UltUse> {
        let mut u = self.uses;
        u.sort_by_key(|u| u.at);
        u
    }
}

/// Uses whose player is found dead no more than `ms` later are left out: the icon went because
/// the plate was turning dark, not because the ultimate was used.
pub fn without_deaths(uses: &[UltUse], deaths: &[Death], ms: i64) -> Vec<UltUse> {
    uses.iter()
        .filter(|u| !deaths.iter().any(|d| d.team == u.team && d.player == u.player && d.at >= u.at.saturating_sub(500) && d.at <= u.at.saturating_add(ms)))
        .copied()
        .collect()
}

/// A death counted as an ultimate's kill.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UltKill {
    pub death: Death,
    pub by: UltUse,
}

/// Deaths no more than `window` after an ultimate of the other team: each goes to the latest such
/// ultimate.
pub fn ult_kills(uses: &[UltUse], deaths: &[Death], window: i64) -> Vec<UltKill> {
    deaths
        .iter()
        .filter_map(|d| {
            uses.iter()
                .filter(|u| u.team != d.team && u.at <= d.at && d.at.saturating_sub(u.at) <= window)
                .max_by_key(|u| u.at)
                .map(|u| UltKill { death: *d, by: *u })
        })
        .collect()
}

/// An ultimate kill read off the kill feed ([`crate::killfeed`]), with the death it matches on
/// the plates (the victim) and the latest ultimate of the other team before it (the killer), when
/// there are such.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedKill {
    pub at: i64,
    pub death: Option<Death>,
    pub by: Option<UltUse>,
}

/// The kill feed's ultimate kills (times) matched with the plates: each takes the closest unused
/// death from 1 s before to 2.5 s after it (the plate darkens a moment after the feed row shows),
/// and the latest ultimate of the victim's other team no more than `window` before.
pub fn feed_kills(feed: &[i64], deaths: &[Death], uses: &[UltUse], window: i64) -> Vec<FeedKill> {
    let mut used = vec![false; deaths.len()];
    feed.iter()
        .map(|&at| {
            let best = deaths
                .iter()
                .enumerate()
                .filter(|(i, d)| !used.get(*i).copied().unwrap_or(true) && d.at >= at.saturating_sub(1_000) && d.at <= at.saturating_add(2_500))
                .min_by_key(|(_, d)| d.at.saturating_sub(at).saturating_abs())
                .map(|(i, d)| (i, *d));
            let death = best.map(|(i, d)| {
                if let Some(u) = used.get_mut(i) {
                    *u = true;
                }
                d
            });
            let by =
                uses.iter().filter(|u| death.is_none_or(|d| u.team != d.team) && u.at <= at && at.saturating_sub(u.at) <= window).max_by_key(|u| u.at).copied();
            FeedKill { at, death, by }
        })
        .collect()
}

#[cfg(test)]
#[path = "ults_tests.rs"]
mod tests;

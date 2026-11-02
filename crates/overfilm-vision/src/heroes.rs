//! Which hero each player is on, read off the hero portrait on the HUD's name plates, and when
//! they swap.
//!
//! Each plate of a [`PlayerBar`] shows the player's hero as a small portrait (right of the
//! ultimate charge box on OWCS broadcasts). Like the kill feed, nothing is recognised by built-in
//! artwork: each portrait becomes a small colour grid ([`HeroPrint`]), the same portrait seen
//! again is the same **hero** (numbered "영웅 1", "영웅 2", … in the order they appear), and the
//! user names a hero once ([`HeroNames`], kept in the template file) for every later scan.
//!
//! [`HeroTracker`] follows every plate sample by sample. A plate must show a new portrait on
//! `confirm` samples in a row to count, and dead plates (dark, crossed out) are skipped, so a
//! death, a flash or the plate's ultimate animation is no swap. A stretch of play after a break
//! (round setup, a cut, a replay) starts fresh: the first hero seen on each plate is the round's
//! pick ([`HeroEvent::picked`]), not a swap.

use serde::{Deserialize, Serialize};

use crate::kills::PlayerBar;
use crate::luma::{Luma, NRect};

/// Portrait grid (cells across and down; three colours each).
pub const GRID: usize = 6;
const LEN: usize = GRID * GRID * 3;
/// Mean colour difference (0–255) under which two portraits are the same hero.
pub const SAME: f32 = 22.0;
/// A portrait whose cells differ less than this from their mean is no portrait (the HUD is
/// hidden, a flat plate).
const FLAT: f32 = 6.0;
/// Brightness under which differences stop growing (a black plate is no portrait).
const DARK_LEVEL: f32 = 32.0;
/// Most heroes told apart in one scan (noise included).
const MAX_HEROES: usize = 512;
/// Most named heroes kept in a template file.
const MAX_NAMES: usize = 512;
/// Samples off the game HUD in a row after which the next play starts a new stretch.
const BREAK_SAMPLES: u32 = 4;
/// A hero seen on fewer samples than this in a whole scan is noise (a transition frame).
const MIN_SAMPLES: u32 = 6;

/// The portrait of plate `k`: the first plate's portrait moved along the bar, or, without one,
/// the right part of the plate above the name strip (the OWCS layout).
pub fn portrait(bar: &PlayerBar, k: u8) -> NRect {
    let n = bar.players.max(1) as f32;
    let pw = bar.rect.w / n;
    match bar.portrait {
        Some(p) => NRect::new(p.x + pw * k as f32, p.y, p.w, p.h),
        None => NRect::new(bar.rect.x + pw * (k as f32 + 0.54), bar.rect.y - bar.rect.h * 2.2, pw * 0.34, bar.rect.h * 1.9),
    }
}

/// A portrait as a small colour grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeroPrint(pub Vec<u8>);

impl HeroPrint {
    /// Read the portrait inside `r` of an RGBA frame. `None` outside the frame, for a buffer too
    /// small for the size given, or for a flat area.
    pub fn read(rgba: &[u8], width: usize, height: usize, r: NRect) -> Option<HeroPrint> {
        let need = width.checked_mul(height)?.checked_mul(4)?;
        if width == 0 || height == 0 || rgba.len() < need {
            return None;
        }
        let r = r.clamped()?;
        let x0 = (r.x * width as f32) as usize;
        let y0 = (r.y * height as f32) as usize;
        let x1 = (((r.x + r.w) * width as f32) as usize).clamp(x0 + 1, width);
        let y1 = (((r.y + r.h) * height as f32) as usize).clamp(y0 + 1, height);
        let mut out = Vec::with_capacity(LEN);
        for gy in 0..GRID {
            let (ya, yb) = (y0 + (y1 - y0) * gy / GRID, (y0 + (y1 - y0) * (gy + 1) / GRID).max(y0 + (y1 - y0) * gy / GRID + 1).min(height));
            for gx in 0..GRID {
                let (xa, xb) = (x0 + (x1 - x0) * gx / GRID, (x0 + (x1 - x0) * (gx + 1) / GRID).max(x0 + (x1 - x0) * gx / GRID + 1).min(width));
                let mut acc = [0u32; 3];
                let mut n = 0u32;
                for y in ya..yb {
                    for x in xa..xb {
                        let i = (y * width + x) * 4;
                        for (c, a) in acc.iter_mut().enumerate() {
                            *a += u32::from(rgba.get(i + c).copied().unwrap_or(0));
                        }
                        n += 1;
                    }
                }
                out.extend(acc.map(|a| (a / n.max(1)).min(255) as u8));
            }
        }
        let p = HeroPrint(out);
        (p.spread() >= FLAT).then_some(p)
    }

    /// Mean difference of the cells' gray from their average.
    fn spread(&self) -> f32 {
        let g: Vec<f32> = self.0.chunks_exact(3).map(|c| (u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2])) as f32 / 3.0).collect();
        let m = g.iter().sum::<f32>() / g.len().max(1) as f32;
        g.iter().map(|v| (v - m).abs()).sum::<f32>() / g.len().max(1) as f32
    }

    /// Mean colour difference, scaled to the prints' brightness (0–255 for mid-gray portraits):
    /// dark portraits differ in few levels, so the same levels count for more between them.
    /// Prints of other sizes differ completely.
    pub fn diff(&self, o: &HeroPrint) -> f32 {
        if self.0.len() != o.0.len() || self.0.is_empty() {
            return 255.0;
        }
        let n = self.0.len() as f32;
        let mean = |p: &HeroPrint| p.0.iter().map(|v| f32::from(*v)).sum::<f32>() / n;
        let level = ((mean(self) + mean(o)) / 2.0).max(DARK_LEVEL);
        let d = self.0.iter().zip(&o.0).map(|(a, b)| a.abs_diff(*b) as f32).sum::<f32>() / n;
        (d * 128.0 / level).min(255.0)
    }
}

/// A hero portrait the user named.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedHero {
    pub name: String,
    pub print: HeroPrint,
}

/// Names the user gave to hero portraits (kept in the template file).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HeroNames {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heroes: Vec<NamedHero>,
}

impl HeroNames {
    pub fn is_empty(&self) -> bool {
        self.heroes.is_empty()
    }

    /// The name of the portrait closest to `p`, when it is the same hero.
    pub fn lookup(&self, p: &HeroPrint) -> Option<&str> {
        self.heroes.iter().map(|h| (h.print.diff(p), h)).filter(|(d, _)| *d < SAME).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, h)| h.name.as_str())
    }

    /// Name `p` (replacing the name of the same portrait, if any). An empty name forgets it.
    pub fn set(&mut self, p: HeroPrint, name: &str) {
        let name = name.trim();
        self.heroes.retain(|h| h.print.diff(&p) >= SAME);
        if !name.is_empty() && self.heroes.len() < MAX_NAMES {
            self.heroes.push(NamedHero { name: name.chars().take(64).collect(), print: p });
        }
    }
}

/// A hero told apart in a scan: its running average portrait and how often it was seen.
#[derive(Clone, Debug)]
struct Cluster {
    sum: Vec<f32>,
    n: u32,
}

impl Cluster {
    fn print(&self) -> HeroPrint {
        HeroPrint(self.sum.iter().map(|s| (s / self.n.max(1) as f32).round().clamp(0.0, 255.0) as u8).collect())
    }
}

/// A plate's hero changed (or was first seen in a stretch of play).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeroEvent {
    /// The first sample showing the new hero (the caller's time unit; ms in practice).
    pub at: i64,
    /// Team (1 or 2) and place on the bar (1-based).
    pub team: u8,
    pub player: u8,
    /// The hero before (`None` when picked at the start of a stretch of play).
    pub from: Option<usize>,
    pub to: usize,
}

impl HeroEvent {
    /// The round's pick rather than a swap mid-play.
    pub fn picked(&self) -> bool {
        self.from.is_none()
    }
}

#[derive(Clone, Debug, Default)]
struct Slot {
    team: u8,
    player: u8,
    cur: Option<usize>,
    cand: Option<usize>,
    count: u8,
    since: i64,
    /// The next confirmed hero is a pick (a new stretch of play).
    fresh: bool,
}

/// What a scan found: events and the heroes they refer to (numbered from 0 in the order they
/// first appear; "영웅 1" is hero 0).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HeroScan {
    pub events: Vec<HeroEvent>,
    /// Each hero's average portrait.
    pub prints: Vec<HeroPrint>,
    /// The user's name for each hero, when they named it.
    pub names: Vec<Option<String>>,
}

impl HeroScan {
    /// "트레이서", or "영웅 3" for a hero nobody named.
    pub fn label(&self, hero: usize) -> String {
        match self.names.get(hero).cloned().flatten() {
            Some(n) => n,
            None => format!("영웅 {}", hero + 1),
        }
    }
}

/// Follows every plate's portrait (see the module docs).
pub struct HeroTracker {
    bars: Vec<PlayerBar>,
    /// How long (caller's time unit) a new portrait must stay to count.
    hold: i64,
    /// Mean colour difference under which two portraits are the same hero.
    pub same: f32,
    clusters: Vec<Cluster>,
    slots: Vec<Slot>,
    events: Vec<HeroEvent>,
    off: u32,
}

impl HeroTracker {
    /// Follow `bars`; a new portrait counts once it has stayed for `hold` (the caller's time unit;
    /// a dead player's grayed portrait lasts the respawn time, about 10 s, so 12 s in ms).
    pub fn new(bars: Vec<PlayerBar>, hold: i64) -> Self {
        let slots = bars.iter().flat_map(|b| (0..b.players).map(move |k| Slot { team: b.team, player: k + 1, fresh: true, ..Slot::default() })).collect();
        Self { bars, hold: hold.max(0), same: SAME, clusters: Vec::new(), slots, events: Vec::new(), off: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The hero `p` is: the closest one within [`SAME`], else a new one (`None` once
    /// [`MAX_HEROES`] are known).
    fn assign(&mut self, p: &HeroPrint) -> Option<usize> {
        let same = self.same;
        let best =
            self.clusters.iter().enumerate().map(|(i, c)| (i, c.print().diff(p))).filter(|(_, d)| *d < same).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i);
        match best {
            Some(i) => {
                let c = self.clusters.get_mut(i)?;
                // a running average that keeps following slow changes (compression, lighting)
                if c.n >= 64 {
                    for s in &mut c.sum {
                        *s *= 63.0 / 64.0;
                    }
                    c.n = 63;
                }
                for (s, v) in c.sum.iter_mut().zip(&p.0) {
                    *s += f32::from(*v);
                }
                c.n += 1;
                Some(i)
            }
            None if self.clusters.len() < MAX_HEROES => {
                self.clusters.push(Cluster { sum: p.0.iter().map(|v| f32::from(*v)).collect(), n: 1 });
                Some(self.clusters.len() - 1)
            }
            None => None,
        }
    }

    /// Feed the sample at `at`: the RGBA frame (`width` × `height`), the same frame in gray (for
    /// dead plates) and whether the game HUD is on screen.
    pub fn push(&mut self, at: i64, rgba: &[u8], width: usize, height: usize, gray: &Luma, live: bool) {
        if !live {
            self.off = self.off.saturating_add(1);
            if self.off >= BREAK_SAMPLES {
                for s in &mut self.slots {
                    s.fresh = true;
                    s.cand = None;
                    s.count = 0;
                }
            }
            return;
        }
        self.off = 0;
        let mut reads = Vec::with_capacity(self.slots.len());
        for b in &self.bars {
            let dead = b.dead(gray);
            for k in 0..b.players {
                let d = dead.get(k as usize).copied().unwrap_or(true);
                reads.push(if d { None } else { HeroPrint::read(rgba, width, height, portrait(b, k)) });
            }
        }
        // the HUD hidden (most plates without a portrait): nothing to read
        if reads.iter().filter(|r| r.is_none()).count() * 10 > reads.len() * 8 {
            return;
        }
        for (i, r) in reads.into_iter().enumerate() {
            let Some(p) = r else { continue };
            let Some(h) = self.assign(&p) else { continue };
            let hold = self.hold;
            let Some(s) = self.slots.get_mut(i) else { continue };
            if s.cur == Some(h) && !s.fresh {
                s.cand = None;
                s.count = 0;
                continue;
            }
            if s.cand == Some(h) {
                s.count = s.count.saturating_add(1);
            } else {
                s.cand = Some(h);
                s.count = 1;
                s.since = at;
            }
            if s.count >= 2 && at.saturating_sub(s.since) >= hold {
                let from = if s.fresh { None } else { s.cur };
                self.events.push(HeroEvent { at: s.since, team: s.team, player: s.player, from, to: h });
                s.cur = Some(h);
                s.fresh = false;
                s.cand = None;
                s.count = 0;
            }
        }
    }

    /// The events, with heroes seen too rarely dropped, look-alike heroes merged and every hero
    /// numbered in order of appearance and named from `names`.
    pub fn finish(self, names: &HeroNames) -> HeroScan {
        let prints: Vec<HeroPrint> = self.clusters.iter().map(Cluster::print).collect();
        let counts: Vec<u32> = self.clusters.iter().map(|c| c.n).collect();
        // merge clusters that drifted into one another (the larger one stays)
        let mut root: Vec<usize> = (0..prints.len()).collect();
        let mut order: Vec<usize> = (0..prints.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(counts.get(i).copied().unwrap_or(0)));
        for (a_pos, &a) in order.iter().enumerate() {
            if root.get(a).copied() != Some(a) {
                continue;
            }
            for &b in order.iter().skip(a_pos + 1) {
                if root.get(b).copied() == Some(b)
                    && prints.get(a).zip(prints.get(b)).is_some_and(|(pa, pb)| pa.diff(pb) < self.same)
                    && let Some(r) = root.get_mut(b)
                {
                    *r = a;
                }
            }
        }
        let find = |i: usize| root.get(i).copied().unwrap_or(i);
        let seen = |i: usize| order.iter().filter(|&&j| find(j) == i).map(|&j| counts.get(j).copied().unwrap_or(0)).sum::<u32>();
        // per plate, drop events to rare heroes and repeats after merging
        let mut events: Vec<HeroEvent> = Vec::new();
        let mut last: std::collections::HashMap<(u8, u8), usize> = Default::default();
        for mut e in self.events {
            e.to = find(e.to);
            if seen(e.to) < MIN_SAMPLES {
                continue;
            }
            let key = (e.team, e.player);
            let before = last.get(&key).copied();
            // a new stretch on the hero the plate already had is no news
            if before == Some(e.to) {
                continue;
            }
            if e.from.is_some() {
                // the hero before is the last one kept on this plate
                e.from = before;
            }
            last.insert(key, e.to);
            events.push(e);
        }
        // number heroes in order of appearance
        let mut number: Vec<usize> = Vec::new();
        for e in &events {
            for h in e.from.iter().chain(std::iter::once(&e.to)) {
                if !number.contains(h) {
                    number.push(*h);
                }
            }
        }
        let renum = |h: usize| number.iter().position(|&x| x == h).unwrap_or(0);
        for e in &mut events {
            e.to = renum(e.to);
            e.from = e.from.map(renum);
        }
        let out_prints: Vec<HeroPrint> = number.iter().filter_map(|&h| prints.get(h).cloned()).collect();
        let out_names = out_prints.iter().map(|p| names.lookup(p).map(String::from)).collect();
        HeroScan { events, prints: out_prints, names: out_names }
    }
}

#[cfg(test)]
#[path = "heroes_tests.rs"]
mod tests;

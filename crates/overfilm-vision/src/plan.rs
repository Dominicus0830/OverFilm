//! From labelled samples to segments to cuts.

use filmcraft_time::{TICKS_PER_SECOND, Tick, TimeRange};
use serde::{Deserialize, Serialize};

use crate::Screen;
use crate::luma::Luma;
use crate::template::TemplateSet;

/// Most samples one analysis keeps (10 hours at 4 per second).
const MAX_SAMPLES: usize = 144_000;

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

/// One analysed frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Time of the frame (sequence or media time; the caller decides, the plan keeps it).
    pub t: Tick,
    /// Best template over its threshold.
    pub screen: Option<Screen>,
    pub template: Option<usize>,
    pub score: f32,
    /// Share of the picture unchanged since the previous sample (1.0 = frozen).
    pub still: f32,
    /// A game-HUD template matched.
    #[serde(default)]
    pub live: bool,
}

/// Matches a stream of frames against a template set.
pub struct Analyzer {
    set: TemplateSet,
    prev: Option<Luma>,
    samples: Vec<Sample>,
    /// Gray levels a block may change by and still count as unchanged (compression noise).
    pub still_tol: f32,
}

impl Analyzer {
    pub fn new(set: TemplateSet) -> Analyzer {
        Analyzer { set, prev: None, samples: Vec::new(), still_tol: 1.0 }
    }

    pub fn templates(&self) -> &TemplateSet {
        &self.set
    }

    /// Analyse the frame at `t` (frames must come in time order).
    pub fn push(&mut self, t: Tick, frame: Luma) -> Option<&Sample> {
        if self.samples.len() >= MAX_SAMPLES || self.samples.last().is_some_and(|s| s.t >= t) {
            return None;
        }
        let best = self.set.best(&frame);
        let still = self.prev.as_ref().map(|p| frame.still_fraction(p, self.still_tol)).unwrap_or(0.0);
        let live = self.set.live(&frame);
        self.samples.push(Sample { t, screen: best.map(|b| b.0), template: best.map(|b| b.1), score: best.map(|b| b.2).unwrap_or(0.0), still, live });
        self.prev = Some(frame);
        self.samples.last()
    }

    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    pub fn into_samples(self) -> Vec<Sample> {
        self.samples
    }
}

/// How a segment was found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detected {
    /// By the templates of this screen.
    Template,
    /// A frozen picture during a round.
    Frozen,
}

/// A stretch of one screen state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub screen: Screen,
    pub range: TimeRange,
    pub how: Detected,
}

/// Why a range is cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CutReason {
    /// Break screen or round setup before the match/round starts.
    Waiting,
    /// Game pause.
    Pause,
}

/// A range to remove.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cut {
    pub range: TimeRange,
    pub reason: CutReason,
    /// Join the two sides with a dissolve (and an audio crossfade).
    pub dissolve: bool,
}

/// Rules of the cut plan. Defaults follow the user's request (2026-11-01): waiting time goes up
/// to 10 s before the match or round starts, pauses go whole, nothing else is cut.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CutParams {
    /// Waiting time kept before the round/match starts.
    pub keep_before: Tick,
    /// Shortest break screen that counts.
    pub min_standby: Tick,
    /// Shortest setup banner that counts.
    pub min_setup: Tick,
    /// Shortest pause notice that counts.
    pub min_pause: Tick,
    /// Shortest frozen picture that counts as a pause.
    pub min_frozen: Tick,
    /// Share of the picture that must be unchanged for a frozen picture.
    pub frozen_still: f32,
    /// Recognise frozen pictures as pauses at all.
    pub frozen_pauses: bool,
    /// A frozen picture is a pause only while the game HUD ([`Screen::Live`]) is matched. Set it
    /// when the template set has a HUD template; without one a frozen picture counts only inside
    /// a round (after a setup banner, before the next break screen).
    pub frozen_needs_live: bool,
    /// Gaps of unmatched samples up to this long inside one state are bridged (a replay wipe, a
    /// caster box sliding over the banner).
    pub fill_gap: Tick,
    /// Waiting segments this close together are one wait (break screen straight into setup).
    pub join_gap: Tick,
    /// Shortest cut made.
    pub min_cut: Tick,
    /// Dissolve at pause joins.
    pub pause_dissolve: bool,
    /// Dissolve at waiting joins.
    pub waiting_dissolve: bool,
}

impl Default for CutParams {
    fn default() -> Self {
        CutParams {
            keep_before: secs(10.0),
            min_standby: secs(5.0),
            min_setup: secs(3.0),
            min_pause: secs(2.0),
            min_frozen: secs(4.0),
            frozen_still: 0.97,
            frozen_pauses: true,
            frozen_needs_live: false,
            fill_gap: secs(1.5),
            join_gap: secs(3.0),
            min_cut: secs(2.0),
            pause_dissolve: true,
            waiting_dissolve: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lab {
    None,
    S(Screen),
    Frozen { live: bool },
}

fn same(a: Lab, b: Lab) -> bool {
    match (a, b) {
        (Lab::S(x), Lab::S(y)) => x == y,
        (Lab::Frozen { .. }, Lab::Frozen { .. }) => true,
        _ => false,
    }
}

/// Start and end time of each sample (midpoints between neighbours; the ends extend half a step).
fn spans(samples: &[Sample]) -> Vec<(Tick, Tick)> {
    let n = samples.len();
    let step = |i: usize| -> Tick {
        let a = samples.get(i).map(|s| s.t).unwrap_or_default();
        let b = samples.get(i + 1).map(|s| s.t).unwrap_or(a);
        Tick((b.0.saturating_sub(a.0)) / 2)
    };
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let t = samples[i].t;
        let start = if i == 0 { t - step(0).min(secs(5.0)) } else { t - step(i - 1) };
        let end = if i + 1 == n { t + step(i.saturating_sub(1)).min(secs(5.0)) } else { t + step(i) };
        v.push((start, end.max(start)));
    }
    v
}

/// Segments of each recognised state, in time order.
pub fn segments(samples: &[Sample], p: &CutParams) -> Vec<Segment> {
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }
    let sp = spans(samples);
    let thr = if p.frozen_still.is_finite() { p.frozen_still.clamp(0.5, 1.0) } else { 0.97 };
    let mut lab: Vec<Lab> = samples
        .iter()
        .map(|s| match s.screen {
            Some(Screen::Live) | None if p.frozen_pauses && s.still >= thr => Lab::Frozen { live: s.live || s.screen == Some(Screen::Live) },
            Some(sc) => Lab::S(sc),
            None => Lab::None,
        })
        .collect();
    // bridge short unmatched gaps between two samples of the same state
    let mut i = 0;
    while i < n {
        if lab[i] == Lab::None || matches!(lab[i], Lab::S(Screen::Live)) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < n && matches!(lab[j], Lab::None | Lab::S(Screen::Live)) {
            j += 1;
        }
        if j < n && j > i + 1 && same(lab[i], lab[j]) && sp[j].0 - sp[i].1 <= p.fill_gap {
            let fill = lab[i];
            for l in lab.iter_mut().take(j).skip(i + 1) {
                *l = fill;
            }
        }
        i = j;
    }
    // runs
    let mut runs: Vec<(Lab, usize, usize)> = Vec::new();
    for (k, l) in lab.iter().enumerate() {
        match runs.last_mut() {
            Some((rl, _, e)) if same(*rl, *l) && *e + 1 == k => {
                *e = k;
                if let (Lab::Frozen { live: a }, Lab::Frozen { live: b }) = (rl, l) {
                    *a |= *b;
                }
            }
            _ => runs.push((*l, k, k)),
        }
    }
    let mut out: Vec<Segment> = Vec::new();
    // in a round: a setup ended and no break screen has started since
    let mut in_round = false;
    for (l, a, b) in runs {
        let range = TimeRange::from_bounds(sp[a].0, sp[b].1);
        match l {
            Lab::S(Screen::Standby) if range.duration >= p.min_standby => {
                in_round = false;
                out.push(Segment { screen: Screen::Standby, range, how: Detected::Template });
            }
            Lab::S(Screen::Setup) if range.duration >= p.min_setup => {
                in_round = true;
                out.push(Segment { screen: Screen::Setup, range, how: Detected::Template });
            }
            Lab::S(Screen::Pause) if range.duration >= p.min_pause => {
                out.push(Segment { screen: Screen::Pause, range, how: Detected::Template });
            }
            Lab::Frozen { live } if (live || (in_round && !p.frozen_needs_live)) && range.duration >= p.min_frozen => {
                out.push(Segment { screen: Screen::Pause, range, how: Detected::Frozen });
            }
            _ => {}
        }
    }
    // a pause notice and the frozen picture around it are one pause
    let mut merged: Vec<Segment> = Vec::new();
    for s in out {
        match merged.last_mut() {
            Some(m) if m.screen == Screen::Pause && s.screen == Screen::Pause && s.range.start <= m.range.end() + p.fill_gap => {
                let end = m.range.end().max(s.range.end());
                m.range = TimeRange::from_bounds(m.range.start, end);
                if s.how == Detected::Template {
                    m.how = Detected::Template;
                }
            }
            _ => merged.push(s),
        }
    }
    // a frozen picture running straight into a break screen or setup (the scoreboard between
    // halves) is part of that wait, not a pause
    for i in 0..merged.len() {
        let next_wait = merged.get(i + 1).filter(|n| matches!(n.screen, Screen::Standby | Screen::Setup)).map(|n| n.range.start);
        if let (Some(m), Some(start)) = (merged.get_mut(i), next_wait)
            && m.screen == Screen::Pause
            && m.how == Detected::Frozen
            && start <= m.range.end() + p.join_gap
        {
            m.screen = Screen::Setup;
        }
    }
    merged
}

/// The cuts for `segments`, in time order, never overlapping.
pub fn plan_cuts(segments: &[Segment], p: &CutParams) -> Vec<Cut> {
    let mut cuts: Vec<Cut> = Vec::new();
    // waiting: group break screens and setups that follow each other
    let mut waits: Vec<TimeRange> = Vec::new();
    for s in segments.iter().filter(|s| matches!(s.screen, Screen::Standby | Screen::Setup)) {
        match waits.last_mut() {
            Some(w) if s.range.start <= w.end() + p.join_gap => *w = TimeRange::from_bounds(w.start, w.end().max(s.range.end())),
            _ => waits.push(s.range),
        }
    }
    for w in &waits {
        let end = w.end() - p.keep_before.max(Tick::ZERO);
        if end - w.start >= p.min_cut.max(Tick(1)) {
            cuts.push(Cut { range: TimeRange::from_bounds(w.start, end), reason: CutReason::Waiting, dissolve: p.waiting_dissolve });
        }
    }
    for s in segments.iter().filter(|s| s.screen == Screen::Pause) {
        // a pause inside the waiting time is cut with it; one overlapping the kept lead-in is not
        // cut at all (the lead-in is kept whole)
        if waits.iter().any(|w| w.overlaps(&s.range)) {
            continue;
        }
        if s.range.duration >= p.min_cut.max(Tick(1)) {
            cuts.push(Cut { range: s.range, reason: CutReason::Pause, dissolve: p.pause_dissolve });
        }
    }
    cuts.sort_by_key(|c| c.range.start);
    let mut out: Vec<Cut> = Vec::new();
    for c in cuts {
        match out.last_mut() {
            Some(l) if c.range.start <= l.range.end() => {
                l.range = TimeRange::from_bounds(l.range.start, l.range.end().max(c.range.end()));
                l.dissolve |= c.dissolve;
            }
            _ => out.push(c),
        }
    }
    out
}

/// Total time the cuts remove.
pub fn removed(cuts: &[Cut]) -> Tick {
    cuts.iter().fold(Tick::ZERO, |a, c| a + c.range.duration)
}

/// Seconds as a readable `m:ss` (for reports).
pub fn clock(t: Tick) -> String {
    let s = (t.0.max(0) / TICKS_PER_SECOND).max(0);
    format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;

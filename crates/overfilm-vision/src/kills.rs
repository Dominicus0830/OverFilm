//! Deaths read off the broadcast HUD's player bars.
//!
//! Each team's bar has one name plate per player, in the team colour while the player is alive
//! and dark (black) while they wait to respawn. A **player bar** is learnt from a frame of the
//! same broadcast (the rectangle around one team's plates and how many players it holds); each
//! plate is a slot. A slot that turns dark and stays dark is a death; the times become kills and
//! team fights. Like the screen templates, the bars carry no artwork of anyone else's: only where
//! the plates are.

use serde::{Deserialize, Serialize};

use crate::VisionError;
use crate::luma::{Luma, NRect};

/// A pixel this dark (gray level) counts as a dead plate's black.
const DARK: f32 = 45.0;
/// Share of dark pixels over which a plate reads dead (white name text stays bright).
const DEAD_SHARE: f32 = 0.35;
/// Grid samples across and down a plate.
const GX: usize = 24;
const GY: usize = 6;
/// Most players on one bar.
pub const MAX_PLAYERS: u8 = 12;

/// One team's row of name plates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerBar {
    /// 1 or 2 (left / right on most broadcasts).
    pub team: u8,
    /// Around all of the team's plates (frame-relative).
    pub rect: NRect,
    pub players: u8,
    /// Dark share of each plate on the frame it was learnt from (players alive), so a plate with
    /// dark graphics of its own needs more to read dead.
    #[serde(default)]
    pub alive_dark: Vec<f32>,
    /// The ready-ultimate icon on these plates, once learnt (see [`crate::ults`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ult: Option<crate::ults::UltIcon>,
}

impl PlayerBar {
    /// Learn a bar on `frame`, where every player of the team is alive.
    pub fn learn(frame: &Luma, team: u8, rect: NRect, players: u8) -> Result<PlayerBar, VisionError> {
        let rect = rect.clamped().ok_or(VisionError::EmptyRect)?;
        let players = players.clamp(1, MAX_PLAYERS);
        let mut bar = PlayerBar { team: team.clamp(1, 2), rect, players, alive_dark: Vec::new(), ult: None };
        bar.alive_dark = (0..players).map(|k| dark_share(frame, bar.slot(k))).collect();
        if bar.alive_dark.iter().all(|&d| d > DEAD_SHARE) {
            return Err(VisionError::Templates("every plate looks dark: learn the bar on a frame where the team is alive".into()));
        }
        Ok(bar)
    }

    /// Plate `k`: the k-th equal share of the bar, without the gaps at its sides.
    pub fn slot(&self, k: u8) -> NRect {
        let n = self.players.max(1) as f32;
        let w = self.rect.w / n;
        NRect::new(self.rect.x + w * (k as f32 + 0.15), self.rect.y, w * 0.7, self.rect.h)
    }

    /// Whether each player reads dead on `frame`.
    pub fn dead(&self, frame: &Luma) -> Vec<bool> {
        (0..self.players)
            .map(|k| {
                let base = self.alive_dark.get(k as usize).copied().unwrap_or(0.0);
                dark_share(frame, self.slot(k)) > DEAD_SHARE.max(base + 0.25)
            })
            .collect()
    }
}

/// Share of dark pixels in `r` on `frame`.
pub fn dark_share(frame: &Luma, r: NRect) -> f32 {
    let mut dark = 0usize;
    for j in 0..GY {
        for i in 0..GX {
            let u = r.x + r.w * (i as f32 + 0.5) / GX as f32;
            let v = r.y + r.h * (j as f32 + 0.5) / GY as f32;
            if frame.sample(u, v) < DARK {
                dark += 1;
            }
        }
    }
    dark as f32 / (GX * GY) as f32
}

/// A player went down.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Death {
    /// Time of the first sample that read dead (whatever unit the caller uses; ms in practice).
    pub at: i64,
    /// The dead player's team (1 or 2) and place on the bar (1-based).
    pub team: u8,
    pub player: u8,
}

/// Follows the bars sample by sample. A plate must read dead (or alive) on `confirm` samples in a
/// row to change state, so a flicker or a camera flash is no death; samples where most plates
/// read dark at once (the HUD is gone: replays, break screens, a black frame) are skipped. A
/// player who died less than `respawn` ago (in the caller's time unit) cannot die again: a plate
/// that flickers back to the team colour mid-respawn is not a second death.
pub struct DeathTracker {
    bars: Vec<PlayerBar>,
    confirm: u8,
    respawn: i64,
    /// Per slot: time of the last death.
    last: Vec<Option<i64>>,
    /// Per slot: (is dead, samples the other way in a row, time of the first of them).
    state: Vec<(bool, u8, i64)>,
    deaths: Vec<Death>,
}

impl DeathTracker {
    pub fn new(bars: Vec<PlayerBar>, confirm: u8, respawn: i64) -> Self {
        let n = bars.iter().map(|b| b.players as usize).sum();
        Self { bars, confirm: confirm.max(1), respawn: respawn.max(0), last: vec![None; n], state: vec![(false, 0, 0); n], deaths: Vec::new() }
    }

    /// Feed the sample at `at`. `live`: the game HUD is on screen (pass true without a HUD template).
    pub fn push(&mut self, at: i64, frame: &Luma, live: bool) {
        if !live || self.state.is_empty() {
            return;
        }
        let read: Vec<(u8, u8, bool)> =
            self.bars.iter().flat_map(|b| b.dead(frame).into_iter().enumerate().map(move |(k, d)| (b.team, k as u8 + 1, d))).collect();
        let dark = read.iter().filter(|r| r.2).count();
        if dark * 10 > read.len() * 8 {
            return;
        }
        for ((st, last), (team, player, dead)) in self.state.iter_mut().zip(self.last.iter_mut()).zip(read) {
            if dead == st.0 {
                st.1 = 0;
                continue;
            }
            if st.1 == 0 {
                st.2 = at;
            }
            st.1 = st.1.saturating_add(1);
            if st.1 >= self.confirm {
                st.0 = dead;
                st.1 = 0;
                if dead && last.is_none_or(|l| st.2.saturating_sub(l) >= self.respawn) {
                    self.deaths.push(Death { at: st.2, team, player });
                    *last = Some(st.2);
                }
            }
        }
    }

    pub fn into_deaths(self) -> Vec<Death> {
        let mut d = self.deaths;
        d.sort_by_key(|d| d.at);
        d
    }
}

/// Deaths close together: a team fight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fight {
    pub start: i64,
    pub end: i64,
    /// Deaths of team 1 and team 2.
    pub dead: [usize; 2],
}

impl Fight {
    pub fn kills(&self) -> usize {
        self.dead[0] + self.dead[1]
    }
}

/// Group deaths no more than `gap` apart into fights of at least `min_kills`.
pub fn fights(deaths: &[Death], gap: i64, min_kills: usize) -> Vec<Fight> {
    let mut out: Vec<Fight> = Vec::new();
    let mut cur: Option<Fight> = None;
    for d in deaths {
        let side = usize::from(d.team == 2);
        match cur.as_mut() {
            Some(f) if d.at - f.end <= gap => {
                f.end = d.at;
                if let Some(n) = f.dead.get_mut(side) {
                    *n += 1;
                }
            }
            _ => {
                if let Some(f) = cur.take().filter(|f| f.kills() >= min_kills) {
                    out.push(f);
                }
                let mut f = Fight { start: d.at, end: d.at, dead: [0, 0] };
                if let Some(n) = f.dead.get_mut(side) {
                    *n += 1;
                }
                cur = Some(f);
            }
        }
    }
    if let Some(f) = cur.filter(|f| f.kills() >= min_kills) {
        out.push(f);
    }
    out
}

#[cfg(test)]
#[path = "kills_tests.rs"]
mod tests;

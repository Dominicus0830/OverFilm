//! Which teams and players are on screen, from text read off the HUD (team names over each
//! player bar, player names on the name plates).
//!
//! The crate does no text recognition itself: the caller reads the text (the editor uses an OCR
//! model) and hands each reading in with where it was. Readings of small, compressed broadcast
//! text are noisy (`RODE ZANSIDE GAMING`, `D2 Blat`, `WUTAN`), so they are only ever compared
//! with a **roster** (the teams and players of the glossary) and many readings vote:
//!
//! - each side's team is the roster team the team-name readings match best overall;
//! - players are matched only against that team's roster, plate by plate, and the votes are
//!   counted per **stretch** of play (a round between breaks): players can be substituted
//!   between rounds, and the plates' order changes from round to round;
//! - in a stretch, each player takes at most one plate.
//!
//! [`attribute`] joins the result with the hero portraits ([`crate::heroes`]): who picked or
//! swapped to which hero, and when.

use serde::{Deserialize, Serialize};

use crate::heroes::{HeroEvent, HeroScan};
use crate::kills::PlayerBar;
use crate::luma::NRect;

/// Lowest similarity of a reading and a team name to count as a vote.
const TEAM_MIN: f32 = 0.55;
/// Lowest similarity of a plate reading and a roster player's handle to count as a vote.
const PLAYER_MIN: f32 = 0.5;
/// Votes a plate needs in a stretch to be known.
const PLAYER_VOTES: f32 = 0.9;
/// Samples off the game HUD in a row that end a stretch (as in [`crate::heroes`]).
const BREAK_SAMPLES: u32 = 4;
/// Most readings kept (each is a few dozen bytes).
const MAX_READS: usize = 200_000;
/// Longest reading kept (characters).
const MAX_TEXT: usize = 96;

/// A team the broadcast may show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterTeam {
    pub id: String,
    /// The name captions use (`로제`).
    pub name: String,
    /// How the HUD may write it (`RØDE ZANSIDE GAMING`, `ROZE`).
    pub reads: Vec<String>,
}

/// A player the broadcast may show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterPlayer {
    pub id: String,
    /// The name captions use (`우톈`).
    pub name: String,
    /// The handle on the name plate (`WuTian`).
    pub handle: String,
    /// The team's [`RosterTeam::id`].
    pub team: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Roster {
    pub teams: Vec<RosterTeam>,
    pub players: Vec<RosterPlayer>,
}

/// Upper-case letters and digits only, with the look-alikes OCR swaps folded together
/// (`Ø`→`O`, `0`→`O`, `1`/`|`→`I`, `5`→`S`).
pub fn canon(s: &str) -> String {
    s.chars()
        .flat_map(char::to_uppercase)
        .filter_map(|c| match c {
            'Ø' | '0' | 'Ö' | 'Ó' => Some('O'),
            '1' | '|' | '!' | 'Í' => Some('I'),
            '5' => Some('S'),
            c if c.is_ascii_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// 1 − edit distance / longer length, on [`canon`] forms (0 when either is empty).
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (canon(a).chars().collect(), canon(b).chars().collect());
    let n = a.len().max(b.len());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    1.0 - edit(&a, &b) as f32 / n as f32
}

fn edit(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        if let Some(c) = cur.first_mut() {
            *c = i + 1;
        }
        for (j, cb) in b.iter().enumerate() {
            let sub = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let del = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let ins = cur.get(j).copied().unwrap_or(0) + 1;
            if let Some(c) = cur.get_mut(j + 1) {
                *c = sub.min(del).min(ins);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.get(b.len()).copied().unwrap_or(0)
}

/// How well a team-name reading matches `team`: the best of its spellings against the reading
/// or any run of the reading's words (the score digit or a seed number may come along).
pub fn team_score(read: &str, team: &RosterTeam) -> f32 {
    let words: Vec<&str> = read.split_whitespace().take(12).collect();
    let mut best = 0.0f32;
    for form in &team.reads {
        for i in 0..words.len() {
            for j in i + 1..=words.len() {
                let run: String = words.get(i..j).map(|w| w.concat()).unwrap_or_default();
                best = best.max(similarity(&run, form));
            }
        }
    }
    best
}

/// Where the team name and the name plates are read for one bar: the team name above the
/// plates (`team_rect`) and the bar (the name strip).
pub fn team_rect(bar: &PlayerBar, team_names: Option<NRect>) -> NRect {
    team_names.unwrap_or_else(|| NRect::new(bar.rect.x, bar.rect.y - bar.rect.h * 6.0, bar.rect.w, bar.rect.h * 3.0))
}

/// The part of the frame to read for one bar: its team name and its name strip.
pub fn read_rect(bar: &PlayerBar, team_names: Option<NRect>) -> NRect {
    let t = team_rect(bar, team_names);
    let x0 = t.x.min(bar.rect.x) - 0.008;
    let y0 = t.y.min(bar.rect.y) - 0.005;
    let x1 = (t.x + t.w).max(bar.rect.x + bar.rect.w) + 0.008;
    let y1 = (t.y + t.h).max(bar.rect.y + bar.rect.h) + 0.008;
    NRect::new(x0, y0, x1 - x0, y1 - y0)
}

/// A word read off the frame: its centre (frame-relative) and text.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub x: f32,
    pub y: f32,
    pub text: String,
}

/// Sort the words read in [`read_rect`] into the team name and each plate's name (0-based plate,
/// text). Words in neither are dropped; a word wider than a plate (names run together) too.
pub fn sort_words(bar: &PlayerBar, team_names: Option<NRect>, words: &[(Word, f32)]) -> (Option<String>, Vec<(u8, String)>) {
    let t = team_rect(bar, team_names);
    let pw = bar.rect.w / bar.players.max(1) as f32;
    let mut team: Vec<&Word> = Vec::new();
    let mut plates: Vec<(u8, &Word)> = Vec::new();
    for (w, width) in words {
        let in_x = |x0: f32, ww: f32| w.x >= x0 && w.x <= x0 + ww;
        if in_x(t.x - 0.01, t.w + 0.02) && w.y >= t.y - t.h * 0.5 && w.y <= t.y + t.h * 1.5 {
            team.push(w);
        } else if in_x(bar.rect.x, bar.rect.w) && (w.y - (bar.rect.y + bar.rect.h * 0.5)).abs() <= bar.rect.h && *width <= pw * 1.1 {
            let k = ((w.x - bar.rect.x) / pw).floor().clamp(0.0, f32::from(bar.players.max(1) - 1)) as u8;
            plates.push((k, w));
        }
    }
    team.sort_by(|a, b| a.x.total_cmp(&b.x));
    let team = (!team.is_empty()).then(|| team.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "));
    plates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.x.total_cmp(&b.1.x)));
    let mut names: Vec<(u8, String)> = Vec::new();
    for (k, w) in plates {
        match names.last_mut() {
            Some((lk, s)) if *lk == k => s.push_str(&w.text),
            _ => names.push((k, w.text.clone())),
        }
    }
    (team, names)
}

#[derive(Clone, Debug)]
struct Read {
    at: i64,
    /// 1 or 2 (the bar's team).
    side: u8,
    /// 0-based plate; `None` for a team-name reading.
    plate: Option<u8>,
    text: String,
}

/// One stretch of play and who was on each plate (indexes into [`Roster::players`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stretch {
    pub start: i64,
    pub end: i64,
    /// Per side (team 1, team 2), per plate.
    pub plates: [Vec<Option<usize>>; 2],
}

impl Stretch {
    /// The player on `side`'s plate (1-based side and plate).
    pub fn player(&self, side: u8, plate: u8) -> Option<usize> {
        self.plates.get(usize::from(side.saturating_sub(1))).and_then(|p| p.get(usize::from(plate.saturating_sub(1)))).copied().flatten()
    }
}

/// What the readings say.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchInfo {
    /// Each side's team (index into [`Roster::teams`]).
    pub teams: [Option<usize>; 2],
    pub stretches: Vec<Stretch>,
}

impl MatchInfo {
    /// Every player seen, in order of first appearance.
    pub fn players(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for s in &self.stretches {
            for p in s.plates.iter().flatten().flatten() {
                if !out.contains(p) {
                    out.push(*p);
                }
            }
        }
        out
    }

    /// The stretch around `at`.
    pub fn stretch_at(&self, at: i64) -> Option<&Stretch> {
        self.stretches.iter().find(|s| at >= s.start && at <= s.end)
    }
}

/// Collects readings and stretches of play (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct LineupReader {
    plates: [u8; 2],
    reads: Vec<Read>,
    /// Stretches so far: (start, end).
    stretches: Vec<(i64, i64)>,
    open: bool,
    off: u32,
}

impl LineupReader {
    /// Read the two bars of `bars` (team 1 and team 2).
    pub fn new(bars: &[PlayerBar]) -> Self {
        let n = |t: u8| bars.iter().find(|b| b.team == t).map(|b| b.players).unwrap_or(0);
        Self { plates: [n(1), n(2)], ..Self::default() }
    }

    /// Every sample, read or not: whether the game HUD is on screen (stretches of play).
    pub fn sample(&mut self, at: i64, live: bool) {
        if !live {
            self.off = self.off.saturating_add(1);
            if self.off >= BREAK_SAMPLES {
                self.open = false;
            }
            return;
        }
        self.off = 0;
        match self.stretches.last_mut() {
            Some(s) if self.open => s.1 = at,
            _ => {
                self.stretches.push((at, at));
                self.open = true;
            }
        }
    }

    /// Whether the current stretch already has enough readings of every plate (the caller can
    /// read less often).
    pub fn settled(&self, at: i64) -> bool {
        let Some(&(start, _)) = self.stretches.last() else { return false };
        if self.plates.iter().all(|&n| n == 0) {
            return false;
        }
        let mut counts = [[0u32; 16]; 2];
        for r in self.reads.iter().rev().take_while(|r| r.at >= start) {
            if let (Some(p), Some(c)) = (r.plate, counts.get_mut(usize::from(r.side.saturating_sub(1))))
                && let Some(n) = c.get_mut(usize::from(p))
            {
                *n += 1;
            }
        }
        at >= start
            && (0..2)
                .all(|s| (0..self.plates.get(s).copied().unwrap_or(0)).all(|p| counts.get(s).and_then(|c| c.get(usize::from(p))).copied().unwrap_or(0) >= 6))
    }

    /// The team name read over `side`'s bar.
    pub fn team(&mut self, at: i64, side: u8, text: &str) {
        self.push(Read { at, side, plate: None, text: text.chars().take(MAX_TEXT).collect() });
    }

    /// The name read on `side`'s plate `plate` (0-based).
    pub fn name(&mut self, at: i64, side: u8, plate: u8, text: &str) {
        self.push(Read { at, side, plate: Some(plate), text: text.chars().take(MAX_TEXT).collect() });
    }

    fn push(&mut self, r: Read) {
        if self.reads.len() < MAX_READS && !canon(&r.text).is_empty() && (1..=2).contains(&r.side) {
            self.reads.push(r);
        }
    }

    /// Resolve the readings against `roster`.
    pub fn finish(&self, roster: &Roster) -> MatchInfo {
        let mut info = MatchInfo::default();
        // teams: summed similarity of every reading over the threshold
        for side in 1..=2u8 {
            let mut votes = vec![0.0f32; roster.teams.len()];
            for r in self.reads.iter().filter(|r| r.side == side && r.plate.is_none()) {
                let best = roster.teams.iter().enumerate().map(|(i, t)| (i, team_score(&r.text, t))).max_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((i, s)) = best.filter(|b| b.1 >= TEAM_MIN)
                    && let Some(v) = votes.get_mut(i)
                {
                    *v += s;
                }
            }
            let top = votes.iter().enumerate().filter(|(_, v)| **v >= 1.0).max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
            if let Some(t) = info.teams.get_mut(usize::from(side - 1)) {
                *t = top;
            }
        }
        // the other side's team cannot be the same one
        if info.teams[0].is_some() && info.teams[0] == info.teams[1] {
            info.teams[1] = None;
        }
        let candidates = |side: u8| -> Vec<usize> {
            let team = info.teams.get(usize::from(side - 1)).copied().flatten().and_then(|t| roster.teams.get(t)).map(|t| t.id.as_str());
            roster.players.iter().enumerate().filter(|(_, p)| team.is_some() && p.team.as_deref() == team).map(|(i, _)| i).collect()
        };
        let cands = [candidates(1), candidates(2)];
        let mut prev: [Vec<Option<usize>>; 2] = Default::default();
        for &(start, end) in &self.stretches {
            let mut st = Stretch { start, end, plates: Default::default() };
            for side in 1..=2u8 {
                let si = usize::from(side - 1);
                let n = usize::from(self.plates.get(si).copied().unwrap_or(0));
                let cand = cands.get(si).cloned().unwrap_or_default();
                // votes[plate][candidate]
                let mut votes = vec![vec![0.0f32; cand.len()]; n];
                for r in self.reads.iter().filter(|r| r.side == side && r.at >= start && r.at <= end) {
                    let Some(p) = r.plate.map(usize::from) else { continue };
                    let best = cand
                        .iter()
                        .enumerate()
                        .filter_map(|(ci, &pi)| roster.players.get(pi).map(|pl| (ci, similarity(&r.text, &pl.handle))))
                        .max_by(|a, b| a.1.total_cmp(&b.1));
                    if let Some((ci, s)) = best.filter(|b| b.1 >= PLAYER_MIN)
                        && let Some(v) = votes.get_mut(p).and_then(|v| v.get_mut(ci))
                    {
                        *v += s;
                    }
                }
                // greedy: the strongest (plate, player) first, each player once
                let mut plates = vec![None; n];
                let mut pairs: Vec<(usize, usize, f32)> =
                    votes.iter().enumerate().flat_map(|(p, v)| v.iter().enumerate().map(move |(c, s)| (p, c, *s))).filter(|x| x.2 >= PLAYER_VOTES).collect();
                pairs.sort_by(|a, b| b.2.total_cmp(&a.2));
                let mut used = vec![false; cand.len()];
                for (p, c, _) in pairs {
                    if plates.get(p).is_some_and(Option::is_none) && used.get(c).is_some_and(|u| !u) {
                        if let Some(x) = plates.get_mut(p) {
                            *x = cand.get(c).copied();
                        }
                        if let Some(u) = used.get_mut(c) {
                            *u = true;
                        }
                    }
                }
                // one plate unread, everyone else from the last stretch: the one left over
                let last = prev.get(si).cloned().unwrap_or_default();
                let missing: Vec<usize> = (0..n).filter(|&p| plates.get(p).is_some_and(Option::is_none)).collect();
                let known: Vec<usize> = plates.iter().flatten().copied().collect();
                let left: Vec<usize> = last.iter().flatten().copied().filter(|p| !known.contains(p)).collect();
                if let ([m], [l]) = (missing.as_slice(), left.as_slice())
                    && known.iter().all(|k| last.contains(&Some(*k)))
                    && let Some(x) = plates.get_mut(*m)
                {
                    *x = Some(*l);
                }
                if let Some(x) = prev.get_mut(si) {
                    *x = plates.clone();
                }
                if let Some(x) = st.plates.get_mut(si) {
                    *x = plates;
                }
            }
            info.stretches.push(st);
        }
        info
    }
}

/// A hero pick or swap with the player it was (when their plate was read).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerHero {
    pub event: HeroEvent,
    /// Index into [`Roster::players`].
    pub player: Option<usize>,
}

/// Join hero events with the players on the plates at the time.
pub fn attribute(heroes: &HeroScan, info: &MatchInfo) -> Vec<PlayerHero> {
    heroes.events.iter().map(|e| PlayerHero { event: e.clone(), player: info.stretch_at(e.at).and_then(|s| s.player(e.team, e.player)) }).collect()
}

#[cfg(test)]
#[path = "lineup_tests.rs"]
mod tests;

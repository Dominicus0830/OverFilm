//! Who is who, from the kill feed and the name plates read together, so that fewer names have to
//! be typed in.
//!
//! The kill feed ([`crate::feedrows`]) and the name plates ([`crate::kills`],
//! [`crate::heroes`], [`crate::lineup`]) see the same deaths: when a row says a player died, one
//! plate of that team goes dark at about the same time. Pairing the two:
//!
//! - which team is blue in the feed (the side whose deaths line up with that team's plates);
//! - each feed **player** is a plate, and the plate's name (read by text recognition) is the
//!   player's name: the feed player is named without the user;
//! - each feed **hero** picture is the hero on that plate at the time: a hero named on either
//!   side (on the plates or in the feed) names the other one;
//! - killers are followed through the same pairing: once a feed player is known to sit on a
//!   plate, the hero picture beside their name when they kill is the plate's hero too;
//! - a feed player whose name the blurry feed confused with another (two names of one length)
//!   is still known kill by kill: each paired kill's victim is the plate's player.
//!
//! Everything is voted on over many kills; one pairing decides nothing. Nothing here is built-in
//! artwork or text: the names come from the user, the glossary's roster or text recognition.

use std::collections::HashMap;

use crate::feedrows::{FeedScan, Side};
use crate::heroes::HeroScan;
use crate::kills::Death;
use crate::lineup::MatchInfo;

/// A feed row and a plate's death this far apart (ms) may be the same kill.
pub const WINDOW_MS: i64 = 2_500;
/// Two deaths of one team closer than this (ms) to a row cannot be told apart by time.
const AMBIGUOUS_MS: i64 = 400;
/// Fewest pairings that decide which team is blue.
const MIN_SIDE_VOTES: u32 = 3;
/// Fewest votes for a link, and the share of all its votes the winner needs.
const MIN_VOTES: u32 = 2;
const MIN_SHARE: f32 = 0.6;

/// A plate: team (1 or 2) and place on the bar (1-based).
pub type Plate = (u8, u8);

/// What the feed and the plates say together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Links {
    /// The team shown blue in the feed.
    pub blue_team: Option<u8>,
    /// Per feed event (same order as [`FeedScan::events`]): the victim's plate, when the row was
    /// paired with a death on the plates.
    pub victims: Vec<Option<Plate>>,
    /// Per feed event: the killer's plate, through the killer's own deaths.
    pub killers: Vec<Option<Plate>>,
    /// Per feed player: the plate they sit on most.
    pub player_plates: Vec<Option<Plate>>,
    /// Per feed player: the roster player read on their plate (index into
    /// [`crate::lineup::Roster::players`]).
    pub player_roster: Vec<Option<usize>>,
    /// Per feed hero: the plate hero ([`HeroScan`] index) it is.
    pub heroes: Vec<Option<usize>>,
}

impl Links {
    /// The feed heroes linked to plate hero `h`.
    pub fn feed_heroes_of(&self, h: usize) -> Vec<usize> {
        self.heroes.iter().enumerate().filter(|(_, x)| **x == Some(h)).map(|(i, _)| i).collect()
    }
}

fn team_of(side: Side, blue: u8) -> u8 {
    match side {
        Side::Blue => blue,
        Side::Red => 3u8.saturating_sub(blue),
    }
}

/// The death of `team` (or of either team) nearest to `at` within [`WINDOW_MS`], not used yet;
/// `None` when two deaths are about as near (several players of that team fell together).
fn nearest(deaths: &[Death], used: &[bool], at: i64, team: Option<u8>) -> Option<usize> {
    let mut c: Vec<(i64, usize)> = deaths
        .iter()
        .enumerate()
        .filter(|(i, d)| !used.get(*i).copied().unwrap_or(true) && team.is_none_or(|t| d.team == t))
        .map(|(i, d)| (d.at.saturating_sub(at).saturating_abs(), i))
        .filter(|(dt, _)| *dt <= WINDOW_MS)
        .collect();
    c.sort_unstable();
    match (c.first(), c.get(1)) {
        (Some(a), Some(b)) if b.0.saturating_sub(a.0) < AMBIGUOUS_MS => None,
        (Some(a), _) => Some(a.1),
        _ => None,
    }
}

/// The plate hero on `plate` at `at`: its last pick or swap at or before then, in the same
/// stretch of play when the stretches are known.
pub fn hero_at(heroes: &HeroScan, info: &MatchInfo, plate: Plate, at: i64) -> Option<usize> {
    let start = info.stretch_at(at).map(|s| s.start).unwrap_or(i64::MIN);
    heroes.events.iter().filter(|e| (e.team, e.player) == plate && e.at <= at && e.at >= start).max_by_key(|e| e.at).map(|e| e.to)
}

/// The winner of `votes` when it has enough of them.
fn winner<K: Copy + Eq + std::hash::Hash + Ord>(votes: &HashMap<K, u32>) -> Option<K> {
    winner_by(votes, MIN_VOTES, MIN_SHARE)
}

/// A feed player is named after a plate only when nearly all their paired deaths agree: two
/// blurred names sharing one player number split their votes, and a name on every kill of such a
/// player would be wrong on many (each paired kill still names its own victim).
const NAME_VOTES: u32 = 3;
const NAME_SHARE: f32 = 0.8;

fn winner_by<K: Copy + Eq + std::hash::Hash + Ord>(votes: &HashMap<K, u32>, min_votes: u32, min_share: f32) -> Option<K> {
    let total: u32 = votes.values().sum();
    let mut v: Vec<(&K, &u32)> = votes.iter().collect();
    // most votes first, then the smaller key (the same answer on every run)
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let (k, n) = v.first()?;
    (**n >= min_votes && **n as f32 >= min_share * total as f32).then_some(**k)
}

/// Pair the feed's kills with the plates' deaths (see the module docs).
pub fn link(feed: &FeedScan, deaths: &[Death], heroes: &HeroScan, info: &MatchInfo) -> Links {
    let n_ev = feed.events.len();
    let mut out = Links {
        victims: vec![None; n_ev],
        killers: vec![None; n_ev],
        player_plates: vec![None; feed.players.len()],
        player_roster: vec![None; feed.players.len()],
        heroes: vec![None; feed.heroes.len()],
        ..Links::default()
    };
    // which team is blue: the nearest death of either team to each row
    let mut side_votes: HashMap<u8, u32> = HashMap::new();
    let none_used = vec![false; deaths.len()];
    for e in &feed.events {
        if let Some(d) = nearest(deaths, &none_used, e.at, None).and_then(|i| deaths.get(i)) {
            let blue = if e.victim.side == Side::Blue { d.team } else { 3u8.saturating_sub(d.team) };
            *side_votes.entry(blue).or_default() += 1;
        }
    }
    let total: u32 = side_votes.values().sum();
    let blue = match winner(&side_votes) {
        Some(b) if total >= MIN_SIDE_VOTES && (b == 1 || b == 2) => b,
        _ => return out,
    };
    out.blue_team = Some(blue);
    // each row with one death of the victim's team, each death used once
    let mut used = vec![false; deaths.len()];
    for (k, e) in feed.events.iter().enumerate() {
        let team = team_of(e.victim.side, blue);
        if let Some(i) = nearest(deaths, &used, e.at, Some(team))
            && let Some(d) = deaths.get(i)
        {
            if let Some(u) = used.get_mut(i) {
                *u = true;
            }
            if let Some(v) = out.victims.get_mut(k) {
                *v = Some((d.team, d.player));
            }
        }
    }
    // rows whose deaths fell together: the one on the plate that player was paired with before
    let mut first: HashMap<usize, HashMap<Plate, u32>> = HashMap::new();
    for (e, v) in feed.events.iter().zip(&out.victims) {
        if let Some(plate) = *v {
            *first.entry(e.victim.player).or_default().entry(plate).or_default() += 1;
        }
    }
    for (k, e) in feed.events.iter().enumerate() {
        if out.victims.get(k).copied().flatten().is_some() {
            continue;
        }
        let Some(plate) = first.get(&e.victim.player).and_then(winner) else { continue };
        let found = deaths
            .iter()
            .enumerate()
            .find(|(i, d)| !used.get(*i).copied().unwrap_or(true) && (d.team, d.player) == plate && d.at.saturating_sub(e.at).saturating_abs() <= WINDOW_MS);
        if let Some((i, _)) = found {
            if let Some(u) = used.get_mut(i) {
                *u = true;
            }
            if let Some(v) = out.victims.get_mut(k) {
                *v = Some(plate);
            }
        }
    }
    // feed players → plates, per stretch of play (substitutes, new maps) and overall
    let stretch = |at: i64| info.stretches.iter().position(|s| at >= s.start && at <= s.end);
    let mut by_stretch: HashMap<(usize, Option<usize>), HashMap<Plate, u32>> = HashMap::new();
    let mut overall: HashMap<usize, HashMap<Plate, u32>> = HashMap::new();
    let mut roster_votes: HashMap<usize, HashMap<usize, u32>> = HashMap::new();
    let mut hero_votes: HashMap<usize, HashMap<usize, u32>> = HashMap::new();
    for (e, v) in feed.events.iter().zip(&out.victims) {
        let Some(plate) = *v else { continue };
        let p = e.victim.player;
        *by_stretch.entry((p, stretch(e.at))).or_default().entry(plate).or_default() += 1;
        *overall.entry(p).or_default().entry(plate).or_default() += 1;
        if let Some(r) = info.stretch_at(e.at).and_then(|s| s.player(plate.0, plate.1)) {
            *roster_votes.entry(p).or_default().entry(r).or_default() += 1;
        }
        if let (Some(fh), Some(ph)) = (e.victim.hero, hero_at(heroes, info, plate, e.at)) {
            *hero_votes.entry(fh).or_default().entry(ph).or_default() += 1;
        }
    }
    for (p, votes) in &overall {
        if let Some(slot) = out.player_plates.get_mut(*p) {
            *slot = winner_by(votes, NAME_VOTES, NAME_SHARE);
        }
    }
    for (p, votes) in &roster_votes {
        if let Some(slot) = out.player_roster.get_mut(*p) {
            *slot = winner_by(votes, NAME_VOTES, NAME_SHARE);
        }
    }
    // killers: their plate in that stretch (else overall), never on the victim's team
    for (k, e) in feed.events.iter().enumerate() {
        let Some(killer) = e.killer else { continue };
        let team = team_of(killer.side, blue);
        let plate = by_stretch
            .get(&(killer.player, stretch(e.at)))
            .and_then(|v| winner_by(v, NAME_VOTES, NAME_SHARE))
            .or_else(|| out.player_plates.get(killer.player).copied().flatten())
            .filter(|pl| pl.0 == team);
        if let Some(slot) = out.killers.get_mut(k) {
            *slot = plate;
        }
        if let (Some(plate), Some(fh)) = (plate, killer.hero)
            && let Some(ph) = hero_at(heroes, info, plate, e.at)
        {
            *hero_votes.entry(fh).or_default().entry(ph).or_default() += 1;
        }
    }
    for (h, votes) in &hero_votes {
        if let Some(slot) = out.heroes.get_mut(*h) {
            *slot = winner(votes);
        }
    }
    out
}

/// Names the links give: feed players named after their plate's player (`roster_names`, by
/// roster index), feed heroes after their plate hero and plate heroes after their feed hero.
/// Only what has no name yet is filled. Returns how many names were filled.
pub fn fill_names(feed: &mut FeedScan, heroes: &mut HeroScan, links: &Links, roster_names: &dyn Fn(usize) -> Option<String>) -> usize {
    let mut filled = 0;
    for (p, r) in links.player_roster.iter().enumerate() {
        if let (Some(c), Some(name)) = (feed.players.get_mut(p), r.and_then(roster_names))
            && c.label.is_none()
        {
            c.label = Some(name);
            c.auto = true;
            filled += 1;
        }
    }
    if heroes.names.len() < heroes.prints.len() {
        heroes.names.resize(heroes.prints.len(), None);
    }
    if heroes.auto.len() < heroes.prints.len() {
        heroes.auto.resize(heroes.prints.len(), false);
    }
    for (fh, ph) in links.heroes.iter().enumerate() {
        let Some(ph) = *ph else { continue };
        let plate_name = heroes.names.get(ph).cloned().flatten();
        let Some(c) = feed.heroes.get_mut(fh) else { continue };
        match (&c.label, plate_name) {
            (None, Some(n)) => {
                c.label = Some(n);
                c.auto = true;
                filled += 1;
            }
            (Some(n), None) => {
                if let Some(slot) = heroes.names.get_mut(ph) {
                    *slot = Some(n.clone());
                    if let Some(a) = heroes.auto.get_mut(ph) {
                        *a = true;
                    }
                    filled += 1;
                }
            }
            _ => {}
        }
    }
    filled
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

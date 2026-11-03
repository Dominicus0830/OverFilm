//! OverFilm: teams, players and heroes read off the broadcast HUD (see `OVERFILM.md`), and the
//! match prior OverFilm Captions takes from them.
//!
//! `overfilm.kills.scan` reads these on the frames it decodes anyway (`players`, on by default):
//!
//! - the hero portrait on every name plate, every sample: a pick at the start of each stretch of
//!   play and each swap later (a new portrait that stays 12 s; a death's grayed portrait does
//!   not). Heroes are numbered ("영웅 3") until named with `overfilm.heroes.name`; the name is
//!   kept in the template file, so later scans know that portrait again;
//! - in builds with text recognition, the team names over the bars and the players' names on the
//!   plates, a few times at the start of each stretch and every 30 s after, matched against the
//!   glossary's teams and players (players per stretch: substitutes and reordered plates are
//!   followed).
//!
//! The markers, replaced by every scan:
//!
//! - "OverFilm 경기: 로제 vs 오투 블래스트" at the first stretch;
//! - "OverFilm 선수" (or "OverFilm 선수 교체: 로제 아이러니 → 오프너") where the players on the
//!   plates change, comment "로제: 태종, 킬로, …" per line (`?` for a plate not read);
//! - "OverFilm 영웅 선택" at each stretch's picks (only the plates whose hero is not the one they
//!   had), comment "태종: 트레이서" per line (the plate, "1팀 2번", when the player is not known);
//! - "OverFilm 영웅 교체: 우톈 트레이서 → 겐지" at each swap, comment "우톈: 트레이서 → 겐지".
//!
//! [`prior_for`] reads them back (so the user can correct a marker by hand) into the
//! [`overfilm_pipeline::game::MatchPrior`] of one media item, on the item's own clock.
//!
//! - `overfilm.heroes.list {}`: the heroes of the last scan, with their numbers and names.
//! - `overfilm.heroes.name {id, name, templates?}`: name hero `id`; the markers are written again.

use filmcraft_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_pipeline::game::MatchPrior;
use overfilm_vision::{HeroScan, MatchInfo, PlayerHero, Roster, lineup};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, has_seq, str_p, u64_p};
use crate::overfilm_autocut::{load_set, templates_path};
use crate::{EngineError, Result, Session};

pub const MARK_MATCH: &str = "OverFilm 경기";
pub const MARK_LINEUP: &str = "OverFilm 선수";
pub const MARK_PICKS: &str = "OverFilm 영웅 선택";
pub const MARK_SWAP: &str = "OverFilm 영웅 교체";
const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;
/// Most markers of each kind written (a long broadcast has a few hundred swaps at most).
const MAX_MARKS: usize = 2_000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.heroes.list",
            label: "List Heroes on the Name Plates (OverFilm)",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_seq,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.heroes.name",
            label: "Name Hero (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"id":n,"name":str,"templates":path?}"#,
            enabled: has_seq,
            run: name,
            journal: true,
        },
    ]
}

/// The last scan's teams, players and heroes, for naming heroes.
#[derive(Clone, Debug)]
pub struct LineupState {
    pub seq: ItemId,
    pub info: MatchInfo,
    pub heroes: HeroScan,
    pub roster: Roster,
    /// Who sits on a plate (team, place) when text recognition did not read it: the kill feed
    /// players the user named, paired with the plates (see `overfilm_vision::link`).
    pub plate_names: Vec<((u8, u8), String)>,
}

impl LineupState {
    fn player(&self, i: Option<usize>) -> Option<&str> {
        i.and_then(|i| self.roster.players.get(i)).map(|p| p.name.as_str())
    }

    fn team(&self, side: u8) -> Option<&str> {
        self.info.teams.get(usize::from(side.saturating_sub(1))).copied().flatten().and_then(|t| self.roster.teams.get(t)).map(|t| t.name.as_str())
    }

    /// "태종", or the plate ("1팀 2번") when nobody was read on it.
    fn who(&self, ph: &PlayerHero) -> String {
        match self.player(ph.player) {
            Some(n) => n.to_string(),
            None => match self.plate_names.iter().find(|(p, _)| *p == (ph.event.team, ph.event.player)) {
                Some((_, n)) => n.clone(),
                None => format!("{}팀 {}번", ph.event.team, ph.event.player),
            },
        }
    }
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let seq = s.state.active_sequence;
    let Some(st) = s.overfilm_lineup.as_ref().filter(|l| Some(l.seq) == seq) else {
        return Ok(json!({"heroes": [], "teams": [], "players": []}));
    };
    let mut seen = vec![0usize; st.heroes.prints.len()];
    for e in &st.heroes.events {
        if let Some(n) = seen.get_mut(e.to) {
            *n += 1;
        }
    }
    let heroes: Vec<Value> = (0..st.heroes.prints.len())
        .map(|i| {
            let first = st.heroes.events.iter().find(|e| e.to == i).map(|e| e.at).unwrap_or(0);
            // where to look at it: a plate showing it, a second after it came on
            let plate = st.heroes.events.iter().find(|e| e.to == i).map(|e| json!([e.team, e.player]));
            json!({"id": i + 1, "name": st.heroes.label(i), "named": st.heroes.names.get(i).cloned().flatten().is_some(),
                "auto": st.heroes.auto.get(i).copied().unwrap_or(false), "picks": seen.get(i).copied().unwrap_or(0),
                "first": overfilm_vision::plan::clock(Tick(first.saturating_mul(TICKS_PER_MS))),
                "preview": {"time": first.saturating_add(1_000).saturating_mul(TICKS_PER_MS), "plate": plate}})
        })
        .collect();
    let teams: Vec<Value> = (1..=2u8).map(|t| json!(st.team(t))).collect();
    let players: Vec<Value> = st.info.players().into_iter().filter_map(|p| st.roster.players.get(p)).map(|p| json!(p.name)).collect();
    Ok(json!({"heroes": heroes, "teams": teams, "players": players}))
}

fn name(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.heroes.name";
    let id = u64_p(p, "id").filter(|i| *i >= 1).ok_or_else(|| bad(CMD, "`id` is the number shown in the list (1, 2, …)"))? as usize;
    let name = str_p(p, "name").map(str::trim).unwrap_or_default().chars().take(64).collect::<String>();
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let path = templates_path(s, p, CMD)?;
    let Some(st) = s.overfilm_lineup.as_mut().filter(|l| l.seq == seq) else {
        return Err(bad(CMD, "read the name plates first (overfilm.kills.scan)"));
    };
    let Some(print) = st.heroes.prints.get(id - 1).cloned() else { return Err(bad(CMD, format!("no hero {id} in the last scan"))) };
    if let Some(n) = st.heroes.names.get_mut(id - 1) {
        *n = (!name.is_empty()).then(|| name.clone());
    }
    if let Some(a) = st.heroes.auto.get_mut(id - 1) {
        *a = false;
    }
    let st = st.clone();
    let mut set = load_set(&path)?;
    set.hero_names.set(print, &name);
    crate::overfilm_autocut::write_file(&path, &set.to_json())?;
    let n = mark(s, &st)?;
    // the kill feed's pictures of the same hero follow, and its markers name it
    let mut feed_heroes = 0;
    if let Some(f) = s.overfilm_feed.as_mut().filter(|f| f.seq == seq) {
        for h in f.links.feed_heroes_of(id - 1) {
            if let Some(c) = f.scan.heroes.get_mut(h).filter(|c| c.label.is_none() || c.auto) {
                c.label = (!name.is_empty()).then(|| name.clone());
                c.auto = true;
                feed_heroes += 1;
            }
        }
        let f = crate::overfilm_kills::FeedState { seq, scan: f.scan.clone(), links: f.links.clone() };
        crate::overfilm_kills::mark_feed_with(s, &f, Some(&st))?;
    }
    Ok(json!({"templates": path, "id": id, "name": name, "markers": n, "feedHeroes": feed_heroes}))
}

/// Who played on each side's plates: "로제: 태종, 킬로, ?, …" per side.
fn lineup_lines(st: &LineupState, stretch: &lineup::Stretch) -> Vec<String> {
    (1..=2u8)
        .filter_map(|side| {
            let plates = stretch.plates.get(usize::from(side - 1))?;
            if plates.iter().all(Option::is_none) {
                return None;
            }
            let names: Vec<&str> = plates.iter().map(|p| st.player(*p).unwrap_or("?")).collect();
            Some(format!("{}: {}", st.team(side).map(String::from).unwrap_or_else(|| format!("{side}팀")), names.join(", ")))
        })
        .collect()
}

/// Write the markers of `st` on the active sequence (see the module docs), replacing earlier
/// ones (one undo step). Times are the sequence's (ms).
pub fn mark(s: &mut Session, st: &LineupState) -> Result<usize> {
    let end = s.active_sequence().ok_or(EngineError::NoSequence)?.duration();
    let t = |ms: i64| Tick(ms.saturating_mul(TICKS_PER_MS)).clamp(Tick::ZERO, end);
    let mut marks: Vec<(Tick, String, String, Label)> = Vec::new();
    if let (Some(a), Some(b)) = (st.team(1), st.team(2))
        && let Some(first) = st.info.stretches.first()
    {
        marks.push((t(first.start), format!("{MARK_MATCH}: {a} vs {b}"), "화면의 팀 이름에서 읽음".into(), Label::Lavender));
    }
    // players, where they change
    let mut last: Option<Vec<String>> = None;
    let mut last_sets: [Vec<usize>; 2] = Default::default();
    for stretch in &st.info.stretches {
        let lines = lineup_lines(st, stretch);
        if lines.is_empty() || last.as_ref() == Some(&lines) {
            continue;
        }
        let mut changes = Vec::new();
        for side in 0..2usize {
            let now: Vec<usize> = stretch.plates.get(side).map(|p| p.iter().flatten().copied().collect()).unwrap_or_default();
            let before = last_sets.get(side).cloned().unwrap_or_default();
            if !before.is_empty() && now.len() >= before.len() {
                let out: Vec<&str> = before.iter().filter(|p| !now.contains(p)).filter_map(|p| st.player(Some(*p))).collect();
                let inn: Vec<&str> = now.iter().filter(|p| !before.contains(p)).filter_map(|p| st.player(Some(*p))).collect();
                if !out.is_empty() && !inn.is_empty() {
                    let team = st.team(side as u8 + 1).map(String::from).unwrap_or_else(|| format!("{}팀", side + 1));
                    changes.push(format!("{team} {} → {}", out.join(", "), inn.join(", ")));
                }
            }
            if let Some(x) = last_sets.get_mut(side)
                && !now.is_empty()
            {
                *x = now;
            }
        }
        let name = if changes.is_empty() { MARK_LINEUP.to_string() } else { format!("{MARK_LINEUP} 교체: {}", changes.join(" / ")) };
        marks.push((t(stretch.start), name, lines.join("\n"), Label::Lavender));
        last = Some(lines);
    }
    // heroes
    let who = lineup::attribute(&st.heroes, &st.info);
    let mut picks: Vec<(i64, Vec<String>)> = Vec::new();
    for ph in &who {
        let e = &ph.event;
        let line = |h: usize| st.heroes.label(h);
        match e.from {
            None => {
                let stretch = st.info.stretch_at(e.at).map(|s| s.start).unwrap_or(e.at);
                let text = format!("{}: {}", st.who(ph), line(e.to));
                match picks.iter_mut().find(|(at, _)| *at == stretch) {
                    Some((_, l)) => l.push(text),
                    None => picks.push((stretch, vec![text])),
                }
            }
            Some(f) => {
                let w = st.who(ph);
                marks.push((t(e.at), format!("{MARK_SWAP}: {w} {} → {}", line(f), line(e.to)), format!("{w}: {} → {}", line(f), line(e.to)), Label::Iris));
            }
        }
    }
    for (at, lines) in picks {
        marks.push((t(at), MARK_PICKS.to_string(), lines.join("\n"), Label::Iris));
    }
    marks.truncate(MAX_MARKS * 4);
    let n = marks.len();
    s.edit("OverFilm Players and Heroes", move |pr, state| {
        let seq = state.active_sequence.ok_or(EngineError::NoSequence)?;
        let ids: Vec<u64> = (0..marks.len()).map(|_| pr.alloc_id()).collect();
        let q = pr.sequence_mut(seq).ok_or(EngineError::NoSequence)?;
        q.markers.retain(|m| !is_lineup_marker(&m.name));
        for ((start, name, comment, color), id) in marks.into_iter().zip(ids) {
            q.markers.push(Marker {
                id: MarkerId(id),
                start,
                duration: Tick::ZERO,
                name,
                comment,
                kind: MarkerKind::Comment,
                color,
                url: String::new(),
                author: String::new(),
                custom_color: None,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    Ok(n)
}

fn is_lineup_marker(name: &str) -> bool {
    [MARK_MATCH, MARK_LINEUP, MARK_PICKS, MARK_SWAP].iter().any(|p| name.starts_with(p))
}

/// "영웅 3": a hero nobody named, no help to recognition.
fn unnamed(hero: &str) -> bool {
    hero.strip_prefix("영웅 ").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// What the markers say, on the sequence's clock (ms): teams, players and the heroes in play
/// between consecutive hero markers.
pub fn prior_on_sequence(markers: &[Marker], end_ms: i64) -> MatchPrior {
    let mut p = MatchPrior::default();
    let mut ms: Vec<&Marker> = markers.iter().filter(|m| is_lineup_marker(&m.name)).collect();
    ms.sort_by_key(|m| m.start);
    let mut state: Vec<(String, String)> = Vec::new();
    let mut since: Option<i64> = None;
    let push_heroes = |p: &mut MatchPrior, state: &[(String, String)], from: Option<i64>, to: i64| {
        let mut names: Vec<String> = Vec::new();
        for (_, h) in state {
            if !unnamed(h) && !names.contains(h) {
                names.push(h.clone());
            }
        }
        if let Some(a) = from
            && !names.is_empty()
            && to > a
        {
            p.heroes.push((a, to, names));
        }
    };
    for m in ms {
        let at = m.start.0 / TICKS_PER_MS;
        if let Some(rest) = m.name.strip_prefix(MARK_MATCH).and_then(|r| r.strip_prefix(": ")) {
            for t in rest.split(" vs ").map(str::trim).filter(|t| !t.is_empty()) {
                if !p.teams.iter().any(|x| x == t) {
                    p.teams.push(t.to_string());
                }
            }
        } else if m.name.starts_with(MARK_LINEUP) {
            for line in m.comment.lines() {
                let Some((_, names)) = line.split_once(": ") else { continue };
                for n in names.split(", ").map(str::trim).filter(|n| !n.is_empty() && *n != "?") {
                    if !p.players.iter().any(|x| x == n) {
                        p.players.push(n.to_string());
                    }
                }
            }
        } else if m.name.starts_with(MARK_PICKS) {
            push_heroes(&mut p, &state, since, at);
            // a stretch's picks list only the plates whose hero changed: the others keep theirs
            for (w, h) in m.comment.lines().filter_map(|l| l.split_once(": ")) {
                let (w, h) = (w.trim().to_string(), h.trim().to_string());
                match state.iter_mut().find(|(x, _)| *x == w) {
                    Some(e) => e.1 = h,
                    None => state.push((w, h)),
                }
            }
            since = Some(at);
        } else if m.name.starts_with(MARK_SWAP) {
            let Some((w, change)) = m.comment.split_once(": ") else { continue };
            let Some((_, to)) = change.split_once(" → ") else { continue };
            push_heroes(&mut p, &state, since, at);
            let (w, to) = (w.trim().to_string(), to.trim().to_string());
            match state.iter_mut().find(|(x, _)| *x == w) {
                Some(e) => e.1 = to,
                None => state.push((w, to)),
            }
            since = Some(at);
        }
    }
    push_heroes(&mut p, &state, since, end_ms);
    p
}

/// The match prior of media item `item` on the active sequence, on the item's own clock (ms from
/// its start): the hero spans are moved through every clip of the item that covers them. `None`
/// when the sequence has no such markers.
pub fn prior_for(s: &Session, item: ItemId) -> Option<MatchPrior> {
    let q = s.active_sequence()?;
    let end_ms = q.duration().0 / TICKS_PER_MS;
    let on_seq = prior_on_sequence(&q.markers, end_ms);
    if on_seq.is_empty() {
        return None;
    }
    let clips: Vec<_> = q.video_tracks.iter().chain(&q.audio_tracks).flat_map(|t| t.items.iter()).filter(|c| c.item == item).cloned().collect();
    let mut p = MatchPrior { teams: on_seq.teams, players: on_seq.players, heroes: Vec::new() };
    for (a, b, names) in on_seq.heroes {
        let (a, b) = (Tick(a.saturating_mul(TICKS_PER_MS)), Tick(b.saturating_mul(TICKS_PER_MS)));
        for c in &clips {
            let (x, y) = (a.max(c.start), b.min(c.end()));
            if x < y {
                let (mx, my) = (c.source_time_at(x).0 / TICKS_PER_MS, c.source_time_at(y).0 / TICKS_PER_MS);
                p.heroes.push((mx.min(my), mx.max(my), names.clone()));
            }
        }
    }
    Some(p)
}

#[cfg(test)]
#[path = "overfilm_lineup_tests.rs"]
mod tests;

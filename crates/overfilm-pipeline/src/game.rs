//! What the screen says about the match, for recognition: the teams, the players on the name
//! plates and the heroes they play over time ([`MatchPrior`]), read by the editor's screen scan
//! (`overfilm_vision::lineup` and `heroes`). It is a **soft** prior: the names in play go first
//! in the recogniser's context and win close calls in the glossary correction, but nothing else
//! is held back, since commentators talk about heroes nobody plays ("아나에서 독트린으로 바꿔야").
//!
//! [`roster`] is the built-in pack's teams and their players (the `# 팀 이름` sections of
//! `players.tsv`), the list screen readings are matched against.

use overfilm_glossary::Preferred;
use overfilm_vision::{Roster, RosterPlayer, RosterTeam};
use serde::{Deserialize, Serialize};

const TEAMS: &str = include_str!("../../../packs/overwatch/ko/glossary/teams.tsv");
const PLAYERS: &str = include_str!("../../../packs/overwatch/ko/glossary/players.tsv");

/// Words in a player's note that say they play (not a coach, analyst or streamer).
const ROLES: [&str; 7] = ["돌격", "공격", "지원", "딜러", "탱커", "힐러", "휴식"];

fn latin(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii() || c == 'Ø' || c == 'ø')
}

fn rows(table: &str) -> impl Iterator<Item = Result<Vec<&str>, &str>> {
    table.lines().skip(1).filter(|l| !l.trim().is_empty()).map(|l| match l.strip_prefix('#') {
        Some(h) => Err(h.trim()),
        None => Ok(l.split('\t').map(str::trim).collect()),
    })
}

/// The built-in teams and their players.
pub fn roster() -> Roster {
    let mut r = Roster::default();
    for row in rows(TEAMS).filter_map(Result::ok) {
        let (Some(id), Some(ko), Some(en)) = (row.first(), row.get(1), row.get(2)) else { continue };
        let mut reads: Vec<String> = vec![en.to_string()];
        reads.extend(row.get(3).into_iter().flat_map(|a| a.split(';')).map(str::trim).filter(|a| latin(a)).map(String::from));
        reads.retain(|s| !s.is_empty());
        if ko.is_empty() || reads.is_empty() {
            continue;
        }
        r.teams.push(RosterTeam { id: id.to_string(), name: ko.to_string(), reads });
    }
    let norm = |s: &str| overfilm_vision::lineup::canon(s);
    let mut team: Option<String> = None;
    for row in rows(PLAYERS) {
        match row {
            Err(header) => {
                let h = header.split(" (").next().unwrap_or(header);
                team = r.teams.iter().find(|t| t.reads.iter().any(|x| norm(x) == norm(h))).map(|t| t.id.clone());
            }
            Ok(row) => {
                let (Some(id), Some(ko), Some(en)) = (row.first(), row.get(1), row.get(2)) else { continue };
                let note = row.get(7).copied().unwrap_or("");
                if team.is_none() || en.is_empty() || ko.is_empty() || note.starts_with("전 ") || !ROLES.iter().any(|w| note.contains(w)) {
                    continue;
                }
                r.players.push(RosterPlayer { id: id.to_string(), name: ko.to_string(), handle: en.to_string(), team: team.clone() });
            }
        }
    }
    r
}

/// Who and what was on screen, on the recognised sound's clock (ms from its start).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchPrior {
    /// The two teams' names (`로제`, `오투 블래스트`).
    pub teams: Vec<String>,
    /// Every player seen on the plates.
    pub players: Vec<String>,
    /// Heroes in play: (start ms, end ms, hero names).
    pub heroes: Vec<(i64, i64, Vec<String>)>,
    /// When the sound holds more than one game (a broadcast day): each game's span, teams and
    /// players. [`Self::teams`] and [`Self::players`] are then every game's together.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub games: Vec<GamePrior>,
}

/// One game of a [`MatchPrior`] (ms on the same clock).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GamePrior {
    pub start: i64,
    pub end: i64,
    pub teams: Vec<String>,
    pub players: Vec<String>,
}

impl MatchPrior {
    pub fn is_empty(&self) -> bool {
        self.teams.is_empty() && self.players.is_empty() && self.heroes.is_empty()
    }

    /// The recogniser's context about this match (goes before the general lists).
    pub fn context(&self) -> String {
        let mut s = String::new();
        let games: Vec<String> = self.games.iter().filter(|g| !g.teams.is_empty()).map(|g| g.teams.join(" 대 ")).collect();
        if games.len() > 1 {
            s.push_str(&format!("이 영상의 경기: {}.", games.join(", ")));
        } else if !self.teams.is_empty() {
            s.push_str(&format!("이번 경기: {}.", self.teams.join(" 대 ")));
        }
        if !self.players.is_empty() {
            s.push_str(&format!(" 출전 선수: {}.", self.players.join(", ")));
        }
        s.trim().to_string()
    }

    /// The names to prefer in a chunk from `start` to `end` (ms): the teams and players of the
    /// games it overlaps (all of them when there are no games or it overlaps none), heroes in
    /// play at some point of it.
    pub fn preferred(&self, start: i64, end: i64) -> Preferred {
        let heroes = self.heroes.iter().filter(|(a, b, _)| *a <= end && *b >= start).flat_map(|(_, _, h)| h.iter().map(|n| (n.as_str(), false)));
        let games: Vec<&GamePrior> = self.games.iter().filter(|g| g.start <= end && g.end >= start).collect();
        if games.is_empty() {
            return Preferred::new(self.teams.iter().map(|t| (t.as_str(), false)).chain(self.players.iter().map(|p| (p.as_str(), true))).chain(heroes));
        }
        let teams = games.iter().flat_map(|g| g.teams.iter().map(|t| (t.as_str(), false)));
        let players = games.iter().flat_map(|g| g.players.iter().map(|p| (p.as_str(), true)));
        Preferred::new(teams.chain(players).chain(heroes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_roster_has_the_owcs_teams() {
        let r = roster();
        let team = |id: &str| r.teams.iter().find(|t| t.id == id).unwrap();
        assert!(team("rode_zanside").reads.iter().any(|s| s == "RØDE ZANSIDE GAMING"));
        assert!(team("rode_zanside").reads.iter().any(|s| s == "ROZE"));
        let of = |t: &str| r.players.iter().filter(|p| p.team.as_deref() == Some(t)).map(|p| p.id.as_str()).collect::<Vec<_>>();
        let o2 = of("o2_blast");
        for p in ["a1ien", "wutian", "homerunball", "gamjung", "faith", "perr"] {
            assert!(o2.contains(&p), "{p} in {o2:?}");
        }
        // coaches and streamers are no players
        assert!(!o2.contains(&"cane") && !o2.contains(&"kaiser") && !o2.contains(&"o2boss"));
        let roze = of("rode_zanside");
        for p in ["taejong", "kilo", "heesung", "irony", "opener"] {
            assert!(roze.contains(&p), "{p} in {roze:?}");
        }
    }

    #[test]
    fn prior_context_and_preferences() {
        let p = MatchPrior {
            teams: vec!["로제".into(), "오투 블래스트".into()],
            players: vec!["태종".into(), "우톈".into()],
            heroes: vec![(0, 60_000, vec!["트레이서".into()]), (60_000, 120_000, vec!["겐지".into()])],
            games: Vec::new(),
        };
        assert_eq!(p.context(), "이번 경기: 로제 대 오투 블래스트. 출전 선수: 태종, 우톈.");
        assert!(MatchPrior::default().context().is_empty());
        assert!(MatchPrior::default().is_empty());
        let g = crate::pack::glossary(None, false).unwrap();
        let c = crate::pack::corrector(&g);
        let all = p.preferred(0, 120_000);
        assert!(!all.is_empty());
        assert_eq!(c.apply_preferring("우텐 선수", &all).text, "우톈 선수");
    }

    #[test]
    fn a_broadcast_with_two_games_prefers_each_games_names() {
        let game = |start, end, teams: [&str; 2], players: &[&str]| GamePrior {
            start,
            end,
            teams: teams.iter().map(|t| t.to_string()).collect(),
            players: players.iter().map(|p| p.to_string()).collect(),
        };
        let p = MatchPrior {
            teams: vec!["로제".into(), "오투 블래스트".into(), "티원".into(), "크레이지 라쿤".into()],
            players: vec!["태종".into(), "우톈".into(), "페이커".into(), "필더".into()],
            heroes: Vec::new(),
            games: vec![game(0, 60_000, ["로제", "오투 블래스트"], &["태종", "우톈"]), game(70_000, 120_000, ["티원", "크레이지 라쿤"], &["페이커", "필더"])],
        };
        assert_eq!(p.context(), "이 영상의 경기: 로제 대 오투 블래스트, 티원 대 크레이지 라쿤. 출전 선수: 태종, 우톈, 페이커, 필더.");
        let names = |a, b| format!("{:?}", p.preferred(a, b));
        assert!(names(0, 30_000).contains("우톈") && !names(0, 30_000).contains("페이커"));
        assert!(names(80_000, 90_000).contains("페이커") && !names(80_000, 90_000).contains("우톈"));
        assert!(names(50_000, 80_000).contains("우톈") && names(50_000, 80_000).contains("페이커"), "a chunk over the boundary");
        assert!(names(61_000, 69_000).contains("우톈") && names(61_000, 69_000).contains("필더"), "between games: everyone");
        // a prior saved before games existed still loads
        let old: MatchPrior = serde_json::from_str(r#"{"teams":["로제"],"players":[],"heroes":[]}"#).unwrap();
        assert!(old.games.is_empty());
        assert_eq!(old.context(), "이번 경기: 로제.");
    }
}

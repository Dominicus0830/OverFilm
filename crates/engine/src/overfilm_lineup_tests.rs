use overfilm_vision::lineup::Stretch;
use overfilm_vision::{HeroEvent, HeroPrint};
use serde_json::json;

use super::*;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn state(s: &Session) -> LineupState {
    let roster = overfilm_pipeline::game::roster();
    let idx = |id: &str| roster.players.iter().position(|p| p.id == id);
    let team = |id: &str| roster.teams.iter().position(|t| t.id == id);
    let plates = |ids: [&str; 5]| ids.iter().map(|i| idx(i)).collect::<Vec<_>>();
    let o2 = |last: &str| plates(["a1ien", "wutian", "homerunball", "gamjung", last]);
    let teams = [team("rode_zanside"), team("o2_blast")];
    let info = MatchInfo {
        teams,
        stretches: vec![
            Stretch { start: 1_000, end: 10_000, plates: [plates(["taejong", "kilo", "heesung", "irony", "opener"]), o2("faith")], teams },
            Stretch { start: 12_000, end: 20_000, plates: [plates(["kilo", "taejong", "heesung", "irony", "opener"]), o2("perr")], teams },
        ],
        games: vec![Game { start: 1_000, end: 20_000, teams }],
    };
    let ev = |at, team, player, from, to| HeroEvent { at, team, player, from, to };
    let heroes = HeroScan {
        events: vec![ev(1_000, 1, 1, None, 0), ev(1_000, 2, 2, None, 1), ev(5_000, 2, 2, Some(1), 2), ev(12_000, 1, 1, None, 0)],
        prints: vec![HeroPrint(vec![1]), HeroPrint(vec![2]), HeroPrint(vec![3])],
        names: vec![Some("트레이서".into()), None, Some("겐지".into())],
    };
    LineupState { seq: s.state.active_sequence.unwrap(), info, heroes, roster }
}

fn markers(s: &Session) -> Vec<(i64, String, String)> {
    let q = s.active_sequence().unwrap();
    q.markers.iter().filter(|m| is_lineup_marker(&m.name)).map(|m| (m.start.0 / TICKS_PER_MS, m.name.clone(), m.comment.clone())).collect()
}

#[test]
fn markers_say_who_plays_what_and_read_back_as_a_prior() {
    let mut s = demo();
    let st = state(&s);
    mark(&mut s, &st).unwrap();
    let m = markers(&s);
    let names: Vec<&str> = m.iter().map(|x| x.1.as_str()).collect();
    assert!(names.contains(&"OverFilm 경기: 로데 잔사이드 vs 오투 블래스트"), "{names:?}");
    assert!(names.contains(&"OverFilm 선수"));
    assert!(names.contains(&"OverFilm 선수 교체: 오투 블래스트 페이스 → 페르"), "{names:?}");
    assert!(names.contains(&"OverFilm 영웅 교체: 우톈 영웅 2 → 겐지"), "{names:?}");
    let picks: Vec<&(i64, String, String)> = m.iter().filter(|x| x.1 == MARK_PICKS).collect();
    assert_eq!(picks.len(), 2);
    assert_eq!(picks[0].2, "태종: 트레이서\n우톈: 영웅 2");
    assert_eq!(picks[1].2, "킬로: 트레이서", "plates reordered: plate 1 is Kilo now");
    // read back: teams, players, heroes in play (unnamed heroes left out)
    let q = s.active_sequence().unwrap();
    let p = prior_on_sequence(&q.markers, 25_000);
    assert_eq!(p.teams, ["로데 잔사이드", "오투 블래스트"]);
    assert!(p.players.contains(&"우톈".to_string()) && p.players.contains(&"페르".to_string()) && p.players.len() == 11, "{:?}", p.players);
    assert_eq!(p.heroes[0], (1_000, 5_000, vec!["트레이서".to_string()]));
    assert_eq!(p.heroes[1], (5_000, 12_000, vec!["트레이서".to_string(), "겐지".to_string()]));
    // the second stretch's picks keep the heroes not picked again
    assert_eq!(p.heroes[2], (12_000, 25_000, vec!["트레이서".to_string(), "겐지".to_string()]));
    // scanning again replaces them; one undo step
    mark(&mut s, &st).unwrap();
    assert_eq!(markers(&s).len(), m.len());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(markers(&s).len(), m.len());
}

/// Two games: 로제 vs 오투 (the teams change sides for the second map), then 크레이지 라쿤 vs
/// 오투 again with the same players (a new game, not a substitution).
fn two_games(s: &Session) -> LineupState {
    let mut st = state(s);
    let roster = &st.roster;
    let team = |id: &str| roster.teams.iter().position(|t| t.id == id);
    let idx = |id: &str| roster.players.iter().position(|p| p.id == id);
    let (rode, o2, cr) = (team("rode_zanside"), team("o2_blast"), roster.teams.iter().position(|t| t.id != "rode_zanside" && t.id != "o2_blast"));
    let cr_player = cr.and_then(|c| roster.teams.get(c)).and_then(|c| roster.players.iter().position(|p| p.team.as_deref() == Some(c.id.as_str())));
    let o2p = vec![idx("a1ien"), idx("wutian"), idx("homerunball"), idx("gamjung"), idx("faith")];
    let rp = vec![idx("taejong"), idx("kilo"), idx("heesung"), idx("irony"), idx("opener")];
    let crp = vec![cr_player, None, None, None, None];
    st.info = MatchInfo {
        teams: [rode, o2],
        stretches: vec![
            Stretch { start: 1_000, end: 5_000, plates: [rp.clone(), o2p.clone()], teams: [rode, o2] },
            Stretch { start: 6_000, end: 12_000, plates: [o2p.clone(), rp], teams: [o2, rode] },
            Stretch { start: 15_000, end: 25_000, plates: [crp, o2p], teams: [cr, o2] },
        ],
        games: vec![Game { start: 1_000, end: 12_000, teams: [rode, o2] }, Game { start: 15_000, end: 25_000, teams: [cr, o2] }],
    };
    st.heroes = HeroScan::default();
    st
}

#[test]
fn a_broadcast_with_two_games() {
    let mut s = demo();
    let st = two_games(&s);
    mark(&mut s, &st).unwrap();
    let q = s.active_sequence().unwrap();
    let games: Vec<&Marker> = q.markers.iter().filter(|m| m.name.starts_with(MARK_MATCH)).collect();
    assert_eq!(games.len(), 2);
    assert_eq!(games[0].name, "OverFilm 경기 1: 로데 잔사이드 vs 오투 블래스트");
    assert!(games[1].name.starts_with("OverFilm 경기 2: ") && games[1].name.ends_with(" vs 오투 블래스트"), "{}", games[1].name);
    assert_eq!((games[0].start.0 / TICKS_PER_MS, games[0].duration.0 / TICKS_PER_MS), (1_000, 11_000), "a range over the game");
    let names: Vec<&str> = q.markers.iter().map(|m| m.name.as_str()).collect();
    assert!(!names.iter().any(|n| n.contains("교체")), "sides swapping or a new game is no substitution: {names:?}");
    let lineup: Vec<&Marker> = q.markers.iter().filter(|m| m.name == MARK_LINEUP).collect();
    assert_eq!(lineup.len(), 3, "each stretch's plates differ");
    assert!(lineup[1].comment.starts_with("오투 블래스트: 에일리언"), "{}", lineup[1].comment);
    // read back: each game with its own teams and players
    let p = prior_on_sequence(&q.markers, 50_000);
    let clip = q.video_tracks.iter().chain(&q.audio_tracks).flat_map(|t| t.items.iter()).next().cloned().unwrap();
    assert_eq!(p.games.len(), 2);
    assert_eq!(p.games[0].teams, ["로데 잔사이드", "오투 블래스트"]);
    assert_eq!((p.games[0].start, p.games[0].end, p.games[1].start, p.games[1].end), (1_000, 15_000, 15_000, 50_000));
    assert!(p.games[0].players.contains(&"태종".to_string()) && p.games[0].players.len() == 10);
    assert!(!p.games[1].players.contains(&"태종".to_string()) && p.games[1].players.contains(&"우톈".to_string()));
    assert_eq!(p.teams.len(), 3);
    assert!(p.context().starts_with("이 영상의 경기: 로데 잔사이드 대 오투 블래스트, "), "{}", p.context());
    // the list says the games too
    s.overfilm_lineup = Some(st);
    let l = s.execute("overfilm.heroes.list", json!({})).unwrap();
    assert_eq!(l["games"].as_array().map(Vec::len), Some(2));
    assert_eq!(l["games"][1]["n"], 2);
    // a clip of the item maps each game onto the item's clock
    let p = prior_for(&s, clip.item).unwrap();
    assert!(!p.games.is_empty());
    for g in &p.games {
        assert!(g.start <= g.end);
    }
}

#[test]
fn prior_follows_the_clips_of_the_item() {
    let mut s = demo();
    let st = state(&s);
    mark(&mut s, &st).unwrap();
    let q = s.active_sequence().unwrap();
    let clip = q.video_tracks.iter().chain(&q.audio_tracks).flat_map(|t| t.items.iter()).next().cloned().unwrap();
    let p = prior_for(&s, clip.item).unwrap();
    assert_eq!(p.teams.len(), 2);
    for (a, b, _) in &p.heroes {
        assert!(a <= b);
    }
    // an item not on the sequence gets the names but no hero spans
    let p = prior_for(&s, ItemId(u64::MAX)).unwrap();
    assert!(p.heroes.is_empty() && !p.players.is_empty());
}

#[test]
fn hand_edited_and_hostile_markers() {
    let mk = |at: i64, name: &str, comment: &str| Marker {
        id: MarkerId(0),
        start: Tick(at * TICKS_PER_MS),
        duration: Tick::ZERO,
        name: name.into(),
        comment: comment.into(),
        kind: MarkerKind::Comment,
        color: Label::Iris,
    };
    let ms = vec![
        mk(0, "OverFilm 경기: ", ""),
        mk(5, "OverFilm 선수", "garbage\n: \n로제: ?, , 태종"),
        mk(10, MARK_SWAP, "no arrow"),
        mk(20, &format!("{MARK_SWAP}: x"), "태종: 겐지 → 아나"),
        mk(i64::MAX / TICKS_PER_MS, MARK_PICKS, "태종: 영웅 7"),
    ];
    let p = prior_on_sequence(&ms, 100);
    assert!(p.teams.is_empty());
    assert_eq!(p.players, ["태종"]);
    assert_eq!(p.heroes, vec![(20, i64::MAX / TICKS_PER_MS, vec!["아나".to_string()])]);
    assert!(unnamed("영웅 12") && !unnamed("영웅") && !unnamed("영웅 둘"));
    assert!(p.games.is_empty());
    assert_eq!(match_teams("OverFilm 경기 3: 로제 vs ?"), Some(vec!["로제".to_string()]));
    assert_eq!(match_teams("OverFilm 경기: ? vs ?"), None);
    assert_eq!(match_teams("OverFilm 경기x: 로제"), None);
    assert_eq!(match_teams("OverFilm 선수"), None);
    // a game marker by hand after the players: they go with the first game
    let ms = vec![mk(0, "OverFilm 선수", "로제: 태종"), mk(10, "OverFilm 경기 1: 로제 vs 오투", ""), mk(20, "OverFilm 경기 2: 티원 vs 오투", "")];
    let p = prior_on_sequence(&ms, 100);
    assert_eq!(p.games.len(), 2);
    assert_eq!(p.games[0].players, ["태종"]);
    let m = matches_on_sequence(&ms, 100);
    assert_eq!((m.len(), m[0].start, m[0].end, m[1].end), (2, 10, 20, 100));
    assert_eq!(m[1].teams, ["티원", "오투"]);
    // one match: one entry over the rest of the video (the prior keeps no games for it)
    let one = vec![mk(5, "OverFilm 경기: 로제 vs 오투", "")];
    assert_eq!(matches_on_sequence(&one, 100).len(), 1);
    assert!(prior_on_sequence(&one, 100).games.is_empty());
    assert!(matches_on_sequence(&[], 100).is_empty());
}

#[test]
fn naming_needs_a_scan() {
    let mut s = demo();
    let r = s.execute("overfilm.heroes.name", json!({"id": 1, "name": "아나"}));
    assert!(r.is_err());
    let l = s.execute("overfilm.heroes.list", json!({})).unwrap();
    assert_eq!(l["heroes"], json!([]));
}

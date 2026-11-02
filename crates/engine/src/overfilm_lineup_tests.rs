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
    let info = MatchInfo {
        teams: [team("rode_zanside"), team("o2_blast")],
        stretches: vec![
            Stretch { start: 1_000, end: 10_000, plates: [plates(["taejong", "kilo", "heesung", "irony", "opener"]), o2("faith")] },
            Stretch { start: 12_000, end: 20_000, plates: [plates(["kilo", "taejong", "heesung", "irony", "opener"]), o2("perr")] },
        ],
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
    assert!(names.contains(&"OverFilm 경기: 로제 vs 오투 블래스트"), "{names:?}");
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
    assert_eq!(p.teams, ["로제", "오투 블래스트"]);
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
}

#[test]
fn naming_needs_a_scan() {
    let mut s = demo();
    let r = s.execute("overfilm.heroes.name", json!({"id": 1, "name": "아나"}));
    assert!(r.is_err());
    let l = s.execute("overfilm.heroes.list", json!({})).unwrap();
    assert_eq!(l["heroes"], json!([]));
}

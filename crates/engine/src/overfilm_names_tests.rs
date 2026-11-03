use overfilm_vision::feedrows::{Cluster, Sig};
use overfilm_vision::lineup::Stretch;
use overfilm_vision::{FeedEvent, FeedScan, HeroEvent, HeroPrint, HeroScan, Links, MatchInfo, Side, Who};
use serde_json::json;

use super::*;
use crate::overfilm_kills::{FeedState, MARK_FEED};
use crate::overfilm_lineup::LineupState;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn templates(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("overfilm-names-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tp = dir.join("screens.json");
    std::fs::write(&tp, overfilm_vision::builtin::OWCS_KOREA_2026).unwrap();
    tp.to_string_lossy().to_string()
}

fn cluster(seed: u8) -> Cluster {
    Cluster { sig: Sig { w: 4, h: 2, aspect: 2.0, px: vec![seed; 8] }, label: None, auto: false, seen: 1, side: None, hero: None, more: Vec::new() }
}

/// Plate hero 1 (트레이서, named) on team 1 plate 2, plate hero 2 (unnamed) on team 2 plate 1;
/// feed hero 1 is plate hero 2, feed hero 2 is plate hero 1.
fn states(s: &mut Session) {
    let seq = s.state.active_sequence.unwrap();
    let roster = overfilm_pipeline::game::roster();
    let wutian = roster.players.iter().position(|p| p.id == "wutian");
    let mut plates: [Vec<Option<usize>>; 2] = [vec![None; 5], vec![None; 5]];
    plates[1][0] = wutian;
    let info = MatchInfo { teams: [None, None], stretches: vec![Stretch { start: 0, end: 20_000, plates }] };
    let ev = |team, player, to| HeroEvent { at: 500, team, player, from: None, to };
    let heroes = HeroScan {
        events: vec![ev(1, 2, 0), ev(2, 1, 1)],
        prints: vec![HeroPrint(vec![10; 108]), HeroPrint(vec![200; 108])],
        names: vec![Some("트레이서".into()), None],
        auto: vec![false, false],
    };
    let who = |side, player, hero| Who { side, player, hero: Some(hero) };
    let scan = FeedScan {
        events: vec![FeedEvent { at: 2_000, killer: Some(who(Side::Blue, 0, 1)), victim: who(Side::Red, 1, 0), ult: false, seen: 3_000, y: 0.4 }],
        players: vec![cluster(1), cluster(2)],
        heroes: vec![cluster(3), cluster(4)],
    };
    let links = Links {
        blue_team: Some(1),
        victims: vec![Some((2, 1))],
        killers: vec![Some((1, 2))],
        player_plates: vec![Some((1, 2)), Some((2, 1))],
        player_roster: vec![None, wutian],
        heroes: vec![Some(1), Some(0)],
    };
    s.overfilm_lineup = Some(LineupState { seq, info, heroes, roster, plate_names: Vec::new() });
    s.overfilm_feed = Some(FeedState { seq, scan, links });
}

#[test]
fn previews_point_at_the_plate_and_the_feed_row() {
    let mut s = demo();
    let tp = templates("preview");
    // nothing scanned yet
    assert!(s.execute("overfilm.names.preview", json!({"templates": tp, "kind": "plate", "id": 1})).is_err());
    states(&mut s);
    let r = s.execute("overfilm.names.preview", json!({"templates": tp, "kind": "plate", "id": 2})).unwrap();
    assert_eq!(r["time"], 1_500 * TICKS_PER_MS, "a second after it came on");
    let rect: Vec<f64> = r["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    // team 2's bar is on the right
    assert!(rect[0] > 0.5 && rect[2] > 0.0 && rect[3] > 0.0 && rect[0] + rect[2] <= 1.0001, "{rect:?}");
    let r = s.execute("overfilm.names.preview", json!({"templates": tp, "kind": "hero", "id": 1})).unwrap();
    assert_eq!(r["time"], 3_000 * TICKS_PER_MS, "the last sample showing the row");
    let r = s.execute("overfilm.names.preview", json!({"templates": tp, "kind": "player", "id": 2})).unwrap();
    assert!(r["rect"][1].as_f64().unwrap() > 0.0);
    // the picture itself, cut out of the frame there
    let (px, w, h, _) = preview_image(&s, Kind::Plate, 1, &tp).unwrap();
    assert!(w > 0 && h > 0 && px.len() == w * h * 4);
    // hostile parameters
    for p in [json!({"kind": "plate", "id": 0}), json!({"kind": "plate", "id": 99}), json!({"kind": "x", "id": 1}), json!({"kind": "hero", "id": 9})] {
        let mut p = p;
        p["templates"] = json!(tp);
        assert!(s.execute("overfilm.names.preview", p.clone()).is_err(), "{p}");
    }
}

#[test]
fn a_name_given_once_names_both_readings() {
    let mut s = demo();
    let tp = templates("both");
    states(&mut s);
    // the feed's hero 2 is the plates' 트레이서: listed as named, automatically
    let l = s.execute("overfilm.killFeed.list", json!({})).unwrap();
    assert_eq!(l["heroes"][1]["named"], false);
    // naming the plate hero names the feed hero linked to it, and the markers say so
    let r = s.execute("overfilm.heroes.name", json!({"templates": tp, "id": 2, "name": "겐지"})).unwrap();
    assert_eq!(r["feedHeroes"], 1);
    let f = s.overfilm_feed.as_ref().unwrap();
    assert_eq!((f.scan.hero(0).as_str(), f.scan.heroes[0].auto), ("겐지", true));
    let feed: Vec<String> = s.active_sequence().unwrap().markers.iter().filter(|m| m.name.starts_with(MARK_FEED)).map(|m| m.name.clone()).collect();
    // the victim is named after their plate (우톈), the killer's hero after theirs (트레이서)
    assert_eq!(feed, ["OverFilm 킬피드: 선수 1 (트레이서) → 우톈 (겐지)"]);
    // naming a feed hero names the plate hero linked to it, kept in the template file
    s.execute("overfilm.heroes.name", json!({"templates": tp, "id": 1, "name": ""})).unwrap();
    let r = s.execute("overfilm.killFeed.name", json!({"templates": tp, "kind": "hero", "id": 2, "name": "소전"})).unwrap();
    assert_eq!(r["plateHeroes"], 1);
    let l = s.execute("overfilm.heroes.list", json!({})).unwrap();
    assert_eq!(l["heroes"][0]["name"], "소전");
    let set = overfilm_vision::TemplateSet::from_json(&std::fs::read_to_string(&tp).unwrap()).unwrap();
    assert_eq!(set.hero_names.lookup(&HeroPrint(vec![10; 108])), Some("소전"));
    // a hero the user named on the plates is not renamed from the feed
    s.execute("overfilm.killFeed.name", json!({"templates": tp, "kind": "hero", "id": 1, "name": "리퍼"})).unwrap();
    assert_eq!(s.execute("overfilm.heroes.list", json!({})).unwrap()["heroes"][1]["name"], "겐지");
    // a feed player the user names sits on their plate in the hero markers
    s.execute("overfilm.killFeed.name", json!({"templates": tp, "kind": "player", "id": 1, "name": "TAEJONG"})).unwrap();
    let lineup = s.overfilm_lineup.as_ref().unwrap();
    assert_eq!(lineup.plate_names, [((1, 2), "TAEJONG".to_string())]);
}

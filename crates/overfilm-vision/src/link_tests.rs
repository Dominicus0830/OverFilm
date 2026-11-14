use super::*;
use crate::feedrows::{Cluster, FeedEvent, Sig, Who};
use crate::heroes::{HeroEvent, HeroPrint};
use crate::lineup::Stretch;

fn cluster(seed: u8) -> Cluster {
    Cluster { sig: Sig { w: 4, h: 2, aspect: 2.0, px: vec![seed; 8] }, label: None, auto: false, seen: 1, side: None, hero: None, more: Vec::new() }
}

/// Team 1 is red in the feed. Feed players: 0 = team 2 plate 3, 1 = team 1 plate 1,
/// 2 = team 1 plate 4. Feed heroes: 0 = plate hero 2, 1 = plate hero 0, 2 = plate hero 1.
fn fixture() -> (FeedScan, Vec<Death>, HeroScan, MatchInfo) {
    let who = |side, player, hero| Who { side, player, hero: Some(hero) };
    let mut events = Vec::new();
    let mut deaths = Vec::new();
    for k in 0..4i64 {
        let t = 10_000 + k * 20_000;
        // blue player 0 (hero 0) kills red player 1 (hero 1)
        events.push(FeedEvent { at: t, killer: Some(who(Side::Blue, 0, 0)), victim: who(Side::Red, 1, 1), ult: false, seen: 0, y: 0.5 });
        deaths.push(Death { at: t + 300, team: 1, player: 1 });
        // red player 2 (hero 2) kills blue player 0 (hero 0)
        events.push(FeedEvent { at: t + 6_000, killer: Some(who(Side::Red, 2, 2)), victim: who(Side::Blue, 0, 0), ult: false, seen: 0, y: 0.5 });
        deaths.push(Death { at: t + 5_500, team: 2, player: 3 });
        // a death nobody saw in the feed
        deaths.push(Death { at: t + 12_000, team: 2, player: 5 });
    }
    // red player 2 dies twice, to give their plate
    events.push(FeedEvent { at: 95_000, killer: None, victim: who(Side::Red, 2, 2), ult: false, seen: 0, y: 0.5 });
    deaths.push(Death { at: 95_500, team: 1, player: 4 });
    events.push(FeedEvent { at: 99_000, killer: None, victim: who(Side::Red, 2, 2), ult: false, seen: 0, y: 0.5 });
    deaths.push(Death { at: 98_800, team: 1, player: 4 });
    events.push(FeedEvent { at: 120_000, killer: None, victim: who(Side::Red, 2, 2), ult: false, seen: 0, y: 0.5 });
    deaths.push(Death { at: 120_200, team: 1, player: 4 });
    deaths.sort_by_key(|d| d.at);
    let feed = FeedScan { events, players: vec![cluster(1), cluster(2), cluster(3)], heroes: vec![cluster(4), cluster(5), cluster(6)] };
    let ev = |team, player, to| HeroEvent { at: 0, team, player, from: None, to };
    let heroes = HeroScan {
        events: vec![ev(2, 3, 2), ev(1, 1, 0), ev(1, 4, 1)],
        prints: vec![HeroPrint(vec![1]), HeroPrint(vec![2]), HeroPrint(vec![3])],
        names: vec![Some("트레이서".into()), None, None],
        auto: Vec::new(),
    };
    let mut plates: [Vec<Option<usize>>; 2] = [vec![None; 5], vec![None; 5]];
    plates[0][0] = Some(10);
    plates[1][2] = Some(20);
    let info = MatchInfo { teams: [None, None], stretches: vec![Stretch { start: 0, end: 200_000, plates }] };
    (feed, deaths, heroes, info)
}

#[test]
fn pairs_feed_with_plates() {
    let (feed, deaths, heroes, info) = fixture();
    let l = link(&feed, &deaths, &heroes, &info);
    assert_eq!(l.blue_team, Some(2));
    assert_eq!(l.player_plates, vec![Some((2, 3)), Some((1, 1)), Some((1, 4))]);
    assert_eq!(l.player_roster, vec![Some(20), Some(10), None]);
    assert_eq!(l.heroes, vec![Some(2), Some(0), Some(1)]);
    assert_eq!(l.victims[0], Some((1, 1)));
    // red player 2's kills are placed through their own deaths
    assert_eq!(l.killers[1], Some((1, 4)));
    assert_eq!(l.feed_heroes_of(0), vec![1]);
}

#[test]
fn names_flow_both_ways() {
    let (mut feed, deaths, mut heroes, info) = fixture();
    feed.heroes[2].label = Some("겐지".into());
    let l = link(&feed, &deaths, &heroes, &info);
    let names = |r: usize| match r {
        10 => Some("태종".to_string()),
        20 => Some("우톈".to_string()),
        _ => None,
    };
    assert_eq!(fill_names(&mut feed, &mut heroes, &l, &names), 4);
    assert_eq!(feed.player(0), "우톈");
    assert!(feed.players[0].auto);
    assert_eq!(feed.player(2), "선수 3");
    // plate 트레이서 names feed hero 1; feed 겐지 names plate hero 1
    assert_eq!(feed.hero(1), "트레이서");
    assert!(feed.heroes[1].auto && !feed.heroes[2].auto);
    assert_eq!(heroes.label(1), "겐지");
    assert_eq!(heroes.auto, vec![false, true, false]);
    // filled once: nothing more to fill
    assert_eq!(fill_names(&mut feed, &mut heroes, &l, &names), 0);
}

#[test]
fn a_feed_player_of_two_names_is_not_named() {
    let (mut feed, mut deaths, heroes, info) = fixture();
    // red player 2's last death is on another plate: 3 of 4 is too few to name them
    deaths.retain(|d| d.at != 120_200);
    deaths.push(Death { at: 120_200, team: 1, player: 2 });
    deaths.sort_by_key(|d| d.at);
    feed.events.sort_by_key(|e| e.at);
    let l = link(&feed, &deaths, &heroes, &info);
    assert_eq!(l.player_plates[2], None);
    // each paired kill still knows its own victim's plate
    assert_eq!(l.victims.last().copied().flatten(), Some((1, 2)));
}

#[test]
fn too_little_to_decide() {
    let (feed, deaths, heroes, info) = fixture();
    // no deaths: nothing pairs
    let l = link(&feed, &[], &heroes, &info);
    assert_eq!(l.blue_team, None);
    assert!(l.victims.iter().all(Option::is_none) && l.heroes.iter().all(Option::is_none));
    // two deaths of one team at once cannot be told apart
    let twin = vec![Death { at: 10_000, team: 1, player: 1 }, Death { at: 10_100, team: 1, player: 2 }];
    assert_eq!(nearest(&twin, &[false, false], 10_000, Some(1)), None);
    // hostile: indexes out of range, an empty scan
    let mut bad = feed.clone();
    bad.events.push(FeedEvent {
        at: i64::MAX,
        killer: None,
        victim: Who { side: Side::Red, player: 999, hero: Some(999) },
        ult: false,
        seen: i64::MAX,
        y: 0.5,
    });
    let l = link(&bad, &deaths, &heroes, &info);
    assert_eq!(l.player_plates.len(), 3);
    let _ = link(&FeedScan::default(), &deaths, &HeroScan::default(), &MatchInfo::default());
    let mut e = (FeedScan::default(), HeroScan::default());
    assert_eq!(fill_names(&mut e.0, &mut e.1, &l, &|_| None), 0);
}

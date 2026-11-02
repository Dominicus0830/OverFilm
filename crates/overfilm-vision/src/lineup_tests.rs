use super::*;

fn roster() -> Roster {
    let team = |id: &str, name: &str, reads: &[&str]| RosterTeam { id: id.into(), name: name.into(), reads: reads.iter().map(|s| s.to_string()).collect() };
    let p = |id: &str, name: &str, handle: &str, team: &str| RosterPlayer { id: id.into(), name: name.into(), handle: handle.into(), team: Some(team.into()) };
    Roster {
        teams: vec![team("rode", "로제", &["RØDE ZANSIDE GAMING", "ROZE"]), team("o2", "오투 블래스트", &["O2 Blast", "O2"]), team("t1", "티원", &["T1"])],
        players: vec![
            p("taejong", "태종", "Taejong", "rode"),
            p("kilo", "킬로", "Kilo", "rode"),
            p("heesung", "희성", "HEESUNG", "rode"),
            p("irony", "아이러니", "iRONY", "rode"),
            p("opener", "오프너", "OPENER", "rode"),
            p("a1ien", "에일리언", "A1IEN", "o2"),
            p("wutian", "우톈", "WuTian", "o2"),
            p("homerunball", "홈런볼", "Homerunball", "o2"),
            p("gamjung", "감정", "Gamjung", "o2"),
            p("faith", "페이스", "Faith", "o2"),
            p("perr", "페르", "perr", "o2"),
            p("faker", "페이커", "Faker", "t1"),
        ],
    }
}

fn bars() -> Vec<PlayerBar> {
    vec![
        PlayerBar { team: 1, rect: NRect::new(0.0234, 0.1458, 0.2719, 0.0167), players: 5, alive_dark: vec![0.0; 5], ult: None, portrait: None },
        PlayerBar { team: 2, rect: NRect::new(0.7, 0.1458, 0.2719, 0.0167), players: 5, alive_dark: vec![0.0; 5], ult: None, portrait: None },
    ]
}

#[test]
fn canon_and_similarity() {
    assert_eq!(canon("RØDE zanside!"), "RODEZANSIDEI");
    assert_eq!(canon("A1IEN"), "AIIEN");
    assert!(similarity("D2 Blat", "O2 Blast") > 0.7);
    assert!(similarity("WUTAN", "WuTian") > 0.8);
    assert_eq!(similarity("", "x"), 0.0);
    let r = roster();
    // the score digit and seed number come along with the name
    assert!(team_score("#7 I RODE ZANSIDE GAMING 9S", &r.teams[0]) > 0.9);
    assert!(team_score("0 D2 Blat #", &r.teams[1]) > 0.6);
    assert!(team_score("RODE ZANSIDE GAMING", &r.teams[2]) < 0.5);
}

#[test]
fn words_sort_into_team_and_plates() {
    let b = &bars()[0];
    let w = |x: f32, y: f32, t: &str| (Word { x, y, text: t.into() }, 0.03);
    let words = vec![
        w(0.14, 0.045, "ZANSIDE"),
        w(0.09, 0.045, "RODE"),
        w(0.2, 0.045, "GAMING"),
        w(0.04, 0.154, "KILO"),
        w(0.285, 0.154, "ER"),
        w(0.27, 0.154, "OPEN"),
        w(0.5, 0.5, "noise"),
    ];
    let (team, names) = sort_words(b, None, &words);
    assert_eq!(team.as_deref(), Some("RODE ZANSIDE GAMING"));
    assert_eq!(names, vec![(0, "KILO".to_string()), (4, "OPENER".to_string())]);
}

#[test]
fn teams_players_and_substitutes_per_stretch() {
    let r = roster();
    let mut l = LineupReader::new(&bars());
    let mut at = 0;
    let round = |l: &mut LineupReader, at: &mut i64, names1: [&str; 5], names2: [&str; 5]| {
        for k in 0..10 {
            l.sample(*at, true);
            l.team(*at, 1, if k % 2 == 0 { "RODE ZANSIDE GAMING 0" } else { "#7 RUDE ZANSIDE GAMINE" });
            l.team(*at, 2, "0 D2Bast #8");
            for (p, n) in names1.iter().enumerate() {
                if k % 3 != 2 {
                    l.name(*at, 1, p as u8, n);
                }
            }
            for (p, n) in names2.iter().enumerate() {
                l.name(*at, 2, p as u8, n);
            }
            *at += 500;
        }
        for _ in 0..6 {
            l.sample(*at, false);
            *at += 500;
        }
    };
    round(&mut l, &mut at, ["TAEJONG", "KHO", "HEESING", "RONY", "OPENER"], ["AIEN", "WUTAN", "HOMFRUNRAIL", "GAMUNG", "FAITH"]);
    // next round: plates reordered, Faith out and perr in, one plate never read
    round(&mut l, &mut at, ["KILO", "TASJONG", "HEESUNG", "WONY", "OSENER"], ["PERR", "WUTIAN", "", "GAMIUNG", "AIIN"]);
    let info = l.finish(&r);
    assert_eq!(info.teams, [Some(0), Some(1)]);
    assert_eq!(info.stretches.len(), 2);
    let names = |s: &Stretch, side: usize| s.plates[side].iter().map(|p| p.map(|i| r.players[i].id.as_str()).unwrap_or("?")).collect::<Vec<_>>();
    assert_eq!(names(&info.stretches[0], 0), ["taejong", "kilo", "heesung", "irony", "opener"]);
    assert_eq!(names(&info.stretches[0], 1), ["a1ien", "wutian", "homerunball", "gamjung", "faith"]);
    assert_eq!(names(&info.stretches[1], 0), ["kilo", "taejong", "heesung", "irony", "opener"]);
    // Homerunball's plate was never read, but Faith left and perr came: not filled in by
    // elimination (two changes would be a guess)
    assert_eq!(names(&info.stretches[1], 1), ["perr", "wutian", "?", "gamjung", "a1ien"]);
    assert_eq!(info.players().len(), 11);
}

#[test]
fn one_unread_plate_is_the_one_left_over() {
    let r = roster();
    let mut l = LineupReader::new(&bars());
    let mut at = 0;
    for round in 0..2 {
        for _ in 0..6 {
            l.sample(at, true);
            l.team(at, 1, "RODE ZANSIDE GAMING");
            for (p, n) in ["TAEJONG", "KILO", "HEESUNG", "IRONY", "OPENER"].iter().enumerate() {
                if !(round == 1 && p == 2) {
                    l.name(at, 1, p as u8, n);
                }
            }
            at += 500;
        }
        for _ in 0..5 {
            l.sample(at, false);
            at += 500;
        }
    }
    let info = l.finish(&r);
    assert_eq!(info.stretches[1].player(1, 3), Some(2), "Heesung by elimination");
    assert!(info.teams[1].is_none(), "nothing read over team 2");
    assert!(info.stretches[1].plates[1].iter().all(Option::is_none));
}

#[test]
fn heroes_get_their_players() {
    let r = roster();
    let mut l = LineupReader::new(&bars());
    for k in 0..6 {
        l.sample(k * 500, true);
        l.team(k * 500, 1, "RODE ZANSIDE GAMING");
        l.name(k * 500, 1, 1, "KILO");
    }
    let info = l.finish(&r);
    let scan = HeroScan {
        events: vec![HeroEvent { at: 1000, team: 1, player: 2, from: None, to: 0 }, HeroEvent { at: 9000, team: 1, player: 2, from: Some(0), to: 1 }],
        ..HeroScan::default()
    };
    let a = attribute(&scan, &info);
    assert_eq!(a[0].player, Some(1));
    assert_eq!(a[1].player, None, "outside every stretch");
}

#[test]
fn hostile_input() {
    let mut l = LineupReader::new(&[]);
    l.team(0, 9, "x");
    l.name(0, 1, 200, &"Z".repeat(10_000));
    l.sample(0, true);
    let info = l.finish(&Roster::default());
    assert_eq!(info.teams, [None, None]);
    assert!(!l.settled(0));
}

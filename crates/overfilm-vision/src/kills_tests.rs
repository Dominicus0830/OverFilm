use super::*;

/// A 200×100 frame: two bars of 5 plates at y 10..20, team 1 at x 0..100, team 2 at 100..200.
/// Plates are gray 120 with a white "name" stripe; `dead` plates are black (with the name).
fn frame(dead: &[(u8, u8)]) -> Luma {
    let (w, h) = (200usize, 100usize);
    let mut px = vec![90u8; w * h];
    for team in 1..=2u8 {
        for k in 0..5u8 {
            let x0 = (team as usize - 1) * 100 + k as usize * 20;
            let v = if dead.contains(&(team, k + 1)) { 10 } else { 120 };
            for y in 10..20 {
                for x in x0 + 2..x0 + 18 {
                    px[y * w + x] = if y == 15 && x % 3 == 0 { 250 } else { v };
                }
            }
        }
    }
    Luma::from_gray(px, w, h).unwrap()
}

fn bars() -> Vec<PlayerBar> {
    let f = frame(&[]);
    vec![PlayerBar::learn(&f, 1, NRect::new(0.0, 0.1, 0.5, 0.1), 5).unwrap(), PlayerBar::learn(&f, 2, NRect::new(0.5, 0.1, 0.5, 0.1), 5).unwrap()]
}

#[test]
fn a_dark_plate_reads_dead() {
    let b = bars();
    assert_eq!(b[0].dead(&frame(&[(1, 3)])), [false, false, true, false, false]);
    assert_eq!(b[1].dead(&frame(&[(1, 3)])), [false; 5]);
    assert!(PlayerBar::learn(&frame(&[(1, 1), (1, 2), (1, 3), (1, 4), (1, 5)]), 1, NRect::new(0.0, 0.1, 0.5, 0.1), 5).is_err());
    assert!(PlayerBar::learn(&frame(&[]), 1, NRect::new(f32::NAN, 0.1, 0.5, 0.1), 5).is_err());
}

#[test]
fn deaths_need_two_samples_and_skip_a_missing_hud() {
    let mut t = DeathTracker::new(bars(), 2, 0);
    let all: Vec<(u8, u8)> = (1..=2).flat_map(|t| (1..=5).map(move |k| (t, k))).collect();
    let seq: Vec<Vec<(u8, u8)>> = vec![
        vec![],
        vec![(2, 4)],         // 2-4 goes down
        vec![(2, 4)],         // confirmed: death at 1
        vec![(2, 4), (1, 1)], // one-sample flicker of 1-1: nothing
        vec![(2, 4)],
        all.clone(), // the HUD is gone: skipped
        vec![],      // 2-4 back
        vec![],
        vec![(2, 4)], // and down again
        vec![(2, 4)],
    ];
    for (i, d) in seq.iter().enumerate() {
        t.push(i as i64, &frame(d), true);
    }
    t.push(10, &frame(&[(1, 2)]), false); // not live: ignored
    t.push(11, &frame(&[(1, 2)]), false);
    let d = t.into_deaths();
    assert_eq!(d, vec![Death { at: 1, team: 2, player: 4 }, Death { at: 8, team: 2, player: 4 }]);
    // with a respawn time the second one is a flicker
    let mut t = DeathTracker::new(bars(), 2, 10);
    for (i, d) in seq.iter().enumerate() {
        t.push(i as i64, &frame(d), true);
    }
    assert_eq!(t.into_deaths(), vec![Death { at: 1, team: 2, player: 4 }]);
}

#[test]
fn deaths_close_together_are_a_fight() {
    let d = |at, team| Death { at, team, player: 1 };
    let deaths = [d(0, 1), d(4_000, 2), d(9_000, 2), d(30_000, 1), d(60_000, 2), d(61_000, 2)];
    let f = fights(&deaths, 10_000, 2);
    assert_eq!(f, vec![Fight { start: 0, end: 9_000, dead: [1, 2] }, Fight { start: 60_000, end: 61_000, dead: [0, 2] }]);
    assert!(fights(&[], 10_000, 2).is_empty());
}

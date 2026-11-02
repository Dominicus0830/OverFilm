use super::*;
use crate::kills::PlayerBar;

#[derive(Clone, Copy, PartialEq)]
enum P {
    Ready,
    /// The charge number (thin digits).
    Charging,
    Dead,
}

const W: usize = 400;
const H: usize = 200;

/// Two bars of 5 plates, 40 px apart: team 1 at x 0..200, team 2 at 200..400. Each plate has an
/// icon box at y 60..95 (gray 130; a white disc when ready, thin white digits while charging) and
/// a name stripe at y 100..120. A dead plate is black but for its name.
fn frame(plates: &[P; 10]) -> Luma {
    let mut px = vec![70u8; W * H];
    for (i, p) in plates.iter().enumerate() {
        let x0 = i * 40;
        let bg = if *p == P::Dead { 10 } else { 130 };
        for y in 60..120 {
            for x in x0 + 2..x0 + 38 {
                let name = y >= 100 && y == 110 && x % 3 == 0;
                px[y * W + x] = if name { 250 } else { bg };
            }
        }
        let (cx, cy) = (x0 as f32 + 16.0, 77.0);
        for y in 62..94 {
            for x in x0 + 4..x0 + 30 {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                let v = match p {
                    P::Ready if dx * dx + dy * dy <= 100.0 => Some(if dx.abs() < 2.0 && dy.abs() < 5.0 { 60 } else { 245 }),
                    P::Charging if (70..86).contains(&y) && (x == x0 + 8 || x == x0 + 14 || x == x0 + 20 || (y == 78 && x < x0 + 24)) => Some(245),
                    _ => None,
                };
                if let Some(v) = v {
                    px[y * W + x] = v;
                }
            }
        }
    }
    Luma::from_gray(px, W, H).unwrap()
}

fn icon_rect(slot: usize) -> NRect {
    NRect::new((slot * 40 + 4) as f32 / W as f32, 62.0 / H as f32, 26.0 / W as f32, 32.0 / H as f32)
}

fn bars() -> Vec<PlayerBar> {
    use P::*;
    let alive = frame(&[Charging; 10]);
    let mut b1 = PlayerBar::learn(&alive, 1, NRect::new(0.0, 0.5, 0.5, 0.1), 5).unwrap();
    let mut b2 = PlayerBar::learn(&alive, 2, NRect::new(0.5, 0.5, 0.5, 0.1), 5).unwrap();
    // learnt on plate 3 of team 1 and plate 2 of team 2
    let f = frame(&[Charging, Charging, Ready, Charging, Charging, Charging, Ready, Charging, Charging, Charging]);
    b1.ult = Some(UltIcon::learn(&f, &b1, icon_rect(2), 3).unwrap());
    b2.ult = Some(UltIcon::learn(&f, &b2, icon_rect(6), 2).unwrap());
    vec![b1, b2]
}

#[test]
fn the_ready_icon_is_found_on_every_plate_and_digits_are_not() {
    use P::*;
    let b = bars();
    let f = frame(&[Ready, Charging, Charging, Dead, Ready, Charging, Charging, Ready, Charging, Ready]);
    assert_eq!(b[0].ult_ready(&f).unwrap(), [true, false, false, false, true]);
    assert_eq!(b[1].ult_ready(&f).unwrap(), [false, false, true, false, true]);
    // a charge number is no icon to learn
    let digits = frame(&[Charging; 10]);
    assert!(UltIcon::learn(&digits, &b[0], icon_rect(0), 1).is_err());
    assert!(UltIcon::learn(&digits, &b[0], NRect::new(f32::NAN, 0.0, 0.1, 0.1), 1).is_err());
    // a bar without an icon reads nothing
    let mut none = b[0].clone();
    none.ult = None;
    assert!(none.ult_ready(&f).is_none());
    // and the icon survives a save and load
    let mut set = crate::TemplateSet::default();
    set.insert_bar(b[0].clone());
    assert_eq!(crate::TemplateSet::from_json(&set.to_json()).unwrap().bars[0], b[0]);
}

#[test]
fn a_use_is_ready_then_gone_while_alive_and_a_death_is_not() {
    use P::*;
    let mut t = UltTracker::new(bars(), 2);
    let mut at = 0;
    let mut feed = |t: &mut UltTracker, p: [P; 10], n: usize| {
        for _ in 0..n {
            t.push(at, &frame(&p), true);
            at += 500;
        }
    };
    let c = Charging;
    feed(&mut t, [c; 10], 4);
    // team 1 player 1 and team 2 player 3 get their ultimates
    feed(&mut t, [Ready, c, c, c, c, c, c, Ready, c, c], 4);
    // a one-sample flicker is nothing
    feed(&mut t, [c, c, c, c, c, c, c, Ready, c, c], 1);
    feed(&mut t, [Ready, c, c, c, c, c, c, Ready, c, c], 3);
    // team 1 player 1 uses it (at 6000); team 2 player 3 dies holding it (at 7000)
    feed(&mut t, [c, c, c, c, c, c, c, Ready, c, c], 2);
    feed(&mut t, [c, c, c, c, c, c, c, Dead, c, c], 6);
    // the HUD goes (everyone dark): skipped
    feed(&mut t, [Dead; 10], 3);
    // team 2 player 3 respawns still holding it, then uses it (at 13500)
    feed(&mut t, [c, c, c, c, c, c, c, Ready, c, c], 4);
    feed(&mut t, [c; 10], 3);
    let u = t.into_uses();
    assert_eq!(u, [UltUse { at: 6_000, team: 1, player: 1 }, UltUse { at: 13_500, team: 2, player: 3 }]);
    assert!(UltTracker::new(Vec::new(), 2).is_empty());
}

#[test]
fn kills_go_to_the_latest_ultimate_of_the_other_team() {
    let d = |at, team, player| Death { at, team, player };
    let u = |at, team, player| UltUse { at, team, player };
    let uses = [u(1_000, 1, 2), u(3_000, 1, 4), u(4_000, 2, 1), u(20_000, 2, 5)];
    let deaths = [d(500, 2, 1), d(2_000, 2, 3), d(4_500, 2, 4), d(5_000, 1, 3), d(9_500, 2, 2), d(20_300, 2, 5)];
    let k = ult_kills(&uses, &deaths, 6_000);
    let got: Vec<(i64, i64)> = k.iter().map(|k| (k.death.at, k.by.at)).collect();
    // 500: before any ultimate; 9 500: 6.5 s after; 20 300: own team's ultimate
    assert_eq!(got, [(2_000, 1_000), (4_500, 3_000), (5_000, 4_000)]);
    // a use right before its own player's death is the plate going dark
    assert_eq!(without_deaths(&uses, &deaths, 1_500), [u(1_000, 1, 2), u(3_000, 1, 4), u(4_000, 2, 1)]);
    assert!(ult_kills(&[], &deaths, i64::MAX).is_empty());
    assert!(ult_kills(&[u(i64::MIN, 1, 1)], &[d(i64::MAX, 2, 1)], 10).is_empty());
}

#[test]
fn feed_kills_find_their_victim_and_killer() {
    let d = |at, team, player| Death { at, team, player };
    let u = |at, team, player| UltUse { at, team, player };
    let deaths = [d(10_800, 2, 3), d(11_000, 2, 4), d(30_000, 1, 1)];
    let uses = [u(9_000, 1, 5), u(10_500, 2, 1)];
    let k = feed_kills(&[10_500, 10_700, 20_000], &deaths, &uses, 6_000);
    // two feed rows, two deaths: each death once; the killer is the other team's ultimate
    assert_eq!(k[0], FeedKill { at: 10_500, death: Some(d(10_800, 2, 3)), by: Some(u(9_000, 1, 5)) });
    assert_eq!(k[1].death, Some(d(11_000, 2, 4)));
    // no death on the plates near it, and no ultimate within the window
    assert_eq!(k[2], FeedKill { at: 20_000, death: None, by: None });
    assert!(feed_kills(&[i64::MAX, i64::MIN], &deaths, &uses, i64::MAX).len() == 2);
}

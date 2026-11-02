use super::*;

const W: usize = 200;
const H: usize = 100;

fn bar() -> PlayerBar {
    PlayerBar { team: 1, rect: NRect::new(0.0, 0.5, 1.0, 0.1), players: 2, alive_dark: vec![0.0; 2], ult: None, portrait: None }
}

/// A frame with each plate's name strip lit (alive) and a two-tone portrait per plate.
fn frame(colors: [[u8; 3]; 2]) -> (Vec<u8>, Luma) {
    let mut px = vec![60u8; W * H * 4];
    let b = bar();
    for (k, c) in colors.iter().enumerate() {
        let r = portrait(&b, k as u8);
        let (x0, y0) = ((r.x * W as f32) as usize, (r.y * H as f32) as usize);
        let (x1, y1) = (((r.x + r.w) * W as f32) as usize, ((r.y + r.h) * H as f32) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * W + x) * 4;
                let half = x < (x0 + x1) / 2;
                let v = if half { *c } else { [20, 20, 20] };
                px[i..i + 3].copy_from_slice(&v);
            }
        }
    }
    for y in 50..60 {
        for x in 0..W {
            px[(y * W + x) * 4] = 200;
            px[(y * W + x) * 4 + 1] = 200;
            px[(y * W + x) * 4 + 2] = 200;
        }
    }
    let l = Luma::from_rgba(&px, W, H, W).unwrap();
    (px, l)
}

#[test]
fn prints_and_names() {
    let (a, _) = frame([[200, 40, 40], [40, 200, 40]]);
    let p0 = HeroPrint::read(&a, W, H, portrait(&bar(), 0)).unwrap();
    let p1 = HeroPrint::read(&a, W, H, portrait(&bar(), 1)).unwrap();
    assert!(p0.diff(&p0) < 0.01);
    assert!(p0.diff(&p1) > SAME);
    // a flat area is no portrait; a short buffer reads nothing
    assert!(HeroPrint::read(&vec![90u8; W * H * 4], W, H, portrait(&bar(), 0)).is_none());
    assert!(HeroPrint::read(&a[..10], W, H, portrait(&bar(), 0)).is_none());
    let mut n = HeroNames::default();
    n.set(p0.clone(), "트레이서");
    assert_eq!(n.lookup(&p0), Some("트레이서"));
    assert_eq!(n.lookup(&p1), None);
    n.set(p0.clone(), "겐지");
    assert_eq!(n.heroes.len(), 1, "renaming replaces");
    n.set(p0.clone(), " ");
    assert!(n.is_empty(), "an empty name forgets");
}

#[test]
fn swaps_need_confirming_and_breaks_start_fresh() {
    let red = [200, 40, 40];
    let green = [40, 200, 40];
    let blue = [40, 40, 220];
    let mut t = HeroTracker::new(vec![bar()], 1_000);
    let mut at = 0;
    let mut feed = |t: &mut HeroTracker, c: [[u8; 3]; 2], n: usize, live: bool| {
        let (px, l) = frame(c);
        for _ in 0..n {
            t.push(at, &px, W, H, &l, live);
            at += 500;
        }
    };
    feed(&mut t, [red, green], 10, true);
    // one odd sample is no swap
    feed(&mut t, [blue, green], 1, true);
    feed(&mut t, [red, green], 4, true);
    // a real swap on plate 1
    feed(&mut t, [blue, green], 10, true);
    // a break, then the same heroes again: no news; then plate 2 picks red
    feed(&mut t, [blue, green], 6, false);
    feed(&mut t, [blue, red], 10, true);
    let names = HeroNames::default();
    let s = t.finish(&names);
    let short: Vec<(i64, u8, Option<usize>, usize)> = s.events.iter().map(|e| (e.at, e.player, e.from, e.to)).collect();
    // heroes numbered by appearance: red 0, green 1, blue 2
    assert_eq!(short, vec![(0, 1, None, 0), (0, 2, None, 1), (7_500, 1, Some(0), 2), (15_500, 2, None, 0)]);
    assert!(s.events[2].from.is_some() && !s.events[2].picked());
    assert_eq!(s.label(2), "영웅 3");
}

#[test]
fn hostile_sizes_do_not_panic() {
    let mut t = HeroTracker::new(vec![bar()], 1_000);
    let l = Luma::from_gray(vec![0; 4], 2, 2).unwrap();
    t.push(0, &[], 0, 0, &l, true);
    t.push(0, &[1, 2, 3], usize::MAX, 3, &l, true);
    let s = t.finish(&HeroNames::default());
    assert!(s.events.is_empty());
    assert_eq!(HeroPrint(vec![1]).diff(&HeroPrint(vec![1, 2])), 255.0);
}

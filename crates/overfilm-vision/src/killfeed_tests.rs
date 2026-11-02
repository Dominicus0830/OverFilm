use super::*;

const W: usize = 320;
const H: usize = 180;
const ROW: usize = 22;

/// A feed row at `y` (its centre): a name box, a "portrait" with a bright irregular patch, an
/// ultimate icon at `icon_x` when given (white rim and disc, a dark symbol), an arrow, the victim.
fn row(px: &mut [u8], y: usize, icon_x: Option<usize>) {
    for yy in y - 8..y + 8 {
        for x in 150..200 {
            px[yy * W + x] = 110;
        }
        for x in 250..310 {
            px[yy * W + x] = 70;
        }
    }
    // portrait: a bright blob that is not round
    for yy in y - 6..y + 2 {
        for x in 202..214 + (yy % 3) {
            px[yy * W + x] = 230;
        }
    }
    if let Some(cx) = icon_x {
        for yy in y - 10..=y + 10 {
            for x in cx - 10..=cx + 10 {
                let (dx, dy) = (x as f32 - cx as f32, yy as f32 - y as f32);
                let d = (dx * dx + dy * dy).sqrt();
                if d <= 9.0 {
                    // the symbol: a dark bar across the middle, cutting the rim at the top
                    let symbol = dx.abs() < 1.5 && dy < 4.0;
                    px[yy * W + x] = if symbol { 40 } else { 240 };
                }
            }
        }
    }
    // arrow
    for yy in y - 3..y + 3 {
        px[yy * W + 240 + (yy % 3)] = 200;
    }
}

/// Rows top to bottom from y 40: `Some(x)` has an ultimate icon at x. A thin reticle circle
/// crosses the feed.
fn frame(rows: &[Option<usize>]) -> Luma {
    let mut px = vec![30u8; W * H];
    for (i, r) in rows.iter().enumerate() {
        row(&mut px, 40 + i * ROW, *r);
    }
    for i in 0..720 {
        let a = i as f32 * std::f32::consts::TAU / 720.0;
        let (x, y) = ((210.0 + 40.0 * a.cos()) as usize, (90.0 + 40.0 * a.sin()) as usize);
        px[y * W + x] = 235;
    }
    Luma::from_gray(px, W, H).unwrap()
}

fn feed_rect() -> NRect {
    NRect::new(140.0 / W as f32, 25.0 / H as f32, 175.0 / W as f32, 150.0 / H as f32)
}

fn learnt() -> KillFeed {
    let f = frame(&[None, Some(225), None]);
    KillFeed::learn(&f, feed_rect(), NRect::new(214.0 / W as f32, 51.0 / H as f32, 22.0 / W as f32, 22.0 / H as f32)).unwrap()
}

#[test]
fn ultimate_icons_are_found_and_portraits_and_reticles_are_not() {
    let feed = learnt();
    assert!((feed.radius * H as f32 - 9.0).abs() < 2.0, "{}", feed.radius * H as f32);
    let f = frame(&[Some(222), None, Some(228), None, Some(225)]);
    let ys: Vec<usize> = feed.icons(&f).iter().map(|i| (i.y * H as f32).round() as usize).collect();
    assert_eq!(ys.len(), 3, "{ys:?}");
    for (y, want) in ys.iter().zip([40, 84, 128]) {
        assert!(y.abs_diff(want) <= 2, "{ys:?}");
    }
    assert!(feed.icons(&frame(&[None, None, None])).is_empty());
    // no icon to learn from, or none inside the feed
    let plain = frame(&[None, None]);
    assert!(KillFeed::learn(&plain, feed_rect(), NRect::new(214.0 / W as f32, 51.0 / H as f32, 22.0 / W as f32, 22.0 / H as f32)).is_err());
    let f = frame(&[None, Some(225)]);
    let icon = NRect::new(214.0 / W as f32, 51.0 / H as f32, 22.0 / W as f32, 22.0 / H as f32);
    assert!(KillFeed::learn(&f, NRect::new(0.0, 0.0, 0.3, 0.3), icon).is_err());
    assert!(KillFeed::learn(&f, NRect::new(f32::NAN, 0.0, 0.3, 0.3), icon).is_err());
    // a damaged radius reads nothing
    let bad = KillFeed { rect: feed_rect(), radius: f32::NAN };
    assert!(bad.icons(&f).is_empty());
}

#[test]
fn each_new_icon_counts_once_as_rows_move() {
    let mut t = FeedTracker::new(learnt(), 2);
    let mut at = 0;
    let mut feed_live = |t: &mut FeedTracker, rows: &[Option<usize>], n: usize, live: bool| {
        for _ in 0..n {
            t.push(at, &frame(rows), live);
            at += 500;
        }
    };
    feed_live(&mut t, &[None, None], 3, true);
    // an ultimate kill appears (1500) and stays while the rows move up
    feed_live(&mut t, &[None, None, Some(222)], 3, true);
    feed_live(&mut t, &[None, Some(222)], 3, true);
    // a second one with another killer name (another x) (4500), a one-sample flash is nothing
    feed_live(&mut t, &[None, Some(222), Some(232)], 1, true);
    feed_live(&mut t, &[None, Some(222), Some(232)], 3, true);
    feed_live(&mut t, &[Some(222), Some(232), None, Some(212)], 1, true);
    // rows leave
    feed_live(&mut t, &[None], 4, true);
    // the HUD is gone: skipped
    feed_live(&mut t, &[Some(225)], 2, false);
    // the same killer again later is a new kill (10000)
    feed_live(&mut t, &[Some(222)], 3, true);
    assert_eq!(t.into_kills(), [1_500, 4_500, 10_000]);
}

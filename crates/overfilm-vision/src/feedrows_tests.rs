use super::*;

const W: usize = 420;
const H: usize = 200;
const ROW: usize = 24;
const BLUE: [u8; 3] = [60, 140, 205];
const RED: [u8; 3] = [200, 50, 62];

struct Canvas(Vec<u8>);

impl Canvas {
    fn new() -> Self {
        Canvas([18u8, 20, 24].repeat(W * H))
    }
    fn put(&mut self, x: usize, y: usize, c: [u8; 3]) {
        if x < W && y < H {
            let i = (y * W + x) * 3;
            self.0[i..i + 3].copy_from_slice(&c);
        }
    }
    fn fill(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, c: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, c);
            }
        }
    }
    /// A name box: the box colour with "letters" (white strokes made from the name's bytes).
    fn name(&mut self, x0: usize, y0: usize, name: &str, c: [u8; 3]) -> usize {
        let width = 10 + name.len() * 7;
        self.fill(x0, y0, x0 + width, y0 + ROW, c);
        for (i, b) in name.bytes().enumerate() {
            let lx = x0 + 5 + i * 7;
            for y in y0 + 8..y0 + 16 {
                for x in lx..lx + 5 {
                    // a 5×8 glyph from the letter's bits
                    let glyph = (b as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20;
                    let bit = (x - lx) + (y - y0 - 8) * 5;
                    if glyph >> bit & 1 == 1 || x == lx {
                        self.put(x, y, [245, 245, 245]);
                    }
                }
            }
        }
        x0 + width
    }
    /// A hero picture: blocks of grays from the hero's number.
    fn hero(&mut self, x0: usize, y0: usize, hero: u32) {
        let pw = (ROW as f32 * PORTRAIT) as usize;
        for y in y0..y0 + ROW {
            for x in x0..x0 + pw {
                let v = ((((x - x0) / 6) as u32 * 7 + ((y - y0) / 6) as u32 * 13 + hero * 31).wrapping_mul(2654435761u32) >> 24) as u8;
                self.put(x, y, [v, v / 2 + 40, 255 - v]);
            }
        }
    }
    /// An ultimate icon: a white disc with a dark mark, centred at `(cx, cy)`.
    fn ult(&mut self, cx: f32, cy: f32) {
        let r = ULT_RADIUS * ROW as f32;
        for y in 0..H {
            for x in 0..W {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                if d <= r {
                    let mark = (x as f32 + 0.5 - cx).abs() < 1.5 && d < r * 0.6;
                    self.put(x, y, if mark { [30, 30, 30] } else { [240, 240, 240] });
                }
            }
        }
    }
    /// A whole row at `y0`: killer (name, side, hero) → victim.
    fn row(&mut self, y0: usize, killer: Option<(&str, [u8; 3], u32)>, victim: (&str, [u8; 3], u32), ult: bool) {
        let pw = (ROW as f32 * PORTRAIT) as usize;
        let mut x = 20;
        if let Some((n, c, h)) = killer {
            x = self.name(x, y0, n, c);
            self.hero(x, y0, h);
            x += pw;
        }
        x += 6;
        if ult {
            self.ult(x as f32 + 12.0, y0 as f32 + ROW as f32 / 2.0);
        }
        x += 30;
        self.hero(x, y0, victim.2);
        self.name(x + pw, y0, victim.0, victim.1);
    }
    fn img(self) -> Rgb {
        Rgb::from_rgb(self.0, W, H).unwrap()
    }
}

#[test]
fn reads_who_killed_whom() {
    let mut c = Canvas::new();
    c.row(10, Some(("NECROS", BLUE, 1)), ("FRANKY", RED, 2), false);
    c.row(40, Some(("FRANKY", RED, 2)), ("OMEGALUL", BLUE, 3), false);
    c.row(70, Some(("NECROS", BLUE, 1)), ("BASIL", RED, 4), true);
    c.row(100, None, ("LEEVI", RED, 5), false);
    let rows = read_rows(&c.img(), None);
    assert_eq!(rows.len(), 4, "{rows:?}");
    let k = |i: usize| rows[i].killer.as_ref().map(|k| k.side);
    assert_eq!((k(0), rows[0].victim.side), (Some(Side::Blue), Side::Red));
    assert_eq!((k(1), rows[1].victim.side), (Some(Side::Red), Side::Blue));
    assert_eq!(k(3), None);
    assert_eq!(rows.iter().map(|r| r.ult).collect::<Vec<_>>(), [false, false, true, false]);
    // the same name and picture read on two rows are alike, different ones are not
    let necros = &rows[0].killer.as_ref().unwrap().name;
    assert!(necros.similarity(&rows[2].killer.as_ref().unwrap().name) > 0.95);
    assert!(necros.similarity(&rows[0].victim.name) < SAME_NAME);
    let franky_hero = rows[0].victim.hero.as_ref().unwrap();
    assert!(franky_hero.similarity(rows[1].killer.as_ref().unwrap().hero.as_ref().unwrap()) > SAME_HERO);

    let mut r = FeedReader::new(FeedNames::default(), 1, None);
    r.push_rows(0, &rows);
    let scan = r.finish();
    let lines: Vec<String> = scan.events.iter().map(|e| scan.line(e)).collect();
    assert_eq!(
        lines,
        ["선수 1 (영웅 1) → 선수 2 (영웅 2)", "선수 2 (영웅 2) → 선수 3 (영웅 3)", "선수 1 (영웅 1) → 선수 4 (영웅 4)", "선수 5 (영웅 5) 처치됨"]
    );
    assert_eq!(scan.ult_times(), [0]);
}

#[test]
fn rows_run_together_are_cut_apart() {
    // rows 2 px apart with a blue streak (a reticle) joining them
    let mut c = Canvas::new();
    c.row(10, Some(("AAAA", BLUE, 1)), ("BBBBBB", RED, 2), false);
    c.row(36, Some(("CCCCC", BLUE, 3)), ("DD", RED, 4), false);
    c.row(62, Some(("EEE", RED, 5)), ("FFFFFFF", BLUE, 6), false);
    c.fill(200, 34, 240, 36, BLUE);
    c.fill(200, 60, 240, 62, BLUE);
    // another row shows how high a row is
    c.row(120, Some(("GG", RED, 7)), ("HHHHH", BLUE, 8), false);
    let rows = read_rows(&c.img(), None);
    assert_eq!(rows.len(), 4, "{rows:?}");
    // with the row height learnt the same
    let rh = ROW as f32 / H as f32;
    assert_eq!(read_rows(&Canvas::new().img(), Some(rh)).len(), 0);
    let mut c = Canvas::new();
    c.row(10, Some(("AAAA", BLUE, 1)), ("BBBBBB", RED, 2), false);
    c.row(36, Some(("CCCCC", BLUE, 3)), ("DD", RED, 4), false);
    c.fill(200, 34, 240, 36, BLUE);
    let img = c.img();
    assert_eq!(read_rows(&img, Some(rh)).len(), 2);
    let mut c = Canvas::new();
    c.row(10, Some(("AAAA", BLUE, 1)), ("BBBBBB", RED, 2), false);
    c.row(50, Some(("CCCCC", BLUE, 3)), ("DD", RED, 4), false);
    let learnt = row_height(&c.img()).unwrap();
    assert!((learnt * H as f32 - ROW as f32).abs() <= 3.0, "{learnt}");
}

#[test]
fn a_row_on_screen_is_one_kill() {
    let frame = |rows: &[(usize, &str, &str)]| {
        let mut c = Canvas::new();
        for (y, a, b) in rows {
            c.row(*y, Some((a, BLUE, 1)), (b, RED, 2), false);
        }
        c.img()
    };
    let mut r = FeedReader::new(FeedNames::default(), 2, None);
    // a flash for one sample is nothing; a row for two is a kill, at its first sample
    r.push(0, &frame(&[(10, "XX", "YYYY")]), true);
    r.push(500, &frame(&[]), true);
    r.push(1000, &frame(&[(10, "AAA", "BBBBB")]), true);
    r.push(1500, &frame(&[(10, "AAA", "BBBBB")]), true);
    // it moves down as a new row comes in on top: still the same kill
    r.push(2000, &frame(&[(10, "CCCCCC", "DD"), (40, "AAA", "BBBBB")]), true);
    r.push(2500, &frame(&[(10, "CCCCCC", "DD"), (40, "AAA", "BBBBB")]), true);
    // not live (a replay): ignored
    r.push(3000, &frame(&[(10, "EEEEEEE", "FFF")]), false);
    r.push(3500, &frame(&[(10, "EEEEEEE", "FFF")]), false);
    // gone, then the same pair again much later: a second kill
    r.push(4000, &frame(&[]), true);
    r.push(9000, &frame(&[(10, "AAA", "BBBBB")]), true);
    r.push(9500, &frame(&[(10, "AAA", "BBBBB")]), true);
    let scan = r.finish();
    let got: Vec<(i64, String)> = scan.events.iter().map(|e| (e.at, scan.line(e))).collect();
    assert_eq!(
        got,
        [
            (1000, "선수 1 (영웅 1) → 선수 2 (영웅 2)".to_string()),
            (2000, "선수 3 (영웅 1) → 선수 4 (영웅 2)".to_string()),
            (9000, "선수 1 (영웅 1) → 선수 2 (영웅 2)".to_string())
        ]
    );
}

#[test]
fn names_given_once_are_known_again() {
    let mut c = Canvas::new();
    c.row(10, Some(("NECROS", BLUE, 1)), ("FRANKY", RED, 2), false);
    let rows = read_rows(&c.img(), None);
    let mut names = FeedNames::default();
    let k = rows[0].killer.clone().unwrap();
    names.teach(false, "NECROS", &k.name);
    names.teach(true, "위도우메이커", k.hero.as_ref().unwrap());
    // naming again replaces, an empty name forgets
    names.teach(false, " Necros ", &k.name);
    assert_eq!(names.players.len(), 1);
    names.teach(false, "x", &rows[0].victim.name);
    names.teach(false, "", &rows[0].victim.name);
    assert_eq!(names.players.len(), 1);
    let json = serde_json::to_string(&names).unwrap();
    let names: FeedNames = serde_json::from_str(&json).unwrap();
    let mut r = FeedReader::new(names, 1, None);
    r.push_rows(0, &rows);
    let scan = r.finish();
    assert_eq!(scan.line(&scan.events[0]), "Necros (위도우메이커) → 선수 2 (영웅 2)");
}

#[test]
fn hostile_input_reads_nothing() {
    assert!(read_rows(&Rgb::from_rgb(vec![0; 3 * 4], 2, 2).unwrap(), None).is_empty());
    assert!(read_rows(&Canvas::new().img(), Some(f32::NAN)).is_empty());
    assert!(Rgb::from_rgb(vec![0; 5], 2, 2).is_none());
    assert!(Rgb::crop_rgba(&[0; 16], 2, 2, crate::NRect::new(0.0, 0.0, 1.0, 1.0), 64).is_some());
    assert!(Rgb::crop_rgba(&[0; 15], 2, 2, crate::NRect::new(0.0, 0.0, 1.0, 1.0), 64).is_none());
    assert!(Rgb::crop_rgba(&[0; 16], 2, 2, crate::NRect::new(2.0, 0.0, 1.0, 1.0), 64).is_none());
    assert!(Rgb::crop_rgba(&[0; 16], usize::MAX, 2, crate::NRect::new(0.0, 0.0, 1.0, 1.0), 64).is_none());
    let s = Sig { w: 4, h: 2, aspect: 1.0, px: vec![1, 2, 3] };
    assert_eq!(s.similarity(&s), 0.0);
    let flat = Sig { w: 2, h: 2, aspect: f32::NAN, px: vec![9; 4] };
    assert_eq!(flat.similarity(&flat), 1.0);
    // a full frame of box colour (a blue screen) is no row
    let blue = Rgb::from_rgb(BLUE.repeat(W * H), W, H).unwrap();
    assert!(read_rows(&blue, None).is_empty());
    let mut r = FeedReader::new(FeedNames::default(), 0, Some(-1.0));
    r.push(i64::MIN, &blue, true);
    r.push(i64::MAX, &blue, true);
    assert!(r.finish().events.is_empty());
}

#[test]
fn crop_keeps_colour_and_shrinks() {
    // 4×2 RGBA: red left half, blue right half; the right half cropped and halved
    let mut rgba = Vec::new();
    for _ in 0..2 {
        for x in 0..4 {
            rgba.extend(if x < 2 { [200, 0, 0, 255] } else { [0, 0, 200, 255] });
        }
    }
    let c = Rgb::crop_rgba(&rgba, 4, 2, crate::NRect::new(0.5, 0.0, 0.5, 1.0), 8).unwrap();
    assert_eq!((c.width, c.height, c.at(0, 0), c.at(9, 9)), (2, 2, [0, 0, 200], [0, 0, 0]));
    let c = Rgb::crop_rgba(&rgba, 4, 2, crate::NRect::new(0.0, 0.0, 1.0, 1.0), 2).unwrap_or_else(|| Rgb::from_rgb(vec![0; 3], 1, 1).unwrap());
    assert!(c.width <= 8);
    assert_eq!(c.luma().px.len(), c.width * c.height);
}

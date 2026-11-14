//! Who killed whom, read off the game's kill feed.
//!
//! A kill feed row is two coloured boxes with an arrow between them:
//!
//! ```text
//! [ KILLER NAME ][killer hero][assists…] (ultimate icon) >  [victim hero][ VICTIM NAME ]
//! ```
//!
//! The box colour is the team (blue: the viewer's team, red: the other one). A row with one box
//! is a death with nobody to credit (a fall, a self-kill). Nothing is recognised by built-in
//! artwork or text: the white name text and the hero picture beside each box are kept as small
//! gray grids ([`Sig`]), the same name or hero seen again is the same **cluster** ("선수 3",
//! "영웅 2"), and the user names a cluster once ([`FeedNames`], kept in their template file) for
//! every later scan. A row that stays on screen for a few samples is one kill ([`FeedReader`]).

use serde::{Deserialize, Serialize};

use crate::killfeed::ring_score;
use crate::luma::Rgb;

/// Name text grid.
const NAME_W: usize = 48;
const NAME_H: usize = 10;
/// Hero picture grid.
const HERO_W: usize = 24;
const HERO_H: usize = 16;
/// A hero picture is this many row heights wide.
const PORTRAIT: f32 = 1.5;
/// The ultimate icon's radius in row heights.
const ULT_RADIUS: f32 = 0.45;
/// Lowest similarity of two readings of the same name.
pub const SAME_NAME: f32 = 0.82;
/// Lowest similarity of two readings of the same hero picture.
pub const SAME_HERO: f32 = 0.7;
/// Most rows read on one frame (a feed holds about six).
const MAX_ROWS: usize = 12;
/// Most players or heroes told apart in one scan.
const MAX_CLUSTERS: usize = 512;
/// Most names kept in a template file, of each kind.
const MAX_NAMES: usize = 1024;
/// A row not seen for this long (ms) has left the feed.
const GONE_MS: i64 = 2_500;
/// Victims' box ends kept to find the feed's right edge.
const RIGHTS: usize = 64;
/// How far left of the feed's right edge a row may end, in row heights.
const RIGHT_SLACK: f32 = 2.0;
/// A row stays in the feed about this long (ms): the same row found again within it is the same
/// kill read again.
const STAYS_MS: i64 = 8_000;

/// A box's team colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The viewer's team.
    Blue,
    Red,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Blue => "blue",
            Side::Red => "red",
        }
    }

    /// In Korean, as the feed means it (blue: our team).
    pub fn ko(self) -> &'static str {
        match self {
            Side::Blue => "아군",
            Side::Red => "적",
        }
    }
}

fn side_of([r, g, b]: [u8; 3]) -> Option<Side> {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    // strong colour only: a pale sky or a sandy wall behind the feed is not a box
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    if hi < 100 || (hi - lo) * 100 < hi * 45 {
        return None;
    }
    if b - r >= 45 && b + 10 >= g {
        Some(Side::Blue)
    } else if r == hi && r - g >= 70 && r - b >= 30 && g <= b + 12 {
        // red, not orange or a brown wall (both have more green than blue)
        Some(Side::Red)
    } else {
        None
    }
}

/// A small picture to know again: a gray grid and the shape it was taken from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sig {
    pub w: usize,
    pub h: usize,
    /// Width over height of what was sampled (names of different lengths never match).
    #[serde(default)]
    pub aspect: f32,
    pub px: Vec<u8>,
}

impl Sig {
    /// `value` averaged over a `w`×`h` grid of the box `[x0, x1) × [y0, y1)` (pixels).
    fn grab(img: &Rgb, (x0, y0, x1, y1): (f32, f32, f32, f32), w: usize, h: usize, value: impl Fn([u8; 3]) -> f32) -> Option<Sig> {
        let (bw, bh) = (x1 - x0, y1 - y0);
        if !(bw >= 2.0 && bh >= 2.0 && bw.is_finite() && bh.is_finite()) {
            return None;
        }
        let mut px = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                let (cx0, cx1) = (x0 + bw * i as f32 / w as f32, x0 + bw * (i + 1) as f32 / w as f32);
                let (cy0, cy1) = (y0 + bh * j as f32 / h as f32, y0 + bh * (j + 1) as f32 / h as f32);
                let (mut acc, mut n) = (0.0f32, 0u32);
                let mut y = cy0.floor().max(0.0) as usize;
                while (y as f32) < cy1.ceil() && y < img.height {
                    let mut x = cx0.floor().max(0.0) as usize;
                    while (x as f32) < cx1.ceil() && x < img.width {
                        acc += value(img.at(x, y));
                        n += 1;
                        x += 1;
                    }
                    y += 1;
                }
                px.push(if n == 0 { 0 } else { (acc / n as f32).round().clamp(0.0, 255.0) as u8 });
            }
        }
        Some(Sig { w, h, aspect: bw / bh, px })
    }

    /// How alike two readings are: the best normalised correlation of the grids moved by up to
    /// a cell or two against each other (a picture is rarely found to the pixel), 0 for different
    /// grid sizes or shapes more than 12 % apart.
    pub fn similarity(&self, o: &Sig) -> f32 {
        if self.w != o.w || self.h != o.h || self.px.len() != o.px.len() || self.px.len() != self.w * self.h || self.px.is_empty() {
            return 0.0;
        }
        if self.aspect > 0.0 && o.aspect > 0.0 {
            let r = self.aspect / o.aspect;
            if !(0.88..=1.0 / 0.88).contains(&r) {
                return 0.0;
            }
        }
        self.likeness(o)
    }

    /// [`Sig::similarity`] whatever the shapes (for a reading known to be near, blurred wider
    /// or narrower).
    pub fn likeness(&self, o: &Sig) -> f32 {
        if self.w != o.w || self.h != o.h || self.px.len() != o.px.len() || self.px.len() != self.w * self.h || self.px.is_empty() {
            return 0.0;
        }
        let (sx, sy) = ((self.w / 8).clamp(1, 3) as i32, (self.h / 8).clamp(1, 2) as i32);
        let mut best = f32::MIN;
        for dy in -sy..=sy {
            for dx in -sx..=sx {
                best = best.max(self.ncc(o, dx, dy));
            }
        }
        best.max(0.0)
    }

    /// Correlation where `o` is moved by `(dx, dy)` cells, over the cells both cover.
    fn ncc(&self, o: &Sig, dx: i32, dy: i32) -> f32 {
        let (w, h) = (self.w as i32, self.h as i32);
        let mut pairs: Vec<(f32, f32)> = Vec::with_capacity(self.px.len());
        for y in 0..h {
            for x in 0..w {
                let (ox, oy) = (x + dx, y + dy);
                if ox < 0 || oy < 0 || ox >= w || oy >= h {
                    continue;
                }
                if let (Some(&a), Some(&b)) = (self.px.get((y * w + x) as usize), o.px.get((oy * w + ox) as usize)) {
                    pairs.push((a as f32, b as f32));
                }
            }
        }
        let n = pairs.len() as f32;
        if pairs.len() * 2 < self.px.len() {
            return 0.0;
        }
        let ma = pairs.iter().map(|p| p.0).sum::<f32>() / n;
        let mb = pairs.iter().map(|p| p.1).sum::<f32>() / n;
        let (mut ab, mut aa, mut bb) = (0.0f32, 0.0f32, 0.0f32);
        for (a, b) in pairs {
            let (a, b) = (a - ma, b - mb);
            ab += a * b;
            aa += a * a;
            bb += b * b;
        }
        if aa < n || bb < n {
            // flat: alike only when the levels are
            return if aa < n && bb < n && (ma - mb).abs() < 12.0 { 1.0 } else { 0.0 };
        }
        (ab / (aa.sqrt() * bb.sqrt())).clamp(-1.0, 1.0)
    }
}

/// One side of a row: its team, the name text and the hero picture beside the box.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedSide {
    pub side: Side,
    pub name: Sig,
    pub hero: Option<Sig>,
    /// The box's left and right edges, in row heights from the crop's left (a row keeps them while it
    /// is on screen; its name text, blurred by motion or the stream, does not quite).
    pub x0: f32,
    pub x1: f32,
    /// How sharp the name text was read (mean step between neighbouring pixels).
    pub clear: f32,
}

/// One kill feed row on one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedRow {
    /// The row's centre, as a share of the crop's height.
    pub y: f32,
    pub killer: Option<FeedSide>,
    pub victim: FeedSide,
    pub ult: bool,
}

/// The rows of the kill feed on `img` (a crop around the feed), top to bottom. `row`: the height
/// of one row as a share of the crop's height when it is known ([`row_height`] on a frame of the
/// same video); otherwise it is the typical height of the rows found.
pub fn read_rows(img: &Rgb, row: Option<f32>) -> Vec<FeedRow> {
    let (w, h) = (img.width, img.height);
    if w < 16 || h < 6 {
        return Vec::new();
    }
    let side_at = |x: usize, y: usize| side_of(img.at(x, y));
    let counts = profile(img);
    let row = row.filter(|r| r.is_finite() && *r > 0.0 && *r <= 1.0).map(|r| r * h as f32);
    let bands = bands(&counts, w, row);
    let rh = row.or_else(|| typical(&bands)).unwrap_or(0.0);
    let lum = img.luma();
    let aspect = w as f32 / h as f32;
    let mut rows = Vec::new();
    for (y0, y1) in bands {
        if rows.len() >= MAX_ROWS {
            break;
        }
        // cut by the rectangle's edge: half a row, or the game behind the feed
        if y0 == 0 || y1 >= h {
            continue;
        }
        if let Some(r) = read_row(img, &lum, aspect, (y0, y1, rh), &side_at) {
            rows.push(r);
        }
    }
    rows
}

/// The typical height of a row on `img`, as a share of its height (to keep with the feed's
/// rectangle); `None` when no row is found.
pub fn row_height(img: &Rgb) -> Option<f32> {
    let b = raw_bands(&profile(img), img.width);
    typical(&b).map(|t| t / img.height.max(1) as f32)
}

/// Per pixel row, how much of it is box: box colour or white text. A row's name text takes the
/// colour out of the middle of the box, so colour alone dips there as deep as between two rows;
/// with the text counted, only the gap between rows is low.
fn profile(img: &Rgb) -> Vec<usize> {
    (0..img.height)
        .map(|y| {
            (0..img.width)
                .filter(|&x| {
                    let p = img.at(x, y);
                    side_of(p).is_some() || white(p) >= 150.0
                })
                .count()
        })
        .collect()
}

/// Runs of pixel rows with enough box (a one-pixel gap is a line of text, not a new row): over a
/// twenty-fifth of the width, and somewhere over 30 % of the fullest rows (a glow or a reticle
/// between two rows is less; a row with no killer is about that).
fn raw_bands(counts: &[usize], w: usize) -> Vec<(usize, usize)> {
    let h = counts.len();
    let floor = (w / 25).max(4);
    let mut full: Vec<usize> = counts.iter().copied().filter(|&c| c >= floor).collect();
    full.sort_unstable();
    let p90 = full.get(full.len() * 9 / 10).copied().unwrap_or(0);
    let min = floor.max(p90 * 3 / 10);
    // a band reaches as far as half that (a short row sliding in is narrower than the rest)
    let low = floor.max(min / 2);
    let at = |y: usize| counts.get(y).copied().unwrap_or(0);
    let mut out = Vec::new();
    let mut y = 0;
    while y < h {
        if at(y) < low {
            y += 1;
            continue;
        }
        let y0 = y;
        let mut y1 = y + 1;
        while y1 < h && (at(y1) >= low || (y1 + 1 < h && at(y1 + 1) >= low)) {
            y1 += 1;
        }
        if y1 - y0 >= 6 && (y0..y1).any(|y| at(y) >= min) {
            out.push((y0, y1));
        }
        y = y1 + 1;
    }
    out
}

/// The (lower) median band height: rows run together are taller, never shorter.
fn typical(bands: &[(usize, usize)]) -> Option<f32> {
    let mut hs: Vec<usize> = bands.iter().map(|b| b.1 - b.0).collect();
    hs.sort_unstable();
    hs.get(hs.len().saturating_sub(1) / 2).map(|&v| v as f32)
}

/// The rows: bands, those that run into each other (a reticle or an effect between two rows
/// keeps the colour count up) cut at the emptiest pixel row near each expected boundary.
fn bands(counts: &[usize], w: usize, row: Option<f32>) -> Vec<(usize, usize)> {
    let raw = raw_bands(counts, w);
    let Some(rh) = row.or_else(|| typical(&raw)).filter(|r| *r >= 6.0) else { return raw };
    let at = |y: usize| counts.get(y).copied().unwrap_or(0);
    let mut out = Vec::new();
    for (y0, y1) in raw {
        let len = (y1 - y0) as f32;
        if len < rh * 0.55 {
            continue;
        }
        let k = ((len / (rh * 1.1)).round() as usize).clamp(1, MAX_ROWS);
        if k == 1 {
            out.push((y0, y1));
            continue;
        }
        let mut cuts = vec![y0];
        for i in 1..k {
            let target = y0 as f32 + len * i as f32 / k as f32;
            let (lo, hi) = ((target - rh / 3.0).max(y0 as f32 + 1.0) as usize, ((target + rh / 3.0) as usize).min(y1.saturating_sub(1)));
            let cut = (lo..=hi.max(lo)).min_by_key(|&y| at(y)).unwrap_or(target as usize);
            cuts.push(cut);
        }
        cuts.push(y1);
        for pair in cuts.windows(2) {
            let &[a, b] = pair else { continue };
            // trim the gap's pixel rows off each part
            let mut v: Vec<usize> = (a..b).map(at).collect();
            v.sort_unstable();
            let half = v.get(v.len() / 2).copied().unwrap_or(0) / 2;
            let (mut a, mut b) = (a, b);
            while a < b && at(a) < half {
                a += 1;
            }
            while b > a && at(b - 1) < half {
                b -= 1;
            }
            if (b - a) as f32 >= rh * 0.6 {
                out.push((a, b));
            }
        }
    }
    out
}

/// Row `y0..y1` of a feed whose rows are `rh` pixels high (a row sliding in is shorter for a
/// moment, its text is not).
fn read_row(img: &Rgb, lum: &crate::Luma, aspect: f32, (y0, y1, rh): (usize, usize, f32), side_at: &impl Fn(usize, usize) -> Option<Side>) -> Option<FeedRow> {
    let (w, h) = (img.width, img.height);
    let bh = y1 - y0;
    let bhf = bh as f32;
    // the box's colour above and below the name text
    let probe: Vec<usize> = [0.12f32, 0.2, 0.8, 0.88].iter().map(|f| (y0 + (bhf * f) as usize).min(y1 - 1)).collect();
    let col = |x: usize| -> Option<Side> {
        let (mut b, mut r) = (0, 0);
        for &y in &probe {
            match side_at(x, y) {
                Some(Side::Blue) => b += 1,
                Some(Side::Red) => r += 1,
                None => {}
            }
        }
        if b >= 2 && b > r {
            Some(Side::Blue)
        } else if r >= 2 && r > b {
            Some(Side::Red)
        } else {
            None
        }
    };
    // runs of one colour across, allowing short gaps
    let gap = (bh / 4).max(2);
    let mut runs: Vec<(Side, usize, usize)> = Vec::new();
    let mut cur: Option<(Side, usize, usize)> = None;
    for x in 0..w {
        let Some(s) = col(x) else { continue };
        cur = match cur {
            Some((cs, a, last)) if cs == s && x - last <= gap => Some((cs, a, x)),
            other => {
                if let Some(c) = other {
                    runs.push(c);
                }
                Some((s, x, x))
            }
        };
    }
    if let Some(c) = cur {
        runs.push(c);
    }
    // a short name ("KILO") with its picture beside it in the box leaves a short run
    runs.retain(|r| (r.2 + 1 - r.1) as f32 >= bhf * 0.6);
    let side = |(s, a, b): (Side, usize, usize), killer: bool| -> Option<FeedSide> {
        let (name, clear) = name_sig(img, (a, b + 1), (y0, y1), rh.max(bhf), !killer)?;
        let (px0, px1) = if killer { ((b + 1) as f32, (b + 1) as f32 + PORTRAIT * bhf) } else { (a as f32 - PORTRAIT * bhf, a as f32) };
        let hero = (px0 >= 0.0 && px1 <= w as f32).then(|| {
            let inset = (px1 - px0) * 0.1;
            Sig::grab(img, (px0 + inset, y0 as f32 + bhf * 0.12, px1 - inset, y1 as f32 - bhf * 0.12), HERO_W, HERO_H, gray)
        });
        Some(FeedSide { side: s, name, hero: hero.flatten(), x0: a as f32 / rh.max(1.0), x1: (b + 1) as f32 / rh.max(1.0), clear })
    };
    let last = *runs.last()?;
    let victim = side(last, false)?;
    let killer = if runs.len() >= 2 { runs.first().and_then(|r| side(*r, true)) } else { None };
    // the ultimate icon: a white ring between the killer's pictures and the victim's
    let v0 = last.1 as f32 - PORTRAIT * bhf;
    let from = runs.first().filter(|_| killer.is_some()).map_or(v0 - 4.0 * bhf, |k| (k.2 + 1) as f32 + PORTRAIT * bhf);
    let r = ULT_RADIUS * bhf;
    let cy = (y0 + y1) as f32 / 2.0;
    let mut ult = false;
    let mut x = from.max(r);
    'find: while x <= v0 - r * 0.5 {
        for dy in [0.0f32, -1.0, 1.0] {
            if ring_score(lum, x / w as f32, (cy + dy) / h as f32, r / h as f32, aspect) > 0.0 {
                ult = true;
                break 'find;
            }
        }
        x += 1.0;
    }
    Some(FeedRow { y: cy / h as f32, killer, victim, ult })
}

fn gray([r, g, b]: [u8; 3]) -> f32 {
    0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
}

/// White text: every channel bright.
fn white([r, g, b]: [u8; 3]) -> f32 {
    r.min(g).min(b) as f32
}

/// The name text inside the box `[x0, x1) × [y0, y1)`: the columns with white pixels, the group
/// at the box's outer end when there are several (an icon in the box is set apart from the name
/// by a wider gap than letters are). `outer_right`: the victim's box, whose name is on its right.
fn name_sig(img: &Rgb, (x0, x1): (usize, usize), (y0, y1): (usize, usize), rh: f32, outer_right: bool) -> Option<(Sig, f32)> {
    let m = 2usize;
    let bh = (y1 - y0) as f32;
    let (ya, yb) = (y0 + (bh * 0.15) as usize, y1.saturating_sub((bh * 0.15) as usize));
    // white text on the box colour: half way between the box (median) and the brightest pixel,
    // so dimmer or blurrier text is cut the same
    let mut v: Vec<f32> = (ya..yb).flat_map(|y| (x0 + m..x1.saturating_sub(m)).map(move |x| (x, y))).map(|(x, y)| white(img.at(x, y))).collect();
    v.sort_by(f32::total_cmp);
    let (bg, top) = (v.get(v.len() / 2).copied().unwrap_or(0.0), v.last().copied().unwrap_or(0.0));
    if top < 120.0 || top - bg < 60.0 {
        return None;
    }
    let text = bg + (top - bg) * 0.5;
    let lit: Vec<usize> = (x0 + m..x1.saturating_sub(m)).filter(|&x| (ya..yb).any(|y| white(img.at(x, y)) >= text)).collect();
    // groups of lit columns
    let gap = ((rh * 0.15) as usize).max(2);
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for x in lit {
        match groups.last_mut() {
            Some(g) if x - g.1 <= gap => g.1 = x + 1,
            _ => groups.push((x, x + 1)),
        }
    }
    let (tx0, tx1) = if outer_right { *groups.last()? } else { *groups.first()? };
    // a name, not a lone digit (the broadcast's kill count above the feed is boxes too)
    if tx1 < tx0 + 3 || ((tx1 - tx0) as f32) < rh * 0.4 {
        return None;
    }
    // the text is centred in the box at one size: the band's middle, not the bright pixels' own
    // height (one row more or less of a blurry letter would change the shape)
    let cy = (y0 + y1) as f32 / 2.0;
    let half = (rh * 0.3).min(bh / 2.0);
    let mut s = Sig::grab(img, (tx0 as f32, cy - half, tx1 as f32, cy + half), NAME_W, NAME_H, white)?;
    s.aspect = (tx1 - tx0) as f32 / rh;
    let (mut step, mut n) = (0.0f32, 0u32);
    for y in ya..yb {
        for x in tx0..tx1.saturating_sub(1) {
            step += (white(img.at(x + 1, y)) - white(img.at(x, y))).abs();
            n += 1;
        }
    }
    Some((s, if n == 0 { 0.0 } else { step / n as f32 }))
}

/// A name the user gave a player's or a hero's picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Labelled {
    pub name: String,
    pub sig: Sig,
}

/// Names given to players and heroes seen in the kill feed (kept in the template file).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedNames {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<Labelled>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heroes: Vec<Labelled>,
}

impl FeedNames {
    pub fn is_empty(&self) -> bool {
        self.players.is_empty() && self.heroes.is_empty()
    }

    fn list(&self, hero: bool) -> &[Labelled] {
        if hero { &self.heroes } else { &self.players }
    }

    /// The name given to what looks like `sig`.
    pub fn lookup(&self, hero: bool, sig: &Sig) -> Option<String> {
        let thr = if hero { SAME_HERO } else { SAME_NAME };
        self.list(hero).iter().map(|l| (l.sig.similarity(sig), l)).filter(|(s, _)| *s >= thr).max_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, l)| l.name.clone())
    }

    /// Give `sig` the name `name` (replacing a name given to the same picture before; an empty
    /// name forgets it).
    pub fn teach(&mut self, hero: bool, name: &str, sig: &Sig) {
        let thr = if hero { SAME_HERO } else { SAME_NAME };
        let list = if hero { &mut self.heroes } else { &mut self.players };
        list.retain(|l| l.sig.similarity(sig) < thr);
        let name = name.trim();
        if !name.is_empty() && list.len() < MAX_NAMES {
            list.push(Labelled { name: name.chars().take(64).collect(), sig: sig.clone() });
        }
    }
}

/// Readings of one player's name or one hero's picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub sig: Sig,
    /// The name the user gave it, if any.
    pub label: Option<String>,
    /// The label was not given by the user but taken from the name plates
    /// ([`crate::link::fill_names`]).
    pub auto: bool,
    pub seen: u32,
    /// A player's team colour (a player stays on one side of the feed).
    pub side: Option<Side>,
    /// A player's hero picture when first read: two names alike in a small, blurry feed are told
    /// apart by it.
    pub hero: Option<Sig>,
    /// More readings of a player's name (blur and the stream's compression read one name a
    /// little differently each time; a new reading is compared with all of them).
    pub more: Vec<Sig>,
}

/// Most readings kept of a player's name.
const MORE: usize = 24;
/// Readings of each side of a row kept while it is followed (the sharpest).
const READINGS: usize = 8;

/// How much the hero picture counts towards a player being the same.
const HERO_WEIGHT: f32 = 0.25;
/// Lowest name similarity plus [`HERO_WEIGHT`] × picture similarity of the same player.
const SAME_PLAYER: f32 = 0.92;

/// The player cluster of a row's side read on one or more samples (`readings`, sharpest first;
/// all on one side), new when no cluster is alike enough.
fn assign_player(list: &mut Vec<Cluster>, readings: &[FeedSide], names: &FeedNames) -> Option<usize> {
    let first = readings.first()?;
    let score = |c: &Cluster| {
        if c.side.is_some_and(|x| x != first.side) {
            return 0.0;
        }
        let mut best = 0.0f32;
        for s in readings {
            let h = c.hero.as_ref().zip(s.hero.as_ref()).map_or(0.0, |(a, b)| a.similarity(b));
            for sig in std::iter::once(&c.sig).chain(&c.more) {
                best = best.max(sig.similarity(&s.name) + HERO_WEIGHT * h);
            }
        }
        best
    };
    let best = list.iter().enumerate().map(|(i, c)| (score(c), i)).max_by(|a, b| a.0.total_cmp(&b.0));
    let i = match best.filter(|b| b.0 >= SAME_PLAYER) {
        Some((_, i)) => {
            if let Some(c) = list.get_mut(i) {
                c.seen = c.seen.saturating_add(1);
            }
            i
        }
        None => {
            if list.len() >= MAX_CLUSTERS {
                return None;
            }
            let label = readings.iter().find_map(|s| names.lookup(false, &s.name));
            list.push(Cluster { sig: first.name.clone(), label, auto: false, seen: 1, side: Some(first.side), hero: first.hero.clone(), more: Vec::new() });
            list.len() - 1
        }
    };
    if let Some(c) = list.get_mut(i) {
        for s in readings {
            if c.more.len() < MORE && s.name != c.sig && !c.more.contains(&s.name) {
                c.more.push(s.name.clone());
            }
        }
    }
    Some(i)
}

fn assign(list: &mut Vec<Cluster>, sig: &Sig, hero: bool, names: &FeedNames) -> Option<usize> {
    let thr = if hero { SAME_HERO } else { SAME_NAME };
    let best = list.iter().enumerate().map(|(i, c)| (c.sig.similarity(sig), i)).max_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, i)) = best.filter(|b| b.0 >= thr) {
        if let Some(c) = list.get_mut(i) {
            c.seen = c.seen.saturating_add(1);
        }
        return Some(i);
    }
    if list.len() >= MAX_CLUSTERS {
        return None;
    }
    list.push(Cluster { sig: sig.clone(), label: names.lookup(hero, sig), auto: false, seen: 1, side: None, hero: None, more: Vec::new() });
    Some(list.len() - 1)
}

/// Somebody in a kill: team, player cluster, hero cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Who {
    pub side: Side,
    pub player: usize,
    pub hero: Option<usize>,
}

/// One kill read off the feed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedEvent {
    /// Time of the first sample that showed the row (the caller's unit; ms in practice).
    pub at: i64,
    pub killer: Option<Who>,
    pub victim: Who,
    /// The row had an ultimate icon.
    pub ult: bool,
    /// The last sample that showed the row, and where it was then (its centre, as a share of the
    /// feed rectangle's height): where to look at the kill again.
    #[serde(default)]
    pub seen: i64,
    #[serde(default)]
    pub y: f32,
}

/// How far a row's box edges may move between two samples of it on average, in row heights
/// (blur, the stream's compression and a dark hero picture beside the box move an edge by a
/// pixel or several; a row with names of other lengths has its boxes elsewhere).
const SAME_EDGE: f32 = 0.8;

/// How alike two readings of a row are: 0 when not the same row. A row is known by its sides'
/// colours and where its boxes start and end, then by its names when they are read alike (a
/// blurred frame reads a name a little wider or not at all, its box stays about where it was)
/// and by how little it moved. A killer missed on one sample (a short box hidden by a flash)
/// does not make it another row.
fn row_match(a: &FeedRow, b: &FeedRow, tol: f32) -> f32 {
    if a.victim.side != b.victim.side {
        return 0.0;
    }
    let edges = |p: &FeedSide, q: &FeedSide| (p.x0 - q.x0).abs() + (p.x1 - q.x1).abs();
    let mut d = edges(&a.victim, &b.victim) / 2.0;
    let mut names = a.victim.name.likeness(&b.victim.name);
    let mut both = 1.0;
    match (&a.killer, &b.killer) {
        (Some(x), Some(y)) => {
            if x.side != y.side {
                return 0.0;
            }
            d = (edges(&a.victim, &b.victim) + edges(x, y)) / 4.0;
            names += x.name.likeness(&y.name);
            both = 2.0;
        }
        (None, None) => {}
        // a killer missed: the sample was flashed or smeared, and a glow beside the victim's box
        // may have stretched it on one end; the nearer end counts
        _ => d = (a.victim.x0 - b.victim.x0).abs().min((a.victim.x1 - b.victim.x1).abs()) + tol * 0.15,
    }
    if d.is_nan() || d > tol {
        return 0.0;
    }
    ((1.0 - d / tol) + 0.5 * names / both - (a.y - b.y).abs()).max(0.01)
}

/// Which followed row (top to bottom) each row read now (top to bottom) is, keeping their order
/// (rows in the feed never pass each other) and making the matches as good as possible
/// together: a new row at the top that looks a little like the old top row must not take its
/// place when the old row is read just below it.
fn align(at: i64, missed: i64, followed: &[Followed], rows: &[FeedRow]) -> Vec<Option<usize>> {
    let (n, m) = (followed.len().min(4 * MAX_ROWS), rows.len().min(MAX_ROWS));
    // a row missed on the samples between must be found closer to where it was
    let score = |i: usize, j: usize| match (followed.get(i), rows.get(j)) {
        (Some(f), Some(r)) => row_match(&f.row, r, if at.saturating_sub(f.seen) > missed { SAME_EDGE / 2.0 } else { SAME_EDGE }),
        _ => 0.0,
    };
    // best[i][j]: the best total over the first i followed rows and the first j rows read
    let w = m + 1;
    let mut best = vec![0.0f32; (n + 1) * w];
    let at = |b: &[f32], i: usize, j: usize| b.get(i * w + j).copied().unwrap_or(0.0);
    for i in 1..=n {
        for j in 1..=m {
            let s = score(i - 1, j - 1);
            let mut v = at(&best, i - 1, j).max(at(&best, i, j - 1));
            if s > 0.0 {
                v = v.max(at(&best, i - 1, j - 1) + s);
            }
            if let Some(c) = best.get_mut(i * w + j) {
                *c = v;
            }
        }
    }
    let mut out = vec![None; rows.len()];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let here = at(&best, i, j);
        let s = score(i - 1, j - 1);
        if s > 0.0 && (here - (at(&best, i - 1, j - 1) + s)).abs() <= f32::EPSILON * 8.0 * here.max(1.0) {
            if let Some(o) = out.get_mut(j - 1) {
                *o = Some(i - 1);
            }
            i -= 1;
            j -= 1;
        } else if here == at(&best, i - 1, j) {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    out
}

/// A row being followed: its last reading, the sharpest reading of each side (named from those
/// when it is gone), when it was first and last seen.
struct Followed {
    row: FeedRow,
    /// Readings of each side, sharpest first.
    killers: Vec<FeedSide>,
    victims: Vec<FeedSide>,
    seen: i64,
    /// Samples it has been read on.
    n: u8,
    first: i64,
    ult: bool,
}

impl Followed {
    fn new(at: i64, r: &FeedRow) -> Self {
        Self { row: r.clone(), killers: r.killer.iter().cloned().collect(), victims: vec![r.victim.clone()], seen: at, n: 1, first: at, ult: r.ult }
    }

    fn update(&mut self, at: i64, r: &FeedRow) {
        let keep = |list: &mut Vec<FeedSide>, s: &FeedSide| {
            let i = list.iter().position(|o| o.clear < s.clear).unwrap_or(list.len());
            if i < READINGS {
                list.insert(i, s.clone());
                list.truncate(READINGS);
            }
        };
        if let Some(k) = &r.killer {
            keep(&mut self.killers, k);
        }
        keep(&mut self.victims, &r.victim);
        // a killer read on one sample is the row's, even if the next misses it
        let mut row = r.clone();
        if row.killer.is_none() {
            row.killer = self.row.killer.take();
        }
        self.row = row;
        self.seen = at;
        self.n = self.n.saturating_add(1);
        self.ult |= r.ult;
    }
}

/// Follows the feed sample by sample; a row on `confirm` samples in a row that was not on screen
/// before is a kill at the time of its first sample. A row is known again on the next sample by
/// its boxes and names (rows move up or down as others come and go).
pub struct FeedReader {
    confirm: u8,
    /// Row height as a share of the crop's height, when known.
    row: Option<f32>,
    names: FeedNames,
    players: Vec<Cluster>,
    heroes: Vec<Cluster>,
    followed: Vec<Followed>,
    events: Vec<FeedEvent>,
    /// Where victims' boxes ended on recent samples (the feed is lined up on the right).
    rights: Vec<f32>,
    /// The previous sample's time.
    last: Option<i64>,
    /// Rows counted lately.
    recent: Vec<Counted>,
}

/// A row counted: its last reading, when that was, and its kill.
struct Counted {
    row: FeedRow,
    seen: i64,
    event: usize,
}

/// Lowest name likeness of a row found again (a different row with its boxes in about the same
/// places reads below it).
const REFOUND_NAMES: f32 = 0.8;

/// Whether `b` is row `a` found again after it was lost for longer than a flash.
fn refound(a: &FeedRow, b: &FeedRow) -> bool {
    if row_match(a, b, SAME_EDGE / 2.0) <= 0.0 {
        return false;
    }
    let mut names = vec![a.victim.name.likeness(&b.victim.name)];
    if let (Some(x), Some(y)) = (&a.killer, &b.killer) {
        names.push(x.name.likeness(&y.name));
    }
    names.iter().sum::<f32>() / names.len() as f32 >= REFOUND_NAMES
}

impl FeedReader {
    /// `row`: the feed's row height as a share of its rectangle's height, when known.
    pub fn new(names: FeedNames, confirm: u8, row: Option<f32>) -> Self {
        Self {
            confirm: confirm.max(1),
            row,
            names,
            players: Vec::new(),
            heroes: Vec::new(),
            followed: Vec::new(),
            events: Vec::new(),
            rights: Vec::new(),
            last: None,
            recent: Vec::new(),
        }
    }

    /// Who was read on `readings` of one side of a row (sharpest first).
    fn who(&mut self, readings: &[FeedSide]) -> Option<Who> {
        let first = readings.first()?;
        let player = assign_player(&mut self.players, readings, &self.names)?;
        let hero = readings.iter().find_map(|s| s.hero.as_ref()).and_then(|h| assign(&mut self.heroes, h, true, &self.names));
        Some(Who { side: first.side, player, hero })
    }

    /// Feed the sample at `at` (`img`: the crop around the feed). `live`: the game is on screen.
    pub fn push(&mut self, at: i64, img: &Rgb, live: bool) {
        if !live {
            return;
        }
        let rows = read_rows(img, self.row);
        self.push_rows(at, &rows);
    }

    /// Feed the rows read at `at`.
    pub fn push_rows(&mut self, at: i64, rows: &[FeedRow]) {
        // the feed's right edge: rows end there; what ends well left of it is something else
        // read as a row (two rows smeared together, an icon in the game)
        for r in rows.iter().take(MAX_ROWS) {
            if self.rights.len() >= RIGHTS {
                self.rights.remove(0);
            }
            self.rights.push(r.victim.x1);
        }
        let mut sorted = self.rights.clone();
        sorted.sort_by(f32::total_cmp);
        let right = sorted.get(sorted.len() / 2).copied().filter(|_| sorted.len() >= 8);
        let rows: Vec<FeedRow> = rows.iter().take(MAX_ROWS).filter(|r| right.is_none_or(|e| r.victim.x1 >= e - RIGHT_SLACK)).cloned().collect();
        let rows = rows.as_slice();
        // rows gone since the last sample (a long gap between samples, a replay skipped)
        let (mut followed, gone): (Vec<Followed>, Vec<Followed>) =
            std::mem::take(&mut self.followed).into_iter().partition(|f| at.saturating_sub(f.seen) <= GONE_MS);
        for f in gone {
            self.count(f);
        }
        followed.sort_by(|a, b| a.row.y.total_cmp(&b.row.y));
        // longer than a sample and a half since a row was read: it was missed on one between
        let step = self.last.map(|l| at.saturating_sub(l)).filter(|d| *d > 0).unwrap_or(500);
        self.last = Some(at);
        let pairs = align(at, step.saturating_add(step / 2), &followed, rows);
        let mut used = vec![false; followed.len()];
        let mut fresh: Vec<Followed> = Vec::new();
        for (r, m) in rows.iter().zip(pairs) {
            match m.and_then(|i| followed.get_mut(i).map(|f| (i, f))) {
                Some((i, f)) => {
                    if let Some(u) = used.get_mut(i) {
                        *u = true;
                    }
                    f.update(at, r);
                }
                None => fresh.push(Followed::new(at, r)),
            }
        }
        // a row read on `confirm` samples may be missed on a sample or two (a flash) and stays
        // followed; one not read on that many yet must be read on consecutive samples
        let mut keep = Vec::with_capacity(followed.len() + fresh.len());
        for (f, u) in followed.into_iter().zip(used) {
            if u || (f.n >= self.confirm && at.saturating_sub(f.seen) <= GONE_MS) {
                keep.push(f);
            } else {
                self.count(f);
            }
        }
        keep.extend(fresh);
        self.followed = keep;
    }

    /// A row no longer on screen: its kill, unless it was a flicker or the kill already counted
    /// (a row lost for longer than a flash and found again).
    fn count(&mut self, f: Followed) {
        if f.n < self.confirm {
            return;
        }
        // the same row found again after it was lost for a while: its boxes where they were and
        // its names alike (not the player clusters: two names alike on a small, blurry feed may
        // share one, and a kill must not be lost for it)
        self.recent.retain(|c| f.first.saturating_sub(c.seen) <= STAYS_MS);
        let again = self.recent.iter().rev().find(|c| refound(&c.row, &f.row)).map(|c| c.event);
        let event = match again.filter(|i| *i < self.events.len()) {
            Some(i) => {
                let missing = self.events.get(i).is_some_and(|e| e.killer.is_none());
                let killer = if missing { self.who(&f.killers) } else { None };
                if let Some(e) = self.events.get_mut(i) {
                    e.ult |= f.ult;
                    e.at = e.at.min(f.first);
                    if missing {
                        e.killer = killer;
                    }
                }
                i
            }
            None => {
                let killer = self.who(&f.killers);
                let Some(victim) = self.who(&f.victims) else { return };
                self.events.push(FeedEvent { at: f.first, killer, victim, ult: f.ult, seen: f.seen, y: f.row.y });
                self.events.len() - 1
            }
        };
        if self.recent.len() < 4 * MAX_ROWS {
            self.recent.push(Counted { row: f.row, seen: f.seen, event });
        }
    }

    /// The kills, with only the players and heroes in them, numbered as they first appear.
    pub fn finish(mut self) -> FeedScan {
        let mut followed = std::mem::take(&mut self.followed);
        followed.sort_by_key(|f| f.first);
        for f in followed {
            self.count(f);
        }
        let mut events = self.events;
        events.sort_by_key(|e| e.at);
        let (mut players, mut heroes) = (Vec::new(), Vec::new());
        let (mut pmap, mut hmap) = (vec![None; self.players.len()], vec![None; self.heroes.len()]);
        let mut renumber = |w: &mut Who| {
            if let Some(slot) = pmap.get_mut(w.player) {
                w.player = *slot.get_or_insert_with(|| {
                    players.push(self.players.get(w.player).cloned());
                    players.len() - 1
                });
            }
            if let Some(h) = w.hero
                && let Some(slot) = hmap.get_mut(h)
            {
                w.hero = Some(*slot.get_or_insert_with(|| {
                    heroes.push(self.heroes.get(h).cloned());
                    heroes.len() - 1
                }));
            }
        };
        for e in &mut events {
            if let Some(k) = e.killer.as_mut() {
                renumber(k);
            }
            renumber(&mut e.victim);
        }
        FeedScan { events, players: players.into_iter().flatten().collect(), heroes: heroes.into_iter().flatten().collect() }
    }
}

/// What a scan read: the kills and who is who.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeedScan {
    pub events: Vec<FeedEvent>,
    pub players: Vec<Cluster>,
    pub heroes: Vec<Cluster>,
}

impl FeedScan {
    /// The player's name, or "선수 N" until the user names them.
    pub fn player(&self, i: usize) -> String {
        self.players.get(i).and_then(|c| c.label.clone()).unwrap_or_else(|| format!("선수 {}", i + 1))
    }

    /// The hero's name, or "영웅 N" until the user names it.
    pub fn hero(&self, i: usize) -> String {
        self.heroes.get(i).and_then(|c| c.label.clone()).unwrap_or_else(|| format!("영웅 {}", i + 1))
    }

    /// "NAME (HERO)".
    pub fn who(&self, w: &Who) -> String {
        match w.hero {
            Some(h) => format!("{} ({})", self.player(w.player), self.hero(h)),
            None => self.player(w.player),
        }
    }

    /// "KILLER → VICTIM", or "VICTIM 처치됨" without a killer.
    pub fn line(&self, e: &FeedEvent) -> String {
        match &e.killer {
            Some(k) => format!("{} → {}", self.who(k), self.who(&e.victim)),
            None => format!("{} 처치됨", self.who(&e.victim)),
        }
    }

    /// Times of the kills made with an ultimate.
    pub fn ult_times(&self) -> Vec<i64> {
        self.events.iter().filter(|e| e.ult).map(|e| e.at).collect()
    }
}

#[cfg(test)]
#[path = "feedrows_tests.rs"]
mod tests;

//! Screen templates: pieces of a reference frame (a broadcast's break-screen graphics, the
//! "PREPARE TO ATTACK" banner, a pause notice) found again by normalised cross-correlation.
//!
//! A template is learnt from a frame of the same broadcast, so it carries no built-in artwork:
//! the repository ships none (Blizzard's HUD and the league's graphics are not ours to
//! redistribute); each user's template file lives beside their projects.

use serde::{Deserialize, Serialize};

use crate::luma::{Luma, NRect};
use crate::{Screen, VisionError};

/// Longest side of a part's grid.
const MAX_GRID: usize = 128;
/// Shortest side of a part's grid.
const MIN_GRID: usize = 4;
/// Most cells a part is shifted in each direction while searching.
const MAX_SHIFT: i32 = 12;
/// A part needs at least this much contrast (standard deviation, gray levels) to be matched.
const MIN_STD: f32 = 3.0;

/// One rectangle of a template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// Where it was learnt (frame-relative).
    pub rect: NRect,
    /// Grid size.
    pub w: usize,
    pub h: usize,
    /// Gray levels, row by row.
    pub px: Vec<u8>,
    /// 1 where the pixel is compared, 0 where it is ignored (a running clock, a camera feed).
    pub mask: Vec<u8>,
    /// How far the part may have moved, as a share of the frame width.
    #[serde(default = "default_search")]
    pub search: f32,
}

fn default_search() -> f32 {
    0.01
}

/// A named screen state found by one or more parts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    pub screen: Screen,
    pub parts: Vec<Part>,
    /// Lowest correlation (of the worst part) that counts as a match.
    #[serde(default = "default_threshold")]
    pub threshold: f32,
}

fn default_threshold() -> f32 {
    0.7
}

/// All templates of one broadcast (or one game client language).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemplateSet {
    #[serde(default)]
    pub templates: Vec<Template>,
}

impl Part {
    /// Learn the part of `frame` inside `rect`, leaving out the `ignore` rectangles.
    pub fn learn(frame: &Luma, rect: NRect, ignore: &[NRect]) -> Result<Part, VisionError> {
        let rect = rect.clamped().ok_or(VisionError::EmptyRect)?;
        let (pw, ph) = (rect.w * frame.width as f32, rect.h * frame.height as f32);
        let scale = (MAX_GRID as f32 / pw.max(ph)).min(1.0);
        let w = ((pw * scale).round() as usize).clamp(MIN_GRID, MAX_GRID);
        let h = ((ph * scale).round() as usize).clamp(MIN_GRID, MAX_GRID);
        let mut px = Vec::with_capacity(w * h);
        let mut mask = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let u = rect.x + (x as f32 + 0.5) * rect.w / w as f32;
                let v = rect.y + (y as f32 + 0.5) * rect.h / h as f32;
                px.push(frame.sample(u, v).round().clamp(0.0, 255.0) as u8);
                let hidden = ignore.iter().filter_map(|r| r.clamped()).any(|r| u >= r.x && u < r.x + r.w && v >= r.y && v < r.y + r.h);
                mask.push(u8::from(!hidden));
            }
        }
        let part = Part { rect, w, h, px, mask, search: default_search() };
        if part.mask.iter().filter(|m| **m != 0).count() < 16 {
            return Err(VisionError::EmptyRect);
        }
        let (_, sd) = part.stats();
        if sd < MIN_STD {
            return Err(VisionError::Flat);
        }
        Ok(part)
    }

    fn valid(&self) -> bool {
        self.w >= 1 && self.h >= 1 && self.w <= MAX_GRID && self.h <= MAX_GRID && self.px.len() == self.w * self.h && self.mask.len() == self.px.len()
    }

    /// Mean and standard deviation of the compared pixels.
    fn stats(&self) -> (f32, f32) {
        let mut n = 0f32;
        let mut s = 0f32;
        let mut s2 = 0f32;
        for (p, m) in self.px.iter().zip(&self.mask) {
            if *m != 0 {
                let v = *p as f32;
                n += 1.0;
                s += v;
                s2 += v * v;
            }
        }
        if n < 1.0 {
            return (0.0, 0.0);
        }
        let mean = s / n;
        (mean, (s2 / n - mean * mean).max(0.0).sqrt())
    }

    /// Best normalised cross-correlation (−1..1) of the part against `frame` within its search
    /// distance. 0 for a damaged part or a flat picture.
    pub fn score(&self, frame: &Luma) -> f32 {
        let Some(rect) = self.rect.clamped() else { return 0.0 };
        if !self.valid() || frame.width == 0 || frame.height == 0 {
            return 0.0;
        }
        let (cw, ch) = (rect.w / self.w as f32, rect.h / self.h as f32);
        let search = if self.search.is_finite() { self.search.clamp(0.0, 0.2) } else { 0.0 };
        let sx = ((search / cw.max(1e-6)).ceil() as i32).clamp(0, MAX_SHIFT);
        let sy = ((search * frame.width as f32 / frame.height as f32 / ch.max(1e-6)).ceil() as i32).clamp(0, MAX_SHIFT);
        // the frame is sampled at half-cell steps so a part that moved by a fraction of a cell
        // still lines up
        const SUB: usize = 2;
        let (sx, sy) = (sx as usize * SUB, sy as usize * SUB);
        let (gw, gh) = (self.w * SUB + 2 * sx, self.h * SUB + 2 * sy);
        let mut grid = Vec::with_capacity(gw * gh);
        for y in 0..gh {
            for x in 0..gw {
                let u = rect.x + ((x as f32 - sx as f32) / SUB as f32 + 0.5) * cw;
                let v = rect.y + ((y as f32 - sy as f32) / SUB as f32 + 0.5) * ch;
                grid.push(frame.sample(u, v));
            }
        }
        let (tm, tsd) = self.stats();
        if tsd < 1e-3 {
            return 0.0;
        }
        let mut best = -1f32;
        for dy in 0..=2 * sy {
            for dx in 0..=2 * sx {
                let (mut n, mut s, mut s2, mut st) = (0f32, 0f32, 0f32, 0f32);
                for y in 0..self.h {
                    let grow = (y * SUB + dy) * gw + dx;
                    let trow = y * self.w;
                    for x in 0..self.w {
                        if self.mask.get(trow + x).copied().unwrap_or(0) == 0 {
                            continue;
                        }
                        let f = grid.get(grow + x * SUB).copied().unwrap_or(0.0);
                        let t = self.px.get(trow + x).copied().unwrap_or(0) as f32 - tm;
                        n += 1.0;
                        s += f;
                        s2 += f * f;
                        st += f * t;
                    }
                }
                if n < 1.0 {
                    continue;
                }
                let fm = s / n;
                let fsd = (s2 / n - fm * fm).max(0.0).sqrt();
                // a flat patch matches nothing (and must not divide by zero)
                if fsd < MIN_STD * 0.5 {
                    continue;
                }
                // Σ(f−fm)(t) = Σ f·t because Σ t = 0
                let r = st / (n * fsd * tsd);
                if r > best {
                    best = r;
                }
            }
        }
        best.clamp(-1.0, 1.0).max(0.0)
    }
}

impl Template {
    /// Learn a template from `frame`: one part per rectangle, `ignore` rectangles left out of all
    /// of them (put the running clock or a camera feed here).
    pub fn learn(name: &str, screen: Screen, frame: &Luma, rects: &[NRect], ignore: &[NRect]) -> Result<Template, VisionError> {
        if rects.is_empty() {
            return Err(VisionError::EmptyRect);
        }
        let parts = rects.iter().take(8).map(|r| Part::learn(frame, *r, ignore)).collect::<Result<Vec<_>, _>>()?;
        Ok(Template { name: name.to_string(), screen, parts, threshold: default_threshold() })
    }

    /// The worst part's score: every part must be there.
    pub fn score(&self, frame: &Luma) -> f32 {
        if self.parts.is_empty() {
            return 0.0;
        }
        self.parts.iter().map(|p| p.score(frame)).fold(1.0f32, f32::min)
    }
}

impl TemplateSet {
    pub fn from_json(text: &str) -> Result<TemplateSet, VisionError> {
        let set: TemplateSet = serde_json::from_str(text).map_err(|e| VisionError::Templates(e.to_string()))?;
        if set.templates.len() > 256 {
            return Err(VisionError::Templates("more than 256 templates".into()));
        }
        Ok(set)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{\"templates\":[]}".into())
    }

    /// Add a template, replacing one of the same name.
    pub fn insert(&mut self, t: Template) {
        self.templates.retain(|o| o.name != t.name);
        self.templates.push(t);
    }

    /// The best matching template over its threshold: (screen, template index, score).
    pub fn best(&self, frame: &Luma) -> Option<(Screen, usize, f32)> {
        let mut best: Option<(Screen, usize, f32)> = None;
        for (i, t) in self.templates.iter().enumerate() {
            let s = t.score(frame);
            let thr = if t.threshold.is_finite() { t.threshold.clamp(0.1, 1.0) } else { default_threshold() };
            if s >= thr && best.is_none_or(|b| s > b.2) {
                best = Some((t.screen, i, s));
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny deterministic noise.
    fn rng(seed: &mut u32) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        *seed
    }

    /// A 320×180 frame: noisy "gameplay" background (different per seed), and when `banner` is
    /// set, a dark box with white glyph bars at the top centre shifted by `dx` pixels, plus a
    /// clock box whose digits are `clock`.
    fn frame(seed: u32, banner: bool, dx: usize, clock: u32) -> Luma {
        let (w, h) = (320, 180);
        let mut s = seed.max(1);
        let mut px: Vec<u8> = (0..w * h).map(|_| (rng(&mut s) % 200) as u8).collect();
        if banner {
            for y in 10..30 {
                for x in 100 + dx..200 + dx {
                    // glyph-like vertical strokes
                    let ink = (y > 14 && y < 26) && ((x - dx) % 7 < 3) && ((x - dx) / 7) % 3 != 1;
                    px[y * w + x] = if ink { 240 } else { 30 };
                }
                // clock box: its digits change every second
                for x in 205 + dx..225 + dx {
                    let ink = (y > 14 && y < 26) && ((x + clock as usize * 3) % 5 < 2);
                    px[y * w + x] = if ink { 240 } else { 30 };
                }
            }
        }
        Luma::from_gray(px, w, h).unwrap()
    }

    fn banner_rect() -> NRect {
        NRect::new(95.0 / 320.0, 8.0 / 180.0, 135.0 / 320.0, 24.0 / 180.0)
    }

    fn clock_rect() -> NRect {
        NRect::new(203.0 / 320.0, 8.0 / 180.0, 25.0 / 320.0, 24.0 / 180.0)
    }

    #[test]
    fn finds_the_banner_again_and_not_without_it() {
        let t = Template::learn("prepare", Screen::Setup, &frame(1, true, 0, 15), &[banner_rect()], &[clock_rect()]).unwrap();
        // other background, clock ticked, banner moved two pixels
        assert!(t.score(&frame(7, true, 2, 9)) > 0.75, "{}", t.score(&frame(7, true, 2, 9)));
        // drawn tight round the banner the match is near perfect
        let tight = NRect::new(100.0 / 320.0, 10.0 / 180.0, 125.0 / 320.0, 20.0 / 180.0);
        let t2 = Template::learn("prepare", Screen::Setup, &frame(1, true, 0, 15), &[tight], &[clock_rect()]).unwrap();
        assert!(t2.score(&frame(7, true, 2, 9)) > 0.95, "{}", t2.score(&frame(7, true, 2, 9)));
        assert!(t.score(&frame(7, false, 0, 0)) < 0.3, "{}", t.score(&frame(7, false, 0, 0)));
        let mut set = TemplateSet::default();
        set.insert(t);
        assert_eq!(set.best(&frame(3, true, 1, 4)).map(|b| b.0), Some(Screen::Setup));
        assert_eq!(set.best(&frame(3, false, 0, 4)), None);
        // the set survives a save and load
        let back = TemplateSet::from_json(&set.to_json()).unwrap();
        assert_eq!(back, set);
    }

    #[test]
    fn the_clock_must_be_ignored_for_a_strict_match() {
        let t = Template::learn("prepare", Screen::Setup, &frame(1, true, 0, 15), &[banner_rect()], &[]).unwrap();
        let ignoring = Template::learn("prepare", Screen::Setup, &frame(1, true, 0, 15), &[banner_rect()], &[clock_rect()]).unwrap();
        let f = frame(9, true, 0, 2);
        assert!(ignoring.score(&f) > t.score(&f));
    }

    #[test]
    fn flat_and_empty_rects_are_refused() {
        let flat = Luma::from_gray(vec![90; 64 * 64], 64, 64).unwrap();
        assert_eq!(Part::learn(&flat, NRect::new(0.1, 0.1, 0.5, 0.5), &[]), Err(VisionError::Flat));
        assert_eq!(Part::learn(&flat, NRect::new(0.1, 0.1, 0.0, 0.5), &[]), Err(VisionError::EmptyRect));
        assert!(Template::learn("x", Screen::Pause, &flat, &[], &[]).is_err());
    }

    #[test]
    fn damaged_template_files_do_not_panic() {
        let set = TemplateSet::from_json(r#"{"templates":[{"name":"x","screen":"pause","parts":[{"rect":{"x":0.1,"y":0.1,"w":0.2,"h":0.2},"w":90000,"h":3,"px":[1,2],"mask":[]}],"threshold":-3}]}"#).unwrap();
        let f = frame(2, true, 0, 0);
        assert_eq!(set.templates[0].score(&f), 0.0);
        assert_eq!(set.best(&f), None);
        assert!(TemplateSet::from_json("{").is_err());
    }
}

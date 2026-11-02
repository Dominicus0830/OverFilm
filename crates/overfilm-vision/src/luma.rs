//! Small grayscale frames for analysis.

use serde::{Deserialize, Serialize};

/// Widest analysis frame. The broadcast HUD's objective text ("PREPARE TO ATTACK") is about 8
/// pixels tall at 720p, so frames are analysed at up to 1280 wide (under a megabyte of gray).
pub const MAX_WIDTH: usize = 1280;

/// A rectangle in frame-relative coordinates (0..1 on both axes, origin top left), so a template
/// learnt on a 1080p frame matches the same broadcast at 720p.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl NRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The rectangle clamped into the frame, or `None` when nothing (or a non-number) is left.
    pub fn clamped(self) -> Option<NRect> {
        let ok = |v: f32| v.is_finite();
        if !(ok(self.x) && ok(self.y) && ok(self.w) && ok(self.h)) {
            return None;
        }
        let x0 = self.x.clamp(0.0, 1.0);
        let y0 = self.y.clamp(0.0, 1.0);
        let x1 = (self.x + self.w).clamp(0.0, 1.0);
        let y1 = (self.y + self.h).clamp(0.0, 1.0);
        (x1 - x0 > 1e-4 && y1 - y0 > 1e-4).then(|| NRect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// An 8-bit grayscale (Rec. 709 luma) frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Luma {
    pub width: usize,
    pub height: usize,
    pub px: Vec<u8>,
}

impl Luma {
    /// A frame from 8-bit RGBA rows, shrunk by area averaging so it is at most `max_width` wide
    /// (and at most [`MAX_WIDTH`]). `None` when the buffer is too small for the size given.
    pub fn from_rgba(rgba: &[u8], width: usize, height: usize, max_width: usize) -> Option<Luma> {
        let need = width.checked_mul(height)?.checked_mul(4)?;
        if width == 0 || height == 0 || rgba.len() < need {
            return None;
        }
        let max_w = max_width.clamp(8, MAX_WIDTH);
        // whole-pixel factor keeps the box filter exact and cheap
        let f = width.div_ceil(max_w).max(1);
        let (w, h) = ((width / f).max(1), (height / f).max(1));
        let mut px = Vec::with_capacity(w * h);
        let n = (f * f) as u32;
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0u32;
                for yy in y * f..(y * f + f).min(height) {
                    let row = yy * width;
                    for xx in x * f..(x * f + f).min(width) {
                        let i = (row + xx) * 4;
                        let (r, g, b) = (rgba.get(i).copied().unwrap_or(0), rgba.get(i + 1).copied().unwrap_or(0), rgba.get(i + 2).copied().unwrap_or(0));
                        // BT.709 weights in 1/256ths
                        acc += (54 * r as u32 + 183 * g as u32 + 19 * b as u32) >> 8;
                    }
                }
                px.push((acc / n).min(255) as u8);
            }
        }
        Some(Luma { width: w, height: h, px })
    }

    /// A frame from 8-bit gray rows (tests and already-gray sources).
    pub fn from_gray(px: Vec<u8>, width: usize, height: usize) -> Option<Luma> {
        (width > 0 && height > 0 && width.checked_mul(height) == Some(px.len())).then_some(Luma { width, height, px })
    }

    /// Bilinear sample at frame-relative `(u, v)`; outside the frame the edge pixel.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let fx = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let fy = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let p = |x: usize, y: usize| self.px.get(y * self.width + x).copied().unwrap_or(0) as f32;
        let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
        let bot = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
        top * (1.0 - ay) + bot * ay
    }

    /// Share of 8×8 blocks whose mean absolute difference from `prev` is under `tol` gray levels:
    /// 1.0 for a frozen picture. Frames of different sizes give 0.
    pub fn still_fraction(&self, prev: &Luma, tol: f32) -> f32 {
        if self.width != prev.width || self.height != prev.height {
            return 0.0;
        }
        const B: usize = 8;
        let (bw, bh) = (self.width / B, self.height / B);
        if bw == 0 || bh == 0 {
            return 0.0;
        }
        let mut still = 0usize;
        for by in 0..bh {
            for bx in 0..bw {
                let mut d = 0u32;
                for y in by * B..by * B + B {
                    let row = y * self.width;
                    for x in bx * B..bx * B + B {
                        let a = self.px.get(row + x).copied().unwrap_or(0);
                        let b = prev.px.get(row + x).copied().unwrap_or(0);
                        d += a.abs_diff(b) as u32;
                    }
                }
                if (d as f32 / (B * B) as f32) < tol {
                    still += 1;
                }
            }
        }
        still as f32 / (bw * bh) as f32
    }
}

/// An 8-bit RGB crop of a frame, at (nearly) full resolution: the kill feed's small text and
/// pictures are read in colour (the team is the box's colour).
#[derive(Clone, Debug, PartialEq)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    /// R, G, B per pixel, row by row.
    pub px: Vec<u8>,
}

impl Rgb {
    /// The part of an RGBA frame inside `rect`, shrunk by a whole factor to at most `max_width`
    /// wide (at most 2048). `None` for an empty rectangle or a buffer too small for the size given.
    pub fn crop_rgba(rgba: &[u8], width: usize, height: usize, rect: NRect, max_width: usize) -> Option<Rgb> {
        let need = width.checked_mul(height)?.checked_mul(4)?;
        if width == 0 || height == 0 || rgba.len() < need {
            return None;
        }
        let r = rect.clamped()?;
        let x0 = ((r.x * width as f32) as usize).min(width - 1);
        let y0 = ((r.y * height as f32) as usize).min(height - 1);
        let x1 = (((r.x + r.w) * width as f32).ceil() as usize).clamp(x0 + 1, width);
        let y1 = (((r.y + r.h) * height as f32).ceil() as usize).clamp(y0 + 1, height);
        let f = (x1 - x0).div_ceil(max_width.clamp(8, 2048)).max(1);
        let (w, h) = (((x1 - x0) / f).max(1), ((y1 - y0) / f).max(1));
        let mut px = Vec::with_capacity(w * h * 3);
        let n = (f * f) as u32;
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0u32; 3];
                for yy in y0 + y * f..(y0 + y * f + f).min(height) {
                    for xx in x0 + x * f..(x0 + x * f + f).min(width) {
                        let i = (yy * width + xx) * 4;
                        for (c, a) in acc.iter_mut().enumerate() {
                            *a += rgba.get(i + c).copied().unwrap_or(0) as u32;
                        }
                    }
                }
                px.extend(acc.map(|a| (a / n).min(255) as u8));
            }
        }
        Some(Rgb { width: w, height: h, px })
    }

    /// A picture from 8-bit RGB rows (tests).
    pub fn from_rgb(px: Vec<u8>, width: usize, height: usize) -> Option<Rgb> {
        (width > 0 && height > 0 && width.checked_mul(height).and_then(|n| n.checked_mul(3)) == Some(px.len())).then_some(Rgb { width, height, px })
    }

    /// The pixel at `(x, y)`; black outside.
    pub fn at(&self, x: usize, y: usize) -> [u8; 3] {
        if x >= self.width || y >= self.height {
            return [0; 3];
        }
        let i = (y * self.width + x) * 3;
        [self.px.get(i).copied().unwrap_or(0), self.px.get(i + 1).copied().unwrap_or(0), self.px.get(i + 2).copied().unwrap_or(0)]
    }

    /// Rec. 709 gray of the pixel at `(x, y)`.
    pub fn gray(&self, x: usize, y: usize) -> f32 {
        let [r, g, b] = self.at(x, y);
        0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
    }

    /// The whole crop in gray.
    pub fn luma(&self) -> Luma {
        let px = (0..self.height).flat_map(|y| (0..self.width).map(move |x| (y, x))).map(|(y, x)| self.gray(x, y).round().clamp(0.0, 255.0) as u8).collect();
        Luma { width: self.width, height: self.height, px }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrinks_by_area() {
        // 4×2 RGBA: left half white, right half black → 2×1 at factor 2
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 255 } else { 0 };
                rgba.extend([v, v, v, 255]);
            }
        }
        let l = Luma::from_rgba(&rgba, 4, 2, 8).unwrap();
        assert_eq!((l.width, l.height), (4, 2));
        assert!(l.px[0] >= 254 && l.px[3] == 0);
    }

    #[test]
    fn hostile_sizes_are_refused() {
        assert!(Luma::from_rgba(&[0; 8], 4, 4, 64).is_none());
        assert!(Luma::from_rgba(&[], 0, 0, 64).is_none());
        assert!(Luma::from_rgba(&[0; 16], usize::MAX, 2, 64).is_none());
        assert!(NRect::new(f32::NAN, 0.0, 1.0, 1.0).clamped().is_none());
        assert!(NRect::new(2.0, 0.0, 1.0, 1.0).clamped().is_none());
    }

    #[test]
    fn still_fraction_sees_motion() {
        let a = Luma::from_gray(vec![10; 64 * 64], 64, 64).unwrap();
        let mut b = a.clone();
        assert_eq!(b.still_fraction(&a, 2.0), 1.0);
        for y in 0..32 {
            for x in 0..64 {
                b.px[y * 64 + x] = 200;
            }
        }
        assert!((b.still_fraction(&a, 2.0) - 0.5).abs() < 1e-6);
    }
}

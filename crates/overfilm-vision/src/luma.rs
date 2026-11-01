//! Small grayscale frames for analysis.

use serde::{Deserialize, Serialize};

/// Widest analysis frame. Broadcast graphics and HUD text stay readable at this size and a frame
/// is a few hundred kilobytes at most.
pub const MAX_WIDTH: usize = 640;

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

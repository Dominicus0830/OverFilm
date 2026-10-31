//! Kaldi-style log mel filterbank features, the input of the speaker network: 25 ms frames every
//! 10 ms (edges snipped), DC removal, pre-emphasis 0.97, Hamming window, 512-point power
//! spectrum, 80 triangular mel bands from 20 Hz to Nyquist on the `1127·ln(1 + f/700)` scale,
//! natural log floored at `f32::EPSILON`. Samples are scaled to 16-bit range first, as the
//! network was trained. Written from the Kaldi feature documentation.

use std::f64::consts::PI;

/// Mel bands.
pub const N_MELS: usize = 80;
const FRAME: usize = 400;
/// Samples between frames (10 ms at 16 kHz).
pub const SHIFT: usize = 160;
const N_FFT: usize = 512;
const N_BINS: usize = N_FFT / 2;

/// Frames for `n` samples (edges snipped: only whole frames).
pub fn frames(n: usize) -> usize {
    if n < FRAME { 0 } else { 1 + (n - FRAME) / SHIFT }
}

/// Precomputed window, mel weights and FFT tables.
pub struct Fbank {
    window: Vec<f32>,
    /// per band: first bin and weights
    bands: Vec<(usize, Vec<f32>)>,
    cos: Vec<f32>,
    sin: Vec<f32>,
}

impl Default for Fbank {
    fn default() -> Self {
        Self::new()
    }
}

fn mel(f: f64) -> f64 {
    1127.0 * (1.0 + f / 700.0).ln()
}

impl Fbank {
    pub fn new() -> Self {
        let window = (0..FRAME).map(|i| (0.54 - 0.46 * (2.0 * PI * i as f64 / (FRAME - 1) as f64).cos()) as f32).collect();
        let (lo, hi) = (mel(20.0), mel(8000.0));
        let step = (hi - lo) / (N_MELS + 1) as f64;
        let bands = (0..N_MELS)
            .map(|m| {
                let (l, c, r) = (lo + m as f64 * step, lo + (m + 1) as f64 * step, lo + (m + 2) as f64 * step);
                let w: Vec<f32> = (0..N_BINS)
                    .map(|k| {
                        let x = mel(k as f64 * 16_000.0 / N_FFT as f64);
                        let up = (x - l) / (c - l);
                        let down = (r - x) / (r - c);
                        up.min(down).max(0.0) as f32
                    })
                    .collect();
                let first = w.iter().position(|&v| v > 0.0).unwrap_or(0);
                let last = w.iter().rposition(|&v| v > 0.0).map_or(first, |p| p + 1);
                (first, w.get(first..last).map(<[f32]>::to_vec).unwrap_or_default())
            })
            .collect();
        let mut cos = vec![0f32; N_FFT / 2];
        let mut sin = vec![0f32; N_FFT / 2];
        for (k, (c, s)) in cos.iter_mut().zip(sin.iter_mut()).enumerate() {
            let a = -2.0 * PI * k as f64 / N_FFT as f64;
            *c = a.cos() as f32;
            *s = a.sin() as f32;
        }
        Self { window, bands, cos, sin }
    }

    /// Features `frames × N_MELS` (row-major by frame) of 16 kHz audio in [-1, 1].
    pub fn compute(&self, audio: &[f32]) -> Vec<f32> {
        let n = frames(audio.len());
        let mut out = vec![0f32; n * N_MELS];
        let mut re = [0f32; N_FFT];
        let mut im = [0f32; N_FFT];
        let mut power = [0f32; N_BINS];
        for (f, row) in out.chunks_exact_mut(N_MELS).enumerate() {
            let Some(src) = audio.get(f * SHIFT..f * SHIFT + FRAME) else { break };
            let mean = src.iter().map(|&x| f64::from(x)).sum::<f64>() as f32 / FRAME as f32;
            let mut prev = (src.first().copied().unwrap_or(0.0) - mean) * 32768.0;
            for (i, r) in re.iter_mut().enumerate() {
                *r = match src.get(i) {
                    Some(&x) => {
                        let v = (x - mean) * 32768.0;
                        let e = v - 0.97 * prev;
                        prev = v;
                        e * self.window.get(i).copied().unwrap_or(0.0)
                    }
                    None => 0.0,
                };
            }
            im.fill(0.0);
            self.fft(&mut re, &mut im);
            for (k, p) in power.iter_mut().enumerate() {
                *p = re[k] * re[k] + im[k] * im[k];
            }
            for (o, (first, w)) in row.iter_mut().zip(&self.bands) {
                let e: f32 = w.iter().zip(power.iter().skip(*first)).map(|(a, b)| a * b).sum();
                *o = e.max(f32::EPSILON).ln();
            }
        }
        out
    }

    /// In-place radix-2 FFT of length `N_FFT`.
    fn fft(&self, re: &mut [f32; N_FFT], im: &mut [f32; N_FFT]) {
        // bit reversal
        let mut j = 0usize;
        for i in 1..N_FFT {
            let mut bit = N_FFT >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= N_FFT {
            let half = len / 2;
            let stride = N_FFT / len;
            for start in (0..N_FFT).step_by(len) {
                for k in 0..half {
                    let (c, s) = (self.cos[k * stride], self.sin[k * stride]);
                    let (a, b) = (start + k, start + k + half);
                    let (xr, xi) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                    re[b] = re[a] - xr;
                    im[b] = im[a] - xi;
                    re[a] += xr;
                    im[a] += xi;
                }
            }
            len <<= 1;
        }
    }
}

/// Subtract each band's mean over the frames (cepstral mean normalisation, as in training).
pub fn subtract_mean(feats: &mut [f32]) {
    let n = feats.len() / N_MELS;
    if n == 0 {
        return;
    }
    let mut mean = [0f64; N_MELS];
    for row in feats.chunks_exact(N_MELS) {
        for (m, &v) in mean.iter_mut().zip(row) {
            *m += f64::from(v);
        }
    }
    for row in feats.chunks_exact_mut(N_MELS) {
        for (v, m) in row.iter_mut().zip(&mean) {
            *v -= (*m / n as f64) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_count_and_tone_peak() {
        assert_eq!(frames(399), 0);
        assert_eq!(frames(400), 1);
        assert_eq!(frames(16_000), 98);
        // a 1 kHz tone puts its energy in the band around 1 kHz
        let audio: Vec<f32> = (0..16_000).map(|i| 0.5 * (2.0 * PI * 1000.0 * i as f64 / 16_000.0).sin() as f32).collect();
        let f = Fbank::new().compute(&audio);
        let row = &f[10 * N_MELS..11 * N_MELS];
        let peak = row.iter().enumerate().fold((0, f32::MIN), |b, (i, &v)| if v > b.1 { (i, v) } else { b }).0;
        let centre = |m: usize| {
            let (lo, hi) = (mel(20.0), mel(8000.0));
            let c = lo + (m + 1) as f64 * (hi - lo) / 81.0;
            700.0 * ((c / 1127.0).exp() - 1.0)
        };
        assert!((centre(peak) - 1000.0).abs() < 80.0, "peak band {peak} at {} Hz", centre(peak));
    }

    #[test]
    fn fft_matches_dft() {
        let fb = Fbank::new();
        let mut re = [0f32; N_FFT];
        let mut im = [0f32; N_FFT];
        for (i, r) in re.iter_mut().enumerate() {
            *r = ((i * 7919) % 97) as f32 / 97.0 - 0.5;
        }
        let orig = re;
        fb.fft(&mut re, &mut im);
        for k in [0usize, 1, 5, 100, 255] {
            let (mut a, mut b) = (0f64, 0f64);
            for (i, &x) in orig.iter().enumerate() {
                let t = -2.0 * PI * (k * i) as f64 / N_FFT as f64;
                a += f64::from(x) * t.cos();
                b += f64::from(x) * t.sin();
            }
            assert!((f64::from(re[k]) - a).abs() < 1e-3 && (f64::from(im[k]) - b).abs() < 1e-3, "bin {k}");
        }
    }

    #[test]
    fn silence_and_short_input() {
        let fb = Fbank::new();
        assert!(fb.compute(&[0.0; 100]).is_empty());
        let f = fb.compute(&[0.0; 800]);
        assert!(f.iter().all(|&v| (v - f32::EPSILON.ln()).abs() < 1e-6));
    }
}

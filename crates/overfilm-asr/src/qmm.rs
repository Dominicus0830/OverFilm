//! 8-bit (Q8_0-style: one scale per 32 values) weights on the CPU with one product for several
//! input rows: each block of a weight row is loaded once and multiplied with the same block of
//! every input row while it is in registers. candle's own 8-bit product walks the whole weight
//! once per input row, so decoding four chunks side by side cost four full passes over 2 GB; here
//! it costs one, and on AVX2 the per-row work is a handful of instructions per block (the block's
//! absolute values are computed once for all rows).

use candle_core::{Device, Result, Tensor};
use rayon::prelude::*;

/// Values per block.
const QK: usize = 32;
/// Weight rows per parallel task.
const COLS: usize = 32;
/// Input rows one weight pass serves; more rows are processed in further passes.
const ROWS: usize = 8;

/// An `n × k` weight in 8-bit blocks (row-major, `k / 32` blocks per row).
pub struct Q8 {
    n: usize,
    k: usize,
    /// one scale per block
    d: Vec<f32>,
    /// the values, `n · k`
    qs: Vec<i8>,
}

/// Blocks of `x` (a multiple of 32 long) as (scales, values).
fn quantize_rows(x: &[f32], scales: &mut [f32], qs: &mut [i8]) {
    for ((b, s), q) in x.chunks(QK).zip(scales.iter_mut()).zip(qs.chunks_mut(QK)) {
        let amax = b.iter().fold(0f32, |a, v| a.max(v.abs()));
        let d = amax / 127.0;
        let id = if d > 0.0 { 1.0 / d } else { 0.0 };
        *s = d;
        for (o, v) in q.iter_mut().zip(b) {
            *o = (v * id).round().clamp(-127.0, 127.0) as i8;
        }
    }
}

impl Q8 {
    /// Quantize an `(n, k)` f32 weight; `k` must be a multiple of 32.
    pub fn quantize(w: &Tensor) -> Result<Self> {
        let (n, k) = w.dims2()?;
        if k == 0 || !k.is_multiple_of(QK) {
            candle_core::bail!("Q8_0 needs a width divisible by {QK}, got {k}");
        }
        let data = w.flatten_all()?.to_dtype(candle_core::DType::F32)?.to_vec1::<f32>()?;
        let kb = k / QK;
        let mut d = vec![0f32; n * kb];
        let mut qs = vec![0i8; n * k];
        d.par_chunks_mut(kb).zip(qs.par_chunks_mut(k)).zip(data.par_chunks(k)).for_each(|((d, q), row)| quantize_rows(row, d, q));
        Ok(Self { n, k, d, qs })
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.n, self.k)
    }

    /// The weight back in f32, `(n, k)` (prefill: a dense product beats the 8-bit one there).
    pub fn dequantize(&self, dev: &Device) -> Result<Tensor> {
        let mut out = vec![0f32; self.n * self.k];
        out.par_chunks_mut(QK).zip(self.qs.par_chunks(QK)).zip(self.d.par_iter()).for_each(|((o, q), &d)| {
            for (o, &q) in o.iter_mut().zip(q) {
                *o = f32::from(q) * d;
            }
        });
        Tensor::from_vec(out, (self.n, self.k), dev)
    }

    /// `x (…, k)` times the weight transposed: `(…, n)`.
    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let dims = x.dims().to_vec();
        let Some((&k, lead)) = dims.split_last() else {
            candle_core::bail!("Q8 product of a scalar");
        };
        if k != self.k {
            candle_core::bail!("Q8 product: input width {k}, weight width {}", self.k);
        }
        let m: usize = lead.iter().product();
        let xs = x.flatten_all()?.to_dtype(candle_core::DType::F32)?.to_vec1::<f32>()?;
        let kb = self.k / QK;
        let mut xd = vec![0f32; m * kb];
        let mut xq = vec![0i8; m * self.k];
        quantize_rows(&xs, &mut xd, &mut xq);
        let mut out = vec![0f32; m * self.n];
        // ROWS input rows per pass over the weight
        for r0 in (0..m).step_by(ROWS) {
            let rows = ROWS.min(m - r0);
            let xd = xd.get(r0 * kb..(r0 + rows) * kb).unwrap_or_default();
            let xq = xq.get(r0 * self.k..(r0 + rows) * self.k).unwrap_or_default();
            // column-major output: task c fills weight rows c·COLS … for every input row
            let mut out_t = vec![0f32; self.n * rows];
            out_t.par_chunks_mut(COLS * rows).enumerate().for_each(|(c, part)| {
                for (j, col) in part.chunks_mut(rows).enumerate() {
                    let row = c * COLS + j;
                    let (Some(wd), Some(wq)) = (self.d.get(row * kb..(row + 1) * kb), self.qs.get(row * self.k..(row + 1) * self.k)) else {
                        continue;
                    };
                    dots(wd, wq, xd, xq, col);
                }
            });
            for (col, vals) in out_t.chunks(rows).enumerate() {
                for (r, v) in vals.iter().enumerate() {
                    if let Some(o) = out.get_mut((r0 + r) * self.n + col) {
                        *o = *v;
                    }
                }
            }
        }
        let mut shape = lead.to_vec();
        shape.push(self.n);
        Tensor::from_vec(out, shape, x.device())
    }
}

/// One weight row (`wd` scales, `wq` values) dotted with each of `out.len()` input rows (`xd`,
/// `xq` one after another).
fn dots(wd: &[f32], wq: &[i8], xd: &[f32], xq: &[i8], out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if let Some(simd) = pulp::x86::V3::try_new() {
        simd.vectorize(|| avx2::dots(simd, wd, wq, xd, xq, out));
        return;
    }
    dots_scalar(wd, wq, xd, xq, out);
}

fn dots_scalar(wd: &[f32], wq: &[i8], xd: &[f32], xq: &[i8], out: &mut [f32]) {
    let (kb, k) = (wd.len(), wq.len());
    for (r, o) in out.iter_mut().enumerate() {
        let (Some(rd), Some(rq)) = (xd.get(r * kb..(r + 1) * kb), xq.get(r * k..(r + 1) * k)) else { continue };
        let mut acc = 0f32;
        for (((w, x), &a), &b) in wq.chunks(QK).zip(rq.chunks(QK)).zip(wd).zip(rd) {
            let s: i32 = w.iter().zip(x).map(|(&p, &q)| i32::from(p) * i32::from(q)).sum();
            acc += s as f32 * a * b;
        }
        *o = acc;
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::{__m256, __m256i};

    use pulp::x86::V3;

    use super::QK;

    #[inline(always)]
    fn load(s: &[i8]) -> __m256i {
        let a: [i8; 32] = s.get(..32).and_then(|s| s.try_into().ok()).unwrap_or([0; 32]);
        pulp::cast(a)
    }

    #[inline(always)]
    fn hsum(simd: V3, v: __m256) -> f32 {
        let a: [f32; 8] = pulp::cast(v);
        let _ = simd;
        a.iter().sum()
    }

    /// [`super::dots`] with AVX2: per block |w|, sign(x, w), unsigned × signed byte products
    /// summed in pairs, pairs summed to 32-bit, scaled and accumulated (four accumulators, so
    /// consecutive blocks do not wait on each other). The weight row (2–9 KB) stays in the
    /// first-level cache while the rows go over it one after another.
    #[inline(always)]
    pub fn dots(simd: V3, wd: &[f32], wq: &[i8], xd: &[f32], xq: &[i8], out: &mut [f32]) {
        let (kb, k) = (wd.len(), wq.len());
        let ones = simd.avx._mm256_set1_epi16(1);
        let block = |w: &[i8], x: &[i8], s: f32, acc: __m256| -> __m256 {
            let w = load(w);
            let aw = simd.avx2._mm256_sign_epi8(w, w);
            let sx = simd.avx2._mm256_sign_epi8(load(x), w);
            let p = simd.avx2._mm256_madd_epi16(simd.avx2._mm256_maddubs_epi16(aw, sx), ones);
            simd.fma._mm256_fmadd_ps(simd.avx._mm256_set1_ps(s), simd.avx._mm256_cvtepi32_ps(p), acc)
        };
        for (r, o) in out.iter_mut().enumerate() {
            let (Some(rd), Some(rq)) = (xd.get(r * kb..(r + 1) * kb), xq.get(r * k..(r + 1) * k)) else { continue };
            let mut acc = [simd.avx._mm256_setzero_ps(); 4];
            let (w4, x4) = (wq.chunks_exact(4 * QK), rq.chunks_exact(4 * QK));
            let (wt, xt) = (w4.remainder(), x4.remainder());
            for (((w, x), d), e) in w4.zip(x4).zip(wd.chunks_exact(4)).zip(rd.chunks_exact(4)) {
                for (j, a) in acc.iter_mut().enumerate() {
                    let (Some(wj), Some(xj)) = (w.get(j * QK..(j + 1) * QK), x.get(j * QK..(j + 1) * QK)) else { continue };
                    *a = block(wj, xj, d.get(j).copied().unwrap_or(0.0) * e.get(j).copied().unwrap_or(0.0), *a);
                }
            }
            let done = (k - wt.len()) / QK;
            for (((w, x), d), e) in wt.chunks_exact(QK).zip(xt.chunks_exact(QK)).zip(wd.get(done..).unwrap_or_default()).zip(rd.get(done..).unwrap_or_default())
            {
                acc[0] = block(w, x, d * e, acc[0]);
            }
            let sum = simd.avx._mm256_add_ps(simd.avx._mm256_add_ps(acc[0], acc[1]), simd.avx._mm256_add_ps(acc[2], acc[3]));
            *o = hsum(simd, sum);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_dense_product() {
        let dev = Device::Cpu;
        let w: Vec<f32> = (0..96 * 64).map(|i| ((i * 37 % 101) as f32 - 50.0) / 50.0).collect();
        let w = Tensor::from_vec(w, (96, 64), &dev).unwrap();
        // more rows than one pass takes
        let rows = ROWS + 3;
        let x: Vec<f32> = (0..rows * 64).map(|i| ((i * 13 % 29) as f32 - 14.0) / 14.0).collect();
        let x = Tensor::from_vec(x, (rows, 1, 64), &dev).unwrap();
        let q = Q8::quantize(&w).unwrap();
        let got = q.forward(&x).unwrap();
        assert_eq!(got.dims(), &[rows, 1, 96]);
        let want = x.broadcast_matmul(&w.t().unwrap()).unwrap();
        let (g, wv) = (got.flatten_all().unwrap().to_vec1::<f32>().unwrap(), want.flatten_all().unwrap().to_vec1::<f32>().unwrap());
        let err = g.iter().zip(&wv).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        let scale = wv.iter().map(|v| v.abs()).fold(0f32, f32::max);
        assert!(err < 0.02 * scale, "error {err} of {scale}");
        // one row at a time gives the same numbers as all rows together
        let one = q.forward(&x.narrow(0, 1, 1).unwrap()).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(one, g[96..192].to_vec());
        assert!(Q8::quantize(&Tensor::zeros((2, 33), candle_core::DType::F32, &dev).unwrap()).is_err());
        let d = q.dequantize(&dev).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        let derr = d.iter().zip(w.flatten_all().unwrap().to_vec1::<f32>().unwrap()).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(derr < 0.01, "dequantized error {derr}");
    }

    #[test]
    fn simd_and_scalar_agree() {
        let k = 128;
        let mut wv = vec![0f32; 3 * k];
        for (i, v) in wv.iter_mut().enumerate() {
            *v = ((i * 7919 % 255) as f32 - 127.0) / 31.0;
        }
        let q = Q8::quantize(&Tensor::from_vec(wv, (3, k), &Device::Cpu).unwrap()).unwrap();
        let x: Vec<f32> = (0..5 * k).map(|i| ((i * 104_729 % 251) as f32 - 125.0) / 17.0).collect();
        let (mut xd, mut xq) = (vec![0f32; 5 * k / QK], vec![0i8; 5 * k]);
        quantize_rows(&x, &mut xd, &mut xq);
        for row in 0..3 {
            let (wd, wq) = (&q.d[row * k / QK..(row + 1) * k / QK], &q.qs[row * k..(row + 1) * k]);
            let (mut a, mut b) = (vec![0f32; 5], vec![0f32; 5]);
            dots(wd, wq, &xd, &xq, &mut a);
            dots_scalar(wd, wq, &xd, &xq, &mut b);
            for (p, s) in a.iter().zip(&b) {
                assert!((p - s).abs() <= 1e-3 * s.abs().max(1.0), "{p} vs {s}");
            }
        }
    }
}

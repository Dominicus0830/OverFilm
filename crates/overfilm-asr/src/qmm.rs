//! 8-bit (Q8_0) weights on the CPU with one product for several input rows: each weight row is
//! read once and dotted with every input row while it is in the cache. candle's own 8-bit product
//! walks the whole weight once per input row, so decoding four chunks side by side cost four full
//! passes over 2 GB; here it costs one.

use candle_core::quantized::k_quants::{BlockQ8_0, GgmlType};
use candle_core::{Device, Result, Tensor};
use rayon::prelude::*;

/// Values per Q8_0 block.
const QK: usize = 32;
/// Weight rows per parallel task.
const COLS: usize = 32;

/// An `n × k` weight in Q8_0 blocks (row-major, `k / 32` blocks per row).
pub struct Q8 {
    n: usize,
    k: usize,
    blocks: Vec<BlockQ8_0>,
}

impl Q8 {
    /// Quantize an `(n, k)` f32 weight; `k` must be a multiple of 32.
    pub fn quantize(w: &Tensor) -> Result<Self> {
        let (n, k) = w.dims2()?;
        if k == 0 || !k.is_multiple_of(QK) {
            candle_core::bail!("Q8_0 needs a width divisible by {QK}, got {k}");
        }
        let data = w.flatten_all()?.to_vec1::<f32>()?;
        let mut blocks = vec![BlockQ8_0::zeros(); n * (k / QK)];
        blocks.par_chunks_mut(k / QK).zip(data.par_chunks(k)).for_each(|(b, row)| BlockQ8_0::from_float(row, b));
        Ok(Self { n, k, blocks })
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.n, self.k)
    }

    /// The weight back in f32, `(n, k)` (prefill: a dense product beats the 8-bit one there).
    pub fn dequantize(&self, dev: &Device) -> Result<Tensor> {
        let mut out = vec![0f32; self.n * self.k];
        BlockQ8_0::to_float(&self.blocks, &mut out);
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
        let mut xq = vec![BlockQ8_0::zeros(); m * kb];
        BlockQ8_0::from_float(&xs, &mut xq);
        // column-major output: task c fills columns c·COLS … for every row
        let mut out_t = vec![0f32; self.n * m];
        out_t.par_chunks_mut(COLS * m.max(1)).enumerate().for_each(|(c, part)| {
            for (j, col) in part.chunks_mut(m.max(1)).enumerate() {
                let row = c * COLS + j;
                let Some(w) = self.blocks.get(row * kb..(row + 1) * kb) else { continue };
                for (r, v) in col.iter_mut().enumerate() {
                    if let Some(xr) = xq.get(r * kb..(r + 1) * kb) {
                        *v = BlockQ8_0::vec_dot(self.k, w, xr);
                    }
                }
            }
        });
        let mut out = vec![0f32; m * self.n];
        for (col, vals) in out_t.chunks(m.max(1)).enumerate() {
            for (r, v) in vals.iter().enumerate() {
                if let Some(o) = out.get_mut(r * self.n + col) {
                    *o = *v;
                }
            }
        }
        let mut shape = lead.to_vec();
        shape.push(self.n);
        Tensor::from_vec(out, shape, x.device())
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
        let x: Vec<f32> = (0..3 * 64).map(|i| ((i * 13 % 29) as f32 - 14.0) / 14.0).collect();
        let x = Tensor::from_vec(x, (3, 1, 64), &dev).unwrap();
        let q = Q8::quantize(&w).unwrap();
        let got = q.forward(&x).unwrap();
        assert_eq!(got.dims(), &[3, 1, 96]);
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
        assert_eq!(d.len(), 96 * 64);
    }
}

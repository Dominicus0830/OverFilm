//! The overlap network (feature `qwen`): pyannote segmentation-3.0 (MIT), read from the ONNX
//! export published without a sign-in at `onnx-community/pyannote-segmentation-3.0`.
//!
//! It takes 10 s of 16 kHz audio and gives, every [`FRAME_SAMPLES`] samples (about 17 ms), the
//! probability of each of 7 "powerset" classes over three local speakers: nobody, one of A/B/C,
//! or two of them at once ([`CLASSES`]). Two at once is overlapped speech.
//!
//! Layers: SincNet (instance-normalised waveform, 80 band-pass filters of 251 taps at stride 10,
//! |·|, max-pool 3, instance norm, leaky ReLU; then two 5-tap convolutions with the same pooling
//! and norm), four bidirectional LSTM layers of 128, two linear layers of 128 with leaky ReLU
//! and the 7-way classifier with log-softmax. Convolutions and the LSTM input products run on
//! candle; the LSTM recurrence is plain Rust.
//!
//! Only the weights are read from the ONNX file (its `initializer` tensors, through a small
//! protobuf reader that bounds every length); the graph itself is fixed here.

#[cfg(feature = "qwen")]
pub use network::SegNet;

/// Samples per input chunk (10 s).
pub const CHUNK_SAMPLES: usize = 160_000;
/// Output frames per full chunk.
pub const CHUNK_FRAMES: usize = 589;
/// Samples between output frames (stride 10 × 3 × 3 × 3).
pub const FRAME_SAMPLES: usize = 270;
/// Samples the first output frame's receptive field spans.
pub const RECEPTIVE_SAMPLES: usize = 991;
/// Local speakers each class says are talking.
pub const CLASSES: [&[usize]; 7] = [&[], &[0], &[1], &[2], &[0, 1], &[0, 2], &[1, 2]];
/// Local speakers per chunk.
pub const LOCAL: usize = 3;

#[cfg(feature = "qwen")]
const HIDDEN: usize = 128;
#[cfg(feature = "qwen")]
const LEAK: f32 = 0.01;
#[cfg(feature = "qwen")]
const EPS: f32 = 1e-5;
/// Largest model file read (the published one is 6 MB).
#[cfg(feature = "qwen")]
const MAX_FILE: u64 = 64 << 20;

/// Output frames for `n` input samples (0 when too short).
pub fn frames_for(n: usize) -> usize {
    let conv = |n: usize, k: usize, s: usize| if n < k { 0 } else { (n - k) / s + 1 };
    let mut t = conv(n, 251, 10) / 3;
    t = conv(t, 5, 1) / 3;
    conv(t, 5, 1) / 3
}

/// Centre of output frame `f` in samples from the chunk start.
pub fn frame_centre(f: usize) -> usize {
    f.saturating_mul(FRAME_SAMPLES).saturating_add(RECEPTIVE_SAMPLES / 2)
}

#[cfg(feature = "qwen")]
mod network {
    use std::collections::HashMap;
    use std::path::Path;

    use candle_core::{Device, Tensor};

    use super::{CHUNK_SAMPLES, CLASSES, EPS, HIDDEN, LEAK, MAX_FILE};
    use crate::AsrError;

    // ---- protobuf: just enough of ONNX's ModelProto → GraphProto → TensorProto ----

    struct Pb<'a> {
        b: &'a [u8],
        pos: usize,
    }

    enum Field<'a> {
        Varint(u64),
        Bytes(&'a [u8]),
        Fixed,
    }

    impl<'a> Pb<'a> {
        fn new(b: &'a [u8]) -> Self {
            Self { b, pos: 0 }
        }

        fn varint(&mut self) -> Option<u64> {
            let mut v = 0u64;
            for shift in (0..64).step_by(7) {
                let byte = *self.b.get(self.pos)?;
                self.pos += 1;
                v |= u64::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    return Some(v);
                }
            }
            None
        }

        fn take(&mut self, n: usize) -> Option<&'a [u8]> {
            let end = self.pos.checked_add(n)?;
            let s = self.b.get(self.pos..end)?;
            self.pos = end;
            Some(s)
        }

        /// Next (field number, value); `None` at the end, `Some(Err)` on a malformed message.
        fn next(&mut self) -> Option<Result<(u64, Field<'a>), ()>> {
            if self.pos >= self.b.len() {
                return None;
            }
            let Some(key) = self.varint() else { return Some(Err(())) };
            let f = match key & 7 {
                0 => self.varint().map(Field::Varint),
                1 => self.take(8).map(|_| Field::Fixed),
                2 => self.varint().and_then(|n| usize::try_from(n).ok()).and_then(|n| self.take(n)).map(Field::Bytes),
                5 => self.take(4).map(|_| Field::Fixed),
                _ => None,
            };
            Some(f.map(|f| (key >> 3, f)).ok_or(()))
        }
    }

    /// Float tensors among a graph's initializers: name → (dims, values).
    type Tensors = HashMap<String, (Vec<usize>, Vec<f32>)>;

    fn bad(what: &str) -> AsrError {
        AsrError::Model(format!("overlap model: {what}"))
    }

    pub(super) fn tensor(b: &[u8]) -> Result<Option<(String, Vec<usize>, Vec<f32>)>, AsrError> {
        let (mut dims, mut dtype, mut name, mut raw, mut floats) = (Vec::new(), 0u64, String::new(), None, Vec::new());
        let mut pb = Pb::new(b);
        while let Some(f) = pb.next() {
            match f.map_err(|()| bad("bad tensor"))? {
                (1, Field::Varint(d)) => dims.push(usize::try_from(d).map_err(|_| bad("dimension"))?),
                (1, Field::Bytes(p)) => {
                    let mut q = Pb::new(p);
                    while q.pos < p.len() {
                        let d = q.varint().ok_or_else(|| bad("dimension"))?;
                        dims.push(usize::try_from(d).map_err(|_| bad("dimension"))?);
                    }
                }
                (2, Field::Varint(t)) => dtype = t,
                (4, Field::Bytes(p)) => floats.extend(p.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c))),
                (8, Field::Bytes(s)) => name = String::from_utf8_lossy(s).into_owned(),
                (9, Field::Bytes(r)) => raw = Some(r),
                _ => {}
            }
        }
        // 1 = FLOAT; the integer constants of the graph are not needed
        if dtype != 1 {
            return Ok(None);
        }
        if let Some(r) = raw {
            floats = r.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        }
        let n = dims.iter().try_fold(1usize, |a, &d| a.checked_mul(d)).ok_or_else(|| bad("size"))?;
        if n != floats.len() {
            return Err(bad(&format!("{name}: {} values for {dims:?}", floats.len())));
        }
        Ok(Some((name, dims, floats)))
    }

    /// The float initializers of an ONNX model.
    fn initializers(model: &[u8]) -> Result<Tensors, AsrError> {
        let mut out = Tensors::new();
        let mut pb = Pb::new(model);
        while let Some(f) = pb.next() {
            // ModelProto.graph = 7
            if let (7, Field::Bytes(g)) = f.map_err(|()| bad("not an ONNX model"))? {
                let mut gp = Pb::new(g);
                while let Some(f) = gp.next() {
                    // GraphProto.initializer = 5
                    if let (5, Field::Bytes(t)) = f.map_err(|()| bad("bad graph"))?
                        && let Some((name, dims, v)) = tensor(t)?
                    {
                        out.insert(name, (dims, v));
                    }
                }
            }
        }
        Ok(out)
    }

    // ---- the network ----

    struct Lstm {
        /// (2·4H, in): forward then backward, gates i, o, f, c (ONNX order)
        w: Tensor,
        /// per direction (4H × H), row-major
        r: [Vec<f32>; 2],
        /// per direction 4H (input and recurrent biases summed)
        b: [Vec<f32>; 2],
    }

    pub struct SegNet {
        wav_norm: (f32, f32),
        sinc: Tensor,
        convs: [(Tensor, Tensor); 2],
        norms: [(Vec<f32>, Vec<f32>); 3],
        lstm: Vec<Lstm>,
        /// (in, out) row-major and bias
        linear: [(Vec<f32>, Vec<f32>, usize, usize); 3],
        dev: Device,
    }

    fn take(t: &mut Tensors, name: &str, dims: &[usize]) -> Result<Vec<f32>, AsrError> {
        let (d, v) = t.remove(name).ok_or_else(|| bad(&format!("lacks {name}")))?;
        if d != dims {
            return Err(bad(&format!("{name} is {d:?}, expected {dims:?}")));
        }
        Ok(v)
    }

    /// Instance norm over time per channel of a (C, T) row-major buffer, with affine `(w, b)`.
    fn instance_norm(x: &mut [f32], t: usize, w: &[f32], b: &[f32]) {
        if t == 0 {
            return;
        }
        for (row, (&g, &be)) in x.chunks_mut(t).zip(w.iter().zip(b)) {
            let mean = row.iter().map(|&v| f64::from(v)).sum::<f64>() / t as f64;
            let var = row.iter().map(|&v| (f64::from(v) - mean).powi(2)).sum::<f64>() / t as f64;
            let inv = 1.0 / (var + f64::from(EPS)).sqrt();
            for v in row.iter_mut() {
                *v = ((f64::from(*v) - mean) * inv) as f32 * g + be;
            }
        }
    }

    /// Max-pool 3 / stride 3 over time of (C, T) → (C, T / 3).
    fn pool3(x: &[f32], c: usize, t: usize) -> (Vec<f32>, usize) {
        let to = t / 3;
        let mut out = Vec::with_capacity(c * to);
        for row in x.chunks(t.max(1)).take(c) {
            out.extend(row.as_chunks::<3>().0.iter().take(to).map(|w| w[0].max(w[1]).max(w[2])));
        }
        (out, to)
    }

    fn leaky(v: &mut [f32]) {
        for x in v {
            if *x < 0.0 {
                *x *= LEAK;
            }
        }
    }

    fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
    }

    impl SegNet {
        /// Load `model.onnx` (see [`crate::catalogue::SEGMENT`]).
        pub fn load(path: &Path) -> Result<Self, AsrError> {
            let len = std::fs::metadata(path)?.len();
            if len > MAX_FILE {
                return Err(bad("file too large"));
            }
            Self::from_bytes(&std::fs::read(path)?)
        }

        pub fn from_bytes(model: &[u8]) -> Result<Self, AsrError> {
            let mut t = initializers(model)?;
            let dev = Device::Cpu;
            let wn = take(&mut t, "sincnet.wav_norm1d.weight", &[1])?;
            let wb = take(&mut t, "sincnet.wav_norm1d.bias", &[1])?;
            let sinc = Tensor::from_vec(take(&mut t, "/sincnet/conv1d.0/Concat_2_output_0", &[80, 1, 251])?, (80, 1, 251), &dev)?;
            let conv = |t: &mut Tensors, i: usize, cin: usize| -> Result<(Tensor, Tensor), AsrError> {
                let w = take(t, &format!("sincnet.conv1d.{i}.weight"), &[60, cin, 5])?;
                let b = take(t, &format!("sincnet.conv1d.{i}.bias"), &[60])?;
                Ok((Tensor::from_vec(w, (60, cin, 5), &dev)?, Tensor::from_vec(b, (1, 60, 1), &dev)?))
            };
            let convs = [conv(&mut t, 1, 80)?, conv(&mut t, 2, 60)?];
            let norm = |t: &mut Tensors, i: usize, c: usize| -> Result<(Vec<f32>, Vec<f32>), AsrError> {
                Ok((take(t, &format!("sincnet.norm1d.{i}.weight"), &[c])?, take(t, &format!("sincnet.norm1d.{i}.bias"), &[c])?))
            };
            let norms = [norm(&mut t, 0, 80)?, norm(&mut t, 1, 60)?, norm(&mut t, 2, 60)?];
            // the LSTM tensors are named by graph position: biases, W, R in steps of 43
            let mut lstm = Vec::new();
            for (layer, base) in [783usize, 826, 869, 912].into_iter().enumerate() {
                let input = if layer == 0 { 60 } else { 2 * HIDDEN };
                let b = take(&mut t, &format!("onnx::LSTM_{base}"), &[2, 8 * HIDDEN])?;
                let w = take(&mut t, &format!("onnx::LSTM_{}", base + 1), &[2, 4 * HIDDEN, input])?;
                let r = take(&mut t, &format!("onnx::LSTM_{}", base + 2), &[2, 4 * HIDDEN, HIDDEN])?;
                let (r0, r1) = r.split_at(4 * HIDDEN * HIDDEN);
                let bias = |d: usize| -> Vec<f32> {
                    let s = b.get(d * 8 * HIDDEN..(d + 1) * 8 * HIDDEN).unwrap_or(&[]);
                    let (wb, rb) = s.split_at(s.len() / 2);
                    wb.iter().zip(rb).map(|(a, c)| a + c).collect()
                };
                lstm.push(Lstm { w: Tensor::from_vec(w, (8 * HIDDEN, input), &dev)?, r: [r0.to_vec(), r1.to_vec()], b: [bias(0), bias(1)] });
            }
            let lin = |t: &mut Tensors, w: &str, b: &str, i: usize, o: usize| -> Result<(Vec<f32>, Vec<f32>, usize, usize), AsrError> {
                Ok((take(t, w, &[i, o])?, take(t, b, &[o])?, i, o))
            };
            let linear = [
                lin(&mut t, "onnx::MatMul_915", "linear.0.bias", 2 * HIDDEN, 128)?,
                lin(&mut t, "onnx::MatMul_916", "linear.1.bias", 128, 128)?,
                lin(&mut t, "onnx::MatMul_917", "classifier.bias", 128, CLASSES.len())?,
            ];
            let first = |v: Vec<f32>| v.first().copied().unwrap_or(1.0);
            Ok(Self { wav_norm: (first(wn), first(wb)), sinc, convs, norms, lstm, linear, dev })
        }

        /// Class log-probabilities of `audio` (≤ 10 s, 16 kHz; shorter input is zero-padded to
        /// 10 s as in training): [`CHUNK_FRAMES`] rows of 7.
        pub fn forward(&self, audio: &[f32]) -> Result<Vec<[f32; 7]>, AsrError> {
            let mut x = vec![0f32; CHUNK_SAMPLES];
            for (d, s) in x.iter_mut().zip(audio) {
                *d = if s.is_finite() { *s } else { 0.0 };
            }
            instance_norm(&mut x, CHUNK_SAMPLES, &[self.wav_norm.0], &[self.wav_norm.1]);
            // sinc filters
            let y = Tensor::from_vec(x, (1, 1, CHUNK_SAMPLES), &self.dev)?.conv1d(&self.sinc, 0, 10, 1, 1)?;
            let t = y.dim(2)?;
            let mut v: Vec<f32> = y.flatten_all()?.to_vec1()?;
            v.iter_mut().for_each(|a| *a = a.abs());
            let (mut v, mut t) = pool3(&v, 80, t);
            let (w, b) = &self.norms[0];
            instance_norm(&mut v, t, w, b);
            leaky(&mut v);
            let mut c = 80;
            for (i, (cw, cb)) in self.convs.iter().enumerate() {
                let y = Tensor::from_vec(v, (1, c, t), &self.dev)?.conv1d(cw, 0, 1, 1, 1)?.broadcast_add(cb)?;
                let tt = y.dim(2)?;
                let raw: Vec<f32> = y.flatten_all()?.to_vec1()?;
                c = 60;
                let (p, tp) = pool3(&raw, c, tt);
                v = p;
                t = tp;
                let (w, b) = self.norms.get(i + 1).ok_or_else(|| bad("norms"))?;
                instance_norm(&mut v, t, w, b);
                leaky(&mut v);
            }
            // (60, T) → (T, 60)
            let mut h = Tensor::from_vec(v, (c, t), &self.dev)?.t()?.contiguous()?;
            for l in &self.lstm {
                h = self.bilstm(l, &h, t)?;
            }
            let mut z: Vec<f32> = h.flatten_all()?.to_vec1()?;
            let mut width = 2 * HIDDEN;
            for (k, (w, b, i, o)) in self.linear.iter().enumerate() {
                let x = Tensor::from_vec(z, (t, *i), &self.dev)?;
                let wt = Tensor::from_slice(w, (*i, *o), &self.dev)?;
                let bt = Tensor::from_slice(b, (1, *o), &self.dev)?;
                z = x.matmul(&wt)?.broadcast_add(&bt)?.flatten_all()?.to_vec1()?;
                if k < 2 {
                    leaky(&mut z);
                }
                width = *o;
            }
            let mut out = Vec::with_capacity(t);
            for row in z.chunks(width.max(1)) {
                let m = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let lse = m + row.iter().map(|&a| (a - m).exp()).sum::<f32>().ln();
                let mut r = [0f32; 7];
                for (d, &a) in r.iter_mut().zip(row) {
                    *d = a - lse;
                }
                out.push(r);
            }
            Ok(out)
        }

        /// One bidirectional layer over (T, in) → (T, 2H).
        fn bilstm(&self, l: &Lstm, x: &Tensor, t: usize) -> Result<Tensor, AsrError> {
            // input products for both directions at once: (T, 8H)
            let xi: Vec<f32> = x.matmul(&l.w.t()?)?.flatten_all()?.to_vec1()?;
            let g4 = 4 * HIDDEN;
            let mut out = vec![0f32; t * 2 * HIDDEN];
            for d in 0..2 {
                let (r, b) = (&l.r[d], &l.b[d]);
                let mut h = vec![0f32; HIDDEN];
                let mut c = vec![0f32; HIDDEN];
                let mut g = vec![0f32; g4];
                for step in 0..t {
                    let s = if d == 0 { step } else { t - 1 - step };
                    let xs = xi.get(s * 2 * g4 + d * g4..s * 2 * g4 + (d + 1) * g4).ok_or_else(|| bad("lstm input"))?;
                    for (j, gj) in g.iter_mut().enumerate() {
                        let row = r.get(j * HIDDEN..(j + 1) * HIDDEN).unwrap_or(&[]);
                        *gj = xs.get(j).copied().unwrap_or(0.0) + b.get(j).copied().unwrap_or(0.0) + row.iter().zip(&h).map(|(a, b)| a * b).sum::<f32>();
                    }
                    let (gi, rest) = g.split_at(HIDDEN);
                    let (go, rest) = rest.split_at(HIDDEN);
                    let (gf, gc) = rest.split_at(HIDDEN);
                    for k in 0..HIDDEN {
                        let (i, o, f, cc) = (sigmoid(gi[k]), sigmoid(go[k]), sigmoid(gf[k]), gc[k].tanh());
                        c[k] = f * c[k] + i * cc;
                        h[k] = o * c[k].tanh();
                    }
                    if let Some(dst) = out.get_mut(s * 2 * HIDDEN + d * HIDDEN..s * 2 * HIDDEN + (d + 1) * HIDDEN) {
                        dst.copy_from_slice(&h);
                    }
                }
            }
            Ok(Tensor::from_vec(out, (t, 2 * HIDDEN), &self.dev)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_count_matches_the_model() {
        assert_eq!(frames_for(CHUNK_SAMPLES), CHUNK_FRAMES);
        assert_eq!(frames_for(0), 0);
        assert_eq!(frames_for(250), 0);
    }

    #[cfg(feature = "qwen")]
    #[test]
    fn hostile_protobuf_is_an_error_not_a_crash() {
        for b in [&[0xffu8; 12][..], &[0x3a, 0xff, 0xff, 0xff, 0xff, 0x0f], &[0x3a, 0x02, 0x2a, 0x7f], &[0x0b], &[]] {
            assert!(SegNet::from_bytes(b).is_err());
        }
        // a tensor whose dims and data disagree
        let t = [0x08, 0x05, 0x10, 0x01, 0x42, 0x01, b'a', 0x4a, 0x04, 0, 0, 0, 0];
        assert!(network::tensor(&t).is_err());
        let ok = [0x08, 0x01, 0x10, 0x01, 0x42, 0x01, b'a', 0x4a, 0x04, 0, 0, 0x80, 0x3f];
        assert_eq!(network::tensor(&ok).ok().flatten().map(|x| x.2), Some(vec![1.0]));
    }
}

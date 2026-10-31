//! The speaker network (feature `qwen`: candle on the CPU): WeSpeaker ResNet34 trained on
//! VoxCeleb (as published by pyannote, CC-BY-4.0). Batch normalisation is folded into the
//! convolutions when loading.
//!
//! [`Net::frame_map`] runs the convolutional part over a block of features once; any time range
//! of the block then gets a 256-value voice vector from [`Net::embed`] (mean and standard
//! deviation over the range's frames, then the linear layer), so overlapping windows cost almost
//! nothing extra.

use std::collections::HashMap;
use std::path::Path;

use candle_core::{DType, Device, Tensor};

use super::fbank::N_MELS;
use crate::AsrError;

/// Size of a voice vector.
pub const EMBED: usize = 256;
/// Feature frames per output frame (three stride-2 stages).
pub const DOWN: usize = 8;
const BN_EPS: f64 = 1e-5;

struct Conv {
    w: Tensor,
    b: Tensor,
    stride: usize,
    pad: usize,
}

impl Conv {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        x.conv2d(&self.w, self.pad, self.stride, 1, 1)?.broadcast_add(&self.b)
    }
}

struct Block {
    c1: Conv,
    c2: Conv,
    short: Option<Conv>,
}

impl Block {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let y = self.c1.forward(x)?.relu()?;
        let y = self.c2.forward(&y)?;
        let s = match &self.short {
            Some(c) => c.forward(x)?,
            None => x.clone(),
        };
        (y + s)?.relu()
    }
}

pub struct Net {
    stem: Conv,
    blocks: Vec<Block>,
    /// (EMBED, 5120)
    seg_w: Vec<f32>,
    seg_b: Vec<f32>,
    dev: Device,
}

/// Output channels × frequency rows of the last stage (256 × 10).
pub const FEAT: usize = 256 * (N_MELS / DOWN);

struct Weights {
    t: HashMap<String, Tensor>,
}

impl Weights {
    fn get(&self, name: &str) -> Result<Tensor, AsrError> {
        let t = self.t.get(name).ok_or_else(|| AsrError::Model(format!("speaker model lacks {name}")))?;
        Ok(t.to_dtype(DType::F32)?)
    }

    /// Convolution `name` with batch norm `bn` folded in.
    fn conv(&self, name: &str, bn: &str, stride: usize, pad: usize) -> Result<Conv, AsrError> {
        let w = self.get(&format!("{name}.weight"))?;
        let gamma = self.get(&format!("{bn}.weight"))?;
        let beta = self.get(&format!("{bn}.bias"))?;
        let mean = self.get(&format!("{bn}.running_mean"))?;
        let var = self.get(&format!("{bn}.running_var"))?;
        let scale = (gamma / (var + BN_EPS)?.sqrt()?)?;
        let out = scale.dim(0)?;
        if w.dims().first() != Some(&out) || w.rank() != 4 {
            return Err(AsrError::Model(format!("speaker model: unexpected shape of {name}")));
        }
        let w = w.broadcast_mul(&scale.reshape((out, 1, 1, 1))?)?;
        let b = (beta - (mean * &scale)?)?.reshape((1, out, 1, 1))?;
        Ok(Conv { w, b, stride, pad })
    }
}

impl Net {
    /// Load `pytorch_model.bin` (a PyTorch checkpoint with a `state_dict`).
    pub fn load(path: &Path) -> Result<Self, AsrError> {
        let err = |e: candle_core::Error| AsrError::Model(format!("{}: {e}", path.display()));
        let pth = candle_core::pickle::PthTensors::new(path, Some("state_dict")).map_err(err)?;
        let mut t = HashMap::new();
        let names: Vec<String> = pth.tensor_infos().keys().filter(|k| !k.ends_with("num_batches_tracked")).cloned().collect();
        for name in names {
            if let Some(v) = pth.get(&name).map_err(err)? {
                t.insert(name.strip_prefix("resnet.").unwrap_or(&name).to_string(), v);
            }
        }
        let w = Weights { t };
        let stem = w.conv("conv1", "bn1", 1, 1)?;
        let mut blocks = Vec::new();
        for (layer, n, first_stride) in [(1, 3, 1), (2, 4, 2), (3, 6, 2), (4, 3, 2)] {
            for i in 0..n {
                let p = format!("layer{layer}.{i}");
                let stride = if i == 0 { first_stride } else { 1 };
                let short = if w.t.contains_key(&format!("{p}.shortcut.0.weight")) {
                    Some(w.conv(&format!("{p}.shortcut.0"), &format!("{p}.shortcut.1"), stride, 0)?)
                } else {
                    None
                };
                blocks.push(Block {
                    c1: w.conv(&format!("{p}.conv1"), &format!("{p}.bn1"), stride, 1)?,
                    c2: w.conv(&format!("{p}.conv2"), &format!("{p}.bn2"), 1, 1)?,
                    short,
                });
            }
        }
        let seg = w.get("seg_1.weight")?;
        if seg.dims() != [EMBED, 2 * FEAT] {
            return Err(AsrError::Model("speaker model: unexpected seg_1 shape".into()));
        }
        let seg_w = seg.flatten_all()?.to_vec1()?;
        let seg_b = w.get("seg_1.bias")?.to_vec1()?;
        Ok(Self { stem, blocks, seg_w, seg_b, dev: Device::Cpu })
    }

    /// Output frames for `frames` feature frames.
    pub fn out_frames(frames: usize) -> usize {
        frames.div_ceil(DOWN)
    }

    /// The convolutional part over `frames × N_MELS` features (mean already subtracted):
    /// `out_frames × FEAT` values, row-major by output frame.
    pub fn frame_map(&self, feats: &[f32], frames: usize) -> Result<Vec<f32>, AsrError> {
        if frames == 0 || feats.len() != frames * N_MELS {
            return Ok(Vec::new());
        }
        // (1, 1, mels, frames)
        let x = Tensor::from_slice(feats, (frames, N_MELS), &self.dev)?.t()?.reshape((1, 1, N_MELS, frames))?;
        let mut x = self.stem.forward(&x)?.relu()?;
        for b in &self.blocks {
            x = b.forward(&x)?;
        }
        // (1, 256, 10, T') → (T', 2560)
        let t = x.dim(3)?;
        let x = x.reshape((FEAT, t))?.t()?.contiguous()?;
        Ok(x.flatten_all()?.to_vec1()?)
    }

    /// Voice vector of output frames `[a, b)` of a frame map (statistics pooling + linear).
    pub fn embed(&self, map: &[f32], a: usize, b: usize) -> Option<Vec<f32>> {
        let rows = map.get(a * FEAT..b * FEAT)?;
        let n = b.checked_sub(a)?;
        if n == 0 {
            return None;
        }
        let mut mean = vec![0f64; FEAT];
        for r in rows.as_chunks::<FEAT>().0 {
            for (m, &v) in mean.iter_mut().zip(r) {
                *m += f64::from(v);
            }
        }
        for m in &mut mean {
            *m /= n as f64;
        }
        let mut var = vec![0f64; FEAT];
        for r in rows.as_chunks::<FEAT>().0 {
            for ((s, &v), m) in var.iter_mut().zip(r).zip(&mean) {
                let d = f64::from(v) - m;
                *s += d * d;
            }
        }
        // unbiased variance, as in training (a single frame has none)
        let denom = n.saturating_sub(1).max(1) as f64;
        let mut stats = Vec::with_capacity(2 * FEAT);
        stats.extend(mean.iter().map(|&m| m as f32));
        stats.extend(var.iter().map(|&s| ((s / denom) + 1e-7).sqrt() as f32));
        let mut out = self.seg_b.clone();
        for (o, w) in out.iter_mut().zip(self.seg_w.as_chunks::<{ 2 * FEAT }>().0) {
            *o += w.iter().zip(&stats).map(|(a, b)| a * b).sum::<f32>();
        }
        Some(out)
    }
}

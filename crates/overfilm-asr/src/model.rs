//! The Qwen3-ASR network on candle, written from the Apache-2.0 reference implementation
//! (`qwen-asr` 0.0.6, `modeling_qwen3_asr.py`) and the checkpoints' tensor names. Qwen3-ASR and
//! Qwen3-ForcedAligner share it; they differ in sizes and in the output head.
//!
//! Audio encoder: the log-mel input is cut into 100-frame (1 s) chunks; each chunk goes through
//! three 3×3 stride-2 convolutions with GELU (128 mel bins → 16, 100 frames → 13), a linear
//! projection and a sinusoidal position table restarted per chunk. Pre-LayerNorm transformer
//! layers attend within windows of 8 chunks (104 frames). Then LayerNorm, linear, GELU, linear
//! to the text model's width.
//!
//! Text model: Qwen3 decoder (RMSNorm, grouped-query attention with per-head RMSNorm on queries
//! and keys, rotary positions, SwiGLU MLP). For audio-only input the three multimodal rotary
//! sections all carry the same position, which is plain 1-D RoPE.

use std::collections::HashMap;

use std::sync::Arc;

use candle_core::{D, DType, Device, Module, Result, Tensor};
use candle_nn::{Conv2d, Conv2dConfig, Embedding, LayerNorm, Linear, RmsNorm, VarBuilder};
use half::f16;
use half::slice::HalfFloatSliceExt;
use rayon::prelude::*;

use crate::qmm::Q8;

/// Quantized weights by tensor name (`…q_proj.weight`).
pub type QMap = HashMap<String, Arc<Q8>>;

/// A bias-free linear layer, f32 or 8-bit (Q8_0: per-32 block scale, error well below the
/// model's own bf16 rounding; four times less memory traffic per generated token).
pub enum Lin {
    F(Linear),
    Q(Arc<Q8>),
}

impl Lin {
    fn new(i: usize, o: usize, vb: VarBuilder, q: &QMap) -> Result<Self> {
        match q.get(&format!("{}.weight", vb.prefix())) {
            Some(t) => Ok(Lin::Q(t.clone())),
            None => Ok(Lin::F(candle_nn::linear_no_bias(i, o, vb)?)),
        }
    }
}

/// From this many input rows on (prefill), an 8-bit weight is expanded to f32 for the product:
/// the expansion costs less than the 8-bit kernel's per-row work.
const DEQUANT_ROWS: usize = 16;

impl Module for Lin {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let rows = x.elem_count() / x.dims().last().copied().unwrap_or(1).max(1);
        match self {
            Lin::F(l) => l.forward(x),
            Lin::Q(q) if rows >= DEQUANT_ROWS => Linear::new(q.dequantize(x.device())?, None).forward(x),
            Lin::Q(q) => q.forward(x),
        }
    }
}

/// Mel frames per encoder chunk (`n_window * 2`).
const CHUNK: usize = 100;
/// Chunks per attention window (`n_window_infer / (n_window * 2)`).
const WINDOW_CHUNKS: usize = 8;

/// `config.json` → `thinker_config.audio_config`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AudioConfig {
    pub d_model: usize,
    pub encoder_layers: usize,
    pub encoder_attention_heads: usize,
    pub encoder_ffn_dim: usize,
    pub num_mel_bins: usize,
    pub max_source_positions: usize,
    pub downsample_hidden_size: usize,
    pub output_dim: usize,
}

/// `config.json` → `thinker_config.text_config`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct TextConfig {
    pub vocab_size: usize,
    pub hidden_size: usize,
    pub intermediate_size: usize,
    pub num_hidden_layers: usize,
    pub num_attention_heads: usize,
    pub num_key_value_heads: usize,
    pub head_dim: usize,
    pub rms_norm_eps: f64,
    pub rope_theta: f64,
    #[serde(default)]
    pub tie_word_embeddings: bool,
}

/// Output length of the three stride-2 convolutions for `n` input frames.
pub const fn conv_out_len(n: usize) -> usize {
    const fn one(l: usize) -> usize {
        if l == 0 { 0 } else { (l - 1) / 2 + 1 }
    }
    one(one(one(n)))
}

/// Encoder frames for `frames` mel frames (`_get_feat_extract_output_lengths`).
pub fn audio_tokens(frames: usize) -> usize {
    (frames / CHUNK) * conv_out_len(CHUNK) + conv_out_len(frames % CHUNK)
}

fn sinusoids(length: usize, channels: usize, dev: &Device) -> Result<Tensor> {
    let half = channels / 2;
    let inc = (10_000f64).ln() / (half.saturating_sub(1).max(1)) as f64;
    let mut v = vec![0f32; length * channels];
    for t in 0..length {
        for i in 0..half {
            let a = t as f64 * (-inc * i as f64).exp();
            v[t * channels + i] = a.sin() as f32;
            v[t * channels + half + i] = a.cos() as f32;
        }
    }
    Tensor::from_vec(v, (length, channels), dev)
}

/// Values per parallel task in [`gelu`] and [`silu_mul`].
const EW_CHUNK: usize = 1 << 14;

/// gelu (erf form, as candle's `gelu_erf`) on all cores: candle runs element-wise operations on
/// one thread, which made them about a fifth of the encoder's time.
fn gelu(x: &Tensor) -> Result<Tensor> {
    if x.dtype() != DType::F32 || !x.device().is_cpu() || x.elem_count() < EW_CHUNK {
        return x.gelu_erf();
    }
    let mut v = x.flatten_all()?.to_vec1::<f32>()?;
    v.par_chunks_mut(EW_CHUNK).for_each(|c| {
        for e in c {
            *e = (candle_core::cpu::erf::erf_f32(*e * std::f32::consts::FRAC_1_SQRT_2) + 1.0) * 0.5 * *e;
        }
    });
    Tensor::from_vec(v, x.shape(), x.device())
}

/// `silu(g) · u` (the decoder's gated MLP) in one parallel pass.
fn silu_mul(g: &Tensor, u: &Tensor) -> Result<Tensor> {
    if g.dtype() != DType::F32 || u.dtype() != DType::F32 || !g.device().is_cpu() || g.shape() != u.shape() || g.elem_count() < EW_CHUNK {
        return g.silu()? * u;
    }
    let mut a = g.flatten_all()?.to_vec1::<f32>()?;
    let b = u.flatten_all()?.to_vec1::<f32>()?;
    a.par_chunks_mut(EW_CHUNK).zip(b.par_chunks(EW_CHUNK)).for_each(|(a, b)| {
        for (a, &b) in a.iter_mut().zip(b) {
            *a = *a / (1.0 + (-*a).exp()) * b;
        }
    });
    Tensor::from_vec(a, g.shape(), g.device())
}

fn attend(q: &Tensor, k: &Tensor, v: &Tensor, scale: f64, mask: Option<&Tensor>) -> Result<Tensor> {
    let w = scaled_softmax(&q.matmul(&k.t()?)?, scale, mask)?;
    w.matmul(v)
}

/// `softmax(w · scale + mask)` over the last dimension. `mask` (rows, total) applies to every
/// block of `rows` rows of `w` (…, n · rows, total). One parallel pass over `w`; as candle
/// operations the scale and the mask were two single-threaded passes that took longer than the
/// softmax itself.
fn scaled_softmax(w: &Tensor, scale: f64, mask: Option<&Tensor>) -> Result<Tensor> {
    let dims = w.dims().to_vec();
    let (Some(&total), Some(&rows)) = (dims.last(), dims.len().checked_sub(2).and_then(|i| dims.get(i))) else {
        candle_core::bail!("attention scores of shape {dims:?}");
    };
    let mrows = match mask {
        Some(m) => {
            let (mr, mt) = m.dims2()?;
            if mt != total || mr == 0 || !rows.is_multiple_of(mr) {
                candle_core::bail!("mask {:?} for scores {dims:?}", m.dims());
            }
            mr
        }
        None => 1,
    };
    if w.dtype() != DType::F32 || !w.device().is_cpu() {
        let dt = w.dtype();
        let w = (w * scale)?;
        let w = match mask {
            Some(m) => w.broadcast_add(&m.repeat((rows / mrows, 1))?)?,
            None => w,
        };
        return candle_nn::ops::softmax_last_dim(&w.to_dtype(DType::F32)?)?.to_dtype(dt);
    }
    let m = mask.map(|m| m.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()).transpose()?;
    let mut x = w.flatten_all()?.to_vec1::<f32>()?;
    let scale = scale as f32;
    x.par_chunks_mut(total.max(1)).enumerate().for_each(|(r, row)| {
        let mrow = m.as_ref().and_then(|m| m.get((r % mrows) * total..(r % mrows + 1) * total));
        match mrow {
            Some(mr) => row.iter_mut().zip(mr).for_each(|(v, &a)| *v = *v * scale + a),
            None => row.iter_mut().for_each(|v| *v *= scale),
        }
        let max = row.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        for v in row.iter_mut() {
            *v = (*v - max).exp();
        }
        let sum: f32 = row.iter().sum();
        for v in row.iter_mut() {
            *v /= sum;
        }
    });
    Tensor::from_vec(x, dims, w.device())
}

/// [`attend`] for grouped queries: `q` (b, heads, len, d) where each `rep` consecutive heads
/// share one head of `k`/`v` (b, heads / rep, total, d); `mask` (len, total). The query heads of
/// a group become one matrix of `rep · len` rows against their shared keys, rather than the keys
/// being copied `rep` times: larger products that run nearer full speed, and no copies.
fn attend_grouped(q: &Tensor, k: &Tensor, v: &Tensor, rep: usize, scale: f64, mask: Option<&Tensor>) -> Result<Tensor> {
    if rep <= 1 {
        return attend(q, k, v, scale, mask);
    }
    let (b, heads, len, d) = q.dims4()?;
    let qg = q.contiguous()?.reshape((b, heads / rep, rep * len, d))?;
    // the mask's rows repeat for each query head of a group (see [`scaled_softmax`])
    attend(&qg, &k.contiguous()?, &v.contiguous()?, scale, mask)?.reshape((b, heads, len, d))
}

struct EncoderLayer {
    ln1: LayerNorm,
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    ln2: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    heads: usize,
}

impl EncoderLayer {
    fn new(c: &AudioConfig, vb: VarBuilder) -> Result<Self> {
        let d = c.d_model;
        let a = vb.pp("self_attn");
        Ok(Self {
            ln1: candle_nn::layer_norm(d, 1e-5, vb.pp("self_attn_layer_norm"))?,
            q: candle_nn::linear(d, d, a.pp("q_proj"))?,
            k: candle_nn::linear(d, d, a.pp("k_proj"))?,
            v: candle_nn::linear(d, d, a.pp("v_proj"))?,
            o: candle_nn::linear(d, d, a.pp("out_proj"))?,
            ln2: candle_nn::layer_norm(d, 1e-5, vb.pp("final_layer_norm"))?,
            fc1: candle_nn::linear(d, c.encoder_ffn_dim, vb.pp("fc1"))?,
            fc2: candle_nn::linear(c.encoder_ffn_dim, d, vb.pp("fc2"))?,
            heads: c.encoder_attention_heads,
        })
    }

    /// `x`: (n, d); attention stays inside consecutive windows of `window` rows.
    fn forward(&self, x: &Tensor, window: usize) -> Result<Tensor> {
        let (n, d) = x.dims2()?;
        let dh = d / self.heads;
        let h = self.ln1.forward(x)?;
        let split = |t: Tensor| t.reshape((n, self.heads, dh))?.transpose(0, 1)?.contiguous();
        let (q, k, v) = (split(self.q.forward(&h)?)?, split(self.k.forward(&h)?)?, split(self.v.forward(&h)?)?);
        let mut outs = Vec::new();
        let mut start = 0;
        while start < n {
            let len = window.min(n - start);
            let o = attend(&q.narrow(1, start, len)?, &k.narrow(1, start, len)?, &v.narrow(1, start, len)?, (dh as f64).powf(-0.5), None)?;
            outs.push(o);
            start += len;
        }
        let o = Tensor::cat(&outs, 1)?.transpose(0, 1)?.reshape((n, d))?;
        let x = (x + self.o.forward(&o)?)?;
        let h = self.fc2.forward(&gelu(&self.fc1.forward(&self.ln2.forward(&x)?)?)?)?;
        x + h
    }
}

/// The audio encoder's input side: three stride-2 convolutions, the projection and the
/// per-chunk positions (on the CPU in both backends).
pub struct AudioFront {
    conv1: Conv2d,
    conv2: Conv2d,
    conv3: Conv2d,
    conv_out: Linear,
    pos: Tensor,
    n_mels: usize,
}

/// Rows per encoder attention window (`conv_out_len(CHUNK) · WINDOW_CHUNKS`). An input of one
/// short chunk has fewer rows than this, so a window is always at most this long.
pub const ENCODER_WINDOW: usize = conv_out_len(CHUNK) * WINDOW_CHUNKS;

impl AudioFront {
    pub fn new(c: &AudioConfig, vb: VarBuilder) -> Result<Self> {
        let cfg = Conv2dConfig { padding: 1, stride: 2, ..Default::default() };
        let h = c.downsample_hidden_size;
        let freq = conv_out_len(c.num_mel_bins);
        Ok(Self {
            conv1: candle_nn::conv2d(1, h, 3, cfg, vb.pp("conv2d1"))?,
            conv2: candle_nn::conv2d(h, h, 3, cfg, vb.pp("conv2d2"))?,
            conv3: candle_nn::conv2d(h, h, 3, cfg, vb.pp("conv2d3"))?,
            conv_out: candle_nn::linear_no_bias(h * freq, c.d_model, vb.pp("conv_out"))?,
            pos: sinusoids(c.max_source_positions, c.d_model, vb.device())?.to_dtype(vb.dtype())?,
            n_mels: c.num_mel_bins,
        })
    }

    /// `mel`: `n_mels × frames` row-major. Returns (audio_tokens(frames), d_model) and the
    /// attention window in rows.
    pub fn forward(&self, mel: &[f32], frames: usize, dtype: DType, dev: &Device) -> Result<(Tensor, usize)> {
        let chunks = self.chunks(mel, frames)?;
        let x = Tensor::from_vec(chunks.buf.clone(), (chunks.n, 1, self.n_mels, chunks.width), dev)?.to_dtype(dtype)?;
        let x = gelu(&conv3x3s2(&x, &self.conv1)?)?;
        let x = gelu(&conv3x3s2(&x, &self.conv2)?)?;
        let x = gelu(&conv3x3s2(&x, &self.conv3)?)?;
        let (b, c, f, t) = x.dims4()?;
        let x = x.permute((0, 3, 1, 2))?.contiguous()?.reshape((b, t, c * f))?;
        let x = self.conv_out.forward(&x)?;
        self.finish(&x, &chunks.lens)
    }

    /// The mel spectrogram cut into chunks of [`CHUNK`] frames (the last one padded with zeros
    /// when there are several).
    pub fn chunks(&self, mel: &[f32], frames: usize) -> Result<MelChunks> {
        if frames == 0 || mel.len() != self.n_mels * frames {
            return Err(candle_core::Error::Msg(format!("mel has {} values for {frames} frames", mel.len())));
        }
        let n_chunks = frames.div_ceil(CHUNK);
        let padded = if n_chunks > 1 { CHUNK } else { frames };
        let mut buf = vec![0f32; n_chunks * self.n_mels * padded];
        let mut lens = Vec::with_capacity(n_chunks);
        for c in 0..n_chunks {
            let s = c * CHUNK;
            let len = CHUNK.min(frames - s);
            lens.push(len);
            for m in 0..self.n_mels {
                let src = mel.get(m * frames + s..m * frames + s + len).unwrap_or(&[]);
                let dst = (c * self.n_mels + m) * padded;
                if let Some(d) = buf.get_mut(dst..dst + src.len()) {
                    d.copy_from_slice(src);
                }
            }
        }
        Ok(MelChunks { buf, n: n_chunks, width: padded, lens })
    }

    /// After `conv_out` (`x`: (chunks, t, d_model)): add the positions and keep each chunk's
    /// real rows (`lens`: frames per chunk). Returns the rows and the attention window.
    pub fn finish(&self, x: &Tensor, lens: &[usize]) -> Result<(Tensor, usize)> {
        let (_, t, _) = x.dims3()?;
        let x = x.broadcast_add(&self.pos.narrow(0, 0, t)?.unsqueeze(0)?.to_dtype(x.dtype())?)?;
        let mut rows = Vec::with_capacity(lens.len());
        for (i, &len) in lens.iter().enumerate() {
            rows.push(x.get(i)?.narrow(0, 0, conv_out_len(len).min(t))?);
        }
        Ok((Tensor::cat(&rows, 0)?, t * WINDOW_CHUNKS))
    }

    /// The front's weights as plain arrays for the GPU: the three convolutions (weight
    /// `(cout, cin·9)`, bias) and `conv_out` `(d_model, channels · frequencies)`.
    pub fn weights(&self) -> Result<FrontWeights> {
        let flat = |t: &Tensor| -> Result<Vec<f32>> { t.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>() };
        let conv = |c: &Conv2d| -> Result<(Vec<f32>, Vec<f32>, usize, usize)> {
            let (cout, cin, _, _) = c.weight().dims4()?;
            let b = c.bias().map(flat).transpose()?.unwrap_or_else(|| vec![0.0; cout]);
            Ok((flat(c.weight())?, b, cout, cin))
        };
        let (d, k) = self.conv_out.weight().dims2()?;
        Ok(FrontWeights {
            convs: [conv(&self.conv1)?, conv(&self.conv2)?, conv(&self.conv3)?],
            out: flat(self.conv_out.weight())?,
            d_model: d,
            out_in: k,
            n_mels: self.n_mels,
        })
    }
}

/// See [`AudioFront::chunks`]: `(n, 1, n_mels, width)` row-major.
pub struct MelChunks {
    pub buf: Vec<f32>,
    pub n: usize,
    pub width: usize,
    /// real frames per chunk
    pub lens: Vec<usize>,
}

/// See [`AudioFront::weights`]. `convs`: (weight, bias, cout, cin) each.
pub struct FrontWeights {
    pub convs: [(Vec<f32>, Vec<f32>, usize, usize); 3],
    pub out: Vec<f32>,
    pub d_model: usize,
    pub out_in: usize,
    pub n_mels: usize,
}

/// Input positions gathered per product in [`conv3x3s2`] (bounds the gathered copy: 6400
/// positions × 480 channels × 9 taps is 110 MB).
const CONV_POSITIONS: usize = 6400;

/// A 3×3 convolution with stride 2 and padding 1 (the encoder front): `x` (b, cin, h, w) →
/// (b, cout, ⌈h/2⌉, ⌈w/2⌉). The input patches are gathered into rows and multiplied with the
/// kernel in one matrix product per group of chunks; candle's own tiled convolution took 7 s of
/// a 60-second piece's 11 s in the encoder, this takes well under one.
fn conv3x3s2(x: &Tensor, conv: &Conv2d) -> Result<Tensor> {
    let (b, cin, h, w) = x.dims4()?;
    let k = conv.weight();
    let (cout, kin, kh, kw) = k.dims4()?;
    if kin != cin || kh != 3 || kw != 3 {
        candle_core::bail!("conv3x3s2: kernel {:?} for {cin} channels", k.dims());
    }
    let half = |l: usize| l.div_ceil(2);
    let (ho, wo) = (half(h), half(w));
    let xs = x.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()?;
    let km = k.to_dtype(DType::F32)?.reshape((cout, cin * 9))?;
    let bias = conv.bias().map(|b| b.to_dtype(DType::F32)?.reshape((1, cout, 1))).transpose()?;
    let per = (ho * wo).max(1);
    let group = (CONV_POSITIONS / per).max(1);
    let mut outs = Vec::with_capacity(b.div_ceil(group));
    for b0 in (0..b).step_by(group) {
        let nb = group.min(b - b0);
        // (nb, cin·9, positions): one row per input channel and tap, so the product with the
        // (cout, cin·9) kernel comes out in the (cout, positions) order the next layer reads
        let mut cols = vec![0f32; nb * cin * 9 * per];
        cols.par_chunks_mut(per).enumerate().for_each(|(row, line)| {
            let (bi, t) = (b0 + row / (cin * 9), row % (cin * 9));
            let (c, ky, kx) = (t / 9, t % 9 / 3, t % 3);
            let plane = (bi * cin + c) * h * w;
            for (oy, out) in line.chunks_mut(wo.max(1)).enumerate() {
                let Some(iy) = (2 * oy + ky).checked_sub(1).filter(|&v| v < h) else { continue };
                let Some(src) = xs.get(plane + iy * w..plane + (iy + 1) * w) else { continue };
                for (ox, d) in out.iter_mut().enumerate() {
                    if let Some(&v) = (2 * ox + kx).checked_sub(1).and_then(|ix| src.get(ix)) {
                        *d = v;
                    }
                }
            }
        });
        let cols = Tensor::from_vec(cols, (nb, cin * 9, per), x.device())?;
        let mut y = km.broadcast_matmul(&cols)?;
        if let Some(bias) = &bias {
            y = y.broadcast_add(bias)?;
        }
        outs.push(y.reshape((nb, cout, ho, wo))?);
    }
    Tensor::cat(&outs, 0)?.to_dtype(x.dtype())
}

pub struct AudioEncoder {
    front: AudioFront,
    layers: Vec<EncoderLayer>,
    ln_post: LayerNorm,
    proj1: Linear,
    proj2: Linear,
}

impl AudioEncoder {
    pub fn new(c: &AudioConfig, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            front: AudioFront::new(c, vb.clone())?,
            layers: (0..c.encoder_layers).map(|i| EncoderLayer::new(c, vb.pp(format!("layers.{i}")))).collect::<Result<_>>()?,
            ln_post: candle_nn::layer_norm(c.d_model, 1e-5, vb.pp("ln_post"))?,
            proj1: candle_nn::linear(c.d_model, c.d_model, vb.pp("proj1"))?,
            proj2: candle_nn::linear(c.d_model, c.output_dim, vb.pp("proj2"))?,
        })
    }

    /// `mel`: `n_mels × frames` row-major. Returns (audio_tokens(frames), output_dim).
    pub fn forward(&self, mel: &[f32], frames: usize, dtype: DType, dev: &Device) -> Result<Tensor> {
        let (mut h, window) = self.front.forward(mel, frames, dtype, dev)?;
        for l in &self.layers {
            h = l.forward(&h, window)?;
        }
        let h = gelu(&self.proj1.forward(&self.ln_post.forward(&h)?)?)?;
        self.proj2.forward(&h)
    }
}

struct DecoderLayer {
    ln1: RmsNorm,
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    ln2: RmsNorm,
    gate: Lin,
    up: Lin,
    down: Lin,
    heads: usize,
    kv_heads: usize,
    head_dim: usize,
    cache: Option<(Tensor, Tensor)>,
}

impl DecoderLayer {
    fn new(c: &TextConfig, vb: VarBuilder, qm: &QMap) -> Result<Self> {
        let (h, dh) = (c.hidden_size, c.head_dim);
        let a = vb.pp("self_attn");
        let m = vb.pp("mlp");
        Ok(Self {
            ln1: candle_nn::rms_norm(h, c.rms_norm_eps, vb.pp("input_layernorm"))?,
            q: Lin::new(h, c.num_attention_heads * dh, a.pp("q_proj"), qm)?,
            k: Lin::new(h, c.num_key_value_heads * dh, a.pp("k_proj"), qm)?,
            v: Lin::new(h, c.num_key_value_heads * dh, a.pp("v_proj"), qm)?,
            o: Lin::new(c.num_attention_heads * dh, h, a.pp("o_proj"), qm)?,
            q_norm: candle_nn::rms_norm(dh, c.rms_norm_eps, a.pp("q_norm"))?,
            k_norm: candle_nn::rms_norm(dh, c.rms_norm_eps, a.pp("k_norm"))?,
            ln2: candle_nn::rms_norm(h, c.rms_norm_eps, vb.pp("post_attention_layernorm"))?,
            gate: Lin::new(h, c.intermediate_size, m.pp("gate_proj"), qm)?,
            up: Lin::new(h, c.intermediate_size, m.pp("up_proj"), qm)?,
            down: Lin::new(c.intermediate_size, h, m.pp("down_proj"), qm)?,
            heads: c.num_attention_heads,
            kv_heads: c.num_key_value_heads,
            head_dim: dh,
            cache: None,
        })
    }

    fn forward(&mut self, x: &Tensor, cos: &Tensor, sin: &Tensor, mask: Option<&Tensor>, cached: bool) -> Result<Tensor> {
        let (b, t, _) = x.dims3()?;
        let h = self.ln1.forward(x)?;
        let shape = |y: Tensor, n: usize| y.reshape((b, t, n, self.head_dim));
        let q = self.q_norm.forward(&shape(self.q.forward(&h)?, self.heads)?)?.transpose(1, 2)?.contiguous()?;
        let k = self.k_norm.forward(&shape(self.k.forward(&h)?, self.kv_heads)?)?.transpose(1, 2)?.contiguous()?;
        let v = shape(self.v.forward(&h)?, self.kv_heads)?.transpose(1, 2)?.contiguous()?;
        let q = candle_nn::rotary_emb::rope(&q, cos, sin)?;
        let k = candle_nn::rotary_emb::rope(&k, cos, sin)?;
        let (k, v) = match (&self.cache, cached) {
            (Some((pk, pv)), true) => (Tensor::cat(&[pk, &k], 2)?, Tensor::cat(&[pv, &v], 2)?),
            _ => (k, v),
        };
        if cached {
            self.cache = Some((k.clone(), v.clone()));
        }
        let rep = self.heads / self.kv_heads.max(1);
        let o = attend_grouped(&q, &k, &v, rep, (self.head_dim as f64).powf(-0.5), mask)?;
        let o = o.transpose(1, 2)?.reshape((b, t, self.heads * self.head_dim))?;
        let x = (x + self.o.forward(&o)?)?;
        let h = self.ln2.forward(&x)?;
        let m = self.down.forward(&silu_mul(&self.gate.forward(&h)?, &self.up.forward(&h)?)?)?;
        x + m
    }
}

impl DecoderLayer {
    /// One new token for each of several independent sequences: `x` (n, 1, hidden), row `i` at
    /// position `kvs[i].len()` of its own cache. The products run on all rows at once (one pass
    /// over the weights); rotary positions and attention are plain loops over the rows' caches,
    /// in parallel over (row, key/value head): one step needs a handful of small products per row
    /// and layer, and as separate tensor operations their set-up cost was half of each step.
    fn forward_rows(&self, x: &Tensor, inv_freq: &[f32], kvs: &mut [&mut Kv]) -> Result<Tensor> {
        let (n, t, _) = x.dims3()?;
        if t != 1 || kvs.len() != n {
            candle_core::bail!("batched step: {n} rows, {} caches", kvs.len());
        }
        let dh = self.head_dim;
        let (heads, kvh) = (self.heads, self.kv_heads.max(1));
        let h = self.ln1.forward(x)?;
        let shape = |y: Tensor, k: usize| y.reshape((n, 1, k, dh));
        let flat = |y: Tensor| -> Result<Vec<f32>> { y.flatten_all()?.to_dtype(DType::F32)?.to_vec1::<f32>() };
        let mut q = flat(self.q_norm.forward(&shape(self.q.forward(&h)?, heads)?)?)?;
        let mut k = flat(self.k_norm.forward(&shape(self.k.forward(&h)?, kvh)?)?)?;
        let v = flat(shape(self.v.forward(&h)?, kvh)?)?;
        let (qw, kw) = (heads * dh, kvh * dh);
        if q.len() != n * qw || k.len() != n * kw || v.len() != n * kw {
            candle_core::bail!("batched step: projection sizes");
        }
        for (r, kv) in kvs.iter_mut().enumerate() {
            let pos = kv.len(kw);
            if let Some(qr) = q.get_mut(r * qw..(r + 1) * qw) {
                rope_rows(qr, dh, pos, inv_freq);
            }
            if let Some(kr) = k.get_mut(r * kw..(r + 1) * kw) {
                rope_rows(kr, dh, pos, inv_freq);
                kv.k.extend(kr.iter().map(|&x| f16::from_f32(x)));
            }
            if let Some(vr) = v.get(r * kw..(r + 1) * kw) {
                kv.v.extend(vr.iter().map(|&x| f16::from_f32(x)));
            }
        }
        let rep = heads / kvh;
        let scale = (dh as f32).powf(-0.5);
        let mut out = vec![0f32; n * qw];
        let kvs: Vec<&Kv> = kvs.iter().map(|k| &**k).collect();
        out.par_chunks_mut(rep * dh).enumerate().for_each(|(task, o)| {
            let (r, g) = (task / kvh, task % kvh);
            let (Some(kv), Some(qr)) = (kvs.get(r), q.get(r * qw..(r + 1) * qw)) else { return };
            let len = kv.len(kw);
            // the `rep` query heads of this key/value head read each cached position once
            let Some(qg) = qr.get(g * rep * dh..(g + 1) * rep * dh) else { return };
            let mut w = vec![0f32; rep * len];
            let mut buf = vec![0f32; dh];
            for p in 0..len {
                let at = p * kw + g * dh;
                // (always dh long: the cache holds len whole positions)
                let Some(kp) = kv.k.get(at..at + dh) else { continue };
                kp.convert_to_f32_slice(&mut buf);
                for (j, qh) in qg.chunks_exact(dh).enumerate() {
                    if let Some(wp) = w.get_mut(j * len + p) {
                        *wp = dot(qh, &buf) * scale;
                    }
                }
            }
            for wj in w.chunks_mut(len.max(1)) {
                let top = wj.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
                let mut sum = 0f32;
                for wp in wj.iter_mut() {
                    *wp = (*wp - top).exp();
                    sum += *wp;
                }
                let inv = if sum > 0.0 { 1.0 / sum } else { 0.0 };
                for wp in wj.iter_mut() {
                    *wp *= inv;
                }
            }
            for p in 0..len {
                let at = p * kw + g * dh;
                let Some(vp) = kv.v.get(at..at + dh) else { continue };
                vp.convert_to_f32_slice(&mut buf);
                for (j, oh) in o.chunks_mut(dh).enumerate() {
                    let c = w.get(j * len + p).copied().unwrap_or(0.0);
                    for (a, b) in oh.iter_mut().zip(&buf) {
                        *a += c * b;
                    }
                }
            }
        });
        let o = Tensor::from_vec(out, (n, 1, qw), x.device())?.to_dtype(x.dtype())?;
        let x = (x + self.o.forward(&o)?)?;
        let h = self.ln2.forward(&x)?;
        let m = self.down.forward(&silu_mul(&self.gate.forward(&h)?, &self.up.forward(&h)?)?)?;
        x + m
    }
}

/// Dot product in eight running sums, so it vectorises (a single running sum must add in order).
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut s = [0f32; 8];
    let (a8, b8) = (a.chunks_exact(8), b.chunks_exact(8));
    let tail: f32 = a8.remainder().iter().zip(b8.remainder()).map(|(x, y)| x * y).sum();
    for (x, y) in a8.zip(b8) {
        for ((s, x), y) in s.iter_mut().zip(x).zip(y) {
            *s += x * y;
        }
    }
    s.iter().sum::<f32>() + tail
}

/// Rotary positions (the rotate-half form of [`candle_nn::rotary_emb::rope`]) on one row of
/// heads `x` (heads · dh) at position `pos`.
fn rope_rows(x: &mut [f32], dh: usize, pos: usize, inv_freq: &[f32]) {
    let half = dh / 2;
    for head in x.chunks_mut(dh) {
        let (a, b) = head.split_at_mut(half.min(head.len()));
        for ((x1, x2), w) in a.iter_mut().zip(b.iter_mut()).zip(inv_freq) {
            let (sin, cos) = (pos as f32 * w).sin_cos();
            let (u, v) = (*x1, *x2);
            *x1 = u * cos - v * sin;
            *x2 = v * cos + u * sin;
        }
    }
}

impl DecoderLayer {
    /// The prefills of several sequences at once: `x` (1, T, hidden) holds their rows one after
    /// another, `segs[i] = (start, len)`. The products run on all T rows together (an 8-bit
    /// weight is expanded once, not once per sequence); attention runs per sequence, causal,
    /// over its cache (the shared prefix) and its own rows.
    fn forward_segments(&self, x: &Tensor, segs: &[(usize, usize)], rope: &[(Tensor, Tensor)], caches: &mut [Option<(Tensor, Tensor)>]) -> Result<Tensor> {
        let (b, t, _) = x.dims3()?;
        if segs.len() != rope.len() || segs.len() != caches.len() {
            candle_core::bail!("{} segments, {} positions, {} caches", segs.len(), rope.len(), caches.len());
        }
        let h = self.ln1.forward(x)?;
        let shape = |y: Tensor, n: usize| y.reshape((b, t, n, self.head_dim));
        let q = self.q_norm.forward(&shape(self.q.forward(&h)?, self.heads)?)?.transpose(1, 2)?.contiguous()?;
        let k = self.k_norm.forward(&shape(self.k.forward(&h)?, self.kv_heads)?)?.transpose(1, 2)?.contiguous()?;
        let v = shape(self.v.forward(&h)?, self.kv_heads)?.transpose(1, 2)?.contiguous()?;
        let rep = self.heads / self.kv_heads.max(1);
        let scale = (self.head_dim as f64).powf(-0.5);
        let mut outs = Vec::with_capacity(segs.len());
        for ((&(start, len), (cos, sin)), cache) in segs.iter().zip(rope).zip(caches.iter_mut()) {
            let qs = candle_nn::rotary_emb::rope(&q.narrow(2, start, len)?.contiguous()?, cos, sin)?;
            let ks = candle_nn::rotary_emb::rope(&k.narrow(2, start, len)?.contiguous()?, cos, sin)?;
            let vs = v.narrow(2, start, len)?.contiguous()?;
            let (kc, vc) = match cache.as_ref() {
                Some((pk, pv)) => (Tensor::cat(&[pk, &ks], 2)?, Tensor::cat(&[pv, &vs], 2)?),
                None => (ks, vs),
            };
            let total = kc.dim(2)?;
            let past = total - len;
            let mask = if len > 1 {
                let m: Vec<f32> = (0..len).flat_map(|i| (0..total).map(move |j| if j > past + i { f32::NEG_INFINITY } else { 0.0 })).collect();
                Some(Tensor::from_vec(m, (len, total), x.device())?.to_dtype(x.dtype())?)
            } else {
                None
            };
            outs.push(attend_grouped(&qs, &kc, &vc, rep, scale, mask.as_ref())?);
            *cache = Some((kc, vc));
        }
        let o = Tensor::cat(&outs, 2)?.transpose(1, 2)?.reshape((b, t, self.heads * self.head_dim))?;
        let x = (x + self.o.forward(&o)?)?;
        let h = self.ln2.forward(&x)?;
        let m = self.down.forward(&silu_mul(&self.gate.forward(&h)?, &self.up.forward(&h)?)?)?;
        x + m
    }
}

/// One layer's keys and values for [`TextModel::forward_rows`], position after position
/// (`[pos][kv head][head dim]`), so a new token is appended in place. Kept in 16-bit floats
/// (more precision than the model's own bf16): reading them is most of a decoding step's
/// attention time, so half the bytes is half that time.
#[derive(Clone, Default)]
pub struct Kv {
    k: Vec<f16>,
    v: Vec<f16>,
}

impl Kv {
    /// From a cache tensor pair (1, kv heads, positions, head dim).
    fn from_tensors(k: &Tensor, v: &Tensor) -> Result<Self> {
        let flat = |t: &Tensor| -> Result<Vec<f16>> { t.transpose(1, 2)?.contiguous()?.flatten_all()?.to_dtype(DType::F16)?.to_vec1::<f16>() };
        Ok(Self { k: flat(k)?, v: flat(v)? })
    }

    /// Positions held, for rows of `width` (kv heads · head dim) values.
    fn len(&self, width: usize) -> usize {
        self.k.len() / width.max(1)
    }
}

/// One sequence's key/value cache for [`TextModel::forward_rows`]: per layer the keys and values
/// of positions `0 .. pos`.
#[derive(Clone, Default)]
pub struct SeqCache {
    pub layers: Vec<Kv>,
    pub pos: usize,
}

pub struct TextModel {
    embed: Embedding,
    layers: Vec<DecoderLayer>,
    norm: RmsNorm,
    head: Lin,
    inv_freq: Vec<f32>,
    /// tokens in the key/value cache
    pub pos: usize,
}

impl TextModel {
    /// `vb` at `thinker`; `head_rows`: vocabulary (ASR) or timestamp classes (aligner).
    pub fn new(c: &TextConfig, head_rows: usize, vb: VarBuilder, qm: &QMap) -> Result<Self> {
        let m = vb.pp("model");
        let embed = candle_nn::embedding(c.vocab_size, c.hidden_size, m.pp("embed_tokens"))?;
        let head_vb = vb.pp("lm_head");
        let head = if qm.contains_key(&format!("{}.weight", head_vb.prefix())) || vb.contains_tensor("lm_head.weight") {
            Lin::new(c.hidden_size, head_rows, head_vb, qm)?
        } else {
            Lin::F(Linear::new(embed.embeddings().clone(), None))
        };
        let half = c.head_dim / 2;
        let inv_freq = (0..half).map(|i| (1.0 / c.rope_theta.powf(2.0 * i as f64 / c.head_dim as f64)) as f32).collect();
        Ok(Self {
            embed,
            layers: (0..c.num_hidden_layers).map(|i| DecoderLayer::new(c, m.pp(format!("layers.{i}")), qm)).collect::<Result<_>>()?,
            norm: candle_nn::rms_norm(c.hidden_size, c.rms_norm_eps, m.pp("norm"))?,
            head,
            inv_freq,
            pos: 0,
        })
    }

    pub fn embed(&self, ids: &[u32], dev: &Device) -> Result<Tensor> {
        self.embed.forward(&Tensor::new(ids, dev)?)
    }

    /// The key/value cache (to reuse a shared prompt prefix).
    pub fn cache(&self) -> (Vec<Option<(Tensor, Tensor)>>, usize) {
        (self.layers.iter().map(|l| l.cache.clone()).collect(), self.pos)
    }

    pub fn set_cache(&mut self, cache: &(Vec<Option<(Tensor, Tensor)>>, usize)) {
        for (l, c) in self.layers.iter_mut().zip(&cache.0) {
            l.cache = c.clone();
        }
        self.pos = cache.1;
    }

    pub fn clear_cache(&mut self) {
        for l in &mut self.layers {
            l.cache = None;
        }
        self.pos = 0;
    }

    fn rope(&self, start: usize, t: usize, dtype: DType, dev: &Device) -> Result<(Tensor, Tensor)> {
        let half = self.inv_freq.len();
        let mut f = vec![0f32; t * half];
        for i in 0..t {
            for (j, w) in self.inv_freq.iter().enumerate() {
                f[i * half + j] = (start + i) as f32 * w;
            }
        }
        let f = Tensor::from_vec(f, (t, half), dev)?;
        Ok((f.cos()?.to_dtype(dtype)?, f.sin()?.to_dtype(dtype)?))
    }

    /// Prefill several sequences after the cached prompt prefix at once (see
    /// [`DecoderLayer::forward_segments`]): `xs[i]` (len_i, hidden). Returns each sequence's
    /// last-row logits (n, head_rows) and its cache; the model's own cache (the prefix) is left
    /// as it is.
    pub fn prefill_seqs(&mut self, xs: &[Tensor]) -> Result<(Tensor, Vec<SeqCache>)> {
        let prefix = self.pos;
        let mut segs = Vec::with_capacity(xs.len());
        let mut start = 0;
        for x in xs {
            let n = x.dim(0)?;
            if n == 0 {
                candle_core::bail!("empty sequence");
            }
            segs.push((start, n));
            start += n;
        }
        let dtype = xs.first().map_or(DType::F32, Tensor::dtype);
        let dev = xs.first().map_or(Device::Cpu, |x| x.device().clone());
        let rope = segs.iter().map(|&(_, n)| self.rope(prefix, n, dtype, &dev)).collect::<Result<Vec<_>>>()?;
        let mut seqs: Vec<SeqCache> = segs.iter().map(|_| SeqCache { layers: Vec::with_capacity(self.layers.len()), pos: prefix }).collect();
        let mut h = Tensor::cat(xs, 0)?.unsqueeze(0)?;
        for l in &self.layers {
            let mut caches: Vec<Option<(Tensor, Tensor)>> = segs.iter().map(|_| l.cache.clone()).collect();
            h = l.forward_segments(&h, &segs, &rope, &mut caches)?;
            for (s, c) in seqs.iter_mut().zip(caches) {
                let Some((k, v)) = c else { candle_core::bail!("prefill left no cache") };
                s.layers.push(Kv::from_tensors(&k, &v)?);
            }
        }
        let mut lasts = Vec::with_capacity(segs.len());
        for (s, &(start, n)) in seqs.iter_mut().zip(&segs) {
            s.pos = prefix + n;
            lasts.push(h.narrow(1, start + n - 1, 1)?);
        }
        let last = Tensor::cat(&lasts, 1)?;
        Ok((self.head.forward(&self.norm.forward(&last)?)?.squeeze(0)?, seqs))
    }

    /// One new token for each of several sequences: `x` (n, hidden), `seqs[i]` the cache of
    /// row `i`. Returns the logits (n, head_rows); every cache grows by one.
    pub fn forward_rows(&mut self, x: &Tensor, seqs: &mut [&mut SeqCache]) -> Result<Tensor> {
        let (n, _) = x.dims2()?;
        if seqs.len() != n {
            candle_core::bail!("{n} rows for {} sequences", seqs.len());
        }
        let mut h = x.unsqueeze(1)?;
        for (li, l) in self.layers.iter().enumerate() {
            let mut kvs: Vec<&mut Kv> = seqs.iter_mut().filter_map(|s| s.layers.get_mut(li)).collect();
            if kvs.len() != n {
                candle_core::bail!("a sequence cache is missing layer {li}");
            }
            h = l.forward_rows(&h, &self.inv_freq, &mut kvs)?;
        }
        for s in seqs.iter_mut() {
            s.pos += 1;
        }
        self.head.forward(&self.norm.forward(&h)?)?.squeeze(1)
    }

    /// Run `x` (1, t, hidden) after the cached tokens; returns logits (1, t, head_rows) or, when
    /// `last_only`, of the last position (1, 1, head_rows).
    pub fn forward(&mut self, x: &Tensor, cached: bool, last_only: bool) -> Result<Tensor> {
        let (_, t, _) = x.dims3()?;
        let start = if cached { self.pos } else { 0 };
        let (cos, sin) = self.rope(start, t, x.dtype(), x.device())?;
        let mask = if t > 1 {
            let total = start + t;
            let m: Vec<f32> = (0..t).flat_map(|i| (0..total).map(move |j| if j > start + i { f32::NEG_INFINITY } else { 0.0 })).collect();
            Some(Tensor::from_vec(m, (t, total), x.device())?.to_dtype(x.dtype())?)
        } else {
            None
        };
        let mut h = x.clone();
        for l in &mut self.layers {
            h = l.forward(&h, &cos, &sin, mask.as_ref(), cached)?;
        }
        if cached {
            self.pos = start + t;
        }
        let h = if last_only { h.narrow(1, t - 1, 1)? } else { h };
        self.head.forward(&self.norm.forward(&h)?)
    }
}

/// Qwen3-ASR or Qwen3-ForcedAligner weights.
pub struct Thinker {
    pub audio: AudioEncoder,
    pub text: TextModel,
    pub dtype: DType,
    pub dev: Device,
}

impl Thinker {
    pub fn new(
        audio: &AudioConfig,
        text: &TextConfig,
        head_rows: usize,
        tensors: HashMap<String, Tensor>,
        qm: &QMap,
        dtype: DType,
        dev: &Device,
    ) -> Result<Self> {
        let vb = VarBuilder::from_tensors(tensors, dtype, dev).pp("thinker");
        Ok(Self { audio: AudioEncoder::new(audio, vb.pp("audio_tower"))?, text: TextModel::new(text, head_rows, vb, qm)?, dtype, dev: dev.clone() })
    }

    /// Text embeddings of `before`, the audio features, then `after`: (1, len, hidden).
    pub fn inputs(&self, before: &[u32], audio: &Tensor, after: &[u32]) -> Result<Tensor> {
        let mut parts = Vec::with_capacity(3);
        if !before.is_empty() {
            parts.push(self.text.embed(before, &self.dev)?);
        }
        parts.push(audio.to_dtype(self.dtype)?);
        if !after.is_empty() {
            parts.push(self.text.embed(after, &self.dev)?);
        }
        Tensor::cat(&parts, 0)?.unsqueeze(0)
    }
}

/// Index of the largest value of a 1-D tensor.
pub fn argmax(logits: &Tensor) -> Result<u32> {
    logits.to_dtype(DType::F32)?.argmax(D::Minus1)?.to_scalar::<u32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_lengths_match_reference() {
        // _get_feat_extract_output_lengths
        let reference = |n: usize| {
            let leave = n % 100;
            let feat = (leave as i64 - 1).div_euclid(2) + 1;
            (((feat - 1).div_euclid(2) + 1 - 1).div_euclid(2) + 1 + (n / 100) as i64 * 13) as usize
        };
        for n in [1usize, 2, 7, 8, 9, 99, 100, 101, 250, 2999, 3000] {
            assert_eq!(audio_tokens(n), reference(n), "{n}");
        }
        assert_eq!(conv_out_len(128), 16);
    }

    #[test]
    fn parallel_elementwise_matches_candle() {
        let n = EW_CHUNK + 17;
        let v: Vec<f32> = (0..3 * n).map(|i| ((i * 37) % 1001) as f32 / 100.0 - 5.0).collect();
        let w: Vec<f32> = (0..3 * n).map(|i| ((i * 11) % 97) as f32 / 10.0 - 4.0).collect();
        // transposed views: not contiguous
        let x = Tensor::from_vec(v, (n, 3), &Device::Cpu).unwrap().t().unwrap();
        let y = Tensor::from_vec(w, (n, 3), &Device::Cpu).unwrap().t().unwrap();
        let bits = |t: Tensor| t.flatten_all().unwrap().to_vec1::<f32>().unwrap().iter().map(|f| f.to_bits()).collect::<Vec<_>>();
        assert_eq!(gelu(&x).unwrap().dims(), &[3, n]);
        assert_eq!(bits(gelu(&x).unwrap()), bits(x.gelu_erf().unwrap()));
        assert_eq!(bits(silu_mul(&x, &y).unwrap()), bits((x.silu().unwrap() * &y).unwrap()));
    }

    #[test]
    fn grouped_attention_matches_copied_keys() {
        let dev = Device::Cpu;
        let (b, kvh, rep, len, total, d) = (1, 2, 3, 4, 6, 8);
        let q = Tensor::randn(0f32, 1.0, (b, kvh * rep, len, d), &dev).unwrap();
        let k = Tensor::randn(0f32, 1.0, (b, kvh, total, d), &dev).unwrap();
        let v = Tensor::randn(0f32, 1.0, (b, kvh, total, d), &dev).unwrap();
        let past = total - len;
        let m: Vec<f32> = (0..len).flat_map(|i| (0..total).map(move |j| if j > past + i { f32::NEG_INFINITY } else { 0.0 })).collect();
        let mask = Tensor::from_vec(m, (len, total), &dev).unwrap();
        let expand = |y: &Tensor| y.unsqueeze(2).unwrap().expand((b, kvh, rep, total, d)).unwrap().reshape((b, kvh * rep, total, d)).unwrap();
        let want = attend(&q, &expand(&k), &expand(&v), 0.3, Some(&mask)).unwrap();
        let got = attend_grouped(&q, &k, &v, rep, 0.3, Some(&mask)).unwrap();
        assert_eq!(got.dims(), want.dims());
        let diff = (got - want).unwrap().abs().unwrap().max_all().unwrap().to_scalar::<f32>().unwrap();
        assert!(diff < 1e-5, "{diff}");
    }

    #[test]
    fn scaled_softmax_matches_candle() {
        let dev = Device::Cpu;
        let w = Tensor::randn(0f32, 3.0, (2, 3, 4, 6), &dev).unwrap();
        let m: Vec<f32> = (0..2 * 6).map(|i| if i % 6 > 3 + i / 6 { f32::NEG_INFINITY } else { 0.0 }).collect();
        let mask = Tensor::from_vec(m, (2, 6), &dev).unwrap();
        let want = candle_nn::ops::softmax_last_dim(&(&w * 0.25).unwrap().broadcast_add(&mask.repeat((2, 1)).unwrap()).unwrap()).unwrap();
        let got = scaled_softmax(&w, 0.25, Some(&mask)).unwrap();
        let diff = (got - want).unwrap().abs().unwrap().max_all().unwrap().to_scalar::<f32>().unwrap();
        assert!(diff < 1e-6, "{diff}");
        let plain = candle_nn::ops::softmax_last_dim(&(&w * 0.5).unwrap()).unwrap();
        let diff = (scaled_softmax(&w, 0.5, None).unwrap() - plain).unwrap().abs().unwrap().max_all().unwrap().to_scalar::<f32>().unwrap();
        assert!(diff < 1e-6, "{diff}");
        let bad = Tensor::zeros((3, 6), DType::F32, &dev).unwrap();
        assert!(scaled_softmax(&w, 1.0, Some(&bad)).is_err(), "4 rows are not blocks of 3");
    }

    #[test]
    fn front_convolution_matches_candle() {
        let dev = Device::Cpu;
        let val = |n: usize, seed: usize| -> Vec<f32> { (0..n).map(|i| (((i + seed) * 7919 % 211) as f32 - 105.0) / 70.0).collect() };
        // odd sizes, more chunks than one gathered group holds
        for (b, cin, h, w, cout) in [(3usize, 1usize, 9usize, 7usize, 5usize), (2, 4, 6, 11, 3), (CONV_POSITIONS / 12 + 2, 2, 5, 5, 2)] {
            let x = Tensor::from_vec(val(b * cin * h * w, 1), (b, cin, h, w), &dev).unwrap();
            let k = Tensor::from_vec(val(cout * cin * 9, 2), (cout, cin, 3, 3), &dev).unwrap();
            let bias = Tensor::from_vec(val(cout, 3), cout, &dev).unwrap();
            let conv = Conv2d::new(k, Some(bias), Conv2dConfig { padding: 1, stride: 2, ..Default::default() });
            let want = conv.forward(&x).unwrap();
            let got = conv3x3s2(&x, &conv).unwrap();
            assert_eq!(got.dims(), want.dims());
            let (g, wv) = (got.flatten_all().unwrap().to_vec1::<f32>().unwrap(), want.flatten_all().unwrap().to_vec1::<f32>().unwrap());
            let err = g.iter().zip(&wv).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(err < 1e-4, "{b}×{cin}×{h}×{w}: {err}");
        }
    }
}

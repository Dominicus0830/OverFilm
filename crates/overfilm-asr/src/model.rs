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

use candle_core::quantized::{QMatMul, QTensor};
use candle_core::{D, DType, Device, Module, Result, Tensor};
use candle_nn::{Conv2d, Conv2dConfig, Embedding, LayerNorm, Linear, RmsNorm, VarBuilder};

/// Quantized weights by tensor name (`…q_proj.weight`).
pub type QMap = HashMap<String, Arc<QTensor>>;

/// A bias-free linear layer, f32 or 8-bit (Q8_0: per-32 block scale, error well below the
/// model's own bf16 rounding; four times less memory traffic per generated token).
pub enum Lin {
    F(Linear),
    Q(QMatMul),
}

impl Lin {
    fn new(i: usize, o: usize, vb: VarBuilder, q: &QMap) -> Result<Self> {
        match q.get(&format!("{}.weight", vb.prefix())) {
            Some(t) => Ok(Lin::Q(QMatMul::from_arc(t.clone())?)),
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
            Lin::Q(QMatMul::QTensor(q)) if rows >= DEQUANT_ROWS => Linear::new(q.dequantize(x.device())?, None).forward(x),
            Lin::Q(m) => m.forward(&x.contiguous()?),
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

fn attend(q: &Tensor, k: &Tensor, v: &Tensor, scale: f64, mask: Option<&Tensor>) -> Result<Tensor> {
    let w = (q.matmul(&k.t()?)? * scale)?;
    let w = match mask {
        Some(m) => w.broadcast_add(m)?,
        None => w,
    };
    let w = candle_nn::ops::softmax_last_dim(&w.to_dtype(DType::F32)?)?.to_dtype(q.dtype())?;
    w.matmul(v)
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
        let h = self.fc2.forward(&self.fc1.forward(&self.ln2.forward(&x)?)?.gelu_erf()?)?;
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
        let x = Tensor::from_vec(buf, (n_chunks, 1, self.n_mels, padded), dev)?.to_dtype(dtype)?;
        let x = self.conv1.forward(&x)?.gelu_erf()?;
        let x = self.conv2.forward(&x)?.gelu_erf()?;
        let x = self.conv3.forward(&x)?.gelu_erf()?;
        let (b, c, f, t) = x.dims4()?;
        let x = x.permute((0, 3, 1, 2))?.contiguous()?.reshape((b, t, c * f))?;
        let x = self.conv_out.forward(&x)?.broadcast_add(&self.pos.narrow(0, 0, t)?.unsqueeze(0)?)?;
        let mut rows = Vec::with_capacity(n_chunks);
        for (i, &len) in lens.iter().enumerate() {
            rows.push(x.get(i)?.narrow(0, 0, conv_out_len(len).min(t))?);
        }
        Ok((Tensor::cat(&rows, 0)?, t * WINDOW_CHUNKS))
    }
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
        let h = self.proj1.forward(&self.ln_post.forward(&h)?)?.gelu_erf()?;
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
        let expand = |y: &Tensor| -> Result<Tensor> {
            if rep <= 1 {
                return Ok(y.clone());
            }
            let (b, n, s, d) = y.dims4()?;
            y.unsqueeze(2)?.expand((b, n, rep, s, d))?.reshape((b, n * rep, s, d))
        };
        let o = attend(&q, &expand(&k)?.contiguous()?, &expand(&v)?.contiguous()?, (self.head_dim as f64).powf(-0.5), mask)?;
        let o = o.transpose(1, 2)?.reshape((b, t, self.heads * self.head_dim))?;
        let x = (x + self.o.forward(&o)?)?;
        let h = self.ln2.forward(&x)?;
        let m = self.down.forward(&(self.gate.forward(&h)?.silu()? * self.up.forward(&h)?)?)?;
        x + m
    }
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
}

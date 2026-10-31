//! GPU kernels against the candle CPU model on tiny random networks. They need a GPU (a software
//! Vulkan such as lavapipe will do) and skip without one.

use std::collections::HashMap;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;

use super::*;
use crate::model::{self, AudioConfig, AudioEncoder, AudioFront, QMap, TextConfig, TextModel};

fn ctx() -> Option<Arc<Ctx>> {
    match Ctx::with(true) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("skipped: {e}");
            None
        }
    }
}

struct Rng(u64);

impl Rng {
    /// Uniform in [-a, a), rounded through f16 so both backends hold the same weights.
    fn vec(&mut self, n: usize, a: f32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let u = (self.0 >> 40) as f32 / (1u64 << 24) as f32;
                half::f16::from_f32((u * 2.0 - 1.0) * a).to_f32()
            })
            .collect()
    }
}

fn max_rel(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    let scale = b.iter().fold(0f32, |m, x| m.max(x.abs())).max(1e-6);
    a.iter().zip(b).fold(0f32, |m, (x, y)| m.max((x - y).abs())) / scale
}

fn read_floats(c: &Ctx, buf: &wgpu::Buffer, n: usize) -> Vec<f32> {
    let staging = c.staging(n as u64 * 4);
    let enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    let b = c.finish_read(enc, buf, &staging, n as u64 * 4).unwrap();
    b.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect()
}

/// One product through both kernels, both formats, split into parts, against f32.
#[test]
fn products_match() {
    let Some(c) = ctx() else { return };
    let mut rng = Rng(1);
    let (n, k) = (200, 96);
    let w = rng.vec(n * k, 0.5);
    let bias = rng.vec(n, 0.5);
    for rows in [1usize, 3, 70] {
        let x = rng.vec(rows * k, 1.0);
        let (xr, wr) = (&x, &w);
        let want: Vec<f32> = (0..rows).flat_map(|r| (0..n).map(move |j| (0..k).map(|i| xr[r * k + i] * wr[j * k + i]).sum::<f32>())).collect();
        for fmt in [Fmt::F16, Fmt::Q8] {
            let mut weight = Weight::new(&c, &w, n, k, fmt, Some(&bias)).unwrap();
            // force three parts (64 + 64 + 72 rows): the bias and the output offset follow
            if fmt == Fmt::F16 {
                let mut parts = Vec::new();
                for (off, pn) in [(0, 64), (64, 64), (128, 72)] {
                    parts.push(Part { qs: c.upload(&f16_words(&w[off * k..(off + pn) * k])), sc: c.zero.clone(), off, n: pn });
                }
                weight.parts = parts;
            }
            let step = c.uniform(&[rows as u32, 0, 0, 0]);
            let xb = c.upload_f32(&x);
            let y = c.upload_f32(&vec![1.0; rows * n]);
            let lin = Lin::new(&c, &weight, LinSpec { x: &xb, y: &y, step: &step, ldy: n, acc: true, rows1: false });
            let mut enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
                lin.record(&c, &mut pass, rows);
            }
            c.submit(enc).unwrap();
            let got = read_floats(&c, &y, rows * n);
            let want: Vec<f32> = want.iter().enumerate().map(|(i, v)| v + bias[i % n] + 1.0).collect();
            let tol = if fmt == Fmt::Q8 { 2e-2 } else { 1e-4 };
            let e = max_rel(&got, &want);
            assert!(e < tol, "{fmt:?} rows {rows}: {e}");
        }
    }
}

fn text_config() -> TextConfig {
    serde_json::from_value(serde_json::json!({
        "vocab_size": 50, "hidden_size": 64, "intermediate_size": 96, "num_hidden_layers": 2,
        "num_attention_heads": 4, "num_key_value_heads": 2, "head_dim": 32, "rms_norm_eps": 1e-6,
        "rope_theta": 1000000.0, "tie_word_embeddings": false
    }))
    .unwrap()
}

fn text_tensors(tc: &TextConfig, head_rows: usize, rng: &mut Rng) -> Vec<(String, Vec<usize>, Vec<f32>)> {
    let (h, dh, i) = (tc.hidden_size, tc.head_dim, tc.intermediate_size);
    let mut v = Vec::new();
    let mut add = |name: String, shape: Vec<usize>, a: f32, base: f32| {
        let n = shape.iter().product();
        let data = rng.vec(n, a).into_iter().map(|x| x + base).collect();
        v.push((name, shape, data));
    };
    add("thinker.model.embed_tokens.weight".into(), vec![tc.vocab_size, h], 1.0, 0.0);
    for l in 0..tc.num_hidden_layers {
        let p = format!("thinker.model.layers.{l}");
        add(format!("{p}.input_layernorm.weight"), vec![h], 0.2, 1.0);
        add(format!("{p}.post_attention_layernorm.weight"), vec![h], 0.2, 1.0);
        add(format!("{p}.self_attn.q_norm.weight"), vec![dh], 0.2, 1.0);
        add(format!("{p}.self_attn.k_norm.weight"), vec![dh], 0.2, 1.0);
        add(format!("{p}.self_attn.q_proj.weight"), vec![tc.num_attention_heads * dh, h], 0.2, 0.0);
        add(format!("{p}.self_attn.k_proj.weight"), vec![tc.num_key_value_heads * dh, h], 0.2, 0.0);
        add(format!("{p}.self_attn.v_proj.weight"), vec![tc.num_key_value_heads * dh, h], 0.2, 0.0);
        add(format!("{p}.self_attn.o_proj.weight"), vec![h, tc.num_attention_heads * dh], 0.2, 0.0);
        add(format!("{p}.mlp.gate_proj.weight"), vec![i, h], 0.2, 0.0);
        add(format!("{p}.mlp.up_proj.weight"), vec![i, h], 0.2, 0.0);
        add(format!("{p}.mlp.down_proj.weight"), vec![h, i], 0.2, 0.0);
    }
    add("thinker.model.norm.weight".into(), vec![h], 0.2, 1.0);
    add("thinker.lm_head.weight".into(), vec![head_rows, h], 0.2, 0.0);
    v
}

/// The decoder: a prefill of several rows (tiled kernels), then cached single steps (vector
/// kernels), and an every-row pass (the aligner's head), against candle.
#[test]
fn decoder_matches() {
    let Some(c) = ctx() else { return };
    let tc = text_config();
    let head_rows = 40;
    let mut rng = Rng(7);
    let tensors = text_tensors(&tc, head_rows, &mut rng);
    let dev = Device::Cpu;
    let map: HashMap<String, Tensor> = tensors.iter().map(|(n, s, d)| (n.clone(), Tensor::from_vec(d.clone(), s.as_slice(), &dev).unwrap())).collect();
    let vb = VarBuilder::from_tensors(map, DType::F32, &dev).pp("thinker");
    let mut cpu = TextModel::new(&tc, head_rows, vb, &QMap::new()).unwrap();
    let mut st = Store::new(Fmt::F16);
    for (n, s, d) in tensors.iter().filter(|(n, ..)| !n.contains("embed_tokens")) {
        st.add(&c, n, s, d.clone()).unwrap();
    }
    let d = TextDims {
        hidden: tc.hidden_size,
        inter: tc.intermediate_size,
        layers: tc.num_hidden_layers,
        hq: tc.num_attention_heads,
        hkv: tc.num_key_value_heads,
        dh: tc.head_dim,
        eps: tc.rms_norm_eps as f32,
        theta: tc.rope_theta,
        head_rows,
    };
    let mut gpu = Text::new(&c, d, &mut st, "thinker").unwrap();
    let h = tc.hidden_size;
    let x0 = rng.vec(9 * h, 1.0);
    let t = |v: &[f32], rows: usize| Tensor::from_vec(v.to_vec(), (1, rows, h), &dev).unwrap();
    // prefill
    let want = cpu.forward(&t(&x0, 9), true, true).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
    let ids = gpu.forward(&x0, true, true).unwrap();
    let got = gpu.last_logits().unwrap();
    assert!(max_rel(&got, &want) < 1e-2, "prefill {}", max_rel(&got, &want));
    let best = want.iter().enumerate().fold((0, f32::MIN), |b, (i, &v)| if v > b.1 { (i, v) } else { b }).0;
    assert_eq!(ids, vec![best as u32]);
    // cached steps
    for s in 0..3 {
        let x = rng.vec(h, 1.0);
        let want = cpu.forward(&t(&x, 1), true, true).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        gpu.forward(&x, true, true).unwrap();
        let got = gpu.last_logits().unwrap();
        assert!(max_rel(&got, &want) < 1e-2, "step {s}: {}", max_rel(&got, &want));
    }
    assert_eq!(gpu.pos, 12);
    // every row from position 0 without the cache
    let want = cpu.forward(&t(&x0, 9), false, false).unwrap().squeeze(0).unwrap().argmax(candle_core::D::Minus1).unwrap().to_vec1::<u32>().unwrap();
    let got = gpu.forward(&x0, false, false).unwrap();
    assert_eq!(got, want);
    // the cache bound is an error, not a crash
    assert!(gpu.forward(&rng.vec((KV_CAP + 1) * h, 1.0), false, true).is_err());
}

/// The audio encoder over several attention windows, against candle.
#[test]
fn encoder_matches() {
    let Some(c) = ctx() else { return };
    let ac: AudioConfig = serde_json::from_value(serde_json::json!({
        "d_model": 64, "encoder_layers": 2, "encoder_attention_heads": 4, "encoder_ffn_dim": 128,
        "num_mel_bins": 16, "max_source_positions": 200, "downsample_hidden_size": 8, "output_dim": 32
    }))
    .unwrap();
    let mut rng = Rng(3);
    let dev = Device::Cpu;
    let mut tensors: Vec<(String, Vec<usize>)> = vec![
        ("conv2d1.weight".into(), vec![8, 1, 3, 3]),
        ("conv2d1.bias".into(), vec![8]),
        ("conv2d2.weight".into(), vec![8, 8, 3, 3]),
        ("conv2d2.bias".into(), vec![8]),
        ("conv2d3.weight".into(), vec![8, 8, 3, 3]),
        ("conv2d3.bias".into(), vec![8]),
        ("conv_out.weight".into(), vec![64, 8 * model::conv_out_len(16)]),
        ("ln_post.weight".into(), vec![64]),
        ("ln_post.bias".into(), vec![64]),
        ("proj1.weight".into(), vec![64, 64]),
        ("proj1.bias".into(), vec![64]),
        ("proj2.weight".into(), vec![32, 64]),
        ("proj2.bias".into(), vec![32]),
    ];
    for l in 0..2 {
        let p = format!("layers.{l}");
        for n in ["self_attn_layer_norm", "final_layer_norm"] {
            tensors.push((format!("{p}.{n}.weight"), vec![64]));
            tensors.push((format!("{p}.{n}.bias"), vec![64]));
        }
        for n in ["q_proj", "k_proj", "v_proj", "out_proj"] {
            tensors.push((format!("{p}.self_attn.{n}.weight"), vec![64, 64]));
            tensors.push((format!("{p}.self_attn.{n}.bias"), vec![64]));
        }
        tensors.push((format!("{p}.fc1.weight"), vec![128, 64]));
        tensors.push((format!("{p}.fc1.bias"), vec![128]));
        tensors.push((format!("{p}.fc2.weight"), vec![64, 128]));
        tensors.push((format!("{p}.fc2.bias"), vec![64]));
    }
    let data: Vec<(String, Vec<usize>, Vec<f32>)> = tensors
        .into_iter()
        .map(|(n, s)| {
            let base = if n.contains("norm.weight") || n.contains("ln_post.weight") { 1.0 } else { 0.0 };
            let v = rng.vec(s.iter().product(), 0.3).into_iter().map(|x| x + base).collect();
            (format!("thinker.audio_tower.{n}"), s, v)
        })
        .collect();
    let map: HashMap<String, Tensor> = data.iter().map(|(n, s, d)| (n.clone(), Tensor::from_vec(d.clone(), s.as_slice(), &dev).unwrap())).collect();
    let vb = VarBuilder::from_tensors(map, DType::F32, &dev).pp("thinker.audio_tower");
    let cpu = AudioEncoder::new(&ac, vb.clone()).unwrap();
    let front = AudioFront::new(&ac, vb).unwrap();
    let mut st = Store::new(Fmt::F16);
    for (n, s, d) in data.iter().filter(|(n, ..)| !n.contains(".conv")) {
        st.add(&c, n, s, d.clone()).unwrap();
    }
    let e = EncDims { d: 64, layers: 2, heads: 4, ffn: 128, out: 32, window: model::ENCODER_WINDOW };
    let enc = Encoder::new(&c, e, &mut st, "thinker.audio_tower").unwrap();
    // 12.5 s: 163 rows, two windows
    let frames = 1250;
    let mel = rng.vec(16 * frames, 1.0);
    let want = cpu.forward(&mel, frames, DType::F32, &dev).unwrap();
    let (h, _) = front.forward(&mel, frames, DType::F32, &dev).unwrap();
    let got = enc.forward(&h.flatten_all().unwrap().to_vec1::<f32>().unwrap()).unwrap();
    let want = want.flatten_all().unwrap().to_vec1::<f32>().unwrap();
    assert_eq!(got.len(), model::audio_tokens(frames) * 32);
    assert!(max_rel(&got, &want) < 1e-2, "{}", max_rel(&got, &want));
    // too many rows: an error
    assert!(enc.forward(&vec![0.0; (MAX_ROWS + 1) * 64]).is_err());
}

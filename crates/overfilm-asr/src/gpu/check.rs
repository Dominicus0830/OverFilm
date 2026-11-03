//! A self-check of the GPU: every kernel on realistic sizes against a plain-Rust reference, then
//! tiny random decoder and encoder networks against the candle CPU model. It runs before the GPU
//! is used for real work (well under a second on a GPU); any mismatch keeps the work on the CPU.
//! `overfilm-subs gpu-check` prints the report.

use std::collections::HashMap;
use std::sync::Arc;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;

use super::{Ctx, EncDims, Encoder, Fmt, Lin, LinSpec, Store, Text, TextDims, Weight, f16_words, gpu_err, quantize_rows};
use crate::AsrError;
use crate::model::{self, AudioConfig, AudioEncoder, AudioFront, QMap, TextConfig, TextModel};

/// One check: the largest error relative to the reference's largest magnitude.
#[derive(Clone, Debug)]
pub struct Report {
    pub name: String,
    pub err: f32,
    pub tol: f32,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.err.is_finite() && self.err <= self.tol
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {:<28} error {:.2e} (limit {:.0e})", if self.ok() { "ok  " } else { "FAIL" }, self.name, self.err, self.tol)
    }
}

pub struct Rng(pub u64);

impl Rng {
    /// Uniform in [-a, a), rounded through f16 so both sides hold the same values.
    pub fn vec(&mut self, n: usize, a: f32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let u = (self.0 >> 40) as f32 / (1u64 << 24) as f32;
                half::f16::from_f32((u * 2.0 - 1.0) * a).to_f32()
            })
            .collect()
    }
}

pub fn max_rel(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::INFINITY;
    }
    let scale = b.iter().fold(0f32, |m, x| m.max(x.abs())).max(1e-6);
    a.iter().zip(b).fold(0f32, |m, (x, y)| if x.is_finite() { m.max((x - y).abs()) } else { f32::INFINITY }) / scale
}

fn floats(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect()
}

pub(super) fn read(c: &Ctx, buf: &wgpu::Buffer, n: usize) -> Result<Vec<u8>, AsrError> {
    let staging = c.staging(n as u64 * 4);
    let enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    c.finish_read(enc, buf, &staging, n as u64 * 4)
}

fn run(c: &Ctx, f: impl FnOnce(&mut wgpu::ComputePass)) -> Result<(), AsrError> {
    let mut enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("check") });
    {
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("check"), timestamp_writes: None });
        f(&mut pass);
    }
    c.submit(enc)
}

fn dispatch(pass: &mut wgpu::ComputePass, k: &super::Kernel, bg: &wgpu::BindGroup, x: u32, y: u32) {
    pass.set_pipeline(&k.pipe);
    pass.set_bind_group(0, bg, &[]);
    pass.dispatch_workgroups(x, y, 1);
}

fn f16_round(v: &[f32]) -> Vec<f32> {
    v.iter().map(|x| half::f16::from_f32(*x).to_f32()).collect()
}

fn unpack_f16(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(2).map(|w| half::f16::from_bits(u16::from_le_bytes([w[0], w[1]])).to_f32()).collect()
}

/// Largest relative error of `y += x·wᵀ + bias` on random data, against the CPU.
fn product_err(c: &Ctx, rng: &mut Rng, fmt: Fmt, n: usize, k: usize, rows: usize) -> Result<f32, AsrError> {
    let w = rng.vec(n * k, 0.05);
    let bias = rng.vec(n, 0.5);
    let weight = Weight::new(c, &w, n, k, fmt, Some(&bias))?;
    let wd: Vec<f32> = match fmt {
        Fmt::F16 => f16_round(&w),
        Fmt::Q8 => {
            let (q, s) = quantize_rows(&w);
            q.iter().enumerate().map(|(i, b)| f32::from(*b as i8) * s.get(i / 32).copied().unwrap_or(0.0)).collect()
        }
    };
    let x = rng.vec(rows * k, 1.0);
    let y0 = rng.vec(rows * n, 1.0);
    let want: Vec<f32> = (0..rows)
        .flat_map(|r| {
            let (x, wd, bias, y0) = (&x, &wd, &bias, &y0);
            (0..n).map(move |j| (0..k).map(|i| x[r * k + i] * wd[j * k + i]).sum::<f32>() + bias[j] + y0[r * n + j])
        })
        .collect();
    let step = c.uniform(&[rows as u32, 0, 0, 0]);
    let xb = c.upload_f32(&x);
    let y = c.upload_f32(&y0);
    let lin = Lin::new(c, &weight, LinSpec { x: &xb, y: &y, step: &step, ldy: n, acc: true, rows1: false });
    run(c, |p| lin.record(c, p, rows))?;
    let got = floats(&read(c, &y, rows * n)?);
    Ok(max_rel(&got, &want))
}

/// Error limit of the products: the 16-bit tiles round inputs and short sums to f16.
fn product_tol(c: &Ctx) -> f32 {
    if c.half() { 2e-3 } else { 1e-3 }
}

/// Whether the tiled product kernel (the 16-bit one when switched on) matches the CPU on this
/// device (checked once at start-up).
pub(super) fn tile_ok(c: &Ctx) -> Result<bool, AsrError> {
    let mut rng = Rng(0x7_11E5);
    for fmt in [Fmt::F16, Fmt::Q8] {
        if product_err(c, &mut rng, fmt, 200, 512, 70)? > product_tol(c) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether the 16-bit tiled kernel does a prefill-sized product (512 rows of a 2048 × 2048 8-bit
/// weight, eight times in one submission) at least 10 % faster than the 32-bit one. Leaves the
/// 16-bit kernel switched off.
pub(super) fn tile16_faster(c: &Ctx) -> Result<bool, AsrError> {
    let (n, k, rows) = (2048usize, 2048usize, 512usize);
    let mut rng = Rng(0x7_11E6);
    let weight = Weight::new(c, &rng.vec(n * k, 0.05), n, k, Fmt::Q8, None)?;
    let step = c.uniform(&[rows as u32, 0, 0, 0]);
    let xb = c.upload_f32(&rng.vec(rows * k, 1.0));
    let y = c.floats(rows * n);
    let lin = Lin::new(c, &weight, LinSpec { x: &xb, y: &y, step: &step, ldy: n, acc: false, rows1: false });
    let mut best = [f64::INFINITY; 2];
    for round in 0..3 {
        for (i, half) in [false, true].into_iter().enumerate() {
            c.set_half(half);
            let t = std::time::Instant::now();
            run(c, |p| {
                for _ in 0..8 {
                    lin.record(c, p, rows);
                }
            })?;
            read(c, &y, 1)?;
            // the first round warms up (pipeline creation, caches)
            if round > 0 {
                best[i] = best[i].min(t.elapsed().as_secs_f64());
            }
        }
    }
    c.set_half(false);
    Ok(best[1] < best[0] * 0.9)
}

fn products(c: &Ctx, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    let (n, k) = (1000, 2048);
    for fmt in [Fmt::F16, Fmt::Q8] {
        for rows in [1usize, 3, 8, 150] {
            let kind = if rows <= super::MV_ROWS {
                "vec"
            } else if c.half() {
                "tile16"
            } else if c.tiled() {
                "tile"
            } else {
                "vec(prefill)"
            };
            out.push(Report { name: format!("matmul {fmt:?} {kind} rows {rows}"), err: product_err(c, rng, fmt, n, k, rows)?, tol: product_tol(c) });
        }
    }
    Ok(())
}

fn norms(c: &Ctx, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    for (layer, cols) in [(false, 2048usize), (true, 1024)] {
        let rows = 5;
        let x = rng.vec(rows * cols, 2.0);
        let w = rng.vec(cols, 1.0);
        let b = rng.vec(cols, 1.0);
        let eps = 1e-6f32;
        let mut want = Vec::with_capacity(rows * cols);
        for r in x.chunks(cols) {
            let n = cols as f32;
            let mean = if layer { r.iter().sum::<f32>() / n } else { 0.0 };
            let var = r.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
            let inv = 1.0 / (var + eps).sqrt();
            want.extend(r.iter().enumerate().map(|(i, v)| (v - mean) * inv * w[i] + if layer { b[i] } else { 0.0 }));
        }
        let step = c.uniform(&[rows as u32, 0, 0, 0]);
        let (xb, wb, bb, y) = (c.upload_f32(&x), c.upload_f32(&w), c.upload_f32(&b), c.floats(rows * cols));
        let bg = super::norm_bg(c, &step, cols, layer, false, eps, [&xb, &wb, &bb, &y]);
        run(c, |p| dispatch(p, &c.k.norm, &bg, rows as u32, 1))?;
        let got = floats(&read(c, &y, rows * cols)?);
        out.push(Report { name: format!("{} {cols}", if layer { "layernorm" } else { "rmsnorm" }), err: max_rel(&got, &want), tol: 1e-4 });
    }
    Ok(())
}

/// q/k norm, rotary positions and the cache write (decoder), then attention over the cache.
fn rope_attention(c: &Ctx, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    let (hq, hkv, dh, cap, rows, pos) = (16usize, 8usize, 128usize, 64usize, 7usize, 30usize);
    let half = dh / 2;
    let heads = hq + 2 * hkv;
    let eps = 1e-6f32;
    // the cache before this step: random, positions < pos
    let k_old = f16_round(&rng.vec(hkv * cap * dh, 1.0));
    let v_old = f16_round(&rng.vec(hkv * cap * dh, 1.0));
    let qkv = rng.vec(rows * heads * dh, 2.0);
    let norms = rng.vec(2 * dh, 1.0).iter().map(|x| x + 1.0).collect::<Vec<_>>();
    let inv: Vec<f32> = (0..half).map(|i| (1.0 / 1e6f64.powf(2.0 * i as f64 / dh as f64)) as f32).collect();
    let table: Vec<f32> = (0..cap).flat_map(|p| inv.iter().flat_map(move |w| [(p as f32 * w).cos(), (p as f32 * w).sin()])).collect();
    // reference
    let (mut q, mut kc, mut vc) = (vec![0f32; rows * hq * dh], k_old.clone(), v_old.clone());
    for r in 0..rows {
        let p = pos + r;
        for j in 0..heads {
            let x = &qkv[(r * heads + j) * dh..(r * heads + j + 1) * dh];
            let mut y = x.to_vec();
            if j < hq + hkv {
                let nb = if j < hq { 0 } else { dh };
                let inv_rms = 1.0 / (x.iter().map(|v| v * v).sum::<f32>() / dh as f32 + eps).sqrt();
                for i in 0..half {
                    let (x1, x2) = (x[i] * inv_rms * norms[nb + i], x[i + half] * inv_rms * norms[nb + i + half]);
                    let (cs, sn) = (table[(p * half + i) * 2], table[(p * half + i) * 2 + 1]);
                    y[i] = x1 * cs - x2 * sn;
                    y[i + half] = x2 * cs + x1 * sn;
                }
            }
            if j < hq {
                q[(r * hq + j) * dh..(r * hq + j + 1) * dh].copy_from_slice(&y);
            } else {
                let (dst, h) = if j < hq + hkv { (&mut kc, j - hq) } else { (&mut vc, j - hq - hkv) };
                dst[(h * cap + p) * dh..(h * cap + p + 1) * dh].copy_from_slice(&f16_round(&y));
            }
        }
    }
    let scale = (dh as f32).powf(-0.5);
    let mut o = vec![0f32; rows * hq * dh];
    for r in 0..rows {
        for h in 0..hq {
            let kvh = h / (hq / hkv);
            let qv = &q[(r * hq + h) * dh..(r * hq + h + 1) * dh];
            let keys = pos + r + 1;
            let s: Vec<f32> = (0..keys).map(|t| (0..dh).map(|d| qv[d] * kc[(kvh * cap + t) * dh + d]).sum::<f32>() * scale).collect();
            let m = s.iter().fold(f32::MIN, |a, b| a.max(*b));
            let e: Vec<f32> = s.iter().map(|v| (v - m).exp()).collect();
            let sum: f32 = e.iter().sum();
            for d in 0..dh {
                o[(r * hq + h) * dh + d] = (0..keys).map(|t| e[t] * vc[(kvh * cap + t) * dh + d]).sum::<f32>() / sum;
            }
        }
    }
    // GPU
    let step = c.uniform(&[rows as u32, pos as u32, rows as u32 - 1, 0]);
    let (qkvb, nb, csb) = (c.upload_f32(&qkv), c.upload_f32(&norms), c.upload_f32(&table));
    let qb = c.floats(rows * hq * dh);
    let kb = c.upload(&f16_words(&k_old));
    let vb = c.upload(&f16_words(&v_old));
    let ob = c.floats(rows * hq * dh);
    let ru = c.uniform(&[hq as u32, hkv as u32, dh as u32, cap as u32, 0, eps.to_bits(), 0, 0]);
    let au = c.uniform(&[hq as u32, hkv as u32, dh as u32, cap as u32, 0, scale.to_bits(), 0, 0]);
    let rope = c.bind(&c.k.rope, &[&ru, &step, &qkvb, &nb, &csb, &qb, &kb, &vb, &c.zero]);
    let attn = c.bind(&c.k.attn, &[&au, &step, &qb, &kb, &vb, &ob, &c.zero]);
    run(c, |p| {
        dispatch(p, &c.k.rope, &rope, heads as u32, rows as u32);
        dispatch(p, &c.k.attn, &attn, hq as u32, rows as u32);
    })?;
    out.push(Report { name: "rope q".into(), err: max_rel(&floats(&read(c, &qb, rows * hq * dh)?), &q), tol: 1e-4 });
    out.push(Report { name: "rope k cache".into(), err: max_rel(&unpack_f16(&read(c, &kb, hkv * cap * half)?), &kc), tol: 2e-3 });
    out.push(Report { name: "rope v cache".into(), err: max_rel(&unpack_f16(&read(c, &vb, hkv * cap * half)?), &vc), tol: 2e-3 });
    out.push(Report { name: "attention causal".into(), err: max_rel(&floats(&read(c, &ob, rows * hq * dh)?), &o), tol: 1e-3 });
    Ok(())
}

/// The encoder's split (no norm, positions from 0) and windowed attention.
fn windowed(c: &Ctx, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    let (h, dh, rows, window) = (16usize, 64usize, 230usize, model::ENCODER_WINDOW);
    let qkv = rng.vec(rows * 3 * h * dh, 2.0);
    let at = |r: usize, part: usize, hh: usize, d: usize| qkv[((r * 3 + part) * h + hh) * dh + d];
    let scale = (dh as f32).powf(-0.5);
    let mut o = vec![0f32; rows * h * dh];
    for r in 0..rows {
        let lo = r / window * window;
        let hi = (lo + window).min(rows);
        for hh in 0..h {
            let s: Vec<f32> = (lo..hi).map(|t| (0..dh).map(|d| at(r, 0, hh, d) * half::f16::from_f32(at(t, 1, hh, d)).to_f32()).sum::<f32>() * scale).collect();
            let m = s.iter().fold(f32::MIN, |a, b| a.max(*b));
            let e: Vec<f32> = s.iter().map(|v| (v - m).exp()).collect();
            let sum: f32 = e.iter().sum();
            for d in 0..dh {
                o[(r * h + hh) * dh + d] = (lo..hi).zip(&e).map(|(t, w)| w * half::f16::from_f32(at(t, 2, hh, d)).to_f32()).sum::<f32>() / sum;
            }
        }
    }
    let step = c.uniform(&[rows as u32, 0, rows as u32 - 1, 0]);
    let qkvb = c.upload_f32(&qkv);
    let (qb, ob) = (c.floats(rows * h * dh), c.floats(rows * h * dh));
    let (kb, vb) = (c.storage((h * rows * dh * 2) as u64), c.storage((h * rows * dh * 2) as u64));
    let su = c.uniform(&[h as u32, h as u32, dh as u32, rows as u32, 1, 0, 0, 0]);
    let au = c.uniform(&[h as u32, h as u32, dh as u32, rows as u32, window as u32, scale.to_bits(), 0, 0]);
    let split = c.bind(&c.k.rope, &[&su, &step, &qkvb, &c.zero, &c.zero, &qb, &kb, &vb, &c.zero]);
    let attn = c.bind(&c.k.attn, &[&au, &step, &qb, &kb, &vb, &ob, &c.zero]);
    run(c, |p| {
        dispatch(p, &c.k.rope, &split, (3 * h) as u32, rows as u32);
        dispatch(p, &c.k.attn, &attn, h as u32, rows as u32);
    })?;
    out.push(Report { name: "attention windowed".into(), err: max_rel(&floats(&read(c, &ob, rows * h * dh)?), &o), tol: 1e-3 });
    Ok(())
}

fn elementwise(c: &Ctx, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    let (rows, n) = (3usize, 3000usize);
    let a = rng.vec(rows * 2 * n, 6.0);
    let step = c.uniform(&[rows as u32, 0, 0, 0]);
    let ab = c.upload_f32(&a);
    for mode in [0u32, 1] {
        let want: Vec<f32> = (0..rows * n)
            .map(|i| {
                if mode == 0 {
                    let (g, u) = (a[(i / n) * 2 * n + i % n], a[(i / n) * 2 * n + n + i % n]);
                    g / (1.0 + (-g).exp()) * u
                } else {
                    let x = f64::from(a[i]);
                    (0.5 * x * (1.0 + erf(x / std::f64::consts::SQRT_2))) as f32
                }
            })
            .collect();
        let y = c.floats(rows * n);
        let u = c.uniform(&[n as u32, mode, 0, 0]);
        let bg = c.bind(&c.k.ew, &[&u, &step, &ab, &y]);
        let (x, yy) = super::groups2((rows * n).div_ceil(256));
        run(c, |p| dispatch(p, &c.k.ew, &bg, x, yy))?;
        out.push(Report { name: if mode == 0 { "swiglu".into() } else { "gelu".into() }, err: max_rel(&floats(&read(c, &y, rows * n)?), &want), tol: 1e-4 });
    }
    Ok(())
}

/// Plain-loop convolution (the reference for `conv.wgsl`): `(cin, h, w)` in, `(cout, ho, wo)` out.
pub fn conv_ref(x: &[f32], c: &super::speaker::ConvSpec, h: usize, w: usize, res: Option<&[f32]>, relu: bool) -> (Vec<f32>, usize, usize) {
    let ho = (h + 2 * c.pad - c.k) / c.stride + 1;
    let wo = (w + 2 * c.pad - c.k) / c.stride + 1;
    let mut out = vec![0f32; c.cout * ho * wo];
    for oc in 0..c.cout {
        for oy in 0..ho {
            for ox in 0..wo {
                let mut acc = c.b.get(oc).copied().unwrap_or(0.0);
                for ic in 0..c.cin {
                    for ky in 0..c.k {
                        for kx in 0..c.k {
                            let iy = (oy * c.stride + ky) as isize - c.pad as isize;
                            let ix = (ox * c.stride + kx) as isize - c.pad as isize;
                            if iy < 0 || ix < 0 || iy as usize >= h || ix as usize >= w {
                                continue;
                            }
                            let xv = x.get((ic * h + iy as usize) * w + ix as usize).copied().unwrap_or(0.0);
                            let wv = c.w.get(((oc * c.cin + ic) * c.k + ky) * c.k + kx).copied().unwrap_or(0.0);
                            acc += xv * wv;
                        }
                    }
                }
                let i = (oc * ho + oy) * wo + ox;
                if let Some(r) = res {
                    acc += r.get(i).copied().unwrap_or(0.0);
                }
                if let Some(o) = out.get_mut(i) {
                    *o = if relu { acc.max(0.0) } else { acc };
                }
            }
        }
    }
    (out, ho, wo)
}

/// A tiny residual network (stem, one block with a stride-2 shortcut) on the GPU against
/// [`conv_ref`].
fn speaker_convs(c: &Arc<Ctx>, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    use super::speaker::{ConvSpec, SpeakerNet};
    let spec = |rng: &mut Rng, cout: usize, cin: usize, k: usize, stride: usize| ConvSpec {
        w: rng.vec(cout * cin * k * k, 0.5),
        b: rng.vec(cout, 0.1),
        cout,
        cin,
        k,
        stride,
        pad: k / 2,
    };
    let (mels, frames) = (16usize, 37usize);
    let stem = spec(rng, 4, 1, 3, 1);
    let block = (spec(rng, 8, 4, 3, 2), spec(rng, 8, 8, 3, 1), Some(spec(rng, 8, 4, 1, 2)));
    let feats = rng.vec(frames * mels, 2.0);
    // reference, laid out (mels, frames)
    let mut x0 = vec![0f32; frames * mels];
    for f in 0..frames {
        for m in 0..mels {
            x0[m * frames + f] = feats[f * mels + m];
        }
    }
    let (s, h, w) = conv_ref(&x0, &stem, mels, frames, None, true);
    let (y, yh, yw) = conv_ref(&s, &block.0, h, w, None, true);
    let (sc, _, _) = conv_ref(&s, block.2.as_ref().unwrap_or(&block.1), h, w, None, false);
    let (want, _, wo) = conv_ref(&y, &block.1, yh, yw, Some(&sc), true);
    let g = SpeakerNet::new(c, &stem, &[block])?;
    let (got, t) = g.forward(&feats, frames, mels)?;
    let err = if t == wo { max_rel(&got, &want) } else { f32::INFINITY };
    out.push(Report { name: "speaker convolutions".into(), err, tol: 1e-4 });
    Ok(())
}

/// erf for the reference (series / continued fraction, double precision).
fn erf(x: f64) -> f64 {
    // Abramowitz–Stegun 7.1.28 is too coarse; use the Taylor series near 0 and erfc's continued
    // fraction further out.
    let ax = x.abs();
    let v = if ax < 3.0 {
        let mut sum = ax;
        let mut term = ax;
        let mut n = 0.0;
        while term.abs() > 1e-17 {
            n += 1.0;
            term *= -ax * ax / n;
            sum += term / (2.0 * n + 1.0);
        }
        2.0 / std::f64::consts::PI.sqrt() * sum
    } else {
        let mut f = 0.0;
        for k in (1..60).rev() {
            f = f64::from(k) / 2.0 / (ax + f);
        }
        1.0 - (-ax * ax).exp() / std::f64::consts::PI.sqrt() / (ax + f)
    };
    v.copysign(x)
}

fn argmax(c: &Ctx, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    let (rows, n) = (3usize, 151_936usize);
    let mut x = rng.vec(rows * n, 1.0);
    let want: Vec<u32> = (0..rows as u32).map(|r| [12_345u32, 151_935, 7][r as usize]).collect();
    for (r, w) in want.iter().enumerate() {
        if let Some(v) = x.get_mut(r * n + *w as usize) {
            *v = 5.0;
        }
    }
    let step = c.uniform(&[rows as u32, 0, 0, 0]);
    let xb = c.upload_f32(&x);
    let ids = c.storage(rows as u64 * 4);
    let u = c.uniform(&[n as u32, 0, 0, 0]);
    let bg = c.bind(&c.k.argmax, &[&u, &step, &xb, &ids]);
    run(c, |p| dispatch(p, &c.k.argmax, &bg, rows as u32, 1))?;
    let got: Vec<u32> = read(c, &ids, rows)?.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
    out.push(Report { name: "argmax".into(), err: if got == want { 0.0 } else { 1.0 }, tol: 0.0 });
    // one row: the best class and its softmax probability
    let row = x.get(..n).unwrap_or_default();
    let best = row.iter().copied().fold(f32::MIN, f32::max);
    let want_p = 1.0 / row.iter().map(|v| (v - best).exp()).sum::<f32>();
    let step1 = c.uniform(&[1, 0, 0, 0]);
    let u1 = c.uniform(&[n as u32, 1, 0, 0]);
    let ids1 = c.storage(16);
    let bg1 = c.bind(&c.k.argmax, &[&u1, &step1, &xb, &ids1]);
    run(c, |p| dispatch(p, &c.k.argmax, &bg1, 1, 1))?;
    let b = read(c, &ids1, 2)?;
    let got_i = b.get(..4).map_or(u32::MAX, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
    let got_p = b.get(4..8).map_or(f32::NAN, |w| f32::from_le_bytes([w[0], w[1], w[2], w[3]]));
    let err = if got_i == want.first().copied().unwrap_or(0) { (got_p - want_p).abs() / want_p.max(1e-12) } else { 1.0 };
    out.push(Report { name: "argmax probability".into(), err, tol: 1e-3 });
    Ok(())
}

fn text_config() -> Result<TextConfig, AsrError> {
    serde_json::from_value(serde_json::json!({
        "vocab_size": 50, "hidden_size": 128, "intermediate_size": 192, "num_hidden_layers": 2,
        "num_attention_heads": 4, "num_key_value_heads": 2, "head_dim": 128, "rms_norm_eps": 1e-6,
        "rope_theta": 1000000.0, "tie_word_embeddings": false
    }))
    .map_err(gpu_err)
}

pub(super) fn text_tensors(tc: &TextConfig, head_rows: usize, rng: &mut Rng) -> Vec<(String, Vec<usize>, Vec<f32>)> {
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

fn to_vec(t: &Tensor) -> Result<Vec<f32>, AsrError> {
    Ok(t.flatten_all()?.to_vec1::<f32>()?)
}

/// A tiny random decoder: a 150-row prefill (tiled products), cached single steps, against candle.
fn decoder(c: &Arc<Ctx>, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    let tc = text_config()?;
    let head_rows = 40;
    let tensors = text_tensors(&tc, head_rows, rng);
    let dev = Device::Cpu;
    let mut map = HashMap::new();
    for (n, s, d) in &tensors {
        map.insert(n.clone(), Tensor::from_vec(d.clone(), s.as_slice(), &dev)?);
    }
    let vb = VarBuilder::from_tensors(map, DType::F32, &dev).pp("thinker");
    let mut cpu = TextModel::new(&tc, head_rows, vb, &QMap::new())?;
    let mut st = Store::new(Fmt::F16);
    for (n, s, d) in tensors.iter().filter(|(n, ..)| !n.contains("embed_tokens")) {
        st.add(c, n, s, d.clone())?;
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
        kv_cap: super::KV_CAP,
        per_row: true,
    };
    let mut gpu = Text::new(c, d, &mut st, "thinker")?;
    let h = tc.hidden_size;
    let rows = 150;
    let x0 = rng.vec(rows * h, 1.0);
    let want = to_vec(&cpu.forward(&Tensor::from_vec(x0.clone(), (1, rows, h), &dev)?, true, true)?)?;
    gpu.forward(&x0, true, true)?;
    out.push(Report { name: "decoder prefill".into(), err: max_rel(&gpu.last_logits()?, &want), tol: 2e-2 });
    let mut worst = 0f32;
    for _ in 0..3 {
        let x = rng.vec(h, 1.0);
        let want = to_vec(&cpu.forward(&Tensor::from_vec(x.clone(), (1, 1, h), &dev)?, true, true)?)?;
        gpu.forward(&x, true, true)?;
        worst = worst.max(max_rel(&gpu.last_logits()?, &want));
    }
    out.push(Report { name: "decoder steps".into(), err: worst, tol: 2e-2 });
    Ok(())
}

fn encoder(c: &Arc<Ctx>, rng: &mut Rng, out: &mut Vec<Report>) -> Result<(), AsrError> {
    let ac: AudioConfig = serde_json::from_value(serde_json::json!({
        "d_model": 128, "encoder_layers": 2, "encoder_attention_heads": 2, "encoder_ffn_dim": 256,
        "num_mel_bins": 16, "max_source_positions": 200, "downsample_hidden_size": 8, "output_dim": 64
    }))
    .map_err(gpu_err)?;
    let (d, ffn, outd) = (128usize, 256usize, 64usize);
    let mut shapes: Vec<(String, Vec<usize>)> = vec![
        ("conv2d1.weight".into(), vec![8, 1, 3, 3]),
        ("conv2d1.bias".into(), vec![8]),
        ("conv2d2.weight".into(), vec![8, 8, 3, 3]),
        ("conv2d2.bias".into(), vec![8]),
        ("conv2d3.weight".into(), vec![8, 8, 3, 3]),
        ("conv2d3.bias".into(), vec![8]),
        ("conv_out.weight".into(), vec![d, 8 * model::conv_out_len(16)]),
        ("ln_post.weight".into(), vec![d]),
        ("ln_post.bias".into(), vec![d]),
        ("proj1.weight".into(), vec![d, d]),
        ("proj1.bias".into(), vec![d]),
        ("proj2.weight".into(), vec![outd, d]),
        ("proj2.bias".into(), vec![outd]),
    ];
    for l in 0..2 {
        let p = format!("layers.{l}");
        for n in ["self_attn_layer_norm", "final_layer_norm"] {
            shapes.push((format!("{p}.{n}.weight"), vec![d]));
            shapes.push((format!("{p}.{n}.bias"), vec![d]));
        }
        for n in ["q_proj", "k_proj", "v_proj", "out_proj"] {
            shapes.push((format!("{p}.self_attn.{n}.weight"), vec![d, d]));
            shapes.push((format!("{p}.self_attn.{n}.bias"), vec![d]));
        }
        shapes.push((format!("{p}.fc1.weight"), vec![ffn, d]));
        shapes.push((format!("{p}.fc1.bias"), vec![ffn]));
        shapes.push((format!("{p}.fc2.weight"), vec![d, ffn]));
        shapes.push((format!("{p}.fc2.bias"), vec![d]));
    }
    let data: Vec<(String, Vec<usize>, Vec<f32>)> = shapes
        .into_iter()
        .map(|(n, s)| {
            let base = if n.contains("norm.weight") || n.contains("ln_post.weight") { 1.0 } else { 0.0 };
            let v = rng.vec(s.iter().product(), 0.3).into_iter().map(|x| x + base).collect();
            (format!("thinker.audio_tower.{n}"), s, v)
        })
        .collect();
    let dev = Device::Cpu;
    let mut map = HashMap::new();
    for (n, s, v) in &data {
        map.insert(n.clone(), Tensor::from_vec(v.clone(), s.as_slice(), &dev)?);
    }
    let vb = VarBuilder::from_tensors(map, DType::F32, &dev).pp("thinker.audio_tower");
    let cpu = AudioEncoder::new(&ac, vb.clone())?;
    let front = AudioFront::new(&ac, vb)?;
    let mut st = Store::new(Fmt::F16);
    for (n, s, v) in data.iter().filter(|(n, ..)| !n.contains(".conv")) {
        st.add(c, n, s, v.clone())?;
    }
    let e = EncDims { d, layers: 2, heads: 2, ffn, out: outd, window: model::ENCODER_WINDOW };
    let enc = Encoder::new(c, e, &mut st, "thinker.audio_tower")?;
    // 12.5 s: 163 rows, two windows
    let frames = 1250;
    let mel = rng.vec(16 * frames, 1.0);
    let want = to_vec(&cpu.forward(&mel, frames, DType::F32, &dev)?)?;
    let (h, _) = front.forward(&mel, frames, DType::F32, &dev)?;
    let got = enc.forward(&to_vec(&h)?)?;
    out.push(Report { name: "encoder".into(), err: max_rel(&got, &want), tol: 2e-2 });
    Ok(())
}

/// Every check, in order; an error inside one check is reported as that check failing.
pub fn report(c: &Arc<Ctx>) -> Vec<Report> {
    let mut out = Vec::new();
    let mut rng = Rng(0x0F11_5EED);
    type Check = fn(&Arc<Ctx>, &mut Rng, &mut Vec<Report>) -> Result<(), AsrError>;
    let checks: [(&str, Check); 9] = [
        ("matmul", |c, r, o| products(c, r, o)),
        ("norm", |c, r, o| norms(c, r, o)),
        ("rope/attention", |c, r, o| rope_attention(c, r, o)),
        ("windowed attention", |c, r, o| windowed(c, r, o)),
        ("element-wise", |c, r, o| elementwise(c, r, o)),
        ("argmax", |c, r, o| argmax(c, r, o)),
        ("decoder", decoder),
        ("encoder", encoder),
        ("speaker convolutions", speaker_convs),
    ];
    for (name, f) in checks {
        if let Err(e) = c.scoped(|| f(c, &mut rng, &mut out)) {
            out.push(Report { name: format!("{name}: {e}"), err: f32::INFINITY, tol: 0.0 });
        }
    }
    out
}

/// Ok when every check passes, else the failing ones.
pub fn verify(c: &Arc<Ctx>) -> Result<(), AsrError> {
    let failed: Vec<String> = report(c).iter().filter(|r| !r.ok()).map(|r| r.to_string()).collect();
    if failed.is_empty() { Ok(()) } else { Err(gpu_err(format!("self-check failed: {}", failed.join("; ")))) }
}

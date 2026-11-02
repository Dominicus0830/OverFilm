//! The audio encoder's front on the GPU: three 3×3 stride-2 convolutions with GELU and the
//! `conv_out` projection, as matrix products over gathered input patches (`gather.wgsl` then the
//! 16-bit product kernels the encoder uses). On the CPU these took about 4 s of a 60-second
//! piece even in GPU mode; the positions and the per-chunk trimming stay on the CPU
//! ([`crate::model::AudioFront::finish`]), they are tiny.

use std::sync::Arc;

use super::{Ctx, Fmt, Lin, LinSpec, Weight, gpu_err, groups2, words};
use crate::AsrError;
use crate::model::{FrontWeights, MelChunks};

/// Most bytes of patches gathered at once (bounds the chunks per pass: a 60-second piece
/// needs about 830 MB of patches for its second convolution).
const MAX_PATCH_BYTES: u64 = 256 << 20;
/// Mel frames per chunk the buffers are sized for (`model::CHUNK`).
const WIDTH: usize = 100;

fn half(l: usize) -> usize {
    l.div_ceil(2)
}

/// Sizes after each convolution for a chunk of `h` mel bins × `w` frames.
fn dims(h: usize, w: usize) -> [(usize, usize); 3] {
    let a = (half(h), half(w));
    let b = (half(a.0), half(a.1));
    [a, b, (half(b.0), half(b.1))]
}

pub struct Front {
    ctx: Arc<Ctx>,
    n_mels: usize,
    ch: usize,
    d: usize,
    /// columns of each product's input (cin · 9 per convolution, then channels · frequencies),
    /// padded with zeros to a multiple of 32
    kpad: [usize; 4],
    /// chunks per pass
    group: usize,
    input: wgpu::Buffer,
    cols: wgpu::Buffer,
    a: wgpu::Buffer,
    b: wgpu::Buffer,
    out: wgpu::Buffer,
    read: wgpu::Buffer,
    steps: [wgpu::Buffer; 4],
    lins: [Lin; 4],
}

impl Front {
    pub fn new(ctx: &Arc<Ctx>, fw: &FrontWeights) -> Result<Self, AsrError> {
        Self::with_cap(ctx, fw, MAX_PATCH_BYTES)
    }

    /// [`Front::new`] gathering at most `cap` bytes of patches at once.
    pub fn with_cap(ctx: &Arc<Ctx>, fw: &FrontWeights, cap: u64) -> Result<Self, AsrError> {
        let c = ctx.as_ref();
        let [(w1, b1, ch, cin1), (w2, b2, ch2, cin2), (w3, b3, ch3, cin3)] = &fw.convs;
        let ch = *ch;
        let n_mels = fw.n_mels;
        let [_, _, (h3, _)] = dims(n_mels, WIDTH);
        if ch == 0 || *cin1 != 1 || *ch2 != ch || *ch3 != ch || *cin2 != ch || *cin3 != ch || fw.out_in != ch * h3 || n_mels == 0 {
            return Err(gpu_err("audio front: unexpected convolution sizes"));
        }
        let pad32 = |k: usize| k.div_ceil(32) * 32;
        let kpad = [pad32(9), pad32(ch * 9), pad32(ch * 9), pad32(fw.out_in)];
        // weight rows padded with zeros to the padded columns
        let pad = |w: &[f32], k: usize, kp: usize| -> Vec<f32> {
            if k == kp {
                return w.to_vec();
            }
            let mut out = vec![0f32; w.len() / k.max(1) * kp];
            for (dst, src) in out.chunks_mut(kp).zip(w.chunks(k.max(1))) {
                if let Some(d) = dst.get_mut(..src.len()) {
                    d.copy_from_slice(src);
                }
            }
            out
        };
        let weights = [
            Weight::new(c, &pad(w1, 9, kpad[0]), ch, kpad[0], Fmt::F16, Some(b1))?,
            Weight::new(c, &pad(w2, ch * 9, kpad[1]), ch, kpad[1], Fmt::F16, Some(b2))?,
            Weight::new(c, &pad(w3, ch * 9, kpad[2]), ch, kpad[2], Fmt::F16, Some(b3))?,
            Weight::new(c, &pad(&fw.out, fw.out_in, kpad[3]), fw.d_model, kpad[3], Fmt::F16, None)?,
        ];
        let [(h1, wd1), (h2, wd2), (h3, wd3)] = dims(n_mels, WIDTH);
        let per_cols = [h1 * wd1 * kpad[0], h2 * wd2 * kpad[1], h3 * wd3 * kpad[2], wd3 * kpad[3]].into_iter().max().unwrap_or(0);
        let cap = cap.min(c.max_bind);
        let group = usize::try_from(cap / (per_cols as u64 * 4).max(1)).unwrap_or(1).max(1);
        let floats = |per: usize| c.floats(group * per);
        let input = floats(n_mels * WIDTH);
        let cols = floats(per_cols);
        let a = floats((h1 * wd1).max(h3 * wd3) * ch);
        let b = floats(h2 * wd2 * ch);
        let out = floats(wd3 * fw.d_model);
        let read = c.staging((group * wd3 * fw.d_model) as u64 * 4);
        let steps = [c.uniform(&[0, 0, 0, 0]), c.uniform(&[0, 0, 0, 0]), c.uniform(&[0, 0, 0, 0]), c.uniform(&[0, 0, 0, 0])];
        let lin = |w: &Weight, y: &wgpu::Buffer, ldy: usize, step: &wgpu::Buffer| Lin::new(c, w, LinSpec { x: &cols, y, step, ldy, acc: false, rows1: false });
        let lins = [
            lin(&weights[0], &a, ch, &steps[0]),
            lin(&weights[1], &b, ch, &steps[1]),
            lin(&weights[2], &a, ch, &steps[2]),
            lin(&weights[3], &out, fw.d_model, &steps[3]),
        ];
        Ok(Self { ctx: ctx.clone(), n_mels, ch, d: fw.d_model, kpad, group, input, cols, a, b, out, read, steps, lins })
    }

    /// `conv_out` of every chunk: `(chunks, t, d_model)` row-major, `t` = frames after the
    /// three halvings of the chunk width.
    pub fn forward(&self, m: &MelChunks) -> Result<Vec<f32>, AsrError> {
        let c = &self.ctx;
        if m.n == 0 || m.width == 0 || m.width > WIDTH || m.buf.len() != m.n * self.n_mels * m.width {
            return Err(gpu_err("audio front: bad input"));
        }
        let [(h1, w1), (h2, w2), (h3, w3)] = dims(self.n_mels, m.width);
        let per_in = self.n_mels * m.width;
        let mut out = Vec::with_capacity(m.n * w3 * self.d);
        let u = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
        for g0 in (0..m.n).step_by(self.group) {
            let nb = self.group.min(m.n - g0);
            let piece = m.buf.get(g0 * per_in..(g0 + nb) * per_in).ok_or_else(|| gpu_err("audio front: input slice"))?;
            c.queue.write_buffer(&self.input, 0, &piece.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
            let rows = [nb * h1 * w1, nb * h2 * w2, nb * h3 * w3, nb * w3];
            for (s, &r) in self.steps.iter().zip(&rows) {
                c.queue.write_buffer(s, 0, &words(&[u(r), 0, u(r).saturating_sub(1), 0]));
            }
            // (mode, cin, h, w, ho, wo, cols, act) per gather
            let gathers = [
                (0, 1, self.n_mels, m.width, h1, w1, self.kpad[0], 0, &self.input),
                (0, self.ch, h1, w1, h2, w2, self.kpad[1], 1, &self.a),
                (0, self.ch, h2, w2, h3, w3, self.kpad[2], 1, &self.b),
                (1, self.ch, h3, w3, 0, 0, self.kpad[3], 1, &self.a),
            ];
            let mut enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("audio front") });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("audio front"), timestamp_writes: None });
                for (i, &(mode, cin, h, w, ho, wo, cols, act, x)) in gathers.iter().enumerate() {
                    let p = c.uniform(&[mode, u(nb), u(cin), u(h), u(w), u(ho), u(wo), u(cols), act, 0, 0, 0]);
                    let bg = c.bind(&c.k.gather, &[&p, x, &self.cols]);
                    let n = rows.get(i).copied().unwrap_or(0) * cols;
                    pass.set_pipeline(&c.k.gather.pipe);
                    pass.set_bind_group(0, &bg, &[]);
                    let (gx, gy) = groups2(n.div_ceil(256));
                    pass.dispatch_workgroups(gx, gy, 1);
                    if let (Some(l), Some(&r)) = (self.lins.get(i), rows.get(i)) {
                        l.record(c, &mut pass, r);
                    }
                }
            }
            let bytes = (nb * w3 * self.d) as u64 * 4;
            let b = c.finish_read(enc, &self.out, &self.read, bytes)?;
            out.extend(b.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])));
        }
        Ok(out)
    }
}

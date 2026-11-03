//! The speaker network's convolutions (WeSpeaker ResNet34, see `crate::speakers::net`) on the
//! GPU: one submission per block of features, every convolution one dispatch of `conv.wgsl`,
//! only the last stage's frame map read back. Statistics pooling and the final linear layer stay
//! on the CPU (they are tiny).

use std::sync::Arc;

use super::{Ctx, gpu_err, groups2};
use crate::AsrError;

/// One convolution: weights `(cout, cin, k, k)` with batch norm folded in, bias per channel.
#[derive(Clone, Debug)]
pub struct ConvSpec {
    pub w: Vec<f32>,
    pub b: Vec<f32>,
    pub cout: usize,
    pub cin: usize,
    pub k: usize,
    pub stride: usize,
    pub pad: usize,
}

struct Conv {
    w: wgpu::Buffer,
    b: wgpu::Buffer,
    cout: usize,
    cin: usize,
    k: usize,
    stride: usize,
    pad: usize,
}

/// Blocks: (first conv, second conv, shortcut conv).
pub struct SpeakerNet {
    ctx: Arc<Ctx>,
    stem: Conv,
    blocks: Vec<(Conv, Conv, Option<Conv>)>,
}

/// Longest block of feature frames per submission (a 10 s block is 998).
pub const MAX_FRAMES: usize = 4000;

impl SpeakerNet {
    pub fn new(ctx: &Arc<Ctx>, stem: &ConvSpec, blocks: &[(ConvSpec, ConvSpec, Option<ConvSpec>)]) -> Result<Self, AsrError> {
        ctx.scoped(|| {
            let up = |c: &ConvSpec| -> Result<Conv, AsrError> {
                if c.w.len() != c.cout * c.cin * c.k * c.k || c.b.len() != c.cout || c.k == 0 || c.stride == 0 {
                    return Err(gpu_err("speaker network: bad convolution shape"));
                }
                Ok(Conv { w: ctx.upload_f32(&c.w), b: ctx.upload_f32(&c.b), cout: c.cout, cin: c.cin, k: c.k, stride: c.stride, pad: c.pad })
            };
            let stem = up(stem)?;
            let blocks = blocks.iter().map(|(a, b, s)| Ok((up(a)?, up(b)?, s.as_ref().map(up).transpose()?))).collect::<Result<Vec<_>, AsrError>>()?;
            Ok(Self { ctx: ctx.clone(), stem, blocks })
        })
    }

    /// The convolutional part over `feats` (`frames × mels`, row-major by frame): the last
    /// stage's output as `(channels × rows, out_frames)` row-major, plus `out_frames`.
    pub fn forward(&self, feats: &[f32], frames: usize, mels: usize) -> Result<(Vec<f32>, usize), AsrError> {
        if frames == 0 || frames > MAX_FRAMES || feats.len() != frames * mels {
            return Err(gpu_err("speaker network: bad input size"));
        }
        let c = &self.ctx;
        // (1, mels, frames): mel rows, frame columns
        let mut x0 = vec![0f32; frames * mels];
        for (f, row) in feats.chunks(mels).enumerate() {
            for (m, &v) in row.iter().enumerate() {
                if let Some(slot) = x0.get_mut(m * frames + f) {
                    *slot = v;
                }
            }
        }
        c.scoped(|| {
            let mut enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("speaker") });
            let input = c.upload_f32(&x0);
            // (wgpu keeps buffers that recorded commands use alive until the submission is done)
            let run = |enc: &mut wgpu::CommandEncoder, cv: &Conv, x: &wgpu::Buffer, h: usize, w: usize, res: Option<&wgpu::Buffer>, relu: bool| {
                let ho = (h + 2 * cv.pad - cv.k) / cv.stride + 1;
                let wo = (w + 2 * cv.pad - cv.k) / cv.stride + 1;
                let n = cv.cout * ho * wo;
                let out = c.floats(n);
                let u = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
                let p = c.uniform(&[
                    u(cv.cin),
                    u(h),
                    u(w),
                    u(cv.cout),
                    u(ho),
                    u(wo),
                    u(cv.k),
                    u(cv.stride),
                    u(cv.pad),
                    u32::from(res.is_some()),
                    u32::from(relu),
                    0,
                ]);
                let bg = c.bind(&c.k.conv, &[&p, x, &cv.w, &cv.b, res.unwrap_or(&c.zero), &out]);
                {
                    let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
                    pass.set_pipeline(&c.k.conv.pipe);
                    pass.set_bind_group(0, &bg, &[]);
                    let (gx, gy) = groups2(n.div_ceil(64));
                    pass.dispatch_workgroups(gx.max(1), gy.max(1), 1);
                }
                (out, ho, wo)
            };
            let (mut x, mut h, mut w) = run(&mut enc, &self.stem, &input, mels, frames, None, true);
            for (a, b, s) in &self.blocks {
                let (y, yh, yw) = run(&mut enc, a, &x, h, w, None, true);
                let short = s.as_ref().map(|sc| run(&mut enc, sc, &x, h, w, None, false).0);
                let res = short.as_ref().unwrap_or(&x);
                let (z, zh, zw) = run(&mut enc, b, &y, yh, yw, Some(res), true);
                x = z;
                (h, w) = (zh, zw);
            }
            let ch = self.blocks.last().map_or(self.stem.cout, |b| b.1.cout);
            let n = ch * h * w;
            let staging = c.staging(n as u64 * 4);
            let bytes = c.finish_read(enc, &x, &staging, n as u64 * 4)?;
            let out: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
            Ok((out, w))
        })
    }
}

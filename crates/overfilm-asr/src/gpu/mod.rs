//! The Qwen3 decoder and the audio encoder's transformer on the GPU through wgpu (Vulkan, DX12,
//! Metal: AMD, NVIDIA, Intel and Apple GPUs alike), in WGSL compute shaders. Same arithmetic as
//! the CPU model in [`crate::model`]: f32 activations, decoder weights 8-bit (Q8: one f32 scale per
//! 32) or 16-bit, encoder weights 16-bit (the checkpoints are bf16, so f16 keeps every bit that
//! matters), key/value cache 16-bit.
//!
//! What stays on the CPU: the log-mel, the encoder's three convolutions (small), the token
//! embedding lookup, and everything outside the networks. One forward is one submission; only the
//! chosen token ids (or the encoder output) are read back.
//!
//! Every wgpu error is caught (error scopes while loading, an uncaptured-error hook while running)
//! and returned as [`AsrError::Model`]; the caller then falls back to the CPU.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use wgpu::util::DeviceExt;

use crate::AsrError;

pub mod front;
pub mod speaker;

const MATMUL: &str = include_str!("matmul.wgsl");
const Q8: &str = include_str!("q8.wgsl");
const F16: &str = include_str!("f16.wgsl");
const NORM: &str = include_str!("norm.wgsl");
const ROPE: &str = include_str!("rope.wgsl");
const ATTN: &str = include_str!("attn.wgsl");
const EW: &str = include_str!("ew.wgsl");
const ARGMAX: &str = include_str!("argmax.wgsl");
const CONV: &str = include_str!("conv.wgsl");
const GATHER: &str = include_str!("gather.wgsl");
const TILE16: &str = include_str!("tile16.wgsl");

/// Rows up to which a product runs as matrix × vector (`mv` in `matmul.wgsl`, which reads
/// each weight once for all of them).
const MV_ROWS: usize = 8;
/// Rows per decoder or encoder pass (longer inputs run in several passes through the cache).
pub const MAX_ROWS: usize = 1024;
/// Positions in a decoder's key/value cache by default (the aligner: one pass over its audio
/// and text; 16-bit keys and values, about 0.9 GB for 28 layers).
pub const KV_CAP: usize = 8192;
/// Positions in the recogniser's cache (prompt + audio + generated text per piece): a 60 s
/// piece takes about 1,800 with room for its text, so eight fit side by side after the prompt
/// (about 1.9 GB for Qwen3-ASR's 28 layers).
pub const KV_CAP_SIDE: usize = 16_384;
/// Sequences decoded side by side at most ([`Text::forward_seqs`]).
pub const MAX_SEQS: usize = 8;
/// Largest single buffer binding asked for (weights larger than this are split by rows).
const MAX_BIND: u64 = 1 << 30;
/// Longest wait for one submission (a long prefill on a slow or software GPU).
const WAIT: Duration = Duration::from_secs(600);

fn gpu_err(e: impl std::fmt::Display) -> AsrError {
    AsrError::Model(format!("GPU: {e}"))
}

fn words(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[derive(Clone, Copy)]
enum B {
    /// uniform
    U,
    /// read-only storage
    R,
    /// read-write storage
    W,
}

struct Kernel {
    bgl: wgpu::BindGroupLayout,
    pipe: wgpu::ComputePipeline,
}

struct Kernels {
    mv_q8: Kernel,
    tile_q8: Kernel,
    mv_f16: Kernel,
    tile_f16: Kernel,
    /// 16-bit tiled products (only when the GPU has shader-f16 and they compiled)
    tile16: Option<(Kernel, Kernel)>,
    norm: Kernel,
    rope: Kernel,
    attn: Kernel,
    ew: Kernel,
    argmax: Kernel,
    conv: Kernel,
    gather: Kernel,
}

fn layout(device: &wgpu::Device, kinds: &[B]) -> wgpu::BindGroupLayout {
    let entries: Vec<_> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| wgpu::BindGroupLayoutEntry {
            binding: i as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: match k {
                    B::U => wgpu::BufferBindingType::Uniform,
                    B::R => wgpu::BufferBindingType::Storage { read_only: true },
                    B::W => wgpu::BufferBindingType::Storage { read_only: false },
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("overfilm-asr"), entries: &entries })
}

fn kernel(device: &wgpu::Device, src: &str, entry: &str, bgl: &wgpu::BindGroupLayout) -> Kernel {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(entry), source: wgpu::ShaderSource::Wgsl(src.into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
    let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(&pl),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    });
    Kernel { bgl: bgl.clone(), pipe }
}

/// Poll a future once: wgpu's native error scopes resolve immediately.
fn poll_now<F: std::future::Future>(f: F) -> Option<F::Output> {
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())) {
        std::task::Poll::Ready(v) => Some(v),
        std::task::Poll::Pending => None,
    }
}

/// A GPU device with the compiled kernels.
pub struct Ctx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    err: Arc<Mutex<Option<String>>>,
    max_bind: u64,
    /// adapter name and backend, for the user
    pub name: String,
    k: Kernels,
    zero: wgpu::Buffer,
    /// the tiled product kernel matched the CPU (otherwise every product uses `mv`)
    tile: AtomicBool,
    /// prefills use the 16-bit tiled kernel (it matched the CPU and was faster)
    half: AtomicBool,
}

impl Ctx {
    /// The high-performance GPU, or an error when there is none (software renderers such as
    /// llvmpipe count as none unless `OVERFILM_GPU=any`; `OVERFILM_GPU=off` disables the GPU).
    pub fn new() -> Result<Arc<Self>, AsrError> {
        let mode = std::env::var("OVERFILM_GPU").unwrap_or_default();
        if mode == "off" {
            return Err(gpu_err("turned off (OVERFILM_GPU=off)"));
        }
        Self::with(mode == "any")
    }

    /// [`Ctx::new`], optionally accepting a software renderer (tests).
    pub fn with(software: bool) -> Result<Arc<Self>, AsrError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
        )
        .map_err(gpu_err)?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu && !software {
            return Err(gpu_err(format!("only a software renderer ({})", info.name)));
        }
        let al = adapter.limits();
        let cap = std::env::var("OVERFILM_GPU_MAX_BIND").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(MAX_BIND);
        let max_bind = al.max_storage_buffer_binding_size.min(al.max_buffer_size).min(cap).max(1 << 20);
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: max_bind.max(wgpu::Limits::default().max_storage_buffer_binding_size).min(al.max_storage_buffer_binding_size),
            max_buffer_size: max_bind.max(wgpu::Limits::default().max_buffer_size).min(al.max_buffer_size),
            ..wgpu::Limits::default()
        };
        let prefill = std::env::var("OVERFILM_GPU_PREFILL").unwrap_or_default();
        let features = if prefill != "f32" { adapter.features() & wgpu::Features::SHADER_F16 } else { wgpu::Features::empty() };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("overfilm-asr"),
            required_features: features,
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(gpu_err)?;
        let err = Arc::new(Mutex::new(None));
        let sink = err.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let mut g = sink.lock().unwrap_or_else(PoisonError::into_inner);
            if g.is_none() {
                *g = Some(e.to_string());
            }
        }));
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mm = layout(&device, &[B::U, B::U, B::R, B::R, B::R, B::R, B::W]);
        let q8 = format!("{MATMUL}\n{Q8}");
        let f16 = format!("{MATMUL}\n{F16}");
        let k = Kernels {
            mv_q8: kernel(&device, &q8, "mv", &mm),
            tile_q8: kernel(&device, &q8, "tile", &mm),
            mv_f16: kernel(&device, &f16, "mv", &mm),
            tile_f16: kernel(&device, &f16, "tile", &mm),
            tile16: None,
            norm: kernel(&device, NORM, "main", &layout(&device, &[B::U, B::U, B::R, B::R, B::R, B::W])),
            rope: kernel(&device, ROPE, "main", &layout(&device, &[B::U, B::U, B::R, B::R, B::R, B::W, B::W, B::W, B::R])),
            attn: kernel(&device, ATTN, "main", &layout(&device, &[B::U, B::U, B::R, B::R, B::R, B::W, B::R])),
            ew: kernel(&device, EW, "main", &layout(&device, &[B::U, B::U, B::R, B::W])),
            argmax: kernel(&device, ARGMAX, "main", &layout(&device, &[B::U, B::U, B::R, B::W])),
            conv: kernel(&device, CONV, "main", &layout(&device, &[B::U, B::R, B::R, B::R, B::R, B::W])),
            gather: kernel(&device, GATHER, "main", &layout(&device, &[B::U, B::R, B::W])),
        };
        if let Some(Some(e)) = poll_now(scope.pop()) {
            return Err(gpu_err(format!("shaders: {e}")));
        }
        let mut k = k;
        if features.contains(wgpu::Features::SHADER_F16) {
            // optional: a driver that rejects it just keeps the 32-bit kernel
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let q8 = kernel(&device, &format!("enable f16;\n{MATMUL}\n{Q8}\n{TILE16}"), "tile16", &mm);
            let f16 = kernel(&device, &format!("enable f16;\n{MATMUL}\n{F16}\n{TILE16}"), "tile16", &mm);
            if let Some(None) = poll_now(scope.pop()) {
                k.tile16 = Some((q8, f16));
            }
        }
        let zero =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("zero"), contents: &[0u8; 16], usage: wgpu::BufferUsages::STORAGE });
        let name = format!("{} ({:?}, {} {})", info.name, info.backend, info.driver, info.driver_info).replace(" ,", "").replace("  ", " ");
        let ctx = Self { device, queue, err, max_bind, name, k, zero, tile: AtomicBool::new(true), half: AtomicBool::new(false) };
        // Some drivers get the tiled kernel wrong; the row-by-row one then does whole prefills.
        if !ctx.scoped(|| check::tile_ok(&ctx)).unwrap_or(false) {
            ctx.tile.store(false, Ordering::Relaxed);
        } else if ctx.k.tile16.is_some() {
            // 16-bit prefills when they match the CPU and are measurably faster (software
            // renderers skip the timing: `OVERFILM_GPU_PREFILL=f16` forces them, `f32` turns them off)
            ctx.half.store(true, Ordering::Relaxed);
            let ok = ctx.scoped(|| check::tile_ok(&ctx)).unwrap_or(false);
            let faster = prefill == "f16" || (info.device_type != wgpu::DeviceType::Cpu && ctx.scoped(|| check::tile16_faster(&ctx)).unwrap_or(false));
            ctx.half.store(ok && faster, Ordering::Relaxed);
        }
        Ok(Arc::new(ctx))
    }

    /// Which product kernel prefills use, for the user.
    pub fn prefill_kind(&self) -> &'static str {
        if !self.tiled() {
            "row by row"
        } else if self.half() {
            "16-bit tiles"
        } else if self.k.tile16.is_some() {
            "32-bit tiles (16-bit available but not faster)"
        } else {
            "32-bit tiles"
        }
    }

    /// Whether prefills use the 16-bit tiled kernel.
    pub fn half(&self) -> bool {
        self.tiled() && self.k.tile16.is_some() && self.half.load(Ordering::Relaxed)
    }

    /// Switch the 16-bit tiled kernel on or off (self-check and timing).
    pub(crate) fn set_half(&self, on: bool) {
        self.half.store(on, Ordering::Relaxed);
    }

    /// Whether prefills use the tiled product kernel (it failed the start-up comparison otherwise).
    pub fn tiled(&self) -> bool {
        self.tile.load(Ordering::Relaxed)
    }

    fn check(&self) -> Result<(), AsrError> {
        match self.err.lock().unwrap_or_else(PoisonError::into_inner).take() {
            Some(e) => Err(gpu_err(e)),
            None => Ok(()),
        }
    }

    /// Run `f` (buffer creation) and fail on any out-of-memory or validation error inside it.
    pub fn scoped<T>(&self, f: impl FnOnce() -> Result<T, AsrError>) -> Result<T, AsrError> {
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let val = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let r = f();
        let v = poll_now(val.pop());
        let o = poll_now(oom.pop());
        if let Some(Some(e)) = v {
            return Err(gpu_err(e));
        }
        if let Some(Some(e)) = o {
            return Err(gpu_err(format!("out of GPU memory: {e}")));
        }
        self.check()?;
        r
    }

    fn storage(&self, bytes: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: bytes.max(16).div_ceil(16) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    fn floats(&self, n: usize) -> wgpu::Buffer {
        self.storage(n as u64 * 4)
    }

    fn upload(&self, bytes: &[u8]) -> wgpu::Buffer {
        let mut v = bytes.to_vec();
        v.resize(v.len().max(16).div_ceil(16) * 16, 0);
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &v,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        })
    }

    fn upload_f32(&self, v: &[f32]) -> wgpu::Buffer {
        self.upload(&v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }

    fn uniform(&self, v: &[u32]) -> wgpu::Buffer {
        let mut b = words(v);
        b.resize(b.len().max(16).div_ceil(16) * 16, 0);
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &b,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn bind(&self, k: &Kernel, bufs: &[&wgpu::Buffer]) -> wgpu::BindGroup {
        let entries: Vec<_> = bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &k.bgl, entries: &entries })
    }

    fn staging(&self, bytes: u64) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read"),
            size: bytes.max(16).div_ceil(16) * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    /// Submit `enc` after copying `bytes` of `src` to `staging`, wait, and return them.
    fn finish_read(&self, mut enc: wgpu::CommandEncoder, src: &wgpu::Buffer, staging: &wgpu::Buffer, bytes: u64) -> Result<Vec<u8>, AsrError> {
        enc.copy_buffer_to_buffer(src, 0, staging, 0, bytes);
        let idx = self.queue.submit([enc.finish()]);
        let slice = staging.slice(..bytes);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: Some(WAIT) }).map_err(gpu_err)?;
        self.check()?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|_| gpu_err("read-back did not finish"))?.map_err(gpu_err)?;
        let out = slice.get_mapped_range().map_err(gpu_err)?.to_vec();
        staging.unmap();
        Ok(out)
    }

    fn submit(&self, enc: wgpu::CommandEncoder) -> Result<(), AsrError> {
        self.queue.submit([enc.finish()]);
        self.check()
    }
}

fn groups2(n: usize) -> (u32, u32) {
    let n = n.max(1) as u32;
    let x = n.min(65_535);
    (x, n.div_ceil(x))
}

/// Weight storage format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Q8,
    F16,
}

struct Part {
    qs: wgpu::Buffer,
    sc: wgpu::Buffer,
    off: usize,
    n: usize,
}

/// An `n × k` weight matrix (rows are output features), optionally with a bias, split by rows
/// into parts no larger than one binding.
pub struct Weight {
    fmt: Fmt,
    k: usize,
    n: usize,
    parts: Vec<Part>,
    bias: Option<wgpu::Buffer>,
}

/// 8-bit rows: per 32 weights, scale = max |w| / 127.
fn quantize_rows(w: &[f32]) -> (Vec<u8>, Vec<f32>) {
    let mut qs = Vec::with_capacity(w.len());
    let mut sc = Vec::with_capacity(w.len() / 32 + 1);
    for blk in w.chunks(32) {
        let amax = blk.iter().fold(0f32, |a, x| a.max(x.abs()));
        let s = amax / 127.0;
        let inv = if s > 0.0 { 1.0 / s } else { 0.0 };
        sc.push(s);
        qs.extend(blk.iter().map(|x| ((x * inv).round().clamp(-127.0, 127.0) as i8) as u8));
    }
    (qs, sc)
}

fn f16_words(w: &[f32]) -> Vec<u8> {
    w.iter().flat_map(|x| half::f16::from_f32(*x).to_bits().to_le_bytes()).collect()
}

impl Weight {
    pub fn new(ctx: &Ctx, w: &[f32], n: usize, k: usize, fmt: Fmt, bias: Option<&[f32]>) -> Result<Self, AsrError> {
        if n == 0 || k == 0 || !k.is_multiple_of(32) || w.len() != n.saturating_mul(k) || bias.is_some_and(|b| b.len() != n) {
            return Err(gpu_err(format!("weight {n}×{k} with {} values", w.len())));
        }
        let row_bytes = match fmt {
            Fmt::Q8 => k,
            Fmt::F16 => 2 * k,
        } as u64;
        let rows = usize::try_from(ctx.max_bind / row_bytes).unwrap_or(usize::MAX) / 64 * 64;
        if rows == 0 {
            return Err(gpu_err(format!("weight row of {k} does not fit a binding")));
        }
        let mut parts = Vec::new();
        let mut off = 0;
        while off < n {
            let pn = rows.min(n - off);
            let data = w.get(off * k..(off + pn) * k).ok_or_else(|| gpu_err("weight slice"))?;
            let (qs, sc) = match fmt {
                Fmt::Q8 => {
                    let (q, s) = quantize_rows(data);
                    (ctx.upload(&q), ctx.upload_f32(&s))
                }
                Fmt::F16 => (ctx.upload(&f16_words(data)), ctx.zero.clone()),
            };
            parts.push(Part { qs, sc, off, n: pn });
            off += pn;
        }
        Ok(Self { fmt, k, n, parts, bias: bias.map(|b| ctx.upload_f32(b)) })
    }

    /// An 8-bit `n × k` weight already quantized (the same blocks [`quantize_rows`] makes).
    pub fn from_q8(ctx: &Ctx, qs: &[i8], sc: &[f32], n: usize, k: usize) -> Result<Self, AsrError> {
        if n == 0 || k == 0 || !k.is_multiple_of(32) || qs.len() != n.saturating_mul(k) || sc.len() != qs.len() / 32 {
            return Err(gpu_err(format!("8-bit weight {n}×{k} with {} values", qs.len())));
        }
        let rows = usize::try_from(ctx.max_bind / k as u64).unwrap_or(usize::MAX) / 64 * 64;
        if rows == 0 {
            return Err(gpu_err(format!("weight row of {k} does not fit a binding")));
        }
        let mut parts = Vec::new();
        let mut off = 0;
        while off < n {
            let pn = rows.min(n - off);
            let q = qs.get(off * k..(off + pn) * k).ok_or_else(|| gpu_err("weight slice"))?;
            let s = sc.get(off * k / 32..(off + pn) * k / 32).ok_or_else(|| gpu_err("scale slice"))?;
            parts.push(Part { qs: ctx.upload(bytemuck::cast_slice(q)), sc: ctx.upload_f32(s), off, n: pn });
            off += pn;
        }
        Ok(Self { fmt: Fmt::Q8, k, n, parts, bias: None })
    }
}

/// A product bound to its input and output buffers.
struct Lin {
    fmt: Fmt,
    parts: Vec<(wgpu::BindGroup, usize)>,
    rows1: bool,
}

struct LinSpec<'a> {
    x: &'a wgpu::Buffer,
    y: &'a wgpu::Buffer,
    step: &'a wgpu::Buffer,
    /// output row stride
    ldy: usize,
    /// add to the output (residual) instead of overwriting it
    acc: bool,
    /// always one input row (the last-position head)
    rows1: bool,
}

impl Lin {
    fn new(ctx: &Ctx, w: &Weight, s: LinSpec) -> Self {
        let k = match w.fmt {
            Fmt::Q8 => &ctx.k.mv_q8,
            Fmt::F16 => &ctx.k.mv_f16,
        };
        let bias = w.bias.as_ref().unwrap_or(&ctx.zero);
        let parts = w
            .parts
            .iter()
            .map(|p| {
                let u =
                    ctx.uniform(&[w.k as u32, p.n as u32, s.ldy as u32, p.off as u32, u32::from(s.acc), u32::from(w.bias.is_some()), u32::from(s.rows1), 0]);
                (ctx.bind(k, &[&u, s.step, s.x, &p.qs, &p.sc, bias, s.y]), p.n)
            })
            .collect();
        Self { fmt: w.fmt, parts, rows1: s.rows1 }
    }

    fn record(&self, ctx: &Ctx, pass: &mut wgpu::ComputePass, rows: usize) {
        let rows = if self.rows1 { 1 } else { rows };
        let small = rows <= MV_ROWS || !ctx.tiled();
        let half = if small || !ctx.half() { None } else { ctx.k.tile16.as_ref() };
        let k = match (self.fmt, small, half) {
            (Fmt::Q8, false, Some((q8, _))) => q8,
            (Fmt::F16, false, Some((_, f16))) => f16,
            (Fmt::Q8, true, _) => &ctx.k.mv_q8,
            (Fmt::Q8, false, None) => &ctx.k.tile_q8,
            (Fmt::F16, true, _) => &ctx.k.mv_f16,
            (Fmt::F16, false, None) => &ctx.k.tile_f16,
        };
        pass.set_pipeline(&k.pipe);
        for (bg, n) in &self.parts {
            pass.set_bind_group(0, bg, &[]);
            if small {
                let (x, y) = groups2(n.div_ceil(4));
                pass.dispatch_workgroups(x, y, rows.div_ceil(MV_ROWS).max(1) as u32);
            } else {
                pass.dispatch_workgroups(n.div_ceil(64) as u32, rows.div_ceil(64) as u32, 1);
            }
        }
    }
}

/// Uploaded weights by name, gathered as the checkpoint streams in: q/k/v (and gate/up) are fused
/// into one product once all their pieces have arrived.
pub struct Store {
    dec_fmt: Fmt,
    pending: HashMap<String, (Vec<usize>, Pending)>,
    weights: HashMap<String, Weight>,
    vectors: HashMap<String, Vec<f32>>,
}

/// A tensor waiting for the rest of its fused product.
enum Pending {
    F32(Vec<f32>),
    Q8(std::sync::Arc<crate::qmm::Q8>),
}

/// The fused product a weight or bias belongs to, and every tensor it needs.
fn group(name: &str) -> Option<(String, Vec<String>)> {
    let base = name.strip_suffix(".weight").or_else(|| name.strip_suffix(".bias"))?;
    let enc = name.contains("audio_tower");
    let (parent, last) = base.rsplit_once('.')?;
    let with_bias = |b: &str| -> Vec<String> {
        let mut v = vec![format!("{b}.weight")];
        if enc {
            v.push(format!("{b}.bias"));
        }
        v
    };
    match last {
        "q_proj" | "k_proj" | "v_proj" => {
            Some((format!("{parent}.qkv"), ["q_proj", "k_proj", "v_proj"].iter().flat_map(|p| with_bias(&format!("{parent}.{p}"))).collect()))
        }
        "gate_proj" | "up_proj" => Some((format!("{parent}.gu"), vec![format!("{parent}.gate_proj.weight"), format!("{parent}.up_proj.weight")])),
        "o_proj" | "down_proj" | "out_proj" | "fc1" | "fc2" | "proj1" | "proj2" | "lm_head" => Some((base.to_string(), with_bias(base))),
        _ => None,
    }
}

impl Store {
    pub fn new(dec_fmt: Fmt) -> Self {
        Self { dec_fmt, pending: HashMap::new(), weights: HashMap::new(), vectors: HashMap::new() }
    }

    /// Take one tensor (row-major f32).
    pub fn add(&mut self, ctx: &Ctx, name: &str, shape: &[usize], data: Vec<f32>) -> Result<(), AsrError> {
        if group(name).is_none() {
            self.vectors.insert(name.to_string(), data);
            return Ok(());
        }
        self.add_pending(ctx, name, shape, Pending::F32(data))
    }

    /// Take one decoder weight already in 8 bits (from the weight cache, or quantized on the CPU
    /// while loading so it can be cached).
    pub fn add_q8(&mut self, ctx: &Ctx, name: &str, q: std::sync::Arc<crate::qmm::Q8>) -> Result<(), AsrError> {
        let (n, k) = q.dims();
        self.add_pending(ctx, name, &[n, k], Pending::Q8(q))
    }

    fn add_pending(&mut self, ctx: &Ctx, name: &str, shape: &[usize], data: Pending) -> Result<(), AsrError> {
        let Some((key, members)) = group(name) else {
            return Err(gpu_err(format!("{name}: not part of a product")));
        };
        self.pending.insert(name.to_string(), (shape.to_vec(), data));
        if !members.iter().all(|m| self.pending.contains_key(m)) {
            return Ok(());
        }
        let (mut w, mut b, mut n, mut k) = (Vec::new(), Vec::new(), 0usize, 0usize);
        let (mut qs, mut sc): (Vec<i8>, Vec<f32>) = (Vec::new(), Vec::new());
        for m in &members {
            let Some((shape, v)) = self.pending.remove(m) else { continue };
            let v = match v {
                Pending::F32(v) => v,
                Pending::Q8(q) => {
                    let (rows, cols) = q.dims();
                    if (k != 0 && cols != k) || !w.is_empty() || m.ends_with(".bias") {
                        return Err(gpu_err(format!("{m}: cannot fuse 8-bit {rows}×{cols} here")));
                    }
                    k = cols;
                    n += rows;
                    qs.extend_from_slice(q.values());
                    sc.extend_from_slice(q.scales());
                    continue;
                }
            };
            if m.ends_with(".bias") {
                b.extend_from_slice(&v);
            } else {
                let (&rows, &cols) = (shape.first().unwrap_or(&0), shape.get(1).unwrap_or(&0));
                if k != 0 && cols != k || !qs.is_empty() {
                    return Err(gpu_err(format!("{m}: {cols} columns, expected {k}")));
                }
                k = cols;
                n += rows;
                w.extend_from_slice(&v);
            }
        }
        let weight = if qs.is_empty() {
            let fmt = if key.contains("audio_tower") { Fmt::F16 } else { self.dec_fmt };
            Weight::new(ctx, &w, n, k, fmt, (!b.is_empty()).then_some(&b[..]))?
        } else {
            if !b.is_empty() {
                return Err(gpu_err(format!("{key}: 8-bit weight with a bias")));
            }
            Weight::from_q8(ctx, &qs, &sc, n, k)?
        };
        self.weights.insert(key, weight);
        Ok(())
    }

    /// Add a weight directly (the tied output head).
    pub fn add_weight(&mut self, ctx: &Ctx, key: &str, w: &[f32], n: usize, k: usize) -> Result<(), AsrError> {
        let weight = Weight::new(ctx, w, n, k, self.dec_fmt, None)?;
        self.weights.insert(key.to_string(), weight);
        Ok(())
    }

    fn weight(&mut self, key: &str) -> Result<Weight, AsrError> {
        self.weights.remove(key).ok_or_else(|| gpu_err(format!("missing weight {key}")))
    }

    fn vector(&mut self, key: &str, len: usize) -> Result<Vec<f32>, AsrError> {
        match self.vectors.remove(key) {
            Some(v) if v.len() == len => Ok(v),
            Some(v) => Err(gpu_err(format!("{key}: {} values, expected {len}", v.len()))),
            None => Err(gpu_err(format!("missing {key}"))),
        }
    }
}

fn norm_bg(ctx: &Ctx, step: &wgpu::Buffer, cols: usize, layer: bool, last: bool, eps: f32, bufs: [&wgpu::Buffer; 4]) -> wgpu::BindGroup {
    let u = ctx.uniform(&[cols as u32, u32::from(layer), u32::from(last), eps.to_bits()]);
    let [x, w, b, y] = bufs;
    ctx.bind(&ctx.k.norm, &[&u, step, x, w, b, y])
}

fn dispatch_rows(pass: &mut wgpu::ComputePass, k: &Kernel, bg: &wgpu::BindGroup, x: u32, y: u32) {
    pass.set_pipeline(&k.pipe);
    pass.set_bind_group(0, bg, &[]);
    pass.dispatch_workgroups(x.max(1), y.max(1), 1);
}

fn dispatch_ew(pass: &mut wgpu::ComputePass, ctx: &Ctx, bg: &wgpu::BindGroup, elems: usize) {
    let (x, y) = groups2(elems.div_ceil(256));
    dispatch_rows(pass, &ctx.k.ew, bg, x, y);
}

/// Decoder sizes.
#[derive(Clone, Copy, Debug)]
pub struct TextDims {
    pub hidden: usize,
    pub inter: usize,
    pub layers: usize,
    pub hq: usize,
    pub hkv: usize,
    pub dh: usize,
    pub eps: f32,
    pub theta: f64,
    pub head_rows: usize,
    /// key/value cache positions ([`KV_CAP`] or [`KV_CAP_SIDE`])
    pub kv_cap: usize,
}

struct DecLayer {
    ln1: wgpu::BindGroup,
    qkv: Lin,
    rope: wgpu::BindGroup,
    attn: wgpu::BindGroup,
    o: Lin,
    ln2: wgpu::BindGroup,
    gu: Lin,
    act: wgpu::BindGroup,
    down: Lin,
}

struct Head {
    norm: wgpu::BindGroup,
    lin: Lin,
    arg: wgpu::BindGroup,
    ids: wgpu::Buffer,
    logits: wgpu::Buffer,
}

/// The Qwen3 decoder on the GPU with its key/value cache.
pub struct Text {
    ctx: Arc<Ctx>,
    d: TextDims,
    step: wgpu::Buffer,
    h: wgpu::Buffer,
    layers: Vec<DecLayer>,
    last: Head,
    all: Option<Head>,
    seqs: Head,
    ri: wgpu::Buffer,
    read: wgpu::Buffer,
    _keep: Vec<wgpu::Buffer>,
    /// tokens in the key/value cache
    pub pos: usize,
    /// Softmax probability of the class returned by the last `last_only` forward.
    pub last_p: f32,
}

impl Text {
    /// `prefix`: `thinker` (tensors `prefix.model.layers.N…`, `prefix.model.norm`, `prefix.lm_head`).
    pub fn new(ctx: &Arc<Ctx>, d: TextDims, st: &mut Store, prefix: &str) -> Result<Self, AsrError> {
        let c = ctx.as_ref();
        if d.hq == 0 || d.hkv == 0 || !d.hq.is_multiple_of(d.hkv) || d.dh == 0 || d.dh > 256 || !d.dh.is_multiple_of(4) {
            return Err(gpu_err(format!("unsupported heads {}/{} × {}", d.hq, d.hkv, d.dh)));
        }
        let r = MAX_ROWS;
        let qkv_n = (d.hq + 2 * d.hkv) * d.dh;
        let step = c.uniform(&[0, 0, 0, 0]);
        let h = c.floats(r * d.hidden);
        let hn = c.floats(r * d.hidden);
        let qkv = c.floats(r * qkv_n);
        let q = c.floats(r * d.hq * d.dh);
        let o = c.floats(r * d.hq * d.dh);
        let gu = c.floats(r * 2 * d.inter);
        let mid = c.floats(r * d.inter);
        let half = d.dh / 2;
        let inv: Vec<f32> = (0..half).map(|i| (1.0 / d.theta.powf(2.0 * i as f64 / d.dh as f64)) as f32).collect();
        let mut table = Vec::with_capacity(d.kv_cap * half * 2);
        for p in 0..d.kv_cap {
            for w in &inv {
                let a = p as f32 * w;
                table.push(a.cos());
                table.push(a.sin());
            }
        }
        let cs = c.upload_f32(&table);
        // per-row position, cache region and prefix length when rows are different sequences
        let ri = c.storage((r * 16) as u64);
        let kv_words = (d.hkv * d.kv_cap * half) as u64 * 4;
        let mut keep = vec![hn.clone(), qkv.clone(), q.clone(), o.clone(), gu.clone(), mid.clone(), cs.clone()];
        let mut layers = Vec::with_capacity(d.layers);
        let lin = |w: &Weight, x: &wgpu::Buffer, y: &wgpu::Buffer, ldy: usize, acc: bool| Lin::new(c, w, LinSpec { x, y, step: &step, ldy, acc, rows1: false });
        for i in 0..d.layers {
            let p = format!("{prefix}.model.layers.{i}");
            let ln1w = c.upload_f32(&st.vector(&format!("{p}.input_layernorm.weight"), d.hidden)?);
            let ln2w = c.upload_f32(&st.vector(&format!("{p}.post_attention_layernorm.weight"), d.hidden)?);
            let mut norms = st.vector(&format!("{p}.self_attn.q_norm.weight"), d.dh)?;
            norms.extend(st.vector(&format!("{p}.self_attn.k_norm.weight"), d.dh)?);
            let norms = c.upload_f32(&norms);
            let (kc, vc) = (c.storage(kv_words), c.storage(kv_words));
            let w_qkv = st.weight(&format!("{p}.self_attn.qkv"))?;
            let w_o = st.weight(&format!("{p}.self_attn.o_proj"))?;
            let w_gu = st.weight(&format!("{p}.mlp.gu"))?;
            let w_down = st.weight(&format!("{p}.mlp.down_proj"))?;
            if w_qkv.n != qkv_n || w_qkv.k != d.hidden || w_gu.n != 2 * d.inter || w_down.k != d.inter {
                return Err(gpu_err(format!("layer {i}: unexpected weight sizes")));
            }
            let ru = c.uniform(&[d.hq as u32, d.hkv as u32, d.dh as u32, d.kv_cap as u32, 0, d.eps.to_bits(), 0, 0]);
            let au = c.uniform(&[d.hq as u32, d.hkv as u32, d.dh as u32, d.kv_cap as u32, 0, ((d.dh as f32).powf(-0.5)).to_bits(), 0, 0]);
            let eu = c.uniform(&[d.inter as u32, 0, 0, 0]);
            layers.push(DecLayer {
                ln1: norm_bg(c, &step, d.hidden, false, false, d.eps, [&h, &ln1w, &c.zero, &hn]),
                qkv: lin(&w_qkv, &hn, &qkv, qkv_n, false),
                rope: c.bind(&c.k.rope, &[&ru, &step, &qkv, &norms, &cs, &q, &kc, &vc, &ri]),
                attn: c.bind(&c.k.attn, &[&au, &step, &q, &kc, &vc, &o, &ri]),
                o: lin(&w_o, &o, &h, d.hidden, true),
                ln2: norm_bg(c, &step, d.hidden, false, false, d.eps, [&h, &ln2w, &c.zero, &hn]),
                gu: lin(&w_gu, &hn, &gu, 2 * d.inter, false),
                act: c.bind(&c.k.ew, &[&eu, &step, &gu, &mid]),
                down: lin(&w_down, &mid, &h, d.hidden, true),
            });
            keep.extend([ln1w, ln2w, norms, kc, vc]);
        }
        let normw = c.upload_f32(&st.vector(&format!("{prefix}.model.norm.weight"), d.hidden)?);
        let w_head = st.weight(&format!("{prefix}.lm_head"))?;
        if w_head.n != d.head_rows || w_head.k != d.hidden {
            return Err(gpu_err("output head size"));
        }
        let head = |rows: usize, last: bool, probs: bool| -> Head {
            let x = c.floats(rows * d.hidden);
            let logits = c.floats(rows * d.head_rows);
            let ids = c.storage(rows as u64 * 8);
            let au = c.uniform(&[d.head_rows as u32, u32::from(last), u32::from(probs), 0]);
            Head {
                norm: norm_bg(c, &step, d.hidden, false, last, d.eps, [&h, &normw, &c.zero, &x]),
                lin: Lin::new(c, &w_head, LinSpec { x: &x, y: &logits, step: &step, ldy: d.head_rows, acc: false, rows1: last }),
                arg: c.bind(&c.k.argmax, &[&au, &step, &logits, &ids]),
                ids,
                logits,
            }
        };
        let last = head(1, true, false);
        // every row's classes (the aligner): only for small heads
        let all = ((MAX_ROWS * d.head_rows * 4) as u64 <= c.max_bind).then(|| head(MAX_ROWS, false, false));
        // one next token for each of several sequences, with probabilities
        let seqs = head(MAX_SEQS, false, true);
        keep.push(normw);
        let read = c.staging(MAX_ROWS as u64 * 8);
        Ok(Self { ctx: ctx.clone(), d, step, h, layers, last, all, seqs, ri, read, _keep: keep, pos: 0, last_p: 1.0 })
    }

    /// Run `x` (`t × hidden`, row-major) after the cached tokens (`cached`) or from position 0;
    /// returns the argmax class of the last row (`last_only`) or of every row.
    /// Positions the key/value cache holds.
    pub fn cap(&self) -> usize {
        self.d.kv_cap
    }

    pub fn forward(&mut self, x: &[f32], cached: bool, last_only: bool) -> Result<Vec<u32>, AsrError> {
        let c = self.ctx.clone();
        let d = self.d;
        let t = x.len() / d.hidden.max(1);
        if t == 0 || x.len() != t * d.hidden {
            return Err(gpu_err("input size"));
        }
        let start = if cached { self.pos } else { 0 };
        if start + t > d.kv_cap {
            return Err(gpu_err(format!("{} tokens exceed the cache ({})", start + t, d.kv_cap)));
        }
        if !last_only && self.all.is_none() {
            return Err(gpu_err("per-row output not available for this head"));
        }
        let mut ids = Vec::with_capacity(if last_only { 1 } else { t });
        let mut off = 0;
        while off < t {
            let rows = MAX_ROWS.min(t - off);
            let final_piece = off + rows == t;
            c.queue.write_buffer(&self.step, 0, &words(&[rows as u32, (start + off) as u32, rows as u32 - 1, 0]));
            let piece = x.get(off * d.hidden..(off + rows) * d.hidden).ok_or_else(|| gpu_err("input slice"))?;
            c.queue.write_buffer(&self.h, 0, &piece.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
            let mut enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("decoder") });
            let head = match (last_only, final_piece) {
                (true, true) => Some((&self.last, 1usize)),
                (false, _) => self.all.as_ref().map(|h| (h, rows)),
                (true, false) => None,
            };
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("decoder"), timestamp_writes: None });
                let r = rows as u32;
                for l in &self.layers {
                    dispatch_rows(&mut pass, &c.k.norm, &l.ln1, r, 1);
                    l.qkv.record(&c, &mut pass, rows);
                    dispatch_rows(&mut pass, &c.k.rope, &l.rope, (d.hq + 2 * d.hkv) as u32, r);
                    dispatch_rows(&mut pass, &c.k.attn, &l.attn, d.hq as u32, r);
                    l.o.record(&c, &mut pass, rows);
                    dispatch_rows(&mut pass, &c.k.norm, &l.ln2, r, 1);
                    l.gu.record(&c, &mut pass, rows);
                    dispatch_ew(&mut pass, &c, &l.act, rows * d.inter);
                    l.down.record(&c, &mut pass, rows);
                }
                if let Some((hd, n)) = head {
                    dispatch_rows(&mut pass, &c.k.norm, &hd.norm, n as u32, 1);
                    hd.lin.record(&c, &mut pass, n);
                    dispatch_rows(&mut pass, &c.k.argmax, &hd.arg, n as u32, 1);
                }
            }
            match head {
                Some((hd, n)) if last_only => {
                    // the class and, after it, its probability (see argmax.wgsl)
                    let b = c.finish_read(enc, &hd.ids, &self.read, 8)?;
                    ids.extend(b.get(..4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])));
                    self.last_p = b.get(4..8).map_or(1.0, |w| f32::from_le_bytes([w[0], w[1], w[2], w[3]]));
                    let _ = n;
                }
                Some((hd, n)) => {
                    let b = c.finish_read(enc, &hd.ids, &self.read, n as u64 * 4)?;
                    ids.extend(b.chunks_exact(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])));
                }
                None => c.submit(enc)?,
            }
            off += rows;
        }
        if cached {
            self.pos = start + t;
        }
        Ok(ids)
    }
}

impl Text {
    /// Rows that belong to different sequences sharing the cached prompt prefix (positions
    /// `0 .. plen`, see `rope.wgsl`): row `r` of `x` (`t × hidden`) is at position `rows[r].0`
    /// and keeps its keys and values in its own cache region from `rows[r].1`; `rows[r].2` is
    /// the prefix length. `each`: one row per sequence, each row's next token and probability
    /// (at most [`MAX_SEQS`] rows); else the rows are one sequence's prefill and only the last
    /// row's token comes back. `pos` (the prefix) is left as it is.
    pub fn forward_seqs(&mut self, x: &[f32], rows: &[(usize, usize, usize)], each: bool) -> Result<Vec<(u32, f32)>, AsrError> {
        let c = self.ctx.clone();
        let d = self.d;
        let t = x.len() / d.hidden.max(1);
        if t == 0 || x.len() != t * d.hidden || rows.len() != t {
            return Err(gpu_err("input size"));
        }
        if each && t > MAX_SEQS {
            return Err(gpu_err(format!("{t} sequences at once (at most {MAX_SEQS})")));
        }
        for &(pos, base, plen) in rows {
            let place = if pos < plen { pos } else { base.saturating_add(pos - plen) };
            if pos >= d.kv_cap || place >= d.kv_cap || plen > d.kv_cap {
                return Err(gpu_err(format!("position {pos} (cache place {place}) outside the cache ({})", d.kv_cap)));
            }
        }
        let mut out = Vec::new();
        let mut off = 0;
        while off < t {
            let n = MAX_ROWS.min(t - off);
            let final_piece = off + n == t;
            let info: Vec<u32> = rows.iter().skip(off).take(n).flat_map(|&(p, b, l)| [p as u32, b as u32, l as u32, 0]).collect();
            c.queue.write_buffer(&self.ri, 0, &words(&info));
            c.queue.write_buffer(&self.step, 0, &words(&[n as u32, 0, n as u32 - 1, 1]));
            let piece = x.get(off * d.hidden..(off + n) * d.hidden).ok_or_else(|| gpu_err("input slice"))?;
            c.queue.write_buffer(&self.h, 0, &piece.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
            let mut enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("decoder rows") });
            let head = match (each, final_piece) {
                (true, _) => Some((&self.seqs, n)),
                (false, true) => Some((&self.last, 1usize)),
                (false, false) => None,
            };
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("decoder rows"), timestamp_writes: None });
                let r = n as u32;
                for l in &self.layers {
                    dispatch_rows(&mut pass, &c.k.norm, &l.ln1, r, 1);
                    l.qkv.record(&c, &mut pass, n);
                    dispatch_rows(&mut pass, &c.k.rope, &l.rope, (d.hq + 2 * d.hkv) as u32, r);
                    dispatch_rows(&mut pass, &c.k.attn, &l.attn, d.hq as u32, r);
                    l.o.record(&c, &mut pass, n);
                    dispatch_rows(&mut pass, &c.k.norm, &l.ln2, r, 1);
                    l.gu.record(&c, &mut pass, n);
                    dispatch_ew(&mut pass, &c, &l.act, n * d.inter);
                    l.down.record(&c, &mut pass, n);
                }
                if let Some((hd, k)) = head {
                    dispatch_rows(&mut pass, &c.k.norm, &hd.norm, k as u32, 1);
                    hd.lin.record(&c, &mut pass, k);
                    dispatch_rows(&mut pass, &c.k.argmax, &hd.arg, k as u32, 1);
                }
            }
            let word = |b: &[u8], i: usize| b.get(4 * i..4 * i + 4).map(|w| [w[0], w[1], w[2], w[3]]);
            match head {
                Some((hd, k)) if each => {
                    // classes, then their probabilities (see argmax.wgsl)
                    let b = c.finish_read(enc, &hd.ids, &self.read, (2 * k) as u64 * 4)?;
                    for i in 0..k {
                        let id = word(&b, i).map_or(0, u32::from_le_bytes);
                        let p = word(&b, k + i).map_or(1.0, f32::from_le_bytes);
                        out.push((id, p));
                    }
                }
                Some((hd, _)) => {
                    let b = c.finish_read(enc, &hd.ids, &self.read, 8)?;
                    out.push((word(&b, 0).map_or(0, u32::from_le_bytes), word(&b, 1).map_or(1.0, f32::from_le_bytes)));
                }
                None => c.submit(enc)?,
            }
            off += n;
        }
        Ok(out)
    }
}

impl Text {
    /// The output scores of the last row of the last `last_only` forward (checks and debugging).
    pub fn last_logits(&self) -> Result<Vec<f32>, AsrError> {
        let c = &self.ctx;
        let n = self.d.head_rows as u64 * 4;
        let staging = c.staging(n);
        let enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("logits") });
        let b = c.finish_read(enc, &self.last.logits, &staging, n)?;
        Ok(b.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect())
    }
}

/// Audio encoder sizes.
#[derive(Clone, Copy, Debug)]
pub struct EncDims {
    pub d: usize,
    pub layers: usize,
    pub heads: usize,
    pub ffn: usize,
    pub out: usize,
    /// attention window in rows
    pub window: usize,
}

struct EncLayer {
    ln1: wgpu::BindGroup,
    qkv: Lin,
    split: wgpu::BindGroup,
    attn: wgpu::BindGroup,
    o: Lin,
    ln2: wgpu::BindGroup,
    fc1: Lin,
    gelu: wgpu::BindGroup,
    fc2: Lin,
}

/// The audio encoder's transformer layers and output projection on the GPU.
pub struct Encoder {
    ctx: Arc<Ctx>,
    e: EncDims,
    step: wgpu::Buffer,
    h: wgpu::Buffer,
    out: wgpu::Buffer,
    layers: Vec<EncLayer>,
    post: (wgpu::BindGroup, Lin, wgpu::BindGroup, Lin),
    read: wgpu::Buffer,
    _keep: Vec<wgpu::Buffer>,
}

impl Encoder {
    /// `prefix`: `thinker.audio_tower`.
    pub fn new(ctx: &Arc<Ctx>, e: EncDims, st: &mut Store, prefix: &str) -> Result<Self, AsrError> {
        let c = ctx.as_ref();
        if e.heads == 0 || !e.d.is_multiple_of(e.heads) || e.d / e.heads > 256 || e.window == 0 {
            return Err(gpu_err("unsupported encoder heads"));
        }
        let dh = e.d / e.heads;
        let r = MAX_ROWS;
        let step = c.uniform(&[0, 0, 0, 0]);
        let h = c.floats(r * e.d);
        let hn = c.floats(r * e.d);
        let qkv = c.floats(r * 3 * e.d);
        let q = c.floats(r * e.d);
        let o = c.floats(r * e.d);
        let f = c.floats(r * e.ffn);
        let g = c.floats(r * e.ffn);
        let p1 = c.floats(r * e.d);
        let p1g = c.floats(r * e.d);
        let out = c.floats(r * e.out);
        let kv_words = (e.heads * r * dh / 2) as u64 * 4;
        let (kc, vc) = (c.storage(kv_words), c.storage(kv_words));
        let mut keep = vec![hn.clone(), qkv.clone(), q.clone(), o.clone(), f.clone(), g.clone(), p1.clone(), p1g.clone(), kc.clone(), vc.clone()];
        let lin = |w: &Weight, x: &wgpu::Buffer, y: &wgpu::Buffer, ldy: usize, acc: bool| Lin::new(c, w, LinSpec { x, y, step: &step, ldy, acc, rows1: false });
        let su = c.uniform(&[e.heads as u32, e.heads as u32, dh as u32, r as u32, 1, 0, 0, 0]);
        let au = c.uniform(&[e.heads as u32, e.heads as u32, dh as u32, r as u32, e.window as u32, ((dh as f32).powf(-0.5)).to_bits(), 0, 0]);
        let fu = c.uniform(&[e.ffn as u32, 1, 0, 0]);
        let mut layers = Vec::with_capacity(e.layers);
        for i in 0..e.layers {
            let p = format!("{prefix}.layers.{i}");
            let mut v = |n: &str| st.vector(&format!("{p}.{n}"), e.d).map(|x| c.upload_f32(&x));
            let (l1w, l1b, l2w, l2b) =
                (v("self_attn_layer_norm.weight")?, v("self_attn_layer_norm.bias")?, v("final_layer_norm.weight")?, v("final_layer_norm.bias")?);
            let w_qkv = st.weight(&format!("{p}.self_attn.qkv"))?;
            let w_o = st.weight(&format!("{p}.self_attn.out_proj"))?;
            let w_1 = st.weight(&format!("{p}.fc1"))?;
            let w_2 = st.weight(&format!("{p}.fc2"))?;
            if w_qkv.n != 3 * e.d || w_1.n != e.ffn || w_2.k != e.ffn {
                return Err(gpu_err(format!("encoder layer {i}: unexpected weight sizes")));
            }
            layers.push(EncLayer {
                ln1: norm_bg(c, &step, e.d, true, false, 1e-5, [&h, &l1w, &l1b, &hn]),
                qkv: lin(&w_qkv, &hn, &qkv, 3 * e.d, false),
                split: c.bind(&c.k.rope, &[&su, &step, &qkv, &c.zero, &c.zero, &q, &kc, &vc, &c.zero]),
                attn: c.bind(&c.k.attn, &[&au, &step, &q, &kc, &vc, &o, &c.zero]),
                o: lin(&w_o, &o, &h, e.d, true),
                ln2: norm_bg(c, &step, e.d, true, false, 1e-5, [&h, &l2w, &l2b, &hn]),
                fc1: lin(&w_1, &hn, &f, e.ffn, false),
                gelu: c.bind(&c.k.ew, &[&fu, &step, &f, &g]),
                fc2: lin(&w_2, &g, &h, e.d, true),
            });
            keep.extend([l1w, l1b, l2w, l2b]);
        }
        let lnw = c.upload_f32(&st.vector(&format!("{prefix}.ln_post.weight"), e.d)?);
        let lnb = c.upload_f32(&st.vector(&format!("{prefix}.ln_post.bias"), e.d)?);
        let w_p1 = st.weight(&format!("{prefix}.proj1"))?;
        let w_p2 = st.weight(&format!("{prefix}.proj2"))?;
        if w_p2.n != e.out {
            return Err(gpu_err("encoder output size"));
        }
        let gu = c.uniform(&[e.d as u32, 1, 0, 0]);
        let post = (
            norm_bg(c, &step, e.d, true, false, 1e-5, [&h, &lnw, &lnb, &hn]),
            lin(&w_p1, &hn, &p1, e.d, false),
            c.bind(&c.k.ew, &[&gu, &step, &p1, &p1g]),
            lin(&w_p2, &p1g, &out, e.out, false),
        );
        keep.extend([lnw, lnb]);
        let read = c.staging((r * e.out) as u64 * 4);
        Ok(Self { ctx: ctx.clone(), e, step, h, out, layers, post, read, _keep: keep })
    }

    /// `x`: `n × d` (after the convolutions and positions) → `n × out`. Longer inputs than one
    /// pass holds run in passes of whole attention windows (windows never see each other, so the
    /// result is the same).
    pub fn forward(&self, x: &[f32]) -> Result<Vec<f32>, AsrError> {
        let e = self.e;
        let n = x.len() / e.d.max(1);
        let per = (MAX_ROWS / e.window.max(1)).max(1) * e.window.max(1);
        if n <= MAX_ROWS || per > MAX_ROWS {
            return self.forward_pass(x);
        }
        let mut out = Vec::with_capacity(n * e.out);
        for piece in x.chunks(per * e.d) {
            out.extend(self.forward_pass(piece)?);
        }
        Ok(out)
    }

    fn forward_pass(&self, x: &[f32]) -> Result<Vec<f32>, AsrError> {
        let c = &self.ctx;
        let e = self.e;
        let n = x.len() / e.d.max(1);
        if n == 0 || n > MAX_ROWS || x.len() != n * e.d {
            return Err(gpu_err(format!("encoder input of {} rows", n)));
        }
        c.queue.write_buffer(&self.step, 0, &words(&[n as u32, 0, n as u32 - 1, 0]));
        c.queue.write_buffer(&self.h, 0, &x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        let mut enc = c.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("encoder") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("encoder"), timestamp_writes: None });
            let r = n as u32;
            for l in &self.layers {
                dispatch_rows(&mut pass, &c.k.norm, &l.ln1, r, 1);
                l.qkv.record(c, &mut pass, n);
                dispatch_rows(&mut pass, &c.k.rope, &l.split, (3 * e.heads) as u32, r);
                dispatch_rows(&mut pass, &c.k.attn, &l.attn, e.heads as u32, r);
                l.o.record(c, &mut pass, n);
                dispatch_rows(&mut pass, &c.k.norm, &l.ln2, r, 1);
                l.fc1.record(c, &mut pass, n);
                dispatch_ew(&mut pass, c, &l.gelu, n * e.ffn);
                l.fc2.record(c, &mut pass, n);
            }
            let (ln, p1, gelu, p2) = &self.post;
            dispatch_rows(&mut pass, &c.k.norm, ln, r, 1);
            p1.record(c, &mut pass, n);
            dispatch_ew(&mut pass, c, gelu, n * e.d);
            p2.record(c, &mut pass, n);
        }
        let b = c.finish_read(enc, &self.out, &self.read, (n * e.out) as u64 * 4)?;
        Ok(b.chunks_exact(4).map(|w| f32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect())
    }
}

pub mod check;
#[cfg(test)]
mod tests;

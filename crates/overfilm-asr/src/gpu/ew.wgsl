// Element-wise: mode 0 SwiGLU (a = gate | up per row → silu(gate)·up), mode 1 GELU (erf).

struct P {
    n: u32,
    mode: u32,
    pad0: u32,
    pad1: u32,
}

struct Step {
    rows: u32,
    pos: u32,
    last: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: Step;
@group(0) @binding(2) var<storage, read> a: array<f32>;
@group(0) @binding(3) var<storage, read_write> y: array<f32>;

// Abramowitz–Stegun 7.1.26 (|error| < 1.5e-7)
fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.3275911 * abs(x));
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    return sign(x) * (1.0 - poly * exp(-x * x));
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let idx = gid.x + gid.y * nw.x * 256u;
    if idx >= s.rows * p.n {
        return;
    }
    if p.mode == 0u {
        let r = idx / p.n;
        let c = idx % p.n;
        let g = a[r * 2u * p.n + c];
        let u = a[r * 2u * p.n + p.n + c];
        y[idx] = g / (1.0 + exp(-g)) * u;
    } else {
        let x = a[idx];
        y[idx] = 0.5 * x * (1.0 + erf(x * 0.70710678118654752));
    }
}

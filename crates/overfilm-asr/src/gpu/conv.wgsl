// 2-D convolution for the speaker network (WeSpeaker ResNet34): input (cin, h, w), weights
// (cout, cin, k, k) with batch norm folded in, bias per output channel, then optionally a residual
// (cout, ho, wo) added and ReLU. One invocation per output value; consecutive invocations share an
// output channel, so they read the same weights.

struct P {
    cin: u32,
    h: u32,
    w: u32,
    cout: u32,
    ho: u32,
    wo: u32,
    k: u32,
    stride: u32,
    pad: u32,
    res: u32,
    relu: u32,
    pad0: u32,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> x: array<f32>;
@group(0) @binding(2) var<storage, read> wt: array<f32>;
@group(0) @binding(3) var<storage, read> bias: array<f32>;
@group(0) @binding(4) var<storage, read> r: array<f32>;
@group(0) @binding(5) var<storage, read_write> y: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let idx = gid.x + gid.y * nw.x * 64u;
    let plane = p.ho * p.wo;
    if idx >= p.cout * plane {
        return;
    }
    let oc = idx / plane;
    let rem = idx % plane;
    let oy = rem / p.wo;
    let ox = rem % p.wo;
    var acc = bias[oc];
    let y0 = i32(oy * p.stride) - i32(p.pad);
    let x0 = i32(ox * p.stride) - i32(p.pad);
    for (var ic = 0u; ic < p.cin; ic = ic + 1u) {
        let wbase = (oc * p.cin + ic) * p.k * p.k;
        let xbase = ic * p.h * p.w;
        for (var ky = 0u; ky < p.k; ky = ky + 1u) {
            let iy = y0 + i32(ky);
            if iy < 0 || iy >= i32(p.h) {
                continue;
            }
            for (var kx = 0u; kx < p.k; kx = kx + 1u) {
                let ix = x0 + i32(kx);
                if ix < 0 || ix >= i32(p.w) {
                    continue;
                }
                acc = acc + x[xbase + u32(iy) * p.w + u32(ix)] * wt[wbase + ky * p.k + kx];
            }
        }
    }
    if p.res != 0u {
        acc = acc + r[idx];
    }
    if p.relu != 0u {
        acc = max(acc, 0.0);
    }
    y[idx] = acc;
}

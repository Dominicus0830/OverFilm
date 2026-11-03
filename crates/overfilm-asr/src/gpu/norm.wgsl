// RMSNorm (mode 0) or LayerNorm (mode 1) of each row; `last`: only row s.last, written as row 0.

struct P {
    cols: u32,
    mode: u32,
    last: u32,
    eps: f32,
}

struct Step {
    rows: u32,
    pos: u32,
    last: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: Step;
@group(0) @binding(2) var<storage, read> x: array<f32>;
@group(0) @binding(3) var<storage, read> w: array<f32>;
@group(0) @binding(4) var<storage, read> b: array<f32>;
@group(0) @binding(5) var<storage, read_write> y: array<f32>;

var<workgroup> red: array<f32, 256>;

fn total(li: u32, v: f32) -> f32 {
    red[li] = v;
    workgroupBarrier();
    for (var st = 128u; st > 0u; st >>= 1u) {
        if li < st {
            red[li] += red[li + st];
        }
        workgroupBarrier();
    }
    let t = red[0];
    workgroupBarrier();
    return t;
}

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let row = wg.x + wg.y * nw.x;
    let nrows = select(s.rows, 1u, p.last != 0u);
    if row >= nrows {
        return;
    }
    let src = select(row, s.last, p.last != 0u) * p.cols;
    let dst = row * p.cols;
    let n = f32(p.cols);
    var mean = 0.0;
    if p.mode == 1u {
        var a = 0.0;
        for (var i = li; i < p.cols; i += 256u) {
            a += x[src + i];
        }
        mean = total(li, a) / n;
    }
    var q = 0.0;
    for (var i = li; i < p.cols; i += 256u) {
        let d = x[src + i] - mean;
        q += d * d;
    }
    let inv = inverseSqrt(total(li, q) / n + p.eps);
    for (var i = li; i < p.cols; i += 256u) {
        var o = (x[src + i] - mean) * inv * w[i];
        if p.mode == 1u {
            o += b[i];
        }
        y[dst + i] = o;
    }
}

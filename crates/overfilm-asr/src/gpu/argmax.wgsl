// Index of the largest value of each row (the first on ties). With a single row (the next token
// while decoding), also its probability under softmax, as f32 bits in out[1].

struct P {
    n: u32,
    rows1: u32,
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
@group(0) @binding(2) var<storage, read> x: array<f32>;
@group(0) @binding(3) var<storage, read_write> out: array<u32>;

var<workgroup> bv: array<f32, 256>;
var<workgroup> bi: array<u32, 256>;
var<workgroup> top: f32;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let row = wg.x;
    let nrows = select(s.rows, p.rows1, p.rows1 > 0u);
    if row >= nrows {
        return;
    }
    var best = -3.4e38;
    var at = 0xffffffffu;
    for (var i = li; i < p.n; i += 256u) {
        let v = x[row * p.n + i];
        if v > best {
            best = v;
            at = i;
        }
    }
    bv[li] = best;
    bi[li] = at;
    workgroupBarrier();
    for (var st = 128u; st > 0u; st >>= 1u) {
        if li < st {
            let ov = bv[li + st];
            let oi = bi[li + st];
            if ov > bv[li] || (ov == bv[li] && oi < bi[li]) {
                bv[li] = ov;
                bi[li] = oi;
            }
        }
        workgroupBarrier();
    }
    if li == 0u {
        out[row] = select(bi[0], 0u, bi[0] == 0xffffffffu);
        top = bv[0];
    }
    if nrows != 1u {
        return;
    }
    // softmax probability of the best class: 1 / sum(exp(v - best))
    workgroupBarrier();
    let m = top;
    var sum = 0.0;
    for (var i = li; i < p.n; i += 256u) {
        sum += exp(x[i] - m);
    }
    bv[li] = sum;
    workgroupBarrier();
    for (var st = 128u; st > 0u; st >>= 1u) {
        if li < st {
            bv[li] += bv[li + st];
        }
        workgroupBarrier();
    }
    if li == 0u {
        out[1] = bitcast<u32>(select(1.0 / bv[0], 0.0, bv[0] < 1.0));
    }
}

// y[r, n_off + n] (=|+=) x[r, :] · w[n, :] (+ bias[n_off + n]) for every row r of x and row n of
// one part of the weight. Two kernels: `vec` (≤ 4 rows: decoding one token, memory-bound) and
// `tile` (64×64 output tiles through workgroup memory: prefill, compute-bound).
// `w4(n, k4)` (prepended: q8.wgsl or f16.wgsl) gives weights k = 4·k4 … 4·k4 + 3 of row n.

struct P {
    k: u32,
    n: u32,
    ldy: u32,
    n_off: u32,
    acc: u32,
    has_bias: u32,
    rows1: u32,
    pad: u32,
}

struct Step {
    rows: u32,
    pos: u32,
    last: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<uniform> s: Step;
@group(0) @binding(2) var<storage, read> x: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> wq: array<u32>;
@group(0) @binding(4) var<storage, read> ws: array<f32>;
@group(0) @binding(5) var<storage, read> bias: array<f32>;
@group(0) @binding(6) var<storage, read_write> y: array<f32>;

fn rows() -> u32 {
    return select(s.rows, p.rows1, p.rows1 > 0u);
}

fn store(r: u32, n: u32, v: f32) {
    let i = r * p.ldy + p.n_off + n;
    var o = v;
    if p.has_bias != 0u {
        o += bias[p.n_off + n];
    }
    if p.acc != 0u {
        o += y[i];
    }
    y[i] = o;
}

const MV: u32 = 4u;
var<workgroup> red: array<f32, 1024>;

// 256 threads: 4 weight rows × 64 lanes; input rows 4·z … 4·z + 3 (z = workgroup id z), so it also
// does a whole prefill when the tiled kernel is unusable on this GPU.
@compute @workgroup_size(256)
fn mv(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let col = li / 64u;
    let lane = li % 64u;
    let n = (wg.x + wg.y * nw.x) * 4u + col;
    let base = wg.z * MV;
    let all = rows();
    if base >= all {
        return;
    }
    let nr = min(all - base, MV);
    let k4n = p.k / 4u;
    var acc = vec4<f32>(0.0);
    if n < p.n {
        for (var i = lane; i < k4n; i += 64u) {
            let w = w4(n, i);
            let xb = base * k4n + i;
            acc.x += dot(w, x[xb]);
            if nr > 1u {
                acc.y += dot(w, x[xb + k4n]);
            }
            if nr > 2u {
                acc.z += dot(w, x[xb + 2u * k4n]);
            }
            if nr > 3u {
                acc.w += dot(w, x[xb + 3u * k4n]);
            }
        }
    }
    let b = col * 64u + lane;
    red[b] = acc.x;
    red[256u + b] = acc.y;
    red[512u + b] = acc.z;
    red[768u + b] = acc.w;
    workgroupBarrier();
    for (var st = 32u; st > 0u; st >>= 1u) {
        if lane < st {
            red[b] += red[b + st];
            red[256u + b] += red[256u + b + st];
            red[512u + b] += red[512u + b + st];
            red[768u + b] += red[768u + b + st];
        }
        workgroupBarrier();
    }
    if lane == 0u && n < p.n {
        for (var r = 0u; r < nr; r++) {
            store(base + r, n, red[r * 256u + b]);
        }
    }
}

// [k][row] and [k][col] tiles, 16 k at a time; each thread owns a 4×4 block of the 64×64 output
// tile, kept in four vec4 accumulators (no dynamically indexed private arrays: some drivers
// miscompile them).
var<workgroup> xs: array<f32, 1024>;
var<workgroup> wt: array<f32, 1024>;

fn store4(r: u32, c: u32, v: vec4<f32>) {
    if r >= rows() {
        return;
    }
    if c < p.n {
        store(r, c, v.x);
    }
    if c + 1u < p.n {
        store(r, c + 1u, v.y);
    }
    if c + 2u < p.n {
        store(r, c + 2u, v.z);
    }
    if c + 3u < p.n {
        store(r, c + 3u, v.w);
    }
}

@compute @workgroup_size(256)
fn tile(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let nr = rows();
    let r0 = wg.y * 64u;
    let c0 = wg.x * 64u;
    let tx = li % 16u;
    let ty = li / 16u;
    let k4n = p.k / 4u;
    var a0 = vec4<f32>(0.0);
    var a1 = vec4<f32>(0.0);
    var a2 = vec4<f32>(0.0);
    var a3 = vec4<f32>(0.0);
    let lr = li / 4u;
    let kq = li % 4u;
    for (var k0 = 0u; k0 < p.k; k0 += 16u) {
        var a = vec4<f32>(0.0);
        if r0 + lr < nr {
            a = x[(r0 + lr) * k4n + k0 / 4u + kq];
        }
        let o = kq * 256u + lr;
        xs[o] = a.x;
        xs[o + 64u] = a.y;
        xs[o + 128u] = a.z;
        xs[o + 192u] = a.w;
        var b = vec4<f32>(0.0);
        if c0 + lr < p.n {
            b = w4(c0 + lr, k0 / 4u + kq);
        }
        wt[o] = b.x;
        wt[o + 64u] = b.y;
        wt[o + 128u] = b.z;
        wt[o + 192u] = b.w;
        workgroupBarrier();
        for (var kk = 0u; kk < 16u; kk++) {
            let xo = kk * 64u + ty * 4u;
            let wo = kk * 64u + tx * 4u;
            let wb = vec4<f32>(wt[wo], wt[wo + 1u], wt[wo + 2u], wt[wo + 3u]);
            a0 += xs[xo] * wb;
            a1 += xs[xo + 1u] * wb;
            a2 += xs[xo + 2u] * wb;
            a3 += xs[xo + 3u] * wb;
        }
        workgroupBarrier();
    }
    let r = r0 + ty * 4u;
    let c = c0 + tx * 4u;
    store4(r, c, a0);
    store4(r + 1u, c, a1);
    store4(r + 2u, c, a2);
    store4(r + 3u, c, a3);
}

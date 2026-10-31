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

// 256 threads: 4 weight rows × 64 lanes; up to MV input rows.
@compute @workgroup_size(256)
fn mv(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let col = li / 64u;
    let lane = li % 64u;
    let n = (wg.x + wg.y * nw.x) * 4u + col;
    let nr = min(rows(), MV);
    let k4n = p.k / 4u;
    var acc = array<f32, 4>(0.0, 0.0, 0.0, 0.0);
    if n < p.n {
        for (var i = lane; i < k4n; i += 64u) {
            let w = w4(n, i);
            for (var r = 0u; r < nr; r++) {
                acc[r] += dot(w, x[r * k4n + i]);
            }
        }
    }
    for (var r = 0u; r < MV; r++) {
        red[(r * 4u + col) * 64u + lane] = acc[r];
    }
    workgroupBarrier();
    for (var st = 32u; st > 0u; st >>= 1u) {
        if lane < st {
            for (var r = 0u; r < MV; r++) {
                let b = (r * 4u + col) * 64u;
                red[b + lane] += red[b + lane + st];
            }
        }
        workgroupBarrier();
    }
    if lane == 0u && n < p.n {
        for (var r = 0u; r < nr; r++) {
            store(r, n, red[(r * 4u + col) * 64u]);
        }
    }
}

// [k][row] and [k][col] tiles, 16 k at a time.
var<workgroup> xs: array<f32, 1024>;
var<workgroup> wt: array<f32, 1024>;

@compute @workgroup_size(256)
fn tile(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let nr = rows();
    let r0 = wg.y * 64u;
    let c0 = wg.x * 64u;
    let tx = li % 16u;
    let ty = li / 16u;
    let k4n = p.k / 4u;
    var acc: array<f32, 16>;
    for (var i = 0u; i < 16u; i++) {
        acc[i] = 0.0;
    }
    let lr = li / 4u;
    let kq = li % 4u;
    for (var k0 = 0u; k0 < p.k; k0 += 16u) {
        var a = vec4<f32>(0.0);
        if r0 + lr < nr {
            a = x[(r0 + lr) * k4n + k0 / 4u + kq];
        }
        xs[(kq * 4u) * 64u + lr] = a.x;
        xs[(kq * 4u + 1u) * 64u + lr] = a.y;
        xs[(kq * 4u + 2u) * 64u + lr] = a.z;
        xs[(kq * 4u + 3u) * 64u + lr] = a.w;
        var b = vec4<f32>(0.0);
        if c0 + lr < p.n {
            b = w4(c0 + lr, k0 / 4u + kq);
        }
        wt[(kq * 4u) * 64u + lr] = b.x;
        wt[(kq * 4u + 1u) * 64u + lr] = b.y;
        wt[(kq * 4u + 2u) * 64u + lr] = b.z;
        wt[(kq * 4u + 3u) * 64u + lr] = b.w;
        workgroupBarrier();
        for (var kk = 0u; kk < 16u; kk++) {
            let xa = vec4<f32>(xs[kk * 64u + ty * 4u], xs[kk * 64u + ty * 4u + 1u], xs[kk * 64u + ty * 4u + 2u], xs[kk * 64u + ty * 4u + 3u]);
            let wb = vec4<f32>(wt[kk * 64u + tx * 4u], wt[kk * 64u + tx * 4u + 1u], wt[kk * 64u + tx * 4u + 2u], wt[kk * 64u + tx * 4u + 3u]);
            for (var i = 0u; i < 4u; i++) {
                acc[i * 4u] += xa[i] * wb.x;
                acc[i * 4u + 1u] += xa[i] * wb.y;
                acc[i * 4u + 2u] += xa[i] * wb.z;
                acc[i * 4u + 3u] += xa[i] * wb.w;
            }
        }
        workgroupBarrier();
    }
    for (var i = 0u; i < 4u; i++) {
        let r = r0 + ty * 4u + i;
        for (var j = 0u; j < 4u; j++) {
            let c = c0 + tx * 4u + j;
            if r < nr && c < p.n {
                store(r, c, acc[i * 4u + j]);
            }
        }
    }
}

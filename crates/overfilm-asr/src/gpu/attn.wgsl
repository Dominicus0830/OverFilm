// Attention of each query row and head over the 16-bit key/value cache, one pass with a running
// softmax (128 keys at a time). window 0: causal (row r at position s.pos + r sees keys
// 0 ..= s.pos + r); else keys inside the row's block of `window` rows (audio encoder).
// Rows of different sequences (s.pad == 1, see rope.wgsl): row r sees positions 0 ..= ri[4r],
// the shared prefix (below ri[4r + 2]) at the start of the cache, the rest in its own region
// from ri[4r + 1].

struct P {
    hq: u32,
    hkv: u32,
    dh: u32,
    cap: u32,
    window: u32,
    scale: f32,
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
@group(0) @binding(2) var<storage, read> q: array<f32>;
@group(0) @binding(3) var<storage, read> kc: array<u32>;
@group(0) @binding(4) var<storage, read> vc: array<u32>;
@group(0) @binding(5) var<storage, read_write> o: array<f32>;
@group(0) @binding(6) var<storage, read> ri: array<u32>;

const B: u32 = 128u;
var<workgroup> qs: array<f32, 256>;
var<workgroup> ps: array<f32, 128>;
var<workgroup> red: array<f32, 128>;

fn reduce_max(li: u32, x: f32) -> f32 {
    red[li] = x;
    workgroupBarrier();
    for (var st = 64u; st > 0u; st >>= 1u) {
        if li < st {
            red[li] = max(red[li], red[li + st]);
        }
        workgroupBarrier();
    }
    let t = red[0];
    workgroupBarrier();
    return t;
}

fn reduce_sum(li: u32, x: f32) -> f32 {
    red[li] = x;
    workgroupBarrier();
    for (var st = 64u; st > 0u; st >>= 1u) {
        if li < st {
            red[li] += red[li + st];
        }
        workgroupBarrier();
    }
    let t = red[0];
    workgroupBarrier();
    return t;
}

@compute @workgroup_size(128)
fn main(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let h = wg.x;
    let row = wg.y;
    if row >= s.rows || h >= p.hq {
        return;
    }
    let dh = p.dh;
    let half = dh / 2u;
    let kvh = h / max(p.hq / max(p.hkv, 1u), 1u);
    for (var i = li; i < dh; i += B) {
        qs[i] = q[(row * p.hq + h) * dh + i];
    }
    workgroupBarrier();
    var lo = 0u;
    var hi = s.pos + row + 1u;
    // cache place of position k: k below `plen`, else base + k - plen
    var plen = 0xffffffffu;
    var base = 0u;
    if s.pad == 1u {
        hi = ri[row * 4u] + 1u;
        base = ri[row * 4u + 1u];
        plen = ri[row * 4u + 2u];
    }
    if p.window != 0u {
        lo = (row / p.window) * p.window;
        hi = min(lo + p.window, s.rows);
    }
    var m = -3.0e38;
    var l = 0.0;
    // output dims li and li + 128 (head_dim ≤ 256)
    var acc = array<f32, 2>(0.0, 0.0);
    for (var blk = lo; blk < hi; blk += B) {
        let k = blk + li;
        var sc = -3.0e38;
        if k < hi {
            let kb = (kvh * p.cap + select(base + k - plen, k, k < plen)) * half;
            var d = 0.0;
            for (var i = 0u; i < half; i++) {
                let kv = unpack2x16float(kc[kb + i]);
                d += qs[2u * i] * kv.x + qs[2u * i + 1u] * kv.y;
            }
            sc = d * p.scale;
        }
        let bm = reduce_max(li, sc);
        let mn = max(m, bm);
        var e = 0.0;
        if k < hi {
            e = exp(sc - mn);
        }
        ps[li] = e;
        let bs = reduce_sum(li, e);
        let corr = exp(m - mn);
        l = l * corr + bs;
        let nk = min(B, hi - blk);
        for (var t = 0u; t < 2u; t++) {
            let i = li + t * B;
            if i < dh {
                var a = acc[t] * corr;
                for (var j = 0u; j < nk; j++) {
                    let kj = blk + j;
                    let vv = unpack2x16float(vc[(kvh * p.cap + select(base + kj - plen, kj, kj < plen)) * half + i / 2u]);
                    a += ps[j] * select(vv.x, vv.y, (i & 1u) == 1u);
                }
                acc[t] = a;
            }
        }
        m = mn;
        workgroupBarrier();
    }
    for (var t = 0u; t < 2u; t++) {
        let i = li + t * B;
        if i < dh {
            o[(row * p.hq + h) * dh + i] = acc[t] / l;
        }
    }
}

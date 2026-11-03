// Split the fused q|k|v projection of each row. Mode 0 (decoder): per-head RMSNorm of q and k,
// rotary positions (cos/sin table by absolute position), q to `qo`, k and v to the 16-bit caches
// at their position. Mode 1 (audio encoder): no norm or rotation; positions start at 0.
// Rows of different sequences (s.pad == 1): row r's position and cache place come from
// ri[4r ..]: position, start of its own cache region, length of the shared prompt prefix at the
// start of the cache (positions below it live there, the rest in the row's region).
// Workgroup (head j of hq + 2·hkv, row).

struct P {
    hq: u32,
    hkv: u32,
    dh: u32,
    cap: u32,
    mode: u32,
    eps: f32,
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
@group(0) @binding(2) var<storage, read> qkv: array<f32>;
@group(0) @binding(3) var<storage, read> norms: array<f32>;
@group(0) @binding(4) var<storage, read> cs: array<f32>;
@group(0) @binding(5) var<storage, read_write> qo: array<f32>;
@group(0) @binding(6) var<storage, read_write> kc: array<u32>;
@group(0) @binding(7) var<storage, read_write> vc: array<u32>;
@group(0) @binding(8) var<storage, read> ri: array<u32>;

var<workgroup> red: array<f32, 128>;
var<workgroup> v: array<f32, 256>;

@compute @workgroup_size(128)
fn main(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let j = wg.x;
    let row = wg.y;
    let heads = p.hq + 2u * p.hkv;
    if row >= s.rows || j >= heads {
        return;
    }
    let dh = p.dh;
    let half = dh / 2u;
    let base = (row * heads + j) * dh;
    for (var i = li; i < dh; i += 128u) {
        v[i] = qkv[base + i];
    }
    workgroupBarrier();
    var pos = s.pos + row;
    var slot = pos;
    if s.pad == 1u {
        pos = ri[row * 4u];
        let plen = ri[row * 4u + 2u];
        slot = select(ri[row * 4u + 1u] + pos - plen, pos, pos < plen);
    }
    if p.mode == 0u && j < p.hq + p.hkv {
        var q = 0.0;
        for (var i = li; i < dh; i += 128u) {
            q += v[i] * v[i];
        }
        red[li] = q;
        workgroupBarrier();
        for (var st = 64u; st > 0u; st >>= 1u) {
            if li < st {
                red[li] += red[li + st];
            }
            workgroupBarrier();
        }
        let inv = inverseSqrt(red[0] / f32(dh) + p.eps);
        // norms: q_norm then k_norm
        let nb = select(dh, 0u, j < p.hq);
        // each pair (i, i + half) belongs to one thread
        for (var i = li; i < half; i += 128u) {
            let x1 = v[i] * inv * norms[nb + i];
            let x2 = v[i + half] * inv * norms[nb + i + half];
            let c = cs[(pos * half + i) * 2u];
            let sn = cs[(pos * half + i) * 2u + 1u];
            v[i] = x1 * c - x2 * sn;
            v[i + half] = x2 * c + x1 * sn;
        }
        workgroupBarrier();
    }
    if j < p.hq {
        for (var i = li; i < dh; i += 128u) {
            qo[(row * p.hq + j) * dh + i] = v[i];
        }
        return;
    }
    let is_k = j < p.hq + p.hkv;
    let kvh = select(j - p.hq - p.hkv, j - p.hq, is_k);
    let at = (kvh * p.cap + slot) * half;
    for (var i = li; i < half; i += 128u) {
        let pk = pack2x16float(vec2<f32>(v[2u * i], v[2u * i + 1u]));
        if is_k {
            kc[at + i] = pk;
        } else {
            vc[at + i] = pk;
        }
    }
}

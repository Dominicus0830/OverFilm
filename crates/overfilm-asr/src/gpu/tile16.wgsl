// 16-bit variant of `tile` (needs shader-f16; prepended: `enable f16;`, matmul.wgsl and the weight
// format). The 64×64 tiles sit in workgroup memory as f16 and are multiplied with packed 16-bit
// math; each run of 16 products is carried into f32 accumulators, so long sums stay 32-bit.
// Inputs are scaled by 1/16 (and clamped) on the way into f16 so large activations cannot overflow
// to infinity; the scale is undone in f32 at the end. The start-up self-check compares this kernel
// with the CPU and it is only used when it matches (and is faster than `tile`).

const XSCALE: f32 = 0.0625;
const XLIM: f32 = 65000.0;

// [k][row / 4] and [k][col / 4]: element (k, row) is component row % 4 of xh[k·16 + row / 4]
var<workgroup> xh: array<vec4<f16>, 256>;
var<workgroup> wh: array<vec4<f16>, 256>;

fn half4(v: vec4<f32>) -> vec4<f16> {
    return vec4<f16>(clamp(v, vec4<f32>(-XLIM), vec4<f32>(XLIM)));
}

@compute @workgroup_size(256)
fn tile16(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wg: vec3<u32>) {
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
    let q = lr / 4u;
    let e = lr % 4u;
    let o = kq * 64u + q;
    for (var k0 = 0u; k0 < p.k; k0 += 16u) {
        var a = vec4<f32>(0.0);
        if r0 + lr < nr {
            a = x[(r0 + lr) * k4n + k0 / 4u + kq];
        }
        let ah = half4(a * XSCALE);
        xh[o][e] = ah.x;
        xh[o + 16u][e] = ah.y;
        xh[o + 32u][e] = ah.z;
        xh[o + 48u][e] = ah.w;
        var b = vec4<f32>(0.0);
        if c0 + lr < p.n {
            b = w4(c0 + lr, k0 / 4u + kq);
        }
        let bh = half4(b);
        wh[o][e] = bh.x;
        wh[o + 16u][e] = bh.y;
        wh[o + 32u][e] = bh.z;
        wh[o + 48u][e] = bh.w;
        workgroupBarrier();
        var h0 = vec4<f16>(0.0);
        var h1 = vec4<f16>(0.0);
        var h2 = vec4<f16>(0.0);
        var h3 = vec4<f16>(0.0);
        for (var kk = 0u; kk < 16u; kk++) {
            let xv = xh[kk * 16u + ty];
            let wv = wh[kk * 16u + tx];
            h0 = fma(vec4<f16>(xv.x), wv, h0);
            h1 = fma(vec4<f16>(xv.y), wv, h1);
            h2 = fma(vec4<f16>(xv.z), wv, h2);
            h3 = fma(vec4<f16>(xv.w), wv, h3);
        }
        a0 += vec4<f32>(h0);
        a1 += vec4<f32>(h1);
        a2 += vec4<f32>(h2);
        a3 += vec4<f32>(h3);
        workgroupBarrier();
    }
    let r = r0 + ty * 4u;
    let c = c0 + tx * 4u;
    let s16 = 1.0 / XSCALE;
    store4(r, c, a0 * s16);
    store4(r + 1u, c, a1 * s16);
    store4(r + 2u, c, a2 * s16);
    store4(r + 3u, c, a3 * s16);
}

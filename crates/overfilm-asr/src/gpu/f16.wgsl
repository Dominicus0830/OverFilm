// 16-bit float weights, two per u32 (no shader-f16 feature needed).
fn w4(n: u32, k4: u32) -> vec4<f32> {
    let b = n * (p.k / 2u) + k4 * 2u;
    return vec4<f32>(unpack2x16float(wq[b]), unpack2x16float(wq[b + 1u]));
}

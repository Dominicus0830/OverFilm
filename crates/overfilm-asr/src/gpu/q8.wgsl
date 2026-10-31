// 8-bit weights: four signed bytes per u32, one f32 scale per 32 weights.
fn w4(n: u32, k4: u32) -> vec4<f32> {
    let q = bitcast<i32>(wq[n * (p.k / 4u) + k4]);
    let sc = ws[n * (p.k / 32u) + k4 / 8u];
    return vec4<f32>(f32(extractBits(q, 0u, 8u)), f32(extractBits(q, 8u, 8u)), f32(extractBits(q, 16u, 8u)), f32(extractBits(q, 24u, 8u))) * sc;
}

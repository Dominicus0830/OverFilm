// Input rows for the encoder front's products (stride-2, padding-1 3×3 convolutions as matrix
// products, then `conv_out`). Inputs are channels-last: x[((b·h + y)·w + x)·cin + c].
// mode 0 (patches): row (b, oy, ox), column c·9 + ky·3 + kx (zero past cin·9, up to `cols`)
//   = x[b, 2·oy + ky − 1, 2·ox + kx − 1, c] (zero outside the input).
// mode 1 (flatten): row (b, t), column c·h + f = x[b, f, t, c], h frequencies × w times (zero
//   past cin·h, up to `cols`).
// `act` applies GELU (erf) to what is read: the previous convolution's activation.

struct P {
    mode: u32,
    nb: u32,
    cin: u32,
    h: u32,
    w: u32,
    ho: u32,
    wo: u32,
    cols: u32,
    act: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> x: array<f32>;
@group(0) @binding(2) var<storage, read_write> y: array<f32>;

// Abramowitz–Stegun 7.1.26 (|error| < 1.5e-7), as ew.wgsl
fn erf(v: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.3275911 * abs(v));
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    return sign(v) * (1.0 - poly * exp(-v * v));
}

fn load_x(i: u32) -> f32 {
    let v = x[i];
    if p.act != 0u {
        return 0.5 * v * (1.0 + erf(v * 0.70710678118654752));
    }
    return v;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let idx = gid.x + gid.y * nw.x * 256u;
    if p.mode == 0u {
        let plane = p.ho * p.wo;
        if idx >= p.nb * plane * p.cols {
            return;
        }
        let row = idx / p.cols;
        let col = idx % p.cols;
        var v = 0.0;
        if col < p.cin * 9u {
            let b = row / plane;
            let oy = (row % plane) / p.wo;
            let ox = row % p.wo;
            let c = col / 9u;
            let ky = (col % 9u) / 3u;
            let kx = col % 3u;
            let iy = i32(2u * oy + ky) - 1;
            let ix = i32(2u * ox + kx) - 1;
            if iy >= 0 && iy < i32(p.h) && ix >= 0 && ix < i32(p.w) {
                v = load_x(((b * p.h + u32(iy)) * p.w + u32(ix)) * p.cin + c);
            }
        }
        y[idx] = v;
    } else {
        if idx >= p.nb * p.w * p.cols {
            return;
        }
        let row = idx / p.cols;
        let col = idx % p.cols;
        var v = 0.0;
        if col < p.cin * p.h {
            let b = row / p.w;
            let t = row % p.w;
            let c = col / p.h;
            let f = col % p.h;
            v = load_x(((b * p.h + f) * p.w + t) * p.cin + c);
        }
        y[idx] = v;
    }
}

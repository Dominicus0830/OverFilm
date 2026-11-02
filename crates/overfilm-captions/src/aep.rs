//! The motion of an After Effects template (milestone M4.3): the keyframes inside the
//! `project.aegraphic` of a `.mogrt`.
//!
//! An After Effects project is a RIFX file (big-endian chunks: a 4-byte tag, a 4-byte length,
//! the body padded to an even length; `LIST` chunks hold a 4-byte kind and more chunks). This
//! module reads only what a caption can use, worked out from templates people share publicly:
//!
//! - a composition's `cdta`: bytes 8..12 are the time base (keyframe times divide by it to give
//!   seconds);
//! - a layer (`LIST Layr`) is a text layer when it has the `ADBE Text Properties` group;
//! - a property's keyframes are a `LIST list` after the property's match name (`tdmn`): `lhd3`
//!   gives the count (bytes 8..12) and record size (16..20), `ldat` the records. A record starts
//!   with the time (`u32`, 0..4) and the in/out interpolation (bytes 4 and 5: 1 linear, 2 Bézier,
//!   3 hold), then `f64`s: the value first, or for `ADBE Position` (a spatial property) at the
//!   seventh `f64`. Scale and opacity are stored as fractions (1 = 100 %), rotation in degrees,
//!   position in composition pixels.
//! - `ADBE Text Percent Start` / `End` / `Offset` keyframes are a per-character text animator.
//!
//! The transform of the first animated text layer (else the first animated layer) becomes a
//! [`CustomMotion`]: the keys before the longest still stretch are the motion in, the keys after
//! it the motion out, all relative to the still state. Effects (glow, gradient, light sweep),
//! masks, expressions and parents are not taken over; a per-character animator becomes the
//! typewriter.
//!
//! Every number is hostile here too: reads are bounds-checked, counts and depth capped.

use crate::styles::{CustomMotion, KeyPoint};

/// What the template's animation gave.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AeMotion {
    pub custom: Option<CustomMotion>,
    /// A per-character text animator is keyframed.
    pub per_character: bool,
}

const MAX_CHUNKS: usize = 200_000;
const MAX_DEPTH: u32 = 64;
const MAX_KEYS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Prop {
    Scale,
    Opacity,
    Position,
    PositionX,
    PositionY,
    Rotation,
}

#[derive(Clone, Debug)]
struct Key {
    t: f64,
    /// Scale and position: x, y; others: value in [0].
    v: [f64; 2],
    eased: bool,
    hold: bool,
}

#[derive(Default)]
struct Layer {
    text: bool,
    props: Vec<(Prop, Vec<Key>)>,
    per_character: bool,
}

struct Walk<'a> {
    d: &'a [u8],
    chunks: usize,
    rate: f64,
    comp_h: f64,
    layers: Vec<Layer>,
}

fn be_u32(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(p..p.checked_add(4)?)?.try_into().ok()?))
}

fn be_f64(d: &[u8], p: usize) -> Option<f64> {
    Some(f64::from_be_bytes(d.get(p..p.checked_add(8)?)?.try_into().ok()?)).filter(|v| v.is_finite())
}

impl Walk<'_> {
    /// The chunks from `p` to `end`; `name` is the match name of the property they belong to (a
    /// property's keyframes sit in a `LIST tdbs` after its `tdmn`).
    fn chunks(&mut self, mut p: usize, end: usize, depth: u32, layer: Option<usize>, inherited: &str) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut name = inherited.to_string();
        while p.saturating_add(8) <= end && self.chunks < MAX_CHUNKS {
            self.chunks += 1;
            let Some(tag) = self.d.get(p..p + 4) else { return };
            let Some(n) = be_u32(self.d, p + 4).map(|n| n as usize) else { return };
            let body = p + 8;
            let stop = body.saturating_add(n).min(end);
            match tag {
                b"LIST" => {
                    let kind = self.d.get(body..body + 4).unwrap_or_default();
                    if kind == b"Layr" {
                        self.layers.push(Layer::default());
                        let at = self.layers.len() - 1;
                        self.chunks(body + 4, stop, depth + 1, Some(at), "");
                    } else if kind == b"list" {
                        self.keys(body + 4, stop, &name, layer);
                    } else {
                        let n = name.clone();
                        self.chunks(body + 4, stop, depth + 1, layer, &n);
                    }
                }
                b"tdmn" => {
                    let raw = self.d.get(body..stop).unwrap_or_default();
                    name = String::from_utf8_lossy(raw.split(|c| *c == 0).next().unwrap_or_default()).into_owned();
                    if name == "ADBE Text Properties"
                        && let Some(l) = layer.and_then(|i| self.layers.get_mut(i))
                    {
                        l.text = true;
                    }
                }
                b"cdta" => {
                    let r = be_u32(self.d, body + 8).unwrap_or(0) as f64;
                    if r > 0.0 {
                        self.rate = r;
                    }
                }
                _ => {}
            }
            p = body.saturating_add(n).saturating_add(n & 1);
        }
    }

    /// A `LIST list` (keyframes) of the property `name`.
    fn keys(&mut self, mut p: usize, end: usize, name: &str, layer: Option<usize>) {
        let prop = match name {
            "ADBE Scale" => Some(Prop::Scale),
            "ADBE Opacity" => Some(Prop::Opacity),
            "ADBE Position" => Some(Prop::Position),
            "ADBE Position_0" => Some(Prop::PositionX),
            "ADBE Position_1" => Some(Prop::PositionY),
            "ADBE Rotate Z" => Some(Prop::Rotation),
            _ => None,
        };
        let animator = matches!(name, "ADBE Text Percent Start" | "ADBE Text Percent End" | "ADBE Text Percent Offset");
        let (mut count, mut size) = (0usize, 0usize);
        while p.saturating_add(8) <= end {
            let Some(tag) = self.d.get(p..p + 4) else { return };
            let Some(n) = be_u32(self.d, p + 4).map(|n| n as usize) else { return };
            let body = p + 8;
            if tag == b"lhd3" {
                count = be_u32(self.d, body + 8).unwrap_or(0) as usize;
                size = be_u32(self.d, body + 16).unwrap_or(0) as usize;
            } else if tag == b"ldat" && count > 0 && size >= 16 {
                let Some(l) = layer.and_then(|i| self.layers.get_mut(i)) else { return };
                if animator && count >= 2 {
                    l.per_character = true;
                }
                let Some(prop) = prop else { return };
                let mut keys = Vec::new();
                for k in 0..count.min(MAX_KEYS) {
                    let r = body.saturating_add(k.saturating_mul(size));
                    if r.saturating_add(size) > body.saturating_add(n) {
                        break;
                    }
                    let Some(t) = be_u32(self.d, r) else { break };
                    let out = self.d.get(r + 5).copied().unwrap_or(1);
                    let at = |i: usize| be_f64(self.d, r + 8 + 8 * i);
                    let v = match prop {
                        Prop::Position => [at(6), at(7)],
                        Prop::Scale => [at(0), at(1)],
                        _ => [at(0), Some(0.0)],
                    };
                    let (Some(a), Some(b)) = (v[0], v[1]) else { continue };
                    keys.push(Key { t: f64::from(t) / self.rate.max(1.0), v: [a, b], eased: out == 2, hold: out == 3 });
                }
                keys.sort_by(|a, b| a.t.total_cmp(&b.t));
                if keys.len() >= 2 {
                    l.props.push((prop, keys));
                }
                return;
            }
            p = body.saturating_add(n).saturating_add(n & 1);
        }
    }
}

/// The value of a property at `t` (held before the first and after the last key).
fn sample(keys: &[Key], t: f64) -> [f64; 2] {
    let Some(first) = keys.first() else { return [0.0, 0.0] };
    if t <= first.t {
        return first.v;
    }
    for w in keys.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        if t <= b.t {
            if a.hold || b.t <= a.t {
                return a.v;
            }
            let f = (t - a.t) / (b.t - a.t);
            return [a.v[0] + (b.v[0] - a.v[0]) * f, a.v[1] + (b.v[1] - a.v[1]) * f];
        }
    }
    keys.last().map_or(first.v, |k| k.v)
}

/// The motion in the `.aep` of a template whose frame is `comp_h` lines high.
pub fn motion(aep: &[u8], comp_h: f64) -> Option<AeMotion> {
    if aep.get(0..4) != Some(&b"RIFX"[..]) {
        return None;
    }
    let mut w = Walk { d: aep, chunks: 0, rate: 30.0, comp_h: if comp_h.is_finite() && comp_h >= 16.0 { comp_h } else { 1080.0 }, layers: Vec::new() };
    w.chunks(12, aep.len(), 0, None, "");
    let per_character = w.layers.iter().any(|l| l.text && l.per_character);
    let layer = w.layers.iter().find(|l| l.text && !l.props.is_empty()).or_else(|| w.layers.iter().find(|l| !l.props.is_empty()));
    let custom = layer.and_then(|l| custom(&l.props, w.comp_h));
    (custom.is_some() || per_character).then_some(AeMotion { custom, per_character })
}

fn custom(props: &[(Prop, Vec<Key>)], comp_h: f64) -> Option<CustomMotion> {
    let mut times: Vec<f64> = props.iter().flat_map(|(_, k)| k.iter().map(|k| k.t)).collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    times.truncate(MAX_KEYS);
    if times.len() < 2 {
        return None;
    }
    let state = |t: f64| -> Vec<[f64; 2]> { props.iter().map(|(_, k)| sample(k, t)).collect() };
    let same = |a: &[[f64; 2]], b: &[[f64; 2]]| a.iter().zip(b).all(|(x, y)| (x[0] - y[0]).abs() < 1e-3 && (x[1] - y[1]).abs() < 1e-3);
    // the longest stretch where nothing moves: the caption at rest
    let mut still: Option<(usize, f64)> = None;
    for i in 0..times.len() - 1 {
        let (a, b) = (times.get(i).copied().unwrap_or(0.0), times.get(i + 1).copied().unwrap_or(0.0));
        if same(&state(a), &state(b)) && still.is_none_or(|(_, len)| b - a > len) {
            still = Some((i, b - a));
        }
    }
    // no still stretch: everything moves towards the end state, an "in" only
    let (rest_at, split) = match still {
        Some((i, _)) => (times.get(i).copied().unwrap_or(0.0), Some(i)),
        None => (times.last().copied().unwrap_or(0.0), None),
    };
    let rest = state(rest_at);
    let eased_at = |t: f64| props.iter().any(|(_, k)| k.iter().any(|k| (k.t - t).abs() < 1e-6 && k.eased));
    let point = |t: f64, from: f64| -> KeyPoint {
        let now = state(t);
        let mut kp = KeyPoint { t: (t - from).max(0.0) as f32, scale: 100.0, opacity: 100.0, dx: 0.0, dy: 0.0, rotation: 0.0, ease: eased_at(t) };
        for (((prop, _), v), r) in props.iter().zip(&now).zip(&rest) {
            let rel = |x: f64, base: f64| if base.abs() > 1e-6 { (x / base * 100.0) as f32 } else { (x * 100.0) as f32 };
            match prop {
                Prop::Scale => kp.scale = rel(v[0], r[0]).clamp(0.0, 1000.0),
                Prop::Opacity => kp.opacity = rel(v[0], r[0]).clamp(0.0, 100.0),
                Prop::Position => {
                    kp.dx = ((v[0] - r[0]) / comp_h) as f32;
                    kp.dy = ((v[1] - r[1]) / comp_h) as f32;
                }
                Prop::PositionX => kp.dx = ((v[0] - r[0]) / comp_h) as f32,
                Prop::PositionY => kp.dy = ((v[0] - r[0]) / comp_h) as f32,
                Prop::Rotation => kp.rotation = (v[0] - r[0]) as f32,
            }
            kp.dx = kp.dx.clamp(-2.0, 2.0);
            kp.dy = kp.dy.clamp(-2.0, 2.0);
            kp.rotation = kp.rotation.clamp(-3600.0, 3600.0);
        }
        kp
    };
    let start = times.first().copied().unwrap_or(0.0);
    let (ins, outs): (Vec<f64>, Vec<f64>) = match split {
        Some(i) => (times.iter().take(i + 1).copied().collect(), times.iter().skip(i + 1).copied().collect()),
        None => (times.clone(), Vec::new()),
    };
    let out_from = outs.first().copied().unwrap_or(0.0);
    let mut m = CustomMotion { r#in: ins.iter().map(|t| point(*t, start)).collect(), out: Vec::new() };
    if !outs.is_empty() {
        // the out starts from rest at the end of the still stretch
        m.out = outs.iter().map(|t| point(*t, out_from)).collect();
        if let Some(first) = m.out.first_mut() {
            first.ease = eased_at(out_from);
        }
    }
    // an "in" that is only the rest state moves nothing
    if m.r#in.len() < 2 {
        m.r#in.clear();
    }
    if m.out.len() < 2 {
        m.out.clear();
    }
    (!m.r#in.is_empty() || !m.out.is_empty()).then_some(m)
}

#[cfg(test)]
#[path = "aep_tests.rs"]
pub(crate) mod tests;

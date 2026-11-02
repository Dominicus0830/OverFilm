//! An After Effects template's motion and look as a caption's (milestone M4.3): from the
//! project [`crate::aep`] reads, the main text layer's movement (through its parents and the
//! precompositions that show it, with expressions evaluated by [`crate::aep_expr`]) becomes a
//! [`CustomMotion`]; its effects, masks, the shape layers beside it and the other text layers
//! become a [`TemplateLook`].
//!
//! Positions are measured in the template's main composition: motion offsets as a share of its
//! height (they move the caption across the frame), effect sizes, mask paths and shapes in text
//! heights (they follow the caption's size).
//!
//! What maps where:
//!
//! - Glow → Wonder Glow; CC Light Sweep → Text Light Sweep (OverFilm's own effect); Gaussian / Fast Blur → Gaussian Blur;
//!   Directional (Motion) Blur → Directional Blur; Drop Shadow → the text's shadow; Gradient
//!   Ramp → the text's linear gradient fill; Fill → the text's (or shape's) colour; Transform →
//!   folded into the motion; a `wiggle()` expression → Wiggle.
//! - Masks → masks on the caption clip's opacity (they stay where the template had them while
//!   the text moves through them).
//! - Rectangle and ellipse shape layers → shape layers of the caption clip, pinned to the text
//!   when they were sized to it.
//!
//! Every number is hostile: all reads are bounds-checked, sample counts and recursion capped.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::aep::{self, Effect, Interp, Key, LayerKind, Project, Prop, VectorKind};
use crate::aep_expr::{self, Host, Name, Program, Wiggle};
use crate::styles::{
    CustomMotion, FxKey, FxLook, FxParam, FxValue, GradientLook, KeyPoint, MaskKey, MaskLook, Rgba, ShadowLook, ShapeLook, TemplateLook, Timing,
};

/// Samples per second of the motion.
const RATE: f64 = 30.0;
const MAX_SECONDS: f64 = 60.0;
const MAX_CHAIN: usize = 16;
const MAX_POINTS: usize = 96;
const MAX_EXPR_DEPTH: u32 = 6;

/// What a template's project gave.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AeTemplate {
    pub motion: Option<CustomMotion>,
    /// A per-character text animator moves the text.
    pub per_character: bool,
    pub look: TemplateLook,
    /// The main text's size at rest relative to its font size (layer and precomp scale).
    pub text_scale: f64,
    /// The colour a Fill effect gives the text.
    pub fill: Option<Rgba>,
    /// What came over (Korean, for the user).
    pub took: Vec<String>,
    /// What could not come over.
    pub missed: Vec<String>,
}

/// A 2-D affine map: `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct M([f64; 6]);

impl M {
    const ID: M = M([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    fn mul(self, o: M) -> M {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = o.0;
        M([a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f])
    }
    fn apply(self, p: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }
    fn trs(pos: [f64; 2], rot_deg: f64, scale: [f64; 2], anchor: [f64; 2]) -> M {
        let (s, c) = rot_deg.to_radians().sin_cos();
        let (sx, sy) = (scale[0], scale[1]);
        let (a, b, cc, d) = (c * sx, s * sx, -s * sy, c * sy);
        M([a, b, cc, d, pos[0] - (a * anchor[0] + cc * anchor[1]), pos[1] - (b * anchor[0] + d * anchor[1])])
    }
    fn scale(self) -> f64 {
        let [a, b, c, d, _, _] = self.0;
        (a * d - b * c).abs().sqrt()
    }
    fn rotation(self) -> f64 {
        let [a, b, ..] = self.0;
        b.atan2(a).to_degrees()
    }
    fn finite(self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct At {
    comp: usize,
    layer: usize,
}

/// How a property's stored numbers relate to what an expression sees.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Units {
    Plain,
    /// Stored as a fraction, seen in percent (scale, opacity).
    Percent,
    /// Stored as alpha, red, green, blue in 0..255; seen as red, green, blue, alpha in 0..1.
    Color,
}

fn units_of(p: &Prop) -> Units {
    if p.color {
        Units::Color
    } else if matches!(p.name.as_str(), "ADBE Scale" | "ADBE Opacity") {
        Units::Percent
    } else {
        Units::Plain
    }
}

fn to_expr(u: Units, v: Vec<f64>) -> Vec<f64> {
    match u {
        Units::Plain => v,
        Units::Percent => v.into_iter().map(|x| x * 100.0).collect(),
        Units::Color => {
            let g = |i: usize| v.get(i).copied().unwrap_or(255.0) / 255.0;
            vec![g(1), g(2), g(3), g(0)]
        }
    }
}

fn from_expr(u: Units, v: Vec<f64>) -> Vec<f64> {
    match u {
        Units::Plain => v,
        Units::Percent => v.into_iter().map(|x| x / 100.0).collect(),
        Units::Color => {
            let g = |i: usize, d: f64| v.get(i).copied().unwrap_or(d).clamp(0.0, 1.0) * 255.0;
            vec![g(3, 1.0), g(0, 0.0), g(1, 0.0), g(2, 0.0)]
        }
    }
}

/// One-dimensional cubic Bézier through (0, 0), (x1, y1), (x2, y2), (1, 1): y at x.
fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let curve = |u: f64, p1: f64, p2: f64| {
        let v = 1.0 - u;
        3.0 * v * v * u * p1 + 3.0 * v * u * u * p2 + u * u * u
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if curve(mid, x1, x2) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    curve((lo + hi) / 2.0, y1, y2)
}

fn fin(v: f64, d: f64) -> f64 {
    if v.is_finite() { v } else { d }
}

/// The value of keyframes at `t` (layer time), with AE's temporal Bézier easing.
fn keyed(keys: &[Key], t: f64) -> Option<Vec<f64>> {
    let first = keys.first()?;
    if t <= first.t {
        return Some(first.v.clone());
    }
    for w in keys.windows(2) {
        let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
        if t > b.t {
            continue;
        }
        let dt = b.t - a.t;
        if a.interp_out == Interp::Hold || dt <= 1e-9 {
            return Some(a.v.clone());
        }
        let s = (t - a.t) / dt;
        if a.interp_out == Interp::Linear && b.interp_in == Interp::Linear {
            return Some(a.v.iter().zip(&b.v).map(|(x, y)| x + (y - x) * s).collect());
        }
        let per_dim = a.out_speed.len() == a.v.len() && a.v.len() > 1;
        let dist = a.v.iter().zip(&b.v).map(|(x, y)| (y - x) * (y - x)).sum::<f64>().sqrt();
        let ease = |i: usize, delta: f64| -> f64 {
            // normalised control points: (influence, speed × influence × dt / Δv)
            let idx = if per_dim { i } else { 0 };
            let get = |v: &[f64], d: f64| fin(v.get(idx).copied().unwrap_or(d), d);
            let (oi, ii) = (get(&a.out_infl, 1.0 / 3.0).clamp(0.01, 1.0), get(&b.in_infl, 1.0 / 3.0).clamp(0.01, 1.0));
            let (os, is) = (get(&a.out_speed, 0.0), get(&b.in_speed, 0.0));
            let norm = |speed: f64| if delta.abs() > 1e-9 { (speed * dt / delta).clamp(-20.0, 20.0) } else { 0.0 };
            let (x1, y1) = if a.interp_out == Interp::Linear { (1.0 / 3.0, 1.0 / 3.0) } else { (oi, oi * norm(os)) };
            let (x2, y2) = if b.interp_in == Interp::Linear { (2.0 / 3.0, 2.0 / 3.0) } else { (1.0 - ii, 1.0 - ii * norm(is)) };
            bezier(x1, y1, x2, y2, s)
        };
        return Some(
            a.v.iter()
                .zip(&b.v)
                .enumerate()
                .map(|(i, (x, y))| {
                    let k = if per_dim { ease(i, y - x) } else { ease(0, dist) };
                    x + (y - x) * k
                })
                .collect(),
        );
    }
    keys.last().map(|k| k.v.clone())
}

/// The project and what evaluating it needs.
struct Look<'p> {
    p: &'p Project,
    programs: RefCell<BTreeMap<String, Option<Rc<Program>>>>,
    depth: Cell<u32>,
    failed: RefCell<Vec<String>>,
    wiggles: RefCell<Vec<(String, Wiggle)>>,
    /// Record `wiggle()` of the layer being measured.
    watch: Cell<Option<At>>,
}

struct ExprHost<'a, 'p> {
    look: &'a Look<'p>,
    at: At,
    prop: &'a Prop,
}

impl Host for ExprHost<'_, '_> {
    fn own_value(&self, t: f64) -> Vec<f64> {
        let start = self.look.layer(self.at).map_or(0.0, |l| l.start);
        to_expr(units_of(self.prop), raw(self.prop, t - start))
    }
    fn own_keys(&self) -> Option<(f64, f64)> {
        let start = self.look.layer(self.at).map_or(0.0, |l| l.start);
        Some((self.prop.keys.first()?.t + start, self.prop.keys.last()?.t + start))
    }
    fn layer(&self, by: &Name) -> Option<usize> {
        let comp = self.look.p.comps.get(self.at.comp)?;
        match by {
            Name::Text(n) => comp.layers.iter().position(|l| &l.name == n),
            Name::Index(i) => i.checked_sub(1).filter(|i| *i < comp.layers.len()),
        }
    }
    fn this_layer(&self) -> usize {
        self.at.layer
    }
    fn transform(&self, layer: usize, prop: &str, t: f64) -> Option<Vec<f64>> {
        self.look.transform_value(At { comp: self.at.comp, layer }, prop, t)
    }
    fn effect(&self, layer: usize, effect: &Name, param: &Name, t: f64) -> Option<Vec<f64>> {
        let at = At { comp: self.at.comp, layer };
        let l = self.look.layer(at)?;
        let e = match effect {
            Name::Text(n) => l.effects.iter().find(|e| &e.display == n || &e.match_name == n),
            Name::Index(i) => i.checked_sub(1).and_then(|i| l.effects.get(i)),
        }?;
        let p = match param {
            Name::Text(n) => e.params.iter().find(|p| (&p.display == n || &p.name == n) && !p.name.ends_with("-0000")),
            Name::Index(i) => u32::try_from(*i).ok().and_then(|i| e.param(i)),
        }?;
        Some(to_expr(units_of(p), self.look.value(at, p, t)))
    }
    fn layer_times(&self, layer: usize) -> (f64, f64) {
        self.look.layer(At { comp: self.at.comp, layer }).map_or((0.0, 0.0), |l| (l.in_point, l.out_point))
    }
    fn comp(&self) -> (f64, f64, f64, f64) {
        self.look.p.comps.get(self.at.comp).map_or((0.0, 1920.0, 1080.0, 1.0 / RATE), |c| (c.duration, c.w, c.h, 1.0 / RATE))
    }
}

/// A property's value without its expression at `t` (layer time), stored units.
fn raw(p: &Prop, t: f64) -> Vec<f64> {
    keyed(&p.keys, t).unwrap_or_else(|| p.value.clone())
}

impl<'p> Look<'p> {
    fn layer(&self, at: At) -> Option<&'p aep::Layer> {
        self.p.comps.get(at.comp)?.layers.get(at.layer)
    }

    fn program(&self, src: &str) -> Option<Rc<Program>> {
        if let Some(p) = self.programs.borrow().get(src) {
            return p.clone();
        }
        let prog = aep_expr::parse(src).map(Rc::new);
        if self.programs.borrow().len() < 512 {
            self.programs.borrow_mut().insert(src.to_string(), prog.clone());
        }
        prog
    }

    /// A property's value at comp time `t` (stored units), its expression applied when it can
    /// be evaluated.
    fn value(&self, at: At, p: &Prop, t: f64) -> Vec<f64> {
        let start = self.layer(at).map_or(0.0, |l| l.start);
        if let Some(src) = &p.expr
            && self.depth.get() < MAX_EXPR_DEPTH
        {
            let ok = self.program(src).and_then(|prog| {
                self.depth.set(self.depth.get() + 1);
                let r = aep_expr::eval(&prog, &ExprHost { look: self, at, prop: p }, t);
                self.depth.set(self.depth.get().saturating_sub(1));
                r
            });
            match ok {
                Some((v, w)) => {
                    if let Some(w) = w
                        && self.watch.get() == Some(at)
                    {
                        let mut ws = self.wiggles.borrow_mut();
                        if !ws.iter().any(|(n, _)| n == &p.name) {
                            ws.push((p.name.clone(), w));
                        }
                    }
                    return from_expr(units_of(p), v);
                }
                None => {
                    let mut f = self.failed.borrow_mut();
                    // Effect params are named "<match>-000n"; show a readable name when there is one.
                    let readable = |s: &str| !s.is_empty() && !s.starts_with('-') && s.chars().any(char::is_alphanumeric);
                    let label = if readable(&p.display) {
                        p.display.clone()
                    } else if readable(&p.name) && !p.name.starts_with("ADBE") {
                        p.name.clone()
                    } else {
                        "효과 값".to_owned()
                    };
                    if !f.contains(&label) && f.len() < 20 {
                        f.push(label);
                    }
                }
            }
        }
        raw(p, t - start)
    }

    fn prop_or(&self, at: At, name: &str, t: f64, default: &[f64]) -> Vec<f64> {
        let mut v = self.layer(at).and_then(|l| l.prop(name)).map(|p| self.value(at, p, t)).unwrap_or_default();
        for (i, d) in default.iter().enumerate() {
            match v.get_mut(i) {
                Some(x) => *x = fin(*x, *d),
                None => v.push(*d),
            }
        }
        v
    }

    /// A transform value in expression units (percent for scale and opacity).
    fn transform_value(&self, at: At, prop: &str, t: f64) -> Option<Vec<f64>> {
        let pos = || self.position(at, t);
        Some(match prop {
            "position" => pos().to_vec(),
            "xPosition" => vec![pos()[0]],
            "yPosition" => vec![pos()[1]],
            "anchorPoint" => self.prop_or(at, "ADBE Anchor Point", t, &[0.0, 0.0]),
            "scale" => self.prop_or(at, "ADBE Scale", t, &[1.0, 1.0]).into_iter().take(2).map(|x| x * 100.0).collect(),
            "rotation" => self.prop_or(at, "ADBE Rotate Z", t, &[0.0]),
            "opacity" => self.prop_or(at, "ADBE Opacity", t, &[1.0]).into_iter().take(1).map(|x| x * 100.0).collect(),
            _ => return None,
        })
    }

    fn position(&self, at: At, t: f64) -> [f64; 2] {
        let l = self.layer(at);
        let moving = |n: &str| l.and_then(|l| l.prop(n)).is_some_and(|p| !p.keys.is_empty() || p.expr.is_some());
        if moving("ADBE Position_0") || moving("ADBE Position_1") {
            let x = self.prop_or(at, "ADBE Position_0", t, &[0.0]);
            let y = self.prop_or(at, "ADBE Position_1", t, &[0.0]);
            [x.first().copied().unwrap_or(0.0), y.first().copied().unwrap_or(0.0)]
        } else {
            let v = self.prop_or(at, "ADBE Position", t, &[0.0, 0.0]);
            [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0)]
        }
    }

    /// The layer's own transform (with a Transform effect) at comp time `t`, and its opacity.
    fn local(&self, at: At, t: f64) -> (M, f64) {
        let anchor = self.prop_or(at, "ADBE Anchor Point", t, &[0.0, 0.0]);
        let scale = self.prop_or(at, "ADBE Scale", t, &[1.0, 1.0]);
        let rot = self.prop_or(at, "ADBE Rotate Z", t, &[0.0]);
        let op = self.prop_or(at, "ADBE Opacity", t, &[1.0]);
        let g = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(0.0);
        let mut m = M::trs(self.position(at, t), g(&rot, 0), [g(&scale, 0), g(&scale, 1)], [g(&anchor, 0), g(&anchor, 1)]);
        let mut opacity = g(&op, 0);
        if let Some(l) = self.layer(at) {
            for e in l.effects.iter().filter(|e| e.match_name == "ADBE Geometry2") {
                let (gm, go) = self.geometry(at, e, t);
                m = m.mul(gm);
                opacity *= go;
            }
        }
        (m, opacity.clamp(0.0, 1.0))
    }

    /// The Transform effect: anchor and position (fractions of the layer), scale height / width
    /// (or uniform), rotation, opacity.
    fn geometry(&self, at: At, e: &Effect, t: f64) -> (M, f64) {
        let size = self.layer_size(at);
        let num = |n: u32, d: f64| e.param(n).map(|p| self.value(at, p, t)).and_then(|v| v.first().copied()).map_or(d, |v| fin(v, d));
        let pt = |n: u32| {
            let v = e.param(n).map(|p| self.value(at, p, t)).unwrap_or_default();
            [v.first().copied().unwrap_or(0.5) * size[0], v.get(1).copied().unwrap_or(0.5) * size[1]]
        };
        let sh = num(3, 100.0) / 100.0;
        let uniform = num(11, 1.0) != 0.0;
        let sw = if uniform { sh } else { num(4, 100.0) / 100.0 };
        (M::trs(pt(2), num(7, 0.0), [sw, sh], pt(1)), (num(8, 100.0) / 100.0).clamp(0.0, 1.0))
    }

    /// The pixel size a layer's fractional points are measured against.
    fn layer_size(&self, at: At) -> [f64; 2] {
        let Some(comp) = self.p.comps.get(at.comp) else { return [1920.0, 1080.0] };
        let own = [comp.w, comp.h];
        let Some(l) = self.layer(at) else { return own };
        match l.kind {
            LayerKind::Footage => self.p.comp(l.source).map_or(own, |c| [c.w, c.h]),
            _ => own,
        }
    }

    /// A fraction of a layer as a point of that layer: text and shape layers measure from their
    /// own origin (their effect area is the composition centred on it), footage and
    /// precomposition layers from their top-left corner.
    fn layer_point(&self, at: At, f: [f64; 2]) -> [f64; 2] {
        let size = self.layer_size(at);
        let centred = self.layer(at).is_some_and(|l| matches!(l.kind, LayerKind::Text | LayerKind::Shape));
        let p = [f[0] * size[0], f[1] * size[1]];
        if centred { [p[0] - size[0] / 2.0, p[1] - size[1] / 2.0] } else { p }
    }

    /// The layer's transform in its composition (through its parents) at comp time `t`.
    fn in_comp(&self, at: At, t: f64) -> (M, f64) {
        let (mut m, op) = self.local(at, t);
        let mut cur = self.layer(at);
        for _ in 0..MAX_CHAIN {
            let Some(l) = cur else { break };
            if l.parent == 0 {
                break;
            }
            let Some(comp) = self.p.comps.get(at.comp) else { break };
            let Some(pi) = comp.layers.iter().position(|x| x.id == l.parent) else { break };
            let pat = At { comp: at.comp, layer: pi };
            let (pm, _) = self.local(pat, t);
            m = pm.mul(m);
            cur = self.layer(pat);
        }
        (m, op)
    }

    /// A layer's transform in the main composition at main time `t`: `chain` is the layer and
    /// then the precomposition layers that show it, inner first.
    fn world(&self, chain: &[At], t: f64) -> (M, f64) {
        let offsets: Vec<f64> = chain.iter().map(|a| self.layer(*a).map_or(0.0, |l| l.start)).collect();
        let mut m = M::ID;
        let mut op = 1.0;
        for (i, at) in chain.iter().enumerate() {
            // time in this link's composition: main time less the starts of the links above it
            let local_t = t - offsets.iter().skip(i + 1).sum::<f64>();
            let (lm, lo) = self.in_comp(*at, local_t);
            m = lm.mul(m);
            op *= lo;
        }
        (m, op)
    }

    /// Main time of a time `t` in the chain's first composition.
    fn to_main(&self, chain: &[At], t: f64) -> f64 {
        t + chain.iter().skip(1).map(|a| self.layer(*a).map_or(0.0, |l| l.start)).sum::<f64>()
    }
}

/// The layers shown by the main composition, each with its chain (the layer, then the
/// precomposition layers above it).
fn chains(p: &Project, root: usize) -> Vec<Vec<At>> {
    let mut out = Vec::new();
    let mut stack: Vec<(usize, Vec<At>)> = vec![(root, Vec::new())];
    while let Some((ci, above)) = stack.pop() {
        if above.len() >= MAX_CHAIN || out.len() > 4000 {
            continue;
        }
        let Some(comp) = p.comps.get(ci) else { continue };
        for (li, l) in comp.layers.iter().enumerate() {
            let at = At { comp: ci, layer: li };
            let mut chain = vec![at];
            chain.extend(above.iter().copied());
            if l.kind == LayerKind::Footage
                && let Some(sub) = p.comps.iter().position(|c| c.id == l.source && l.source != 0)
                && !chain.iter().any(|a| a.comp == sub)
            {
                stack.push((sub, chain.clone()));
            }
            out.push(chain);
        }
    }
    out
}

/// The main composition: one no layer shows, with the most text layers under it.
fn main_comp(p: &Project) -> Option<usize> {
    let shown: Vec<u32> = p.comps.iter().flat_map(|c| c.layers.iter().filter(|l| l.kind == LayerKind::Footage).map(|l| l.source)).collect();
    let texts = |ci: usize| {
        chains(p, ci).iter().filter(|c| c.first().and_then(|a| p.comps.get(a.comp)?.layers.get(a.layer)).is_some_and(|l| l.kind == LayerKind::Text)).count()
    };
    (0..p.comps.len())
        .filter(|ci| p.comps.get(*ci).is_some_and(|c| !shown.contains(&c.id)))
        .map(|ci| (texts(ci), ci))
        .filter(|(n, _)| *n > 0)
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
        .map(|(_, ci)| ci)
}

/// One sampled state: scale, opacity, centre x, centre y, rotation.
type State = [f64; 5];

/// Indices of `pts` kept by Ramer–Douglas–Peucker with per-channel tolerances.
fn simplify(pts: &[(f64, State)], tol: State) -> Vec<usize> {
    let n = pts.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.last_mut() {
        *k = true;
    }
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (Some(pa), Some(pb)) = (pts.get(a), pts.get(b)) else { continue };
        let mut worst = (0.0, a);
        for i in a + 1..b {
            let Some(pi) = pts.get(i) else { continue };
            let f = if pb.0 > pa.0 { (pi.0 - pa.0) / (pb.0 - pa.0) } else { 0.0 };
            let mut err: f64 = 0.0;
            for c in 0..5 {
                let (x, y, z, tl) = (pa.1[c], pb.1[c], pi.1[c], tol[c]);
                let line = x + (y - x) * f;
                err = err.max((z - line).abs() / tl.max(1e-9));
            }
            if err > worst.0 {
                worst = (err, i);
            }
        }
        if worst.0 > 1.0 {
            if let Some(k) = keep.get_mut(worst.1) {
                *k = true;
            }
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    let mut idx: Vec<usize> = keep.iter().enumerate().filter(|(_, k)| **k).map(|(i, _)| i).collect();
    // too many points: keep an even spread
    if idx.len() > MAX_POINTS {
        let step = idx.len() as f64 / MAX_POINTS as f64;
        idx = (0..MAX_POINTS).filter_map(|i| idx.get((i as f64 * step) as usize).copied()).collect();
        if let Some(l) = keep.iter().rposition(|k| *k)
            && idx.last() != Some(&l)
        {
            idx.push(l);
        }
    }
    idx
}

const TOL: State = [0.5, 0.5, 0.0015, 0.0015, 0.25];

/// Sample times over the main composition.
fn times(duration: f64) -> Vec<f64> {
    let d = duration.clamp(0.0, MAX_SECONDS);
    let n = (d * RATE).ceil() as usize;
    (0..=n).map(|i| (i as f64 / RATE).min(d)).collect()
}

/// A layer's sampled state in the main composition (scale and opacity in percent, centre in
/// composition pixels, rotation in degrees).
fn sampled(look: &Look, chain: &[At], ts: &[f64]) -> Vec<(f64, State)> {
    let Some(&at) = chain.first() else { return Vec::new() };
    ts.iter()
        .filter_map(|t| {
            let (m, op) = look.world(chain, *t);
            if !m.finite() {
                return None;
            }
            let anchor = look.prop_or(at, "ADBE Anchor Point", *t, &[0.0, 0.0]);
            let c = m.apply([anchor.first().copied().unwrap_or(0.0), anchor.get(1).copied().unwrap_or(0.0)]);
            Some((*t, [m.scale() * 100.0, op * 100.0, c[0], c[1], m.rotation()]))
        })
        .collect()
}

/// The longest stretch where nothing moves, as (from, to) sample indices.
fn still(s: &[(f64, State)]) -> Option<(usize, usize)> {
    let same = |a: &State, b: &State| (0..5).all(|c| (a[c] - b[c]).abs() <= TOL[c] * 0.5);
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < s.len() {
        let Some(base) = s.get(i) else { break };
        let mut j = i;
        while s.get(j + 1).is_some_and(|n| same(&n.1, &base.1)) {
            j += 1;
        }
        let len = |(a, b): (usize, usize)| s.get(b).map_or(0.0, |x| x.0) - s.get(a).map_or(0.0, |x| x.0);
        if j > i && best.is_none_or(|bst| len((i, j)) > len(bst)) {
            best = Some((i, j));
        }
        i = j + 1;
    }
    best
}

/// Key points for samples `part` (relative to `rest`, times from `from`).
fn points(part: &[(f64, State)], rest: &State, from: f64, comp_h: f64) -> Vec<KeyPoint> {
    let idx = simplify(part, [TOL[0], TOL[1], TOL[2] * comp_h, TOL[3] * comp_h, TOL[4]]);
    idx.iter()
        .filter_map(|i| part.get(*i))
        .map(|(t, s)| {
            let rel = |x: f64, base: f64| if base.abs() > 1e-6 { x / base * 100.0 } else { x };
            KeyPoint {
                t: (t - from).max(0.0) as f32,
                scale: rel(s[0], rest[0]).clamp(0.0, 1000.0) as f32,
                opacity: rel(s[1], rest[1]).clamp(0.0, 100.0) as f32,
                dx: ((s[2] - rest[2]) / comp_h).clamp(-2.0, 2.0) as f32,
                dy: ((s[3] - rest[3]) / comp_h).clamp(-2.0, 2.0) as f32,
                rotation: (s[4] - rest[4]).clamp(-3600.0, 3600.0) as f32,
                ease: false,
            }
        })
        .collect()
}

fn moves(points: &[KeyPoint]) -> bool {
    points.windows(2).any(|w| match (w.first(), w.get(1)) {
        (Some(a), Some(b)) => a.scale != b.scale || a.opacity != b.opacity || a.dx != b.dx || a.dy != b.dy || a.rotation != b.rotation,
        _ => false,
    })
}

/// A motion from samples, split at `timing`.
fn motion(s: &[(f64, State)], timing: &Timing, rest: &State, comp_h: f64) -> Option<CustomMotion> {
    let (in_end, out_at) = (timing.in_end as f64, timing.out_at as f64);
    let ins: Vec<(f64, State)> = s.iter().filter(|(t, _)| *t <= in_end + 1e-9).cloned().collect();
    let outs: Vec<(f64, State)> = s.iter().filter(|(t, _)| *t >= out_at - 1e-9 && out_at > in_end + 1e-9).cloned().collect();
    let mut m = CustomMotion { r#in: points(&ins, rest, timing.start as f64, comp_h), out: points(&outs, rest, out_at, comp_h) };
    if !moves(&m.r#in) {
        m.r#in.clear();
    }
    if !moves(&m.out) {
        m.out.clear();
    }
    (!m.r#in.is_empty() || !m.out.is_empty()).then_some(m)
}

fn rgba_of(v: &[f64]) -> Rgba {
    // stored as alpha, red, green, blue in 0..255
    let b = |i: usize| v.get(i).copied().map_or(255, |x| fin(x, 255.0).round().clamp(0.0, 255.0) as u8);
    [b(1), b(2), b(3), b(0)]
}

fn color01(c: Rgba) -> [f32; 4] {
    c.map(|x| f32::from(x) / 255.0)
}

/// The template measured in the main composition: units and where the text rests.
struct Frame {
    comp_h: f64,
    /// Composition pixels per text height.
    unit: f64,
    rest_t: f64,
    centre: [f64; 2],
    timing: Timing,
}

impl Frame {
    fn rel(&self, p: [f64; 2]) -> [f64; 2] {
        [(p[0] - self.centre[0]) / self.unit, (p[1] - self.centre[1]) / self.unit]
    }
}

/// An effect parameter: its value at rest and its keyframes (main time), converted by `conv`.
fn fx_param(
    look: &Look,
    chain: &[At],
    e: &Effect,
    (n, id, default): (u32, &str, FxValue),
    conv: &dyn Fn(&[f64], f64) -> Option<FxValue>,
    fr: &Frame,
) -> FxParam {
    let Some(&at) = chain.first() else { return FxParam { id: id.into(), value: default, keys: Vec::new() } };
    let Some(p) = e.param(n) else { return FxParam { id: id.into(), value: default, keys: Vec::new() } };
    let local_rest = fr.rest_t - (look.to_main(chain, 0.0));
    let value = conv(&look.value(at, p, local_rest), fr.rest_t).unwrap_or(default);
    let start = look.layer(at).map_or(0.0, |l| l.start);
    let mut keys = Vec::new();
    if p.expr.is_some() {
        // an expression: sampled ten times a second, kept where it changes
        let mut last: Option<FxValue> = None;
        for t in times(fr.timing.end.max(fr.timing.out_at) as f64).into_iter().step_by(3) {
            let local = t - look.to_main(chain, 0.0);
            if let Some(v) = conv(&look.value(at, p, local), t)
                && last.as_ref() != Some(&v)
            {
                keys.push(FxKey { t: t as f32, v: v.clone(), hold: false });
                last = Some(v);
            }
        }
        if keys.len() < 2 {
            keys.clear();
        }
    } else if p.keys.len() >= 2 {
        for k in p.keys.iter().take(aep::MAX_KEYS) {
            let t = look.to_main(chain, k.t + start);
            if let Some(v) = conv(&k.v, t) {
                keys.push(FxKey { t: t as f32, v, hold: k.interp_out == Interp::Hold });
            }
        }
    }
    FxParam { id: id.into(), value, keys }
}

fn first(v: &[f64]) -> Option<f64> {
    v.first().copied().filter(|x| x.is_finite())
}

/// The editor effects for a template effect, or `None` when it has no counterpart (the name is
/// reported).
fn effect(look: &Look, chain: &[At], e: &Effect, fr: &Frame, out: &mut AeTemplate) -> Option<()> {
    let num = |scale: f64| move |v: &[f64], _t: f64| first(v).map(|x| FxValue::Num(x * scale));
    let len = |v: &[f64], _t: f64| first(v).map(|x| FxValue::Len(x / fr.unit));
    let world_pt = |v: &[f64], t: f64| -> Option<FxValue> {
        let &at = chain.first()?;
        let p = look.layer_point(at, [v.first().copied()?, v.get(1).copied()?]);
        let (m, _) = look.world(chain, t);
        let w = m.apply(p);
        let r = fr.rel(w);
        r.iter().all(|x| x.is_finite()).then_some(FxValue::Pt(r))
    };
    let color = |v: &[f64], _t: f64| (v.len() >= 4).then(|| FxValue::Color(color01(rgba_of(v))));
    let choice = |offset: u32| move |v: &[f64], _t: f64| first(v).map(|x| FxValue::Choice((x.round().max(0.0) as u32).saturating_sub(offset)));
    let p = |n: u32, id: &str, d: FxValue, conv: &dyn Fn(&[f64], f64) -> Option<FxValue>| fx_param(look, chain, e, (n, id, d), conv, fr);
    let fx = match e.match_name.as_str() {
        "ADBE Glo2" => {
            // Glow: threshold (0..255), radius, intensity, colours (1 original, 2 A & B), colour A
            let a_b = e.param(7).and_then(|p| first(&p.value)).is_some_and(|v| v.round() as i64 == 2);
            let mut params = vec![
                p(2, "threshold", FxValue::Num(60.0), &num(100.0 / 255.0)),
                p(3, "radius", FxValue::Len(10.0 / fr.unit), &len),
                p(4, "intensity", FxValue::Num(100.0), &num(100.0)),
            ];
            if a_b {
                params.push(FxParam { id: "use_color".into(), value: FxValue::Bool(true), keys: Vec::new() });
                params.push(p(12, "color", FxValue::Color([1.0; 4]), &color));
            }
            FxLook { effect: "wonder_glow".into(), params }
        }
        "CC Light Sweep" => FxLook {
            effect: "text_light_sweep".into(),
            params: vec![
                p(1, "center", FxValue::Pt([0.0, 0.0]), &world_pt),
                p(2, "direction", FxValue::Num(-30.0), &num(1.0)),
                p(3, "shape", FxValue::Choice(1), &choice(1)),
                p(4, "width", FxValue::Len(50.0 / fr.unit), &len),
                p(5, "sweep_intensity", FxValue::Num(50.0), &num(1.0)),
                p(6, "edge_intensity", FxValue::Num(100.0), &num(1.0)),
                p(7, "edge_thickness", FxValue::Num(1.0), &num(1.0)),
                p(8, "color", FxValue::Color([1.0; 4]), &color),
                p(9, "reception", FxValue::Choice(0), &choice(1)),
            ],
        },
        "ADBE Gaussian Blur 2" | "ADBE Fast Blur" => FxLook {
            effect: "gaussian_blur".into(),
            params: vec![p(1, "blurriness", FxValue::Len(0.0), &len), p(2, "dimensions", FxValue::Choice(0), &choice(1))],
        },
        "ADBE Motion Blur" => {
            FxLook { effect: "directional_blur".into(), params: vec![p(1, "direction", FxValue::Num(0.0), &num(1.0)), p(2, "length", FxValue::Len(0.0), &len)] }
        }
        // the same look is often repeated on every layer of a precomp chain; keep the first
        "ADBE Drop Shadow" if out.look.shadow.is_some() => return Some(()),
        "ADBE Ramp" if out.look.gradient.is_some() => return Some(()),
        "ADBE Drop Shadow" => {
            let at0 = chain.first().copied()?;
            let local = fr.rest_t - look.to_main(chain, 0.0);
            let at_rest = |n: u32, d: f64| e.param(n).and_then(|pp| first(&look.value(at0, pp, local))).unwrap_or(d);
            let colour = e.param(1).map(|pp| rgba_of(&look.value(at0, pp, local))).unwrap_or([0, 0, 0, 255]);
            out.look.shadow = Some(ShadowLook {
                color: colour,
                opacity: (at_rest(2, 127.5) / 2.55).clamp(0.0, 100.0) as f32,
                angle: at_rest(3, 135.0) as f32,
                distance: (at_rest(4, 5.0) / fr.unit).clamp(0.0, 10.0) as f32,
                blur: (at_rest(5, 0.0) / fr.unit).clamp(0.0, 10.0) as f32,
            });
            out.took.push("그림자".into());
            return Some(());
        }
        "ADBE Ramp" => {
            let &at = chain.first()?;
            let local = fr.rest_t - look.to_main(chain, 0.0);
            let val = |n: u32| e.param(n).map(|pp| look.value(at, pp, local));
            let pt = |n: u32, d: [f64; 2]| {
                let v = val(n).unwrap_or_default();
                let f = [v.first().copied().unwrap_or(d[0]), v.get(1).copied().unwrap_or(d[1])];
                look.world(chain, fr.rest_t).0.apply(look.layer_point(at, f))
            };
            let (a, b) = (pt(1, [0.5, 0.0]), pt(3, [0.5, 1.0]));
            let mut start = val(2).map_or([0, 0, 0, 255], |v| rgba_of(&v));
            let mut end = val(4).map_or([255, 255, 255, 255], |v| rgba_of(&v));
            // Blend With Original: towards white (the usual text fill)
            let blend = val(7).and_then(|v| first(&v)).map_or(0.0, |x| if x > 1.0 { x / 100.0 } else { x }).clamp(0.0, 1.0);
            for c in [&mut start, &mut end] {
                for ch in c.iter_mut().take(3) {
                    *ch = (f64::from(*ch) + (255.0 - f64::from(*ch)) * blend).round() as u8;
                }
            }
            let angle = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
            out.look.gradient = Some(GradientLook { start, end, angle: if angle.is_finite() { angle as f32 } else { 90.0 } });
            out.took.push("그라데이션".into());
            return Some(());
        }
        "ADBE Geometry2" => {
            // folded into the motion
            return Some(());
        }
        "ADBE Slider Control"
        | "ADBE Angle Control"
        | "ADBE Checkbox Control"
        | "ADBE Color Control"
        | "ADBE Point Control"
        | "ADBE Layer Control"
        | "ADBE Point3D Control"
        | "ADBE Dropdown Control" => return Some(()),
        "ADBE Fill" => return Some(()),
        _ => return None,
    };
    let label = match fx.effect.as_str() {
        "wonder_glow" => "글로우",
        "text_light_sweep" => "빛 지나가기",
        "gaussian_blur" => "흐림",
        _ => "방향 흐림",
    };
    if !out.took.iter().any(|t| t == label) {
        out.took.push(label.into());
    }
    out.look.effects.push(fx);
    Some(())
}

/// `wiggle()` on the text's transform → the Wiggle effect.
fn wiggles(look: &Look, fr: &Frame, out: &mut AeTemplate) {
    for (prop, w) in look.wiggles.borrow().iter() {
        let amount = w.amount.abs();
        let (id, v) = match prop.as_str() {
            "ADBE Rotate Z" => ("rotation", FxValue::Num(amount.min(180.0))),
            "ADBE Scale" => ("scale", FxValue::Num(amount.min(100.0))),
            _ => ("amount", FxValue::Len(amount / fr.unit)),
        };
        out.look.effects.push(FxLook {
            effect: "wiggle".into(),
            params: vec![
                FxParam { id: "frequency".into(), value: FxValue::Num(w.freq.clamp(0.0, 60.0)), keys: Vec::new() },
                FxParam { id: "amount".into(), value: if id == "amount" { v.clone() } else { FxValue::Len(0.0) }, keys: Vec::new() },
                FxParam { id: id.into(), value: v, keys: Vec::new() },
            ],
        });
        if !out.took.iter().any(|t| t == "흔들림(wiggle)") {
            out.took.push("흔들림(wiggle)".into());
        }
    }
}

/// Rectangle and ellipse shape layers shown beside the text.
fn shapes(look: &Look, all: &[Vec<At>], main_chain: &[At], fr: &Frame, ts: &[f64], rest_i: usize, out: &mut AeTemplate) {
    let mut paths = 0;
    let mut covering = 0;
    let text_order = all.iter().position(|c| c.as_slice() == main_chain).unwrap_or(0);
    for (order, c) in all.iter().enumerate() {
        let Some(l) = c.first().and_then(|a| look.layer(*a)) else { continue };
        if l.kind != LayerKind::Shape || !l.visible || l.guide {
            continue;
        }
        match l.vector {
            VectorKind::Rect | VectorKind::Ellipse => {
                if let Some(s) = shape(look, c, fr, ts, rest_i, order < text_order) {
                    if hides_text(&s) {
                        covering += 1;
                    } else {
                        out.look.shapes.push(s);
                    }
                }
            }
            VectorKind::Path => paths += 1,
            VectorKind::None => {}
        }
        if out.look.shapes.len() >= 8 {
            break;
        }
    }
    if !out.look.shapes.is_empty() {
        out.took.push(format!("도형 레이어 {}개", out.look.shapes.len()));
    }
    if paths > 0 {
        out.missed.push(format!("자유 도형(선) 레이어 {paths}개"));
    }
    if covering > 0 {
        out.missed.push(format!("글자를 가리는 도형 {covering}개(매트나 화면 전환용으로 보여 뺐어요)"));
    }
}

/// An opaque shape in front of the text that covers all of it at rest: in a template that is a
/// matte or a wipe (shown through a blending mode or a track matte we do not carry over), never
/// something meant to hide the caption.
fn hides_text(s: &ShapeLook) -> bool {
    let opaque = s.fill.is_some_and(|c| c[3] >= 250);
    // over the text's centre and bigger than a line of it
    let covers = |i: usize, min: f64| {
        let (at, size) = (s.at.get(i).copied().unwrap_or(0.0), s.size.get(i).copied().unwrap_or(0.0));
        at.abs() < size / 2.0 && size >= min
    };
    s.front && opaque && covers(0, 3.0) && covers(1, 1.5)
}

/// A Fill effect on the text sets its colour.
fn fill(look: &Look, main_chain: &[At], rest_t: f64, out: &mut AeTemplate) {
    if let Some(&at) = main_chain.first()
        && let Some(p) = look.layer(at).and_then(|l| l.effects.iter().find(|e| e.match_name == "ADBE Fill")).and_then(|e| e.param(2))
    {
        let v = look.value(at, p, rest_t - look.to_main(main_chain, 0.0));
        if v.len() >= 4 {
            out.fill = Some(rgba_of(&v));
            out.took.push("채우기 색".into());
        }
    }
}

/// The template's main text and the look around it.
pub fn template(aep_bytes: &[u8], font_px: Option<f64>) -> Option<AeTemplate> {
    let p = aep::read(aep_bytes)?;
    let root = main_comp(&p)?;
    let comp = p.comps.get(root)?;
    let look =
        Look { p: &p, programs: RefCell::default(), depth: Cell::new(0), failed: RefCell::default(), wiggles: RefCell::default(), watch: Cell::new(None) };
    let all = chains(&p, root);
    let layer_of = |c: &[At]| c.first().and_then(|a| look.layer(*a));
    let ts = times(comp.duration.max(1.0 / RATE));
    let comp_h = comp.h.max(16.0);
    // the text layers, top to bottom at rest
    let mut texts: Vec<(Vec<At>, Vec<(f64, State)>)> = all
        .iter()
        .filter(|c| layer_of(c).is_some_and(|l| l.kind == LayerKind::Text && l.visible && !l.guide))
        .take(8)
        .map(|c| (c.clone(), Vec::new()))
        .collect();
    let main_chain = texts.first()?.0.clone();
    look.watch.set(main_chain.first().copied());
    for (c, s) in &mut texts {
        *s = sampled(&look, c, &ts);
    }
    look.watch.set(None);
    let main = texts.first().map(|t| t.1.clone()).unwrap_or_default();
    // where the main text rests and when its motion in ends and out starts
    let (timing, rest_i) = match still(&main) {
        Some((a, b)) => {
            let t = |i: usize| main.get(i).map_or(0.0, |x| x.0) as f32;
            let moved_in = a > 0;
            let moved_out = b + 1 < main.len();
            let start = main.first().map_or(0.0, |x| x.0) as f32;
            let end = main.last().map_or(0.0, |x| x.0) as f32;
            (Timing { start, in_end: if moved_in { t(a) } else { start }, out_at: if moved_out { t(b) } else { end }, end }, (a + b) / 2)
        }
        None => {
            let end = main.last().map_or(0.0, |x| x.0) as f32;
            (Timing { start: 0.0, in_end: end, out_at: end, end }, main.len().saturating_sub(1))
        }
    };
    let (rest_t, rest) = main.get(rest_i).copied().unwrap_or((0.0, [100.0, 100.0, comp.w / 2.0, comp.h / 2.0, 0.0]));
    let scale_at_rest = (rest[0] / 100.0).max(1e-3);
    let unit = (font_px.filter(|f| f.is_finite() && *f > 0.0).unwrap_or(comp_h * 0.08) * scale_at_rest).max(1.0);
    let fr = Frame { comp_h, unit, rest_t, centre: [rest[2], rest[3]], timing };
    let mut out = AeTemplate { text_scale: scale_at_rest.clamp(0.05, 20.0), ..AeTemplate::default() };
    out.look.timing = timing;
    out.motion = motion(&main, &timing, &rest, comp_h);
    if out.motion.is_some() {
        out.took.push("움직임(크기·위치·투명도·회전)".into());
    }
    out.per_character = main_chain.first().and_then(|a| look.layer(*a)).is_some_and(|l| l.per_character);
    // the other text layers move the caption's other lines, top to bottom
    let mut others: Vec<&(Vec<At>, Vec<(f64, State)>)> = texts.iter().skip(1).collect();
    others.sort_by(|a, b| {
        let y = |s: &Vec<(f64, State)>| s.get(rest_i).map_or(0.0, |x| x.1[3]);
        y(&a.1).total_cmp(&y(&b.1))
    });
    for (_, s) in others {
        let r = s.get(rest_i).map_or(rest, |x| x.1);
        if let Some(m) = motion(s, &timing, &r, comp_h) {
            out.look.lines.push(m);
        }
    }
    if !out.look.lines.is_empty() {
        out.took.push(format!("다른 글자 레이어 {}개의 움직임(자막 둘째 줄부터)", out.look.lines.len()));
    }
    // effects of the text and of the precompositions that show it
    let mut missed_fx: Vec<String> = Vec::new();
    for i in 0..main_chain.len() {
        let chain = main_chain.get(i..).unwrap_or_default();
        let Some(l) = chain.first().and_then(|a| look.layer(*a)) else { continue };
        for e in &l.effects {
            if effect(&look, chain, e, &fr, &mut out).is_none() {
                let name = if e.display.is_empty() { e.match_name.clone() } else { e.display.clone() };
                if !missed_fx.contains(&name) {
                    missed_fx.push(name);
                }
            }
        }
        // masks on the text or its precomposition layers
        for m in &l.masks {
            if let Some(ml) = mask(&look, chain, m, &fr) {
                out.look.masks.push(ml);
            }
        }
    }
    if !out.look.masks.is_empty() {
        out.took.push(format!("마스크 {}개", out.look.masks.len()));
    }
    wiggles(&look, &fr, &mut out);
    shapes(&look, &all, &main_chain, &fr, &ts, rest_i, &mut out);
    fill(&look, &main_chain, rest_t, &mut out);
    if !missed_fx.is_empty() {
        out.missed.push(format!("효과: {}", missed_fx.join(", ")));
    }
    let failed = look.failed.borrow();
    if !failed.is_empty() {
        out.missed.push(format!("계산할 수 없는 익스프레션({})", failed.join(", ")));
    }
    Some(out)
}

/// A mask of a layer in `chain`, in text heights from the text's centre at rest.
fn mask(look: &Look, chain: &[At], m: &aep::Mask, fr: &Frame) -> Option<MaskLook> {
    let &at = chain.first()?;
    if m.mode == 0 || m.shapes.is_empty() {
        return None;
    }
    let size = look.layer_size(at);
    let centred = look.layer(at).is_some_and(|l| matches!(l.kind, LayerKind::Text | LayerKind::Shape));
    let start = look.layer(at).map_or(0.0, |l| l.start);
    let (world, _) = look.world(chain, fr.rest_t);
    let to_rel = |f: [f64; 2]| {
        let mut p = [f[0] * size[0], f[1] * size[1]];
        if centred {
            p = [p[0] - size[0] / 2.0, p[1] - size[1] / 2.0];
        }
        fr.rel(world.apply(p))
    };
    let mut path = Vec::new();
    for (t, interp, shape) in m.shapes.iter().take(aep::MAX_KEYS) {
        let n = shape.pts.len() / 3;
        if n < 2 {
            continue;
        }
        let mut vertices = Vec::with_capacity(n);
        for i in 0..n {
            let (Some(v), Some(o)) = (shape.pts.get(3 * i), shape.pts.get(3 * i + 1)) else { break };
            let prev_in = if i == 0 { shape.pts.get(3 * n - 1) } else { shape.pts.get(3 * i - 1) };
            let (pv, po) = (to_rel(*v), to_rel(*o));
            let pi = prev_in.map_or(pv, |x| to_rel(*x));
            vertices.push([pv[0], pv[1], pi[0] - pv[0], pi[1] - pv[1], po[0] - pv[0], po[1] - pv[1]]);
        }
        if vertices.iter().flatten().any(|x| !x.is_finite()) {
            continue;
        }
        path.push(MaskKey { t: look.to_main(chain, t + start) as f32, closed: shape.closed, vertices, hold: *interp == Interp::Hold });
    }
    if path.is_empty() {
        return None;
    }
    let rest_local = fr.rest_t - look.to_main(chain, 0.0);
    let val = |p: &Option<Prop>, d: f64| p.as_ref().and_then(|p| first(&look.value(at, p, rest_local))).unwrap_or(d);
    let opacity = val(&m.opacity, 100.0);
    Some(MaskLook {
        mode: u8::try_from(m.mode.min(6)).unwrap_or(1),
        inverted: m.inverted,
        feather: (val(&m.feather, 0.0) / fr.unit).clamp(0.0, 20.0),
        opacity: if opacity <= 1.0 { opacity * 100.0 } else { opacity }.clamp(0.0, 100.0),
        expansion: (val(&m.expansion, 0.0) / fr.unit).clamp(-20.0, 20.0),
        path,
    })
}

/// A rectangle or ellipse shape layer.
fn shape(look: &Look, chain: &[At], fr: &Frame, ts: &[f64], rest_i: usize, front: bool) -> Option<ShapeLook> {
    let &at = chain.first()?;
    let l = look.layer(at)?;
    let ellipse = l.vector == VectorKind::Ellipse;
    let (size_name, pos_name) =
        if ellipse { ("ADBE Vector Ellipse Size", "ADBE Vector Ellipse Position") } else { ("ADBE Vector Rect Size", "ADBE Vector Rect Position") };
    let local = fr.rest_t - look.to_main(chain, 0.0);
    let get = |name: &str| l.vector_prop(name).map(|p| look.value(at, p, local));
    let size = get(size_name).unwrap_or_default();
    let pos = get(pos_name).unwrap_or_default();
    let (w, h) = (size.first().copied().unwrap_or(0.0), size.get(1).copied().unwrap_or(0.0));
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (m, _) = look.world(chain, fr.rest_t);
    let centre = m.apply([pos.first().copied().unwrap_or(0.0), pos.get(1).copied().unwrap_or(0.0)]);
    let k = m.scale();
    let world = [w * k / fr.unit, h * k / fr.unit];
    let at_rel = fr.rel(centre);
    // sized by the text (an expression on its size), or round the text's centre: pin it
    let sized_by_expr = l.vector_prop(size_name).is_some_and(|p| p.expr.as_deref().is_some_and(|e| e.contains("sourceRectAtTime")));
    let covers = at_rel[0].abs() < world[0] / 2.0 && at_rel[1].abs() < world[1] / 2.0;
    let fit_text = sized_by_expr || covers;
    let pad_y = ((world[1] - 1.0) / 2.0).clamp(0.1, 2.0);
    let fill = l.vector_prop("ADBE Vector Fill Color").map(|p| rgba_of(&look.value(at, p, local)));
    // a Fill effect on the shape layer paints it
    let fill = l.effects.iter().find(|e| e.match_name == "ADBE Fill").and_then(|e| e.param(2)).map(|p| rgba_of(&look.value(at, p, local))).or(fill);
    let stroke = l.vector_prop("ADBE Vector Stroke Color").map(|p| rgba_of(&look.value(at, p, local)));
    let stroke_width = l.vector_prop("ADBE Vector Stroke Width").and_then(|p| first(&look.value(at, p, local))).unwrap_or(0.0) * k / fr.unit;
    let roundness = l.vector_prop("ADBE Vector Rect Roundness").and_then(|p| first(&look.value(at, p, local))).unwrap_or(0.0) * k / fr.unit;
    let s = sampled(look, chain, ts);
    let rest = s.get(rest_i).map(|x| x.1)?;
    Some(ShapeLook {
        ellipse,
        size: world,
        at: at_rel,
        radius: roundness.clamp(0.0, 50.0),
        fill,
        stroke: stroke.filter(|_| stroke_width > 0.0),
        stroke_width: stroke_width.clamp(0.0, 5.0),
        fit_text,
        pad: [(pad_y * 1.5).clamp(0.1, 3.0), pad_y],
        front,
        motion: motion(&s, &fr.timing, &rest, fr.comp_h),
    })
}

#[cfg(test)]
#[path = "aep_look_tests.rs"]
mod tests;

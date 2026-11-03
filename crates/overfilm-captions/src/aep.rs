//! The animation of an After Effects template (milestone M4.3): what is inside the
//! `project.aegraphic` of a `.mogrt`.
//!
//! An After Effects project is a RIFX file (big-endian chunks: a 4-byte tag, a 4-byte length,
//! the body padded to an even length; `LIST` chunks hold a 4-byte kind and more chunks). This
//! module reads it into a small model ([`Project`]); [`crate::aep_look`] turns that into a
//! caption's motion and look. The layout was worked out from templates people share publicly and
//! from the format notes of the MIT-licensed `py-aep` project; nothing here comes from Adobe
//! software.
//!
//! - **Items** (`LIST Item`): `idta` bytes 0..2 are the item type (4 = composition) and 16..20 its
//!   id; a composition's `cdta` holds the time base (8..12, keyframe times divide by it), the
//!   duration (44..52, dividend / divisor) and the size (140..144, two `u16`).
//! - **Layers** (`LIST Layr`): `ldta` holds the layer id (0..4), start, in and out times (12..36,
//!   dividend / divisor pairs), the source item (40..44, a precomposition for a precomp layer), the
//!   flags (byte 37 bit 1 guide, byte 38 bit 7 null, byte 39 bit 0 video on), the name (64..96), the layer type (byte 131: 0 footage /
//!   precomp, 2 camera, 3 text, 4 shape) and the parent layer id (132..136).
//! - **Properties**: a match name (`tdmn`, 40 bytes) and then the group (`LIST tdgp`) or property
//!   (`LIST tdbs`) it names. A property's `tdb4` gives its dimensions (2..4), whether it is spatial
//!   (byte 5, bit 3) or a colour (byte 59, bit 0) and whether its expression is off (byte 119,
//!   bit 0); `cdat` holds the value when it is not keyframed; `LIST list` the keyframes (`lhd3`:
//!   count at 8..12, record size at 16..20; `ldat`: the records); a `Utf8` the expression; a
//!   `tdsn` its display name. A keyframe record is the time (`i32`, layer time units), the in and
//!   out interpolation (bytes 4 and 5: 1 linear, 2 Bézier, 3 hold) and `f64`s: the value then
//!   the in/out speeds and influences; a spatial value (position) comes after five `f64`s, a
//!   colour (alpha, red, green, blue in 0..255) after six.
//! - **Effects**: under `ADBE Effect Parade`, each effect's match name then its `LIST sspc`
//!   (definitions in `LIST parT`, values in a `LIST tdgp` whose `tdsn` is the name the user gave it and whose properties are `<match name>-0001`, `-0002` …)
//! - **Masks**: under `ADBE Mask Parade`, `ADBE Mask Atom` groups; the n-th `mkif` of the parade
//!   is the n-th mask's mode (6..8) and inverted flag (byte 0). A mask's path (`LIST om-s`) has its
//!   keyframe times in `tdbs` and one `LIST shap` per keyframe in `omks`: `shph` gives the open
//!   flag (byte 3, bit 3) and a bounding box (four `f32` at 4..20); the points (`f32` x, y in
//!   0..1 of that box, in threes: vertex, its out tangent, the next vertex's in tangent) are
//!   fractions of the layer's size.
//!
//! Every number is hostile: reads are bounds-checked, counts and depth capped.

const MAX_CHUNKS: usize = 400_000;
const MAX_DEPTH: u32 = 64;
pub(crate) const MAX_KEYS: usize = 512;
const MAX_LAYERS: usize = 2000;
const MAX_EFFECTS: usize = 64;
const MAX_MASKS: usize = 32;
const MAX_POINTS: usize = 1024;
const MAX_EXPR: usize = 64 << 10;

/// Interpolation of one side of a keyframe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Interp {
    #[default]
    Linear,
    Bezier,
    Hold,
}

impl Interp {
    fn from_byte(b: u8) -> Interp {
        match b {
            2 => Interp::Bezier,
            3 => Interp::Hold,
            _ => Interp::Linear,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Key {
    /// Seconds, layer time.
    pub t: f64,
    pub v: Vec<f64>,
    pub interp_in: Interp,
    pub interp_out: Interp,
    /// Speed (value units per second) and influence (0..1) per dimension; a spatial or colour
    /// keyframe has one of each.
    pub in_speed: Vec<f64>,
    pub in_infl: Vec<f64>,
    pub out_speed: Vec<f64>,
    pub out_infl: Vec<f64>,
}

/// One animatable property.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Prop {
    pub name: String,
    pub display: String,
    /// The value when it is not keyframed (colours: alpha, red, green, blue in 0..255).
    pub value: Vec<f64>,
    pub keys: Vec<Key>,
    pub expr: Option<String>,
    pub spatial: bool,
    pub color: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Effect {
    pub match_name: String,
    /// The name the template's author gave it (what expressions call it by).
    pub display: String,
    pub params: Vec<Prop>,
}

impl Effect {
    /// The parameter `-000n` (n from 1).
    pub fn param(&self, n: u32) -> Option<&Prop> {
        let want = format!("{}-{n:04}", self.match_name);
        self.params.iter().find(|p| p.name == want)
    }
}

/// A mask path: `pts` in threes (vertex, its out tangent, the next vertex's in tangent), as
/// fractions of the layer's size.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Path {
    pub closed: bool,
    pub pts: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Mask {
    /// 0 none, 1 add, 2 subtract, 3 intersect, 4 lighten, 5 darken, 6 difference.
    pub mode: u16,
    pub inverted: bool,
    /// The path at each keyframe (seconds, layer time) or once when still.
    pub shapes: Vec<(f64, Interp, Path)>,
    pub feather: Option<Prop>,
    pub opacity: Option<Prop>,
    pub expansion: Option<Prop>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LayerKind {
    #[default]
    Footage,
    Light,
    Camera,
    Text,
    Shape,
    Other,
}

/// What a shape layer draws (its first rectangle, ellipse or path).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum VectorKind {
    #[default]
    None,
    Rect,
    Ellipse,
    Path,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Layer {
    pub id: u32,
    pub name: String,
    pub kind: LayerKind,
    pub source: u32,
    pub parent: u32,
    pub null: bool,
    /// Its video is switched on (a track matte or a hidden helper layer is not).
    pub visible: bool,
    /// A guide layer (never rendered).
    pub guide: bool,
    pub start: f64,
    pub in_point: f64,
    pub out_point: f64,
    /// Transform properties (`ADBE Anchor Point`, `ADBE Position` …).
    pub props: Vec<Prop>,
    pub effects: Vec<Effect>,
    pub masks: Vec<Mask>,
    pub vector: VectorKind,
    /// The first rectangle / ellipse size and position, fill and stroke of a shape layer.
    pub vector_props: Vec<Prop>,
    /// A per-character text animator is keyframed or driven by an expression.
    pub per_character: bool,
}

impl Layer {
    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }
    pub fn vector_prop(&self, name: &str) -> Option<&Prop> {
        self.vector_props.iter().find(|p| p.name == name)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Comp {
    pub id: u32,
    /// Keyframe time units per second.
    pub rate: f64,
    pub w: f64,
    pub h: f64,
    pub duration: f64,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Project {
    pub comps: Vec<Comp>,
}

impl Project {
    pub fn comp(&self, id: u32) -> Option<&Comp> {
        self.comps.iter().find(|c| c.id == id)
    }
}

fn be_u16(d: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(p..p.checked_add(2)?)?.try_into().ok()?))
}

fn be_u32(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(p..p.checked_add(4)?)?.try_into().ok()?))
}

fn be_i32(d: &[u8], p: usize) -> Option<i32> {
    Some(i32::from_be_bytes(d.get(p..p.checked_add(4)?)?.try_into().ok()?))
}

fn be_f32(d: &[u8], p: usize) -> Option<f64> {
    Some(f64::from(f32::from_be_bytes(d.get(p..p.checked_add(4)?)?.try_into().ok()?))).filter(|v| v.is_finite())
}

fn be_f64(d: &[u8], p: usize) -> Option<f64> {
    Some(f64::from_be_bytes(d.get(p..p.checked_add(8)?)?.try_into().ok()?)).filter(|v| v.is_finite())
}

/// `dividend / divisor` seconds (signed dividend).
fn ratio(d: &[u8], p: usize) -> f64 {
    let a = be_i32(d, p).unwrap_or(0) as f64;
    let b = be_u32(d, p.saturating_add(4)).unwrap_or(1).max(1) as f64;
    a / b
}

/// `dividend / divisor` with an unsigned dividend.
fn ratio_u(d: &[u8], p: usize) -> f64 {
    let a = be_u32(d, p).unwrap_or(0) as f64;
    let b = be_u32(d, p.saturating_add(4)).unwrap_or(1).max(1) as f64;
    a / b
}

fn c_string(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw.split(|c| *c == 0).next().unwrap_or_default()).chars().filter(|c| !c.is_control()).take(200).collect()
}

/// The string of a `tdsn` (it holds a `Utf8` chunk).
fn tdsn(d: &[u8], body: usize, end: usize) -> String {
    if d.get(body..body.saturating_add(4)) == Some(&b"Utf8"[..]) {
        let n = be_u32(d, body + 4).unwrap_or(0) as usize;
        let s = body.saturating_add(8);
        return c_string(d.get(s..s.saturating_add(n).min(end)).unwrap_or_default());
    }
    String::new()
}

/// Where in a layer the walker is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Group {
    Other,
    Transform,
    Effects,
    Effect(usize),
    Masks,
    Mask(usize),
    Vectors,
    VectorTransform,
    Text,
}

#[derive(Clone, Copy)]
struct Ctx {
    comp: Option<usize>,
    layer: Option<usize>,
    group: Group,
    /// Directly inside the layer's `LIST Layr`.
    top: bool,
}

struct Walk<'a> {
    d: &'a [u8],
    chunks: usize,
    p: Project,
}

impl Walk<'_> {
    fn comp_mut(&mut self, c: Option<usize>) -> Option<&mut Comp> {
        self.p.comps.get_mut(c?)
    }

    fn layer_mut(&mut self, cx: Ctx) -> Option<&mut Layer> {
        self.p.comps.get_mut(cx.comp?)?.layers.get_mut(cx.layer?)
    }

    fn rate(&self, cx: Ctx) -> f64 {
        cx.comp.and_then(|c| self.p.comps.get(c)).map_or(30.0, |c| c.rate).max(1.0)
    }

    fn walk(&mut self, mut p: usize, end: usize, depth: u32, mut cx: Ctx) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut name = String::new();
        let mut mkifs = 0usize;
        while p.saturating_add(8) <= end && self.chunks < MAX_CHUNKS {
            self.chunks += 1;
            let Some(tag) = self.d.get(p..p + 4) else { return };
            let Some(n) = be_u32(self.d, p + 4).map(|n| n as usize) else { return };
            let body = p + 8;
            let stop = body.saturating_add(n).min(end);
            match tag {
                b"LIST" => {
                    let kind: [u8; 4] = self.d.get(body..body + 4).and_then(|k| k.try_into().ok()).unwrap_or_default();
                    self.list(&kind, body + 4, stop, depth, &mut cx, &name);
                }
                b"tdmn" => {
                    name = c_string(self.d.get(body..stop).unwrap_or_default());
                    self.marker(&name, cx);
                }
                b"idta" => {
                    // a composition item starts here; its layers follow
                    let kind = be_u16(self.d, body).unwrap_or(0);
                    if kind == 4 && self.p.comps.len() < 1000 {
                        let id = be_u32(self.d, body + 16).unwrap_or(0);
                        self.p.comps.push(Comp { id, rate: 30.0, w: 1920.0, h: 1080.0, duration: 0.0, layers: Vec::new() });
                        cx.comp = Some(self.p.comps.len() - 1);
                    }
                }
                b"cdta" => {
                    let d = self.d;
                    if let Some(c) = self.comp_mut(cx.comp) {
                        let r = be_u32(d, body + 8).unwrap_or(0) as f64;
                        if r > 0.0 {
                            c.rate = r;
                        }
                        let dur = ratio_u(d, body + 44);
                        if dur.is_finite() && dur > 0.0 {
                            c.duration = dur.min(36_000.0);
                        }
                        let (w, h) = (be_u16(d, body + 140).unwrap_or(0), be_u16(d, body + 142).unwrap_or(0));
                        if w >= 4 && h >= 4 {
                            (c.w, c.h) = (f64::from(w), f64::from(h));
                        }
                    }
                }
                b"ldta" => {
                    let d = self.d;
                    if let Some(l) = self.layer_mut(cx) {
                        l.id = be_u32(d, body).unwrap_or(0);
                        l.start = ratio(d, body + 12);
                        l.in_point = ratio(d, body + 20);
                        l.out_point = ratio(d, body + 28);
                        l.source = be_u32(d, body + 40).unwrap_or(0);
                        l.null = d.get(body + 38).is_some_and(|b| b & 0x80 != 0);
                        l.visible = d.get(body + 39).is_none_or(|b| b & 0x01 != 0);
                        l.guide = d.get(body + 37).is_some_and(|b| b & 0x02 != 0);
                        if l.name.is_empty() {
                            l.name = c_string(d.get(body + 64..(body + 96).min(stop)).unwrap_or_default());
                        }
                        l.kind = match d.get(body + 131) {
                            Some(0) => LayerKind::Footage,
                            Some(1) => LayerKind::Light,
                            Some(2) => LayerKind::Camera,
                            Some(3) => LayerKind::Text,
                            Some(4) => LayerKind::Shape,
                            _ => LayerKind::Other,
                        };
                        l.parent = be_u32(d, body + 132).unwrap_or(0);
                    }
                }
                b"Utf8" if cx.top => {
                    // the layer's own (full-length) name
                    let s = c_string(self.d.get(body..stop).unwrap_or_default());
                    if let Some(l) = self.layer_mut(cx)
                        && !s.is_empty()
                    {
                        l.name = s;
                    }
                }
                b"tdsn" => {
                    if let Group::Effect(i) = cx.group {
                        let s = tdsn(self.d, body, stop);
                        if let Some(e) = self.layer_mut(cx).and_then(|l| l.effects.get_mut(i))
                            && e.display.is_empty()
                        {
                            e.display = s;
                        }
                    }
                }
                b"mkif" if cx.group == Group::Masks => {
                    let d = self.d;
                    if let Some(m) = self.layer_mut(cx).and_then(|l| l.masks.get_mut(mkifs)) {
                        m.inverted = d.get(body).is_some_and(|b| *b != 0);
                        m.mode = be_u16(d, body + 6).unwrap_or(1);
                    }
                    mkifs += 1;
                }
                _ => {}
            }
            p = body.saturating_add(n).saturating_add(n & 1);
        }
    }

    /// A match name that marks something without a group of its own.
    fn marker(&mut self, name: &str, cx: Ctx) {
        let Some(l) = self.layer_mut(cx) else { return };
        if cx.group == Group::Vectors && l.vector == VectorKind::None {
            l.vector = match name {
                "ADBE Vector Shape - Rect" => VectorKind::Rect,
                "ADBE Vector Shape - Ellipse" => VectorKind::Ellipse,
                "ADBE Vector Shape - Group" => VectorKind::Path,
                _ => VectorKind::None,
            };
        }
    }

    fn list(&mut self, kind: &[u8; 4], p: usize, end: usize, depth: u32, cx: &mut Ctx, name: &str) {
        match kind {
            b"Layr" => {
                let Some(c) = self.comp_mut(cx.comp) else { return };
                if c.layers.len() >= MAX_LAYERS {
                    return;
                }
                c.layers.push(Layer::default());
                let at = c.layers.len() - 1;
                self.walk(p, end, depth + 1, Ctx { comp: cx.comp, layer: Some(at), group: Group::Other, top: true });
            }
            b"tdbs" => {
                if cx.layer.is_some() {
                    let prop = self.prop(name, p, end, *cx);
                    self.store(prop, *cx);
                }
            }
            b"om-s" => {
                if let Group::Mask(i) = cx.group
                    && name == "ADBE Mask Shape"
                {
                    let shapes = self.mask_shapes(p, end, *cx);
                    if let Some(m) = self.layer_mut(*cx).and_then(|l| l.masks.get_mut(i)) {
                        m.shapes = shapes;
                    }
                }
            }
            b"sspc" if cx.group == Group::Effects => {
                // one effect: its definitions (`LIST parT`) and values (`LIST tdgp`)
                let group = self.group_for(name, *cx);
                self.walk(p, end, depth + 1, Ctx { group, top: false, ..*cx });
            }
            b"parT" => {}
            b"tdgp" => {
                let group = self.group_for(name, *cx);
                self.walk(p, end, depth + 1, Ctx { group, top: false, ..*cx });
            }
            b"Item" => {
                // a new item: its own composition (if any) is found inside
                self.walk(p, end, depth + 1, Ctx { comp: None, layer: None, group: Group::Other, top: false });
            }
            _ => {
                self.walk(p, end, depth + 1, Ctx { top: false, ..*cx });
            }
        }
    }

    /// The group a `LIST tdgp` named `name` opens.
    fn group_for(&mut self, name: &str, cx: Ctx) -> Group {
        match cx.group {
            Group::Effects => {
                let Some(l) = self.layer_mut(cx) else { return Group::Other };
                if l.effects.len() >= MAX_EFFECTS {
                    return Group::Other;
                }
                l.effects.push(Effect { match_name: name.to_string(), ..Effect::default() });
                Group::Effect(l.effects.len() - 1)
            }
            Group::Masks if name == "ADBE Mask Atom" => {
                let Some(l) = self.layer_mut(cx) else { return Group::Other };
                if l.masks.len() >= MAX_MASKS {
                    return Group::Other;
                }
                l.masks.push(Mask { mode: 1, ..Mask::default() });
                Group::Mask(l.masks.len() - 1)
            }
            Group::Vectors | Group::VectorTransform => {
                if name == "ADBE Vector Transform Group" {
                    Group::VectorTransform
                } else {
                    cx.group
                }
            }
            Group::Effect(_) | Group::Mask(_) | Group::Text | Group::Masks => cx.group,
            Group::Other | Group::Transform => match name {
                "ADBE Transform Group" => Group::Transform,
                "ADBE Effect Parade" => Group::Effects,
                "ADBE Mask Parade" => Group::Masks,
                "ADBE Root Vectors Group" => Group::Vectors,
                "ADBE Text Properties" => Group::Text,
                _ => Group::Other,
            },
        }
    }

    fn store(&mut self, prop: Prop, cx: Ctx) {
        let Some(l) = self.layer_mut(cx) else { return };
        match cx.group {
            Group::Transform => {
                if l.prop(&prop.name).is_none() {
                    l.props.push(prop);
                }
            }
            Group::Effect(i) => {
                if let Some(e) = l.effects.get_mut(i)
                    && e.params.len() < 64
                {
                    e.params.push(prop);
                }
            }
            Group::Mask(i) => {
                if let Some(m) = l.masks.get_mut(i) {
                    match prop.name.as_str() {
                        "ADBE Mask Feather" => m.feather = Some(prop),
                        "ADBE Mask Opacity" => m.opacity = Some(prop),
                        "ADBE Mask Offset" => m.expansion = Some(prop),
                        _ => {}
                    }
                }
            }
            Group::Vectors => {
                let wanted = matches!(
                    prop.name.as_str(),
                    "ADBE Vector Rect Size"
                        | "ADBE Vector Rect Position"
                        | "ADBE Vector Rect Roundness"
                        | "ADBE Vector Ellipse Size"
                        | "ADBE Vector Ellipse Position"
                        | "ADBE Vector Fill Color"
                        | "ADBE Vector Stroke Color"
                        | "ADBE Vector Stroke Width"
                );
                if wanted && l.vector_prop(&prop.name).is_none() {
                    l.vector_props.push(prop);
                }
            }
            Group::Text => {
                let animator = matches!(prop.name.as_str(), "ADBE Text Percent Start" | "ADBE Text Percent End" | "ADBE Text Percent Offset");
                if animator && (prop.keys.len() >= 2 || prop.expr.is_some()) {
                    l.per_character = true;
                }
            }
            _ => {}
        }
    }

    /// A property: the chunks of its `LIST tdbs`.
    fn prop(&mut self, name: &str, mut p: usize, end: usize, cx: Ctx) -> Prop {
        let d = self.d;
        let mut prop = Prop { name: name.to_string(), ..Prop::default() };
        let mut dims = 1usize;
        let mut expr_off = false;
        let rate = self.rate(cx);
        let mut list: Option<(usize, usize)> = None;
        while p.saturating_add(8) <= end && self.chunks < MAX_CHUNKS {
            self.chunks += 1;
            let Some(tag) = d.get(p..p + 4) else { break };
            let Some(n) = be_u32(d, p + 4).map(|n| n as usize) else { break };
            let body = p + 8;
            let stop = body.saturating_add(n).min(end);
            match tag {
                b"tdb4" => {
                    dims = usize::from(be_u16(d, body + 2).unwrap_or(1)).clamp(1, 4);
                    prop.spatial = d.get(body + 5).is_some_and(|b| b & 0x08 != 0);
                    prop.color = d.get(body + 59).is_some_and(|b| b & 0x01 != 0);
                    expr_off = d.get(body + 119).is_some_and(|b| b & 0x01 != 0);
                }
                b"tdsn" => prop.display = tdsn(d, body, stop),
                b"cdat" => {
                    let count = (stop.saturating_sub(body) / 8).min(16);
                    prop.value = (0..count).map(|i| be_f64(d, body + 8 * i).unwrap_or(0.0)).collect();
                }
                b"Utf8" => {
                    let raw = d.get(body..stop).unwrap_or_default();
                    if raw.len() <= MAX_EXPR {
                        let s = String::from_utf8_lossy(raw).into_owned();
                        if !s.trim().is_empty() {
                            prop.expr = Some(s);
                        }
                    }
                }
                b"LIST" if d.get(body..body + 4) == Some(&b"list"[..]) => list = Some((body + 4, stop)),
                _ => {}
            }
            p = body.saturating_add(n).saturating_add(n & 1);
        }
        // the keyframes are read once the property's kind is known
        if let Some((a, b)) = list {
            prop.keys = keys(d, a, b, rate, dims, prop.spatial, prop.color);
        }
        let keep = if prop.color { 4 } else { dims };
        prop.value.truncate(keep);
        if expr_off {
            prop.expr = None;
        }
        prop
    }

    /// The keyframed (or still) path of a mask (`LIST om-s`).
    fn mask_shapes(&mut self, mut p: usize, end: usize, cx: Ctx) -> Vec<(f64, Interp, Path)> {
        let d = self.d;
        let rate = self.rate(cx);
        let mut times: Vec<(f64, Interp)> = Vec::new();
        let mut paths: Vec<Path> = Vec::new();
        while p.saturating_add(8) <= end && self.chunks < MAX_CHUNKS {
            self.chunks += 1;
            let Some(tag) = d.get(p..p + 4) else { break };
            let Some(n) = be_u32(d, p + 4).map(|n| n as usize) else { break };
            let body = p + 8;
            let stop = body.saturating_add(n).min(end);
            if tag == b"LIST" {
                match d.get(body..body + 4) {
                    Some(b"tdbs") => times = key_times(d, body + 4, stop, rate),
                    Some(b"omks") => paths = shaps(d, body + 4, stop),
                    _ => {}
                }
            }
            p = body.saturating_add(n).saturating_add(n & 1);
        }
        if times.is_empty() {
            return paths.into_iter().take(1).map(|s| (0.0, Interp::Linear, s)).collect();
        }
        times.into_iter().zip(paths).map(|((t, i), s)| (t, i, s)).collect()
    }
}

/// The records of a keyframe `LIST list`.
fn keys(d: &[u8], mut p: usize, end: usize, rate: f64, dims: usize, spatial: bool, color: bool) -> Vec<Key> {
    let (mut count, mut size) = (0usize, 0usize);
    let mut out = Vec::new();
    while p.saturating_add(8) <= end {
        let Some(tag) = d.get(p..p + 4) else { break };
        let Some(n) = be_u32(d, p + 4).map(|n| n as usize) else { break };
        let body = p + 8;
        if tag == b"lhd3" {
            count = be_u32(d, body + 8).unwrap_or(0) as usize;
            size = be_u32(d, body + 16).unwrap_or(0) as usize;
        } else if tag == b"ldat" && size >= 8 {
            let last = body.saturating_add(n).min(end);
            for k in 0..count.min(MAX_KEYS) {
                let r = body.saturating_add(k.saturating_mul(size));
                if r.saturating_add(size) > last {
                    break;
                }
                let Some(t) = be_i32(d, r) else { break };
                let f = |i: usize| be_f64(d, r + 8 + 8 * i).unwrap_or(0.0);
                let mut key = Key {
                    t: f64::from(t) / rate,
                    interp_in: Interp::from_byte(d.get(r + 4).copied().unwrap_or(1)),
                    interp_out: Interp::from_byte(d.get(r + 5).copied().unwrap_or(1)),
                    ..Key::default()
                };
                if color && size >= 8 + 8 * 10 {
                    key.v = (6..10).map(f).collect();
                    (key.in_speed, key.in_infl, key.out_speed, key.out_infl) = (vec![f(2)], vec![f(3)], vec![f(4)], vec![f(5)]);
                } else if spatial && size >= 8 + 8 * (6 + dims) {
                    key.v = (6..6 + dims).map(f).collect();
                    (key.in_speed, key.in_infl, key.out_speed, key.out_infl) = (vec![f(2)], vec![f(3)], vec![f(4)], vec![f(5)]);
                } else if size >= 8 + 8 * 5 * dims {
                    key.v = (0..dims).map(f).collect();
                    let part = |from: usize| (from * dims..(from + 1) * dims).map(f).collect::<Vec<f64>>();
                    (key.in_speed, key.in_infl, key.out_speed, key.out_infl) = (part(1), part(2), part(3), part(4));
                } else if size >= 16 {
                    key.v = vec![f(0)];
                }
                out.push(key);
            }
            out.sort_by(|a, b| a.t.total_cmp(&b.t));
            return out;
        }
        p = body.saturating_add(n).saturating_add(n & 1);
    }
    out
}

/// Keyframe times inside a `LIST tdbs` (its `LIST list`); a path's keyframes carry no values
/// there.
fn key_times(d: &[u8], mut p: usize, end: usize, rate: f64) -> Vec<(f64, Interp)> {
    while p.saturating_add(8) <= end {
        let Some(tag) = d.get(p..p + 4) else { break };
        let Some(n) = be_u32(d, p + 4).map(|n| n as usize) else { break };
        let body = p + 8;
        if tag == b"LIST" && d.get(body..body + 4) == Some(&b"list"[..]) {
            return keys(d, body + 4, body.saturating_add(n).min(end), rate, 1, false, false).into_iter().map(|k| (k.t, k.interp_out)).collect();
        }
        p = body.saturating_add(n).saturating_add(n & 1);
    }
    Vec::new()
}

/// The `LIST shap` paths of an `omks`, in keyframe order.
fn shaps(d: &[u8], mut p: usize, end: usize) -> Vec<Path> {
    let mut out = Vec::new();
    while p.saturating_add(8) <= end && out.len() < MAX_KEYS {
        let Some(tag) = d.get(p..p + 4) else { break };
        let Some(n) = be_u32(d, p + 4).map(|n| n as usize) else { break };
        let body = p + 8;
        let stop = body.saturating_add(n).min(end);
        if tag == b"LIST" && d.get(body..body + 4) == Some(&b"shap"[..]) {
            out.push(shap(d, body + 4, stop));
        }
        p = body.saturating_add(n).saturating_add(n & 1);
    }
    out
}

/// One mask path: `shph` (open flag, bounding box) and its points (0..1 of the box).
fn shap(d: &[u8], mut p: usize, end: usize) -> Path {
    let mut path = Path { closed: true, pts: Vec::new() };
    let mut bbox = [0.0, 0.0, 1.0, 1.0];
    while p.saturating_add(8) <= end {
        let Some(tag) = d.get(p..p + 4) else { break };
        let Some(n) = be_u32(d, p + 4).map(|n| n as usize) else { break };
        let body = p + 8;
        let stop = body.saturating_add(n).min(end);
        if tag == b"shph" {
            path.closed = d.get(body + 3).is_none_or(|b| b & 0x08 == 0);
            for (i, slot) in bbox.iter_mut().enumerate() {
                if let Some(v) = be_f32(d, body + 4 + 4 * i) {
                    *slot = v;
                }
            }
        } else if tag == b"LIST" && d.get(body..body + 4) == Some(&b"list"[..]) {
            let mut q = body + 4;
            let mut count = 0usize;
            while q.saturating_add(8) <= stop {
                let Some(t) = d.get(q..q + 4) else { break };
                let Some(m) = be_u32(d, q + 4).map(|m| m as usize) else { break };
                let b = q + 8;
                let b_end = b.saturating_add(m).min(stop);
                if t == b"lhd3" {
                    count = be_u32(d, b + 8).unwrap_or(0) as usize;
                } else if t == b"ldat" {
                    for i in 0..count.min(MAX_POINTS) {
                        let at = b + 8 * i;
                        if at + 8 > b_end {
                            break;
                        }
                        let (Some(x), Some(y)) = (be_f32(d, at), be_f32(d, at + 4)) else { break };
                        let [x0, y0, x1, y1] = bbox;
                        path.pts.push([x0 * (1.0 - x) + x1 * x, y0 * (1.0 - y) + y1 * y]);
                    }
                }
                q = b.saturating_add(m).saturating_add(m & 1);
            }
        }
        p = body.saturating_add(n).saturating_add(n & 1);
    }
    path
}

/// Read an After Effects project (`.aep` bytes). `None` when it is not one or has no layers.
pub(crate) fn read(aep: &[u8]) -> Option<Project> {
    if aep.get(0..4) != Some(&b"RIFX"[..]) {
        return None;
    }
    let mut w = Walk { d: aep, chunks: 0, p: Project::default() };
    w.walk(12, aep.len(), 0, Ctx { comp: None, layer: None, group: Group::Other, top: false });
    w.p.comps.retain(|c| !c.layers.is_empty());
    (!w.p.comps.is_empty()).then_some(w.p)
}

#[cfg(test)]
#[path = "aep_tests.rs"]
pub(crate) mod tests;

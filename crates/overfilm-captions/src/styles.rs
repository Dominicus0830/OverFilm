//! Caption styles and motion for the editor (milestone M4.2).
//!
//! A [`CaptionPreset`] turns one caption into a graphic text layer (the editor's `graphic_text`
//! effect: fill, two strokes, box, shadow, transform) with keyframed motion in and out, so the
//! captions can be restyled, animated and moved like any graphic. The built-in presets are
//! original designs; the user's own presets are saved as JSON next to the project.
//!
//! Speakers: a caption whose speaker (or a leading `(화자1)` / `(이름)` label written by
//! OverFilmSubs) has a colour in `speaker_colors` gets that fill, and the label is dropped from
//! the text when `hide_speaker_label` is on.

use filmcraft_geom::Vec2;
use filmcraft_project::graphic::{TEXT_LAYER, new_shape_layer, new_text_layer};
use filmcraft_project::{EffectInstance, Interpolation, Keyframe, LayerExtra, Mask, MaskMode, MaskPath, MaskVertex, Param, ParamValue, Pin, PinTarget};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use serde::{Deserialize, Serialize};

/// How a caption appears or leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Motion {
    #[default]
    None,
    /// Opacity 0 → 100.
    Fade,
    /// Scale 70 → 108 → 100 with a fade (comes out: 100 → 80, fading).
    Pop,
    /// Rises 4 % of the frame height while fading in (out: sinks while fading).
    SlideUp,
    /// One character after another (in only; out behaves like `Fade`).
    Typewriter,
    /// Drops in from above and settles with a small bounce.
    Bounce,
}

impl Motion {
    pub fn parse(s: &str) -> Option<Motion> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "없음" => Some(Motion::None),
            "fade" | "페이드" => Some(Motion::Fade),
            "pop" | "팝" => Some(Motion::Pop),
            "slideup" | "slide" | "올라오기" => Some(Motion::SlideUp),
            "typewriter" | "type" | "타자기" | "한글자씩" => Some(Motion::Typewriter),
            "bounce" | "통통" => Some(Motion::Bounce),
            _ => None,
        }
    }
}

/// One point of a [`CustomMotion`] (a motion taken from a template), relative to the caption
/// at rest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyPoint {
    /// Seconds from the start of this part of the motion.
    pub t: f32,
    /// Percent of the size at rest.
    pub scale: f32,
    /// Percent of the opacity at rest.
    pub opacity: f32,
    /// Offset from the place at rest, as a share of the frame height.
    pub dx: f32,
    pub dy: f32,
    /// Degrees from the angle at rest.
    pub rotation: f32,
    /// The segment leaving this point eases (else it is linear).
    pub ease: bool,
}

/// Keyframed motion in and out taken from a template: `in` runs from the caption's start, `out`
/// ends at the caption's end. Either may be empty.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomMotion {
    #[serde(rename = "in")]
    pub r#in: Vec<KeyPoint>,
    pub out: Vec<KeyPoint>,
}

impl CustomMotion {
    fn span(points: &[KeyPoint]) -> f32 {
        points.iter().map(|k| k.t).filter(|t| t.is_finite()).fold(0.0, f32::max)
    }
    /// Seconds the motion in and the motion out take.
    pub fn lengths(&self) -> (f32, f32) {
        (Self::span(&self.r#in), Self::span(&self.out))
    }
}

/// Where a template's motion in ends and its motion out starts (template seconds), so its effect
/// keyframes can be fitted into a caption the same way as the motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timing {
    pub start: f32,
    pub in_end: f32,
    pub out_at: f32,
    pub end: f32,
}

impl Timing {
    /// Template time `t` → seconds into a caption `duration` long: the in from the start, the out
    /// up to the end (both sped up together when they would take more than 60 % of the caption)
    /// and the time between stretched over the rest.
    pub fn fit(&self, t: f32, duration: Tick) -> f64 {
        let f = |x: f32| if x.is_finite() { x as f64 } else { 0.0 };
        let (start, in_end, out_at, end) = (f(self.start), f(self.in_end).max(f(self.start)), f(self.out_at), f(self.end));
        let out_at = out_at.max(in_end);
        let end = end.max(out_at);
        let dur = duration.seconds().max(0.0);
        let (a, b) = (in_end - start, end - out_at);
        let room = dur * 0.6;
        let k = if a + b > room && a + b > 0.0 { room / (a + b) } else { 1.0 };
        let t = f(t);
        let s = if t <= in_end {
            (t - start) * k
        } else if t >= out_at {
            dur - (end - t) * k
        } else {
            let (lo, hi) = (a * k, dur - b * k);
            lo + (hi - lo) * (t - in_end) / (out_at - in_end).max(1e-9)
        };
        s.clamp(0.0, dur)
    }
}

/// A value of a template effect parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FxValue {
    Num(f64),
    /// A length in text heights (scaled to the caption's size).
    Len(f64),
    /// A point from the caption's centre, in text heights.
    Pt([f64; 2]),
    /// Straight RGBA, 0..1.
    Color([f32; 4]),
    Bool(bool),
    Choice(u32),
}

/// A keyframe of a template effect parameter (template seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FxKey {
    pub t: f32,
    pub v: FxValue,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hold: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FxParam {
    pub id: String,
    pub value: FxValue,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<FxKey>,
}

/// An effect of the editor (by id) put on every caption clip.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FxLook {
    pub effect: String,
    pub params: Vec<FxParam>,
}

/// A mask path keyframe: vertices as `[x, y, in x, in y, out x, out y]` from the caption's
/// centre, in text heights (tangents relative to their vertex).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskKey {
    pub t: f32,
    pub closed: bool,
    pub vertices: Vec<[f64; 6]>,
    pub hold: bool,
}

/// A template mask, put on the caption clip's opacity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskLook {
    /// 0 none, 1 add, 2 subtract, 3 intersect, 4 lighten, 5 darken, 6 difference.
    pub mode: u8,
    pub inverted: bool,
    /// Text heights.
    pub feather: f64,
    /// Percent.
    pub opacity: f64,
    /// Text heights (negative shrinks).
    pub expansion: f64,
    pub path: Vec<MaskKey>,
}

/// A shape layer of a template (a box or a bar behind or beside the text).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShapeLook {
    /// `false` a rectangle, `true` an ellipse.
    pub ellipse: bool,
    /// Size and centre (from the caption's centre) in text heights.
    pub size: [f64; 2],
    pub at: [f64; 2],
    pub radius: f64,
    pub fill: Option<Rgba>,
    pub stroke: Option<Rgba>,
    /// Text heights.
    pub stroke_width: f64,
    /// Sized to the text (pinned to its edges) with this padding, text heights.
    pub fit_text: bool,
    pub pad: [f64; 2],
    /// Drawn in front of the text.
    pub front: bool,
    pub motion: Option<CustomMotion>,
}

/// A linear gradient text fill.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GradientLook {
    pub start: Rgba,
    pub end: Rgba,
    /// Degrees clockwise, 0 = left to right.
    pub angle: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShadowLook {
    pub color: Rgba,
    /// Percent.
    pub opacity: f32,
    /// Degrees.
    pub angle: f32,
    /// Text heights.
    pub distance: f32,
    pub blur: f32,
}

/// What a template brings beyond the text style and its motion: effects, masks, shapes, a
/// gradient fill, a shadow, and the motions of its other text layers (used line by line).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateLook {
    pub timing: Timing,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<FxLook>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub masks: Vec<MaskLook>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<ShapeLook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gradient: Option<GradientLook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowLook>,
    /// The motion of the template's second, third … text layer: a caption's second line moves
    /// with the first of these, and so on.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<CustomMotion>,
}

impl TemplateLook {
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty() && self.masks.is_empty() && self.shapes.is_empty() && self.gradient.is_none() && self.shadow.is_none() && self.lines.is_empty()
    }
}

/// Vertical place on the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    #[default]
    Bottom,
    Middle,
    Top,
}

/// A colour as `#rrggbb` or `#rrggbbaa` in files.
pub type Rgba = [u8; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Rgba,
    /// Pixels at 1080 lines.
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoxStyle {
    pub color: Rgba,
    /// Padding round the text, pixels at 1080 lines.
    pub padding: f32,
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptionPreset {
    pub name: String,
    /// Font family; empty = the editor's default.
    pub font: String,
    pub font_style: String,
    /// Pixels at 1080 lines.
    pub size: f32,
    pub fill: Rgba,
    pub stroke: Option<Stroke>,
    /// A second, outer stroke (the "double outline" of Korean variety captions).
    pub stroke2: Option<Stroke>,
    #[serde(rename = "box")]
    pub boxed: Option<BoxStyle>,
    pub shadow: bool,
    pub place: Place,
    /// Distance from the top/bottom edge as a share of the frame height.
    pub margin: f32,
    /// Line height as a multiple of the size.
    pub line_spacing: f32,
    pub motion_in: Motion,
    pub motion_out: Motion,
    /// Length of the in/out motion, seconds (capped to a third of the caption).
    pub motion_secs: f32,
    /// Fill per speaker name ("화자1", "도미니코" …).
    pub speaker_colors: Vec<(String, Rgba)>,
    pub hide_speaker_label: bool,
    /// Motion taken from a template; when set it is used instead of `motion_in`/`motion_out`
    /// (a typewriter in still types the text).
    pub custom_motion: Option<CustomMotion>,
    /// Effects, masks, shapes and more taken from a template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateLook>,
}

impl Default for CaptionPreset {
    fn default() -> Self {
        CaptionPreset {
            name: "기본".into(),
            font: String::new(),
            font_style: "Bold".into(),
            size: 64.0,
            fill: [255, 255, 255, 255],
            stroke: Some(Stroke { color: [0, 0, 0, 255], width: 7.0 }),
            stroke2: None,
            boxed: None,
            shadow: false,
            place: Place::Bottom,
            margin: 0.08,
            line_spacing: 1.25,
            motion_in: Motion::None,
            motion_out: Motion::None,
            motion_secs: 0.2,
            speaker_colors: Vec::new(),
            hide_speaker_label: true,
            custom_motion: None,
            template: None,
        }
    }
}

/// The built-in presets (original designs).
pub fn builtin_presets() -> Vec<CaptionPreset> {
    let base = CaptionPreset::default();
    vec![
        base.clone(),
        CaptionPreset {
            name: "박스".into(),
            font_style: "SemiBold".into(),
            size: 56.0,
            stroke: None,
            boxed: Some(BoxStyle { color: [0, 0, 0, 190], padding: 14.0, radius: 6.0 }),
            motion_in: Motion::Fade,
            motion_out: Motion::Fade,
            motion_secs: 0.12,
            ..base.clone()
        },
        CaptionPreset {
            name: "예능 노랑".into(),
            font_style: "Black".into(),
            size: 76.0,
            fill: [255, 226, 70, 255],
            stroke: Some(Stroke { color: [20, 20, 20, 255], width: 9.0 }),
            stroke2: Some(Stroke { color: [255, 255, 255, 255], width: 16.0 }),
            shadow: true,
            motion_in: Motion::Pop,
            motion_out: Motion::Fade,
            ..base.clone()
        },
        CaptionPreset {
            name: "해설 두 사람".into(),
            font_style: "SemiBold".into(),
            size: 58.0,
            stroke: Some(Stroke { color: [10, 10, 10, 255], width: 6.0 }),
            motion_in: Motion::SlideUp,
            motion_out: Motion::Fade,
            motion_secs: 0.18,
            speaker_colors: vec![("화자1".into(), [140, 205, 255, 255]), ("화자2".into(), [255, 186, 110, 255]), ("화자3".into(), [170, 235, 160, 255])],
            ..base.clone()
        },
        CaptionPreset {
            name: "강조".into(),
            font_style: "Black".into(),
            size: 92.0,
            fill: [255, 84, 70, 255],
            stroke: Some(Stroke { color: [255, 255, 255, 255], width: 10.0 }),
            shadow: true,
            place: Place::Middle,
            motion_in: Motion::Bounce,
            motion_out: Motion::Pop,
            motion_secs: 0.25,
            ..base.clone()
        },
        CaptionPreset { name: "한 글자씩".into(), motion_in: Motion::Typewriter, motion_out: Motion::Fade, ..base },
    ]
}

/// User presets file: `{"presets": [...]}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresetFile {
    #[serde(default)]
    pub presets: Vec<CaptionPreset>,
}

/// Split a leading speaker label `(화자1) ` off a caption's text.
pub fn split_speaker(text: &str) -> (Option<String>, &str) {
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix('(')
        && let Some(end) = rest.find(')')
    {
        let name = &rest[..end];
        if !name.is_empty() && name.chars().count() <= 20 && !name.contains('\n') {
            return (Some(name.to_string()), rest[end + 1..].trim_start());
        }
    }
    (None, text)
}

fn col(c: Rgba) -> ParamValue {
    ParamValue::Color([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0])
}

fn set(e: &mut EffectInstance, k: &str, v: ParamValue) {
    e.params.insert(k.to_string(), Param::new(v));
}

fn keyed(e: &mut EffectInstance, k: &str, keys: Vec<(Tick, ParamValue, Interpolation)>) {
    if keys.is_empty() {
        return;
    }
    let mut ks: Vec<Keyframe> = keys
        .into_iter()
        .map(|(t, v, i)| {
            let mut kf = Keyframe::new(t, v);
            kf.interp = i;
            kf
        })
        .collect();
    ks.sort_by_key(|k| k.time);
    ks.dedup_by_key(|k| k.time);
    let first = ks.first().map(|k| k.value.clone()).unwrap_or(ParamValue::Float(0.0));
    e.params.insert(k.to_string(), Param { value: first, keyframes: ks });
}

/// What a styled caption needs from the sequence.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
}

/// The text layer for one caption lasting `duration`, styled and animated by `preset`.
/// Where a caption's text block sits: its centre, the anchor offset to the first baseline, the
/// text size and line height (frame pixels).
struct Layout {
    pos: Vec2,
    ay: f64,
    size: f64,
    line_h: f64,
}

fn layout(preset: &CaptionPreset, lines: usize, frame: Frame) -> Layout {
    let (w, h) = (frame.width.max(16) as f64, frame.height.max(16) as f64);
    let k = h / 1080.0;
    let size = (preset.size.clamp(8.0, 400.0) as f64) * k;
    let lines = lines.clamp(1, 1000) as f64;
    let line_h = size * preset.line_spacing.clamp(0.8, 3.0) as f64;
    let margin = preset.margin.clamp(0.0, 0.45) as f64 * h;
    let first_baseline = match preset.place {
        Place::Bottom => h - margin - (lines - 1.0) * line_h - size * 0.25,
        Place::Top => margin + size,
        Place::Middle => h / 2.0 - (lines - 1.0) * line_h / 2.0 + size * 0.35,
    };
    // anchor at the middle of the text block, so scale motion grows from the centre
    let ay = (lines - 1.0) * line_h / 2.0 - size * 0.35;
    Layout { pos: Vec2::new(w / 2.0, first_baseline + ay), ay, size, line_h }
}

pub fn caption_layer(preset: &CaptionPreset, text: &str, speaker: Option<&str>, duration: Tick, frame: Frame) -> EffectInstance {
    let (label, body) = split_speaker(text);
    let who = speaker.map(str::to_string).or(label);
    let shown = if preset.hide_speaker_label { body.to_string() } else { text.to_string() };
    let h = frame.height.max(16) as f64;
    let k = h / 1080.0;
    let Layout { pos, ay, size, line_h } = layout(preset, shown.lines().count(), frame);
    let mut e = new_text_layer(&shown, pos, size);
    set(&mut e, "anchor", ParamValue::Vec2(Vec2::new(0.0, ay)));
    if !preset.font.is_empty() {
        set(&mut e, "font", ParamValue::Text(preset.font.clone()));
    }
    set(&mut e, "font_style", ParamValue::Text(preset.font_style.clone()));
    set(&mut e, "align", ParamValue::Choice(1));
    set(&mut e, "leading", ParamValue::Float(line_h - size * 1.21));
    let fill = who.as_deref().and_then(|n| preset.speaker_colors.iter().find(|(s, _)| s == n)).map(|(_, c)| *c).unwrap_or(preset.fill);
    set(&mut e, "fill_color", col(fill));
    if let Some(s) = &preset.stroke {
        set(&mut e, "stroke", ParamValue::Bool(true));
        set(&mut e, "stroke_color", col(s.color));
        set(&mut e, "stroke_width", ParamValue::Float(s.width.clamp(0.0, 100.0) as f64 * k));
    }
    if let Some(s) = &preset.stroke2 {
        set(&mut e, "stroke2", ParamValue::Bool(true));
        set(&mut e, "stroke2_color", col(s.color));
        set(&mut e, "stroke2_width", ParamValue::Float(s.width.clamp(0.0, 100.0) as f64 * k));
    }
    if let Some(b) = &preset.boxed {
        set(&mut e, "background", ParamValue::Bool(true));
        set(&mut e, "background_color", col([b.color[0], b.color[1], b.color[2], 255]));
        set(&mut e, "background_opacity", ParamValue::Float(b.color[3] as f64 / 2.55));
        set(&mut e, "background_size", ParamValue::Float(b.padding.clamp(0.0, 200.0) as f64 * k));
        set(&mut e, "background_radius", ParamValue::Float(b.radius.clamp(0.0, 200.0) as f64 * k));
    }
    if preset.shadow {
        set(&mut e, "shadow", ParamValue::Bool(true));
        set(&mut e, "shadow_distance", ParamValue::Float(6.0 * k));
        set(&mut e, "shadow_blur", ParamValue::Float(12.0 * k));
    }
    if let Some(t) = &preset.template {
        template_text_look(&mut e, t, size);
    }
    animate(&mut e, preset, &shown, pos, duration, h);
    e
}

fn finite(x: f64, d: f64) -> f64 {
    if x.is_finite() { x } else { d }
}

/// A template's gradient fill and shadow on a text layer `size` pixels high.
fn template_text_look(e: &mut EffectInstance, t: &TemplateLook, size: f64) {
    if let Some(g) = &t.gradient {
        set(e, "fill_kind", ParamValue::Choice(1));
        set(e, "gradient_start", col(g.start));
        set(e, "gradient_end", col(g.end));
        set(e, "gradient_angle", ParamValue::Float(finite(g.angle as f64, 90.0)));
    }
    if let Some(s) = &t.shadow {
        set(e, "shadow", ParamValue::Bool(true));
        set(e, "shadow_color", col([s.color[0], s.color[1], s.color[2], 255]));
        set(e, "shadow_opacity", ParamValue::Float(finite(s.opacity as f64, 75.0).clamp(0.0, 100.0)));
        set(e, "shadow_angle", ParamValue::Float(finite(s.angle as f64, 135.0)));
        set(e, "shadow_distance", ParamValue::Float((finite(s.distance as f64, 0.0) * size).clamp(0.0, 1000.0)));
        set(e, "shadow_blur", ParamValue::Float((finite(s.blur as f64, 0.0) * size).clamp(0.0, 1000.0)));
    }
}

/// Everything one caption puts on its graphic clip.
#[derive(Clone, Debug, Default)]
pub struct CaptionClip {
    /// Graphic layers in paint order (shapes behind, the text, shapes in front).
    pub layers: Vec<EffectInstance>,
    /// Effects on the whole clip (glow, light sweep, blur, wiggle …).
    pub effects: Vec<EffectInstance>,
    /// Masks for the clip's opacity.
    pub masks: Vec<Mask>,
}

/// Most of each kind a template may put on one caption.
const MAX_TEMPLATE_ITEMS: usize = 16;
const MAX_TEMPLATE_KEYS: usize = 512;

/// The caption's layers, effects and masks: [`caption_layer`] plus what the preset's template
/// brings — the other text layers' motion line by line, its shapes (pinned to the text when they
/// were sized to it), its effects and its masks, all fitted into the caption's length.
pub fn caption_clip(preset: &CaptionPreset, text: &str, speaker: Option<&str>, duration: Tick, frame: Frame) -> CaptionClip {
    let Some(t) = preset.template.as_ref().filter(|t| !t.is_empty()) else {
        return CaptionClip { layers: vec![caption_layer(preset, text, speaker, duration, frame)], ..CaptionClip::default() };
    };
    let (label, body) = split_speaker(text);
    let who = speaker.map(str::to_string).or(label);
    let shown = if preset.hide_speaker_label { body } else { text };
    let lines: Vec<&str> = shown.lines().collect();
    let h = frame.height.max(16) as f64;
    let Layout { pos, size, line_h, .. } = layout(preset, lines.len(), frame);
    let fit_shape = t.shapes.iter().any(|s| s.fit_text);
    let mut texts = Vec::new();
    if lines.len() >= 2 && !t.lines.is_empty() && !fit_shape {
        // each line its own layer, the second moving like the template's second text and so on
        let n = lines.len().min(8);
        for (i, line) in lines.iter().take(n).enumerate() {
            let mut p = preset.clone();
            p.hide_speaker_label = false;
            if let Some(m) = i.checked_sub(1).and_then(|j| t.lines.get(j)) {
                p.custom_motion = Some(m.clone());
            }
            let mut e = caption_layer(&p, line, who.as_deref(), duration, frame);
            let dy = (i as f64 - (n as f64 - 1.0) / 2.0) * line_h;
            // a one-line layer sits where a block's last (bottom), first (top) or only line would
            let base = match preset.place {
                Place::Bottom => (n as f64 - 1.0) / 2.0 * line_h,
                Place::Top => -(n as f64 - 1.0) / 2.0 * line_h,
                Place::Middle => 0.0,
            };
            shift_position(&mut e, dy - base);
            texts.push(e);
        }
    } else {
        let mut p = preset.clone();
        if who.is_some() {
            p.hide_speaker_label = false;
        }
        texts.push(caption_layer(&p, shown, who.as_deref(), duration, frame));
    }
    for (i, e) in texts.iter_mut().enumerate() {
        e.layer.get_or_insert_with(Box::<LayerExtra>::default).uid = i as u64 + 1;
    }
    let text_uid = 1;
    let mut behind = Vec::new();
    let mut front = Vec::new();
    for (i, s) in t.shapes.iter().take(MAX_TEMPLATE_ITEMS).enumerate() {
        let mut e = shape_layer(s, pos, size, duration, h);
        e.layer.get_or_insert_with(Box::<LayerExtra>::default).uid = (texts.len() + i) as u64 + 1;
        if s.fit_text {
            let (px, py) = (finite(s.pad[0], 0.0).clamp(-50.0, 50.0) * size, finite(s.pad[1], 0.0).clamp(-50.0, 50.0) * size);
            let pin = Pin { to: PinTarget::Layer(text_uid), left: true, top: true, right: true, bottom: true, offsets: [-px, -py, px, py] };
            e.layer.get_or_insert_with(Box::<LayerExtra>::default).pin = Some(pin);
        }
        if s.front { front.push(e) } else { behind.push(e) }
    }
    let mut layers = behind;
    layers.extend(texts);
    layers.extend(front);
    let effects = t.effects.iter().take(MAX_TEMPLATE_ITEMS).filter_map(|f| clip_effect(f, &t.timing, pos, size, duration)).collect();
    let masks = t.masks.iter().take(MAX_TEMPLATE_ITEMS).enumerate().filter_map(|(i, m)| clip_mask(i, m, &t.timing, pos, size, duration)).collect();
    CaptionClip { layers, effects, masks }
}

/// Move a layer's position (and every position keyframe) down by `dy` pixels.
fn shift_position(e: &mut EffectInstance, dy: f64) {
    let Some(p) = e.params.get_mut("position") else { return };
    let shift = |v: &mut ParamValue| {
        if let ParamValue::Vec2(v) = v {
            v.y += dy;
        }
    };
    shift(&mut p.value);
    for k in &mut p.keyframes {
        shift(&mut k.value);
    }
}

fn shape_layer(s: &ShapeLook, pos: Vec2, size: f64, duration: Tick, h: f64) -> EffectInstance {
    let at = Vec2::new(pos.x + finite(s.at[0], 0.0).clamp(-100.0, 100.0) * size, pos.y + finite(s.at[1], 0.0).clamp(-100.0, 100.0) * size);
    let wh = Vec2::new(finite(s.size[0], 1.0).clamp(0.0, 200.0) * size, finite(s.size[1], 1.0).clamp(0.0, 200.0) * size);
    let mut e = new_shape_layer(u32::from(s.ellipse), at, wh, Vec::new());
    set(&mut e, "corner_radius", ParamValue::Float((finite(s.radius, 0.0) * size).clamp(0.0, 10_000.0)));
    match s.fill {
        Some(c) => set(&mut e, "fill_color", col(c)),
        None => set(&mut e, "fill", ParamValue::Bool(false)),
    }
    if let Some(c) = s.stroke {
        set(&mut e, "stroke", ParamValue::Bool(true));
        set(&mut e, "stroke_color", col(c));
        set(&mut e, "stroke_width", ParamValue::Float((finite(s.stroke_width, 0.0) * size).clamp(0.0, 1000.0)));
    }
    if let Some(m) = s.motion.as_ref().filter(|m| !m.r#in.is_empty() || !m.out.is_empty())
        && duration > Tick::ZERO
    {
        animate_custom(&mut e, m, false, "", at, duration, h);
    }
    e
}

fn interp(hold: bool) -> Interpolation {
    if hold { Interpolation::Hold } else { Interpolation::Linear }
}

fn fx_value(v: &FxValue, pos: Vec2, size: f64) -> ParamValue {
    match v {
        FxValue::Num(x) => ParamValue::Float(finite(*x, 0.0)),
        FxValue::Len(x) => ParamValue::Float((finite(*x, 0.0) * size).clamp(-100_000.0, 100_000.0)),
        FxValue::Pt([x, y]) => {
            ParamValue::Vec2(Vec2::new(pos.x + finite(*x, 0.0).clamp(-1000.0, 1000.0) * size, pos.y + finite(*y, 0.0).clamp(-1000.0, 1000.0) * size))
        }
        FxValue::Color(c) => ParamValue::Color(c.map(|x| if x.is_finite() { x.clamp(0.0, 1.0) } else { 1.0 })),
        FxValue::Bool(b) => ParamValue::Bool(*b),
        FxValue::Choice(c) => ParamValue::Choice(*c),
    }
}

/// A template effect as an effect of the editor, its keys fitted into the caption.
fn clip_effect(f: &FxLook, timing: &Timing, pos: Vec2, size: f64, duration: Tick) -> Option<EffectInstance> {
    let def = filmcraft_project::find_effect(&f.effect)?;
    let mut e = def.instance();
    for p in &f.params {
        if def.param(&p.id).is_none() {
            continue;
        }
        let keys: Vec<(Tick, ParamValue, Interpolation)> = p
            .keys
            .iter()
            .take(MAX_TEMPLATE_KEYS)
            .map(|k| (Tick::from_seconds_f64(timing.fit(k.t, duration)), fx_value(&k.v, pos, size), interp(k.hold)))
            .collect();
        if keys.len() >= 2 {
            keyed(&mut e, &p.id, keys);
        } else {
            set(&mut e, &p.id, fx_value(&p.value, pos, size));
        }
    }
    Some(e)
}

fn mask_path(k: &MaskKey, pos: Vec2, size: f64) -> MaskPath {
    let c = |x: f64| finite(x, 0.0).clamp(-1000.0, 1000.0) * size;
    let vertices = k
        .vertices
        .iter()
        .take(1024)
        .map(|v| MaskVertex { p: Vec2::new(pos.x + c(v[0]), pos.y + c(v[1])), t_in: Vec2::new(c(v[2]), c(v[3])), t_out: Vec2::new(c(v[4]), c(v[5])) })
        .collect();
    MaskPath { vertices, closed: k.closed }
}

/// A template mask on the caption clip, its path keys fitted into the caption.
fn clip_mask(i: usize, m: &MaskLook, timing: &Timing, pos: Vec2, size: f64, duration: Tick) -> Option<Mask> {
    let first = m.path.first()?;
    let mut mask = Mask::new(format!("Mask {}", i + 1), mask_path(first, pos, size));
    if m.path.len() >= 2 {
        let mut ks: Vec<Keyframe> = m
            .path
            .iter()
            .take(MAX_TEMPLATE_KEYS)
            .map(|k| {
                let mut kf = Keyframe::new(Tick::from_seconds_f64(timing.fit(k.t, duration)), ParamValue::Path(mask_path(k, pos, size)));
                kf.interp = interp(k.hold);
                kf
            })
            .collect();
        ks.sort_by_key(|k| k.time);
        ks.dedup_by_key(|k| k.time);
        mask.path.keyframes = ks;
    }
    mask.feather = Param::new(ParamValue::Float((finite(m.feather, 0.0) * size).clamp(0.0, 10_000.0)));
    mask.opacity = Param::new(ParamValue::Float(finite(m.opacity, 100.0).clamp(0.0, 100.0)));
    mask.expansion = Param::new(ParamValue::Float((finite(m.expansion, 0.0) * size).clamp(-10_000.0, 10_000.0)));
    mask.inverted = m.inverted;
    mask.mode = match m.mode {
        0 => MaskMode::None,
        2 => MaskMode::Subtract,
        3 => MaskMode::Intersect,
        4 => MaskMode::Lighten,
        5 => MaskMode::Darken,
        6 => MaskMode::Difference,
        _ => MaskMode::Add,
    };
    Some(mask)
}

/// Keyframe the in and out motion (clip-relative times).
fn animate(e: &mut EffectInstance, preset: &CaptionPreset, text: &str, pos: Vec2, duration: Tick, h: f64) {
    if duration <= Tick::ZERO {
        return;
    }
    let want = Tick::from_seconds_f64(preset.motion_secs.clamp(0.0, 2.0) as f64);
    let m = want.min(duration.mul_ratio(1, 3));
    if m <= Tick::ZERO {
        return;
    }
    if let Some(c) = preset.custom_motion.as_ref().filter(|c| !c.r#in.is_empty() || !c.out.is_empty()) {
        animate_custom(e, c, preset.motion_in == Motion::Typewriter, text, pos, duration, h);
        return;
    }
    let end = duration;
    let (lin, ease, out_ease, hold) = (Interpolation::Linear, Interpolation::EaseOut, Interpolation::EaseIn, Interpolation::Hold);
    let f = ParamValue::Float;
    let mut opacity: Vec<(Tick, ParamValue, Interpolation)> = Vec::new();
    let mut scale: Vec<(Tick, ParamValue, Interpolation)> = Vec::new();
    let mut position: Vec<(Tick, ParamValue, Interpolation)> = Vec::new();
    let v = |dy: f64| ParamValue::Vec2(Vec2::new(pos.x, pos.y + dy));
    match preset.motion_in {
        Motion::None => {}
        Motion::Fade => opacity.extend([(Tick::ZERO, f(0.0), ease), (m, f(100.0), lin)]),
        Motion::Pop => {
            opacity.extend([(Tick::ZERO, f(0.0), ease), (m.mul_ratio(1, 2), f(100.0), lin)]);
            scale.extend([(Tick::ZERO, f(70.0), ease), (m.mul_ratio(2, 3), f(108.0), lin), (m, f(100.0), lin)]);
        }
        Motion::SlideUp => {
            opacity.extend([(Tick::ZERO, f(0.0), ease), (m, f(100.0), lin)]);
            position.extend([(Tick::ZERO, v(h * 0.04), ease), (m, v(0.0), lin)]);
        }
        Motion::Bounce => {
            opacity.extend([(Tick::ZERO, f(0.0), ease), (m.mul_ratio(1, 3), f(100.0), lin)]);
            position.extend([
                (Tick::ZERO, v(-h * 0.06), out_ease),
                (m.mul_ratio(3, 5), v(h * 0.01), ease),
                (m.mul_ratio(4, 5), v(-h * 0.004), ease),
                (m, v(0.0), lin),
            ]);
        }
        Motion::Typewriter => {
            let chars: Vec<char> = text.chars().collect();
            let n = chars.len().clamp(1, 400) as i64;
            // at most 60 ms a character, and done within half the caption
            let per = Tick(TICKS_PER_SECOND * 6 / 100).min(Tick(duration.0 / 2 / n)).max(Tick(1));
            let mut keys = Vec::new();
            for i in 1..=n {
                let shown: String = chars.iter().take(i as usize).collect();
                keys.push((Tick(per.0 * (i - 1)), ParamValue::Text(shown), hold));
            }
            keyed(e, "text", keys);
        }
    }
    let out_at = end - m;
    match preset.motion_out {
        Motion::None => {}
        Motion::Fade | Motion::Typewriter => opacity.extend([(out_at, f(100.0), out_ease), (end, f(0.0), lin)]),
        Motion::Pop => {
            opacity.extend([(out_at, f(100.0), out_ease), (end, f(0.0), lin)]);
            scale.extend([(out_at, f(100.0), out_ease), (end, f(80.0), lin)]);
        }
        Motion::SlideUp | Motion::Bounce => {
            opacity.extend([(out_at, f(100.0), out_ease), (end, f(0.0), lin)]);
            position.extend([(out_at, v(0.0), out_ease), (end, v(h * 0.03), lin)]);
        }
    }
    keyed(e, "opacity", opacity);
    keyed(e, "scale", scale);
    keyed(e, "position", position);
}

/// The look of a text layer as a preset (the reverse of [`caption_layer`]): font, size, fill,
/// outlines, box and shadow, sizes scaled back to 1080 lines; its place from where it sits; its
/// motion from what is keyframed (text → typewriter, scale → pop, position → slide up, opacity
/// alone → fade).
pub fn preset_from_layer(e: &EffectInstance, name: &str, frame: Frame) -> CaptionPreset {
    let base = CaptionPreset::default();
    let h = frame.height.max(16) as f64;
    let k = 1080.0 / h;
    let val = |key: &str| e.params.get(key).map(|p| &p.value);
    let num = |key: &str| val(key).and_then(ParamValue::as_f64).filter(|v| v.is_finite());
    let on = |key: &str| val(key).and_then(ParamValue::as_bool).unwrap_or(false);
    let rgba =
        |key: &str| val(key).and_then(ParamValue::as_color).map(|c| c.map(|x| if x.is_finite() { (x.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 }));
    let text = |key: &str| match val(key) {
        Some(ParamValue::Text(t)) => Some(t.clone()),
        _ => None,
    };
    let px = |key: &str, default: f64| (num(key).unwrap_or(default) * k).clamp(0.0, 400.0) as f32;
    let stroke = |on_key: &str, colour: &str, width: &str| {
        on(on_key).then(|| Stroke { color: rgba(colour).unwrap_or([0, 0, 0, 255]), width: px(width, 4.0).min(100.0) })
    };
    let boxed = on("background").then(|| {
        let mut color = rgba("background_color").unwrap_or([0, 0, 0, 255]);
        color[3] = (num("background_opacity").unwrap_or(100.0).clamp(0.0, 100.0) * 2.55).round() as u8;
        BoxStyle { color, padding: px("background_size", 0.0).min(200.0), radius: px("background_radius", 0.0).min(200.0) }
    });
    let y = val("position").and_then(ParamValue::as_vec2).map(|v| v.y).filter(|y| y.is_finite()).unwrap_or(h * 0.85) / h;
    let place = if y < 0.34 {
        Place::Top
    } else if y < 0.66 {
        Place::Middle
    } else {
        Place::Bottom
    };
    let keyed = |key: &str| e.params.get(key).is_some_and(|p| p.keyframes.len() >= 2);
    let motion_in = if keyed("text") {
        Motion::Typewriter
    } else if keyed("scale") {
        Motion::Pop
    } else if keyed("position") {
        Motion::SlideUp
    } else if keyed("opacity") {
        Motion::Fade
    } else {
        Motion::None
    };
    CaptionPreset {
        name: name.to_string(),
        font: text("font").unwrap_or_default(),
        font_style: text("font_style").filter(|s| !s.is_empty()).unwrap_or_else(|| base.font_style.clone()),
        size: px("size", 64.0 / k).max(8.0),
        fill: rgba("fill_color").unwrap_or([255, 255, 255, 255]),
        stroke: stroke("stroke", "stroke_color", "stroke_width"),
        stroke2: stroke("stroke2", "stroke2_color", "stroke2_width"),
        boxed,
        shadow: on("shadow"),
        place,
        motion_in,
        motion_out: if motion_in == Motion::None { Motion::None } else { Motion::Fade },
        speaker_colors: Vec::new(),
        ..base
    }
}

/// Keyframe a template's motion: the in from the start, the out up to the end, both sped up
/// together when they would take more than 60 % of the caption (so it can be read at rest).
fn animate_custom(e: &mut EffectInstance, c: &CustomMotion, typewriter: bool, text: &str, pos: Vec2, duration: Tick, h: f64) {
    let (a, b) = c.lengths();
    let total = (a + b) as f64;
    let room = duration.seconds() * 0.6;
    let k = if total > room && total > 0.0 { room / total } else { 1.0 };
    let end_s = duration.seconds();
    let mut opacity = Vec::new();
    let mut scale = Vec::new();
    let mut position = Vec::new();
    let mut rotation = Vec::new();
    let finite = |x: f32, d: f64| if x.is_finite() { x as f64 } else { d };
    let mut add = |points: &[KeyPoint], at: f64| {
        for p in points.iter().take(256) {
            let t = Tick::from_seconds_f64((at + finite(p.t, 0.0) * k).clamp(0.0, end_s));
            let i = if p.ease { Interpolation::EaseOut } else { Interpolation::Linear };
            opacity.push((t, ParamValue::Float(finite(p.opacity, 100.0).clamp(0.0, 100.0)), i));
            scale.push((t, ParamValue::Float(finite(p.scale, 100.0).clamp(0.0, 1000.0)), i));
            position.push((t, ParamValue::Vec2(Vec2::new(pos.x + finite(p.dx, 0.0) * h, pos.y + finite(p.dy, 0.0) * h)), i));
            rotation.push((t, ParamValue::Float(finite(p.rotation, 0.0)), i));
        }
    };
    add(&c.r#in, 0.0);
    add(&c.out, end_s - b as f64 * k);
    // only what moves gets keyframes
    let moves = |keys: &[(Tick, ParamValue, Interpolation)]| keys.windows(2).any(|w| w.first().map(|x| &x.1) != w.get(1).map(|x| &x.1));
    for (name, keys) in [("opacity", opacity), ("scale", scale), ("position", position), ("rotation", rotation)] {
        if moves(&keys) {
            keyed(e, name, keys);
        }
    }
    if typewriter {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len().clamp(1, 400) as i64;
        let per = Tick(TICKS_PER_SECOND * 6 / 100).min(Tick(duration.0 / 2 / n)).max(Tick(1));
        let keys = (1..=n).map(|i| (Tick(per.0 * (i - 1)), ParamValue::Text(chars.iter().take(i as usize).collect()), Interpolation::Hold)).collect();
        keyed(e, "text", keys);
    }
}

/// Is `e` a text layer this module made (any text layer, really)?
pub fn is_text_layer(e: &EffectInstance) -> bool {
    e.effect == TEXT_LAYER
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: Frame = Frame { width: 1920, height: 1080 };

    fn secs(s: f64) -> Tick {
        Tick::from_seconds_f64(s)
    }

    #[test]
    fn speaker_label_is_split_and_coloured() {
        assert_eq!(split_speaker("(화자1) 안녕하세요"), (Some("화자1".into()), "안녕하세요"));
        assert_eq!(split_speaker("안녕 (괄호)"), (None, "안녕 (괄호)"));
        assert_eq!(split_speaker("()x"), (None, "()x"));
        let p = builtin_presets().into_iter().find(|p| p.name == "해설 두 사람").unwrap();
        let e = caption_layer(&p, "(화자2) 들어갑니다", None, secs(2.0), HD);
        assert_eq!(e.params["text"].value, ParamValue::Text("들어갑니다".into()));
        assert_eq!(e.params["fill_color"].value, col([255, 186, 110, 255]));
        // an explicit speaker wins over the label
        let e = caption_layer(&p, "(화자2) 들어갑니다", Some("화자1"), secs(2.0), HD);
        assert_eq!(e.params["fill_color"].value, col([140, 205, 255, 255]));
    }

    #[test]
    fn pop_keys_scale_and_opacity_inside_the_caption() {
        let p = CaptionPreset { motion_in: Motion::Pop, motion_out: Motion::Fade, motion_secs: 0.2, ..CaptionPreset::default() };
        let e = caption_layer(&p, "한방에 갑니다", None, secs(1.5), HD);
        let sc = &e.params["scale"].keyframes;
        assert_eq!(sc.len(), 3);
        assert_eq!(sc[0].value, ParamValue::Float(70.0));
        let op = &e.params["opacity"].keyframes;
        assert_eq!(op.first().unwrap().time, Tick::ZERO);
        assert_eq!(op.last().unwrap().time, secs(1.5));
        assert_eq!(op.last().unwrap().value, ParamValue::Float(0.0));
        assert!(op.windows(2).all(|w| w[0].time < w[1].time));
    }

    #[test]
    fn short_caption_motion_is_capped_to_a_third() {
        let p = CaptionPreset { motion_in: Motion::Fade, motion_out: Motion::Fade, motion_secs: 1.0, ..CaptionPreset::default() };
        let e = caption_layer(&p, "어", None, secs(0.3), HD);
        let op = &e.params["opacity"].keyframes;
        assert_eq!(op[1].time, secs(0.3).mul_ratio(1, 3));
    }

    #[test]
    fn a_styled_layer_reads_back_as_its_preset() {
        let p = builtin_presets().into_iter().find(|p| p.name == "예능 노랑").unwrap();
        let e = caption_layer(&p, "한방에 갑니다", None, secs(2.0), Frame { width: 3840, height: 2160 });
        let back = preset_from_layer(&e, "다시", Frame { width: 3840, height: 2160 });
        assert_eq!(back.name, "다시");
        assert_eq!(back.font_style, p.font_style);
        assert!((back.size - p.size).abs() < 0.01, "{} {}", back.size, p.size);
        assert_eq!(back.fill, p.fill);
        assert_eq!(back.stroke, p.stroke);
        assert_eq!(back.stroke2, p.stroke2);
        assert_eq!(back.boxed, None);
        assert!(back.shadow);
        assert_eq!(back.place, Place::Bottom);
        assert_eq!(back.motion_in, Motion::Pop);
        let b = builtin_presets().into_iter().find(|p| p.name == "박스").unwrap();
        let back = preset_from_layer(&caption_layer(&b, "x", None, secs(2.0), HD), "b", HD);
        assert_eq!(back.boxed.as_ref().map(|x| x.color), Some([0, 0, 0, 190]));
        assert_eq!(back.motion_in, Motion::Fade);
        // an empty layer still gives a usable preset
        let empty = EffectInstance {
            effect: TEXT_LAYER.into(),
            enabled: true,
            params: Default::default(),
            masks: Vec::new(),
            post_fader: false,
            essential: false,
            layer: None,
        };
        assert_eq!(preset_from_layer(&empty, "e", Frame { width: 0, height: 0 }).size, 64.0);
    }

    #[test]
    fn a_template_motion_is_fitted_into_a_short_caption() {
        let kp = |t: f32, scale: f32, opacity: f32| KeyPoint { t, scale, opacity, ..KeyPoint::default() };
        let c = CustomMotion { r#in: vec![kp(0.0, 0.0, 100.0), kp(2.0, 100.0, 100.0)], out: vec![kp(0.0, 100.0, 100.0), kp(1.0, 100.0, 0.0)] };
        let p = CaptionPreset { custom_motion: Some(c), ..CaptionPreset::default() };
        // 3 s of motion in a 2 s caption: sped up to 1.2 s (60 %)
        let e = caption_layer(&p, "가나", None, secs(2.0), HD);
        let sc = &e.params["scale"].keyframes;
        assert_eq!(sc.first().unwrap().value, ParamValue::Float(0.0));
        assert_eq!(sc[1].time, secs(0.8));
        let op = &e.params["opacity"].keyframes;
        assert_eq!(op.last().unwrap().time, secs(2.0));
        assert_eq!(op.last().unwrap().value, ParamValue::Float(0.0));
        assert_eq!(op[op.len() - 2].time, secs(1.6));
        assert!(!e.params.get("rotation").is_some_and(|r| r.is_animated()), "nothing rotates");
        // a long caption keeps the template's timing
        let e = caption_layer(&p, "가나", None, secs(10.0), HD);
        assert_eq!(e.params["scale"].keyframes[1].time, secs(2.0));
    }

    #[test]
    fn typewriter_holds_each_prefix() {
        let p = CaptionPreset { motion_in: Motion::Typewriter, ..CaptionPreset::default() };
        let e = caption_layer(&p, "가나다", None, secs(2.0), HD);
        let ks = &e.params["text"].keyframes;
        let texts: Vec<_> = ks.iter().map(|k| k.value.clone()).collect();
        assert_eq!(texts, vec![ParamValue::Text("가".into()), ParamValue::Text("가나".into()), ParamValue::Text("가나다".into())]);
        assert!(ks.iter().all(|k| k.interp == Interpolation::Hold));
    }

    #[test]
    fn hostile_values_do_not_panic() {
        let p = CaptionPreset { size: f32::NAN, margin: -4.0, motion_secs: f32::INFINITY, line_spacing: f32::NAN, ..CaptionPreset::default() };
        let _ = caption_layer(&p, "", None, Tick(-5), Frame { width: 0, height: 0 });
        let p = CaptionPreset { motion_in: Motion::Typewriter, ..CaptionPreset::default() };
        let _ = caption_layer(&p, &"가".repeat(5000), None, Tick(3), HD);
        for m in ["pop", "팝", "fade", "slideUp", "typewriter", "bounce", "none"] {
            assert!(Motion::parse(m).is_some(), "{m}");
        }
        let f: PresetFile = serde_json::from_str(r#"{"presets":[{"name":"x","motionIn":"pop"}]}"#).unwrap_or_default();
        let _ = f;
    }

    #[test]
    fn presets_round_trip_through_json() {
        let f = PresetFile { presets: builtin_presets() };
        let back: PresetFile = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    fn templated() -> CaptionPreset {
        let rise = CustomMotion {
            r#in: vec![
                KeyPoint { t: 0.0, scale: 100.0, opacity: 0.0, dy: 0.05, ..KeyPoint::default() },
                KeyPoint { t: 0.5, scale: 100.0, opacity: 100.0, ..KeyPoint::default() },
            ],
            out: vec![],
        };
        let look = TemplateLook {
            timing: Timing { start: 0.0, in_end: 0.5, out_at: 2.5, end: 3.0 },
            effects: vec![FxLook {
                effect: "text_light_sweep".into(),
                params: vec![
                    FxParam { id: "width".into(), value: FxValue::Len(0.5), keys: vec![] },
                    FxParam {
                        id: "center".into(),
                        value: FxValue::Pt([0.0, 0.0]),
                        keys: vec![FxKey { t: 0.0, v: FxValue::Pt([-2.0, 0.0]), hold: false }, FxKey { t: 3.0, v: FxValue::Pt([2.0, 0.0]), hold: false }],
                    },
                    FxParam { id: "no_such_param".into(), value: FxValue::Num(1.0), keys: vec![] },
                ],
            }],
            masks: vec![MaskLook {
                mode: 3,
                opacity: 100.0,
                feather: 0.1,
                path: vec![
                    MaskKey {
                        t: 0.0,
                        closed: true,
                        vertices: vec![[-1.0, -1.0, 0.0, 0.0, 0.0, 0.0], [1.0, -1.0, 0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]],
                        hold: false,
                    },
                    MaskKey {
                        t: 0.5,
                        closed: true,
                        vertices: vec![[-2.0, -1.0, 0.0, 0.0, 0.0, 0.0], [2.0, -1.0, 0.0, 0.0, 0.0, 0.0], [2.0, 1.0, 0.0, 0.0, 0.0, 0.0]],
                        hold: false,
                    },
                ],
                ..MaskLook::default()
            }],
            shapes: vec![ShapeLook { size: [4.0, 1.2], fill: Some([0, 0, 0, 200]), fit_text: true, pad: [0.2, 0.1], ..ShapeLook::default() }],
            gradient: Some(GradientLook { start: [255, 0, 0, 255], end: [0, 0, 255, 255], angle: 90.0 }),
            shadow: Some(ShadowLook { color: [0, 0, 0, 255], opacity: 50.0, angle: 135.0, distance: 0.1, blur: 0.2 }),
            lines: vec![rise],
        };
        CaptionPreset { size: 100.0, template: Some(look), ..CaptionPreset::default() }
    }

    #[test]
    fn a_template_brings_effects_masks_shapes_gradient_and_shadow() {
        let p = templated();
        let c = caption_clip(&p, "한 줄", None, secs(6.0), HD);
        // the box behind, then the text pinned … the box pinned to the text
        assert_eq!(c.layers.len(), 2);
        let (shape, text) = (&c.layers[0], &c.layers[1]);
        assert_eq!(shape.effect, filmcraft_project::graphic::SHAPE_LAYER);
        let text_uid = text.layer.as_ref().unwrap().uid;
        let pin = shape.layer.as_ref().and_then(|l| l.pin.clone()).unwrap();
        assert_eq!(pin.to, PinTarget::Layer(text_uid));
        assert_eq!(pin.offsets, [-20.0, -10.0, 20.0, 10.0]);
        assert_eq!(text.params["fill_kind"].value, ParamValue::Choice(1));
        let num = |k: &str| text.params[k].value.as_f64().unwrap();
        assert!((num("shadow_distance") - 10.0).abs() < 1e-4 && (num("shadow_blur") - 20.0).abs() < 1e-4);
        // the light sweep crosses the caption over its whole length; unknown params are dropped
        let fx = &c.effects[0];
        assert_eq!(fx.effect, "text_light_sweep");
        assert_eq!(fx.params["width"].value, ParamValue::Float(50.0));
        assert!(!fx.params.contains_key("no_such_param"));
        let keys = &fx.params["center"].keyframes;
        assert_eq!(keys.len(), 2);
        assert_eq!(keys.last().map(|k| k.time), Some(secs(6.0)));
        let pos = text.params["position"].value.as_vec2().unwrap();
        assert_eq!(keys[0].value, ParamValue::Vec2(Vec2::new(pos.x - 200.0, pos.y)));
        // the mask opens over the template's 0.5 s in
        let m = &c.masks[0];
        assert_eq!(m.mode, MaskMode::Intersect);
        assert_eq!(m.feather.value, ParamValue::Float(10.0));
        assert_eq!(m.path.keyframes.iter().map(|k| k.time).collect::<Vec<_>>(), vec![Tick::ZERO, secs(0.5)]);
    }

    #[test]
    fn a_templates_second_text_moves_the_second_line() {
        let mut p = templated();
        if let Some(t) = p.template.as_mut() {
            t.shapes.clear();
        }
        let c = caption_clip(&p, "첫 줄\n둘째 줄", None, secs(4.0), HD);
        assert_eq!(c.layers.len(), 2);
        let (a, b) = (&c.layers[0], &c.layers[1]);
        assert_eq!(a.params["text"].value, ParamValue::Text("첫 줄".into()));
        assert_eq!(b.params["text"].value, ParamValue::Text("둘째 줄".into()));
        assert!(!a.params.get("opacity").is_some_and(|o| o.is_animated()));
        assert!(b.params["opacity"].is_animated(), "line two rises in");
        // line two sits one line below line one, where the two-line block puts it
        let one = caption_layer(&p, "첫 줄\n둘째 줄", None, secs(4.0), HD);
        let y = |e: &EffectInstance| {
            e.params["position"].keyframes.last().and_then(|k| k.value.as_vec2()).or_else(|| e.params["position"].value.as_vec2()).unwrap().y
        };
        let line_h = 100.0 * p.line_spacing as f64;
        assert!((y(b) - y(a) - line_h).abs() < 1e-6, "{} {}", y(a), y(b));
        let block = one.params["position"].value.as_vec2().unwrap().y;
        assert!(((y(a) + y(b)) / 2.0 - block).abs() < 1e-6);
        // without a template: just the text
        let plain = caption_clip(&CaptionPreset::default(), "a\nb", None, secs(2.0), HD);
        assert_eq!((plain.layers.len(), plain.effects.len(), plain.masks.len()), (1, 0, 0));
    }

    #[test]
    fn hostile_template_values_stay_finite() {
        let mut p = templated();
        if let Some(t) = p.template.as_mut() {
            t.timing = Timing { start: f32::NAN, in_end: f32::INFINITY, out_at: -1.0, end: f32::NAN };
            t.shapes[0].size = [f64::NAN, f64::INFINITY];
            t.shapes[0].pad = [f64::NAN, 1e300];
            t.masks[0].feather = f64::NAN;
            t.masks[0].path[0].vertices[0] = [f64::NAN; 6];
            t.effects[0].params[0].value = FxValue::Len(f64::INFINITY);
            t.shadow.as_mut().unwrap().distance = f32::NAN;
        }
        let c = caption_clip(&p, "x", None, secs(1.0), HD);
        let json = serde_json::to_string(&(c.layers, c.effects, c.masks)).unwrap();
        // serde writes NaN and infinity as null
        assert!(!json.contains("null"), "{json}");
        let _ = caption_clip(&p, "", None, Tick::ZERO, Frame { width: 0, height: 0 });
    }
}

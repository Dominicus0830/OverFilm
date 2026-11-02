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
use filmcraft_project::graphic::{TEXT_LAYER, new_text_layer};
use filmcraft_project::{EffectInstance, Interpolation, Keyframe, Param, ParamValue};
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
pub fn caption_layer(preset: &CaptionPreset, text: &str, speaker: Option<&str>, duration: Tick, frame: Frame) -> EffectInstance {
    let (label, body) = split_speaker(text);
    let who = speaker.map(str::to_string).or(label);
    let shown = if preset.hide_speaker_label { body.to_string() } else { text.to_string() };
    let (w, h) = (frame.width.max(16) as f64, frame.height.max(16) as f64);
    let k = h / 1080.0;
    let size = (preset.size.clamp(8.0, 400.0) as f64) * k;
    let lines = shown.lines().count().max(1) as f64;
    let line_h = size * preset.line_spacing.clamp(0.8, 3.0) as f64;
    let margin = preset.margin.clamp(0.0, 0.45) as f64 * h;
    let first_baseline = match preset.place {
        Place::Bottom => h - margin - (lines - 1.0) * line_h - size * 0.25,
        Place::Top => margin + size,
        Place::Middle => h / 2.0 - (lines - 1.0) * line_h / 2.0 + size * 0.35,
    };
    // anchor at the middle of the text block, so scale motion grows from the centre
    let ay = (lines - 1.0) * line_h / 2.0 - size * 0.35;
    let pos = Vec2::new(w / 2.0, first_baseline + ay);
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
    animate(&mut e, preset, &shown, pos, duration, h);
    e
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
}

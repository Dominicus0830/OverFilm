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

//! OverFilm: open an OverShorts short in the editor (see `OVERFILM.md` and `overshorts/README.md`).
//!
//! OverShorts (the DC gallery → shorts tool in `overshorts/`) writes a `.overshorts` file: a JSON
//! list of what is on screen when (texts, pictures, sounds, markers) with every position already
//! worked out in frame pixels, beside the files it names. This module turns it into a new
//! vertical sequence a person can keep editing:
//!
//! - `overfilm.shorts.import {path, open=true}` imports the files into a bin "OverShorts · <name>",
//!   makes a sequence `<name>` of the file's size and rate with the tracks the file lists (first
//!   video track at the bottom), and places
//!   - each text (title, captions, the source line) as a graphic text clip, so it is retyped,
//!     restyled and moved like any title (Effect Controls, Program monitor);
//!   - each picture (the DC-style pages OverShorts drew, the reaction pictures, the original
//!     photos and videos) as its own clip, scaled and moved into its rectangle with Motion;
//!   - each sound (the narration, BGM with its ducking as volume keyframes, an original video's
//!     sound) on its audio track, BGM repeated to cover its range;
//!   - each segment as a sequence marker.
//!
//! One undo step. Files that are missing or that the editor can't open are skipped and listed
//! (`missing`, `failed`) instead of stopping the import; every number is clamped, every list
//! capped (the file may come from anywhere).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use filmcraft_geom::Vec2;
use filmcraft_project::graphic::ensure_layer_uids;
use filmcraft_project::{
    EffectInstance, ItemId, Keyframe, Label, Marker, MarkerId, MarkerKind, ParamValue, Project, SequenceSettings, Track, TrackId, TrackItem, TrackKind,
};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use overfilm_captions::styles::{CaptionPreset, Frame, Place, Rgba, Stroke, caption_layer};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, str_p};
use crate::{EngineError, Result, Session};

const CMD: &str = "overfilm.shorts.import";
/// The file's `format` value.
pub const FORMAT: &str = "overshorts.scenes";
/// File extension OverShorts writes (JSON inside).
pub const EXTENSION: &str = "overshorts";

const MAX_SECONDS: f64 = 600.0;
const MAX_TEXTS: usize = 800;
const MAX_PICTURES: usize = 400;
const MAX_SOUNDS: usize = 32;
const MAX_MARKERS: usize = 200;
const MAX_TRACKS: usize = 24;
const MAX_LEVELS: usize = 2000;
const MAX_FILE_BYTES: usize = 16 << 20;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Open OverShorts Short (OverFilm)…",
        menu: &["File"],
        shortcut: None,
        params: r#"{"path":path,"open":bool=true}"#,
        enabled: always,
        run: import,
        journal: true,
    }]
}

/// Whether `path` is an OverShorts scene file (by name).
pub fn is_scene_file(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// File ▸ Import of `.overshorts` files: each opens as its own sequence.
pub(crate) fn import_all(s: &mut Session, paths: &[String]) -> Result<Value> {
    let mut shorts = Vec::new();
    for path in paths {
        shorts.push(import(s, &json!({"path": path}))?);
    }
    Ok(json!({"items": [], "shorts": shorts}))
}

// ---- the file ----

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Scenes {
    format: String,
    version: u32,
    name: String,
    width: f64,
    height: f64,
    fps: f64,
    duration: f64,
    tracks: TrackNames,
    texts: Vec<TextIn>,
    pictures: Vec<PictureIn>,
    sounds: Vec<SoundIn>,
    markers: Vec<MarkerIn>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TrackNames {
    video: Vec<String>,
    audio: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct TextIn {
    track: String,
    text: String,
    start: f64,
    end: f64,
    /// Text height in frame pixels.
    size: f64,
    color: String,
    stroke: String,
    /// Outer stroke width in frame pixels (0 = none).
    stroke_width: f64,
    font: String,
    font_style: String,
    /// Centre of the text block, frame pixels.
    center: [f64; 2],
    /// Wrap at this width (frame pixels); 0 = the frame width less 10 %.
    max_width: f64,
    /// At most this many lines: a longer text gets smaller (down to 55 %).
    max_lines: u32,
    line_spacing: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PictureIn {
    track: String,
    file: String,
    start: f64,
    end: f64,
    /// Where it goes: x, y, width, height in frame pixels (default: the whole frame).
    rect: Option<[f64; 4]>,
    /// `contain` (whole picture inside the rectangle, default), `cover` (fills it) or `width`.
    fit: String,
    /// For a video: where in the file to start, seconds.
    #[serde(rename = "in")]
    source_in: f64,
    label: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SoundIn {
    track: String,
    file: String,
    start: f64,
    /// Sequence time to stop at; 0 or missing = the file's length (or the short's end with `loop`).
    end: f64,
    #[serde(rename = "in")]
    source_in: f64,
    /// Repeat the file until `end` (BGM).
    #[serde(rename = "loop")]
    looped: bool,
    /// Volume in dB when there are no `levels`.
    gain: f64,
    /// Volume over time: `[sequence seconds, dB]`, straight lines between.
    levels: Vec<[f64; 2]>,
    label: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MarkerIn {
    name: String,
    comment: String,
    start: f64,
    end: f64,
}

fn finite(x: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if x.is_finite() { x.clamp(lo, hi) } else { default }
}

fn secs(x: f64) -> Tick {
    Tick::from_seconds_f64(finite(x, 0.0, MAX_SECONDS, 0.0))
}

fn short(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(n).collect()
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`; anything else is `default`.
fn color(s: &str, default: Rgba) -> Rgba {
    let h = s.trim().trim_start_matches('#');
    if !h.is_ascii() {
        return default;
    }
    let hex = |i: usize, n: usize| h.get(i..i + n).and_then(|x| u8::from_str_radix(x, 16).ok());
    match h.len() {
        3 => match (hex(0, 1), hex(1, 1), hex(2, 1)) {
            (Some(r), Some(g), Some(b)) => [r * 17, g * 17, b * 17, 255],
            _ => default,
        },
        6 | 8 => match (hex(0, 2), hex(2, 2), hex(4, 2), if h.len() == 8 { hex(6, 2) } else { Some(255) }) {
            (Some(r), Some(g), Some(b), Some(a)) => [r, g, b, a],
            _ => default,
        },
        _ => default,
    }
}

// ---- text layout ----

/// Width of a character as a share of the text height (bold Korean sans).
fn char_em(c: char) -> f64 {
    match c {
        ' ' => 0.28,
        '\u{1100}'..='\u{11FF}' | '\u{3000}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FF60}' => 0.95,
        '0'..='9' => 0.6,
        'A'..='Z' => 0.68,
        'a'..='z' => 0.56,
        '.' | ',' | '!' | '?' | ':' | ';' | '\'' | '"' | '(' | ')' | '[' | ']' | '·' => 0.32,
        c if c.is_ascii() => 0.5,
        _ => 0.95,
    }
}

fn width_em(s: &str) -> f64 {
    s.chars().map(char_em).sum()
}

/// `text` broken into lines no wider than `max_em` text heights: at spaces, a word wider than a
/// line between its characters. Given line breaks stay.
fn wrap(text: &str, max_em: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let joined = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width_em(&joined) <= max_em || line.is_empty() && width_em(word) <= max_em {
                line = joined;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            if width_em(word) <= max_em {
                line = word.to_string();
                continue;
            }
            // a word wider than the line: break between characters
            for c in word.chars() {
                if !line.is_empty() && width_em(&line) + char_em(c) > max_em {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        if lines.len() > 50 {
            break;
        }
    }
    lines
}

/// The lines and text height (frame pixels) for a text `size` high in `max_w` pixels on at most
/// `max_lines` lines: smaller steps until it fits, never under 55 % of `size`.
fn fit_text(text: &str, size: f64, max_w: f64, max_lines: usize) -> (String, f64) {
    let mut s = size;
    loop {
        let lines = wrap(text, max_w / s.max(1.0));
        if lines.len() <= max_lines || s <= size * 0.55 {
            return (lines.join("\n"), s);
        }
        s *= 0.92;
    }
}

/// The text layer of one text, centred where the file says.
fn text_layer(t: &TextIn, frame: Frame, duration: Tick) -> Option<EffectInstance> {
    let (w, h) = (frame.width as f64, frame.height as f64);
    let text = short(&t.text, 400);
    if text.trim().is_empty() {
        return None;
    }
    let size = finite(t.size, 6.0, h / 3.0, 64.0);
    let max_w = if t.max_width > 0.0 { finite(t.max_width, 40.0, w, w * 0.9) } else { w * 0.9 };
    let max_lines = (t.max_lines as usize).clamp(1, 12);
    let (text, size) = fit_text(&text, size, max_w, if t.max_lines == 0 { 12 } else { max_lines });
    // caption styles measure in pixels at 1080 lines
    let k = 1080.0 / h.max(16.0);
    let stroke_w = finite(t.stroke_width, 0.0, 100.0, 0.0);
    let preset = CaptionPreset {
        name: "OverShorts".into(),
        font: short(&t.font, 80),
        font_style: if t.font_style.trim().is_empty() { "Bold".into() } else { short(&t.font_style, 40) },
        size: (size * k) as f32,
        fill: color(&t.color, [255, 255, 255, 255]),
        stroke: (stroke_w > 0.0).then(|| Stroke { color: color(&t.stroke, [0, 0, 0, 255]), width: (stroke_w * k) as f32 }),
        place: Place::Middle,
        margin: 0.0,
        line_spacing: finite(t.line_spacing, 0.8, 3.0, 1.2) as f32,
        hide_speaker_label: false,
        ..CaptionPreset::default()
    };
    let mut e = caption_layer(&preset, &text, None, duration, frame);
    // the block is centred on the frame; move it to the file's centre
    let cx = finite(t.center[0], 0.0, w, w / 2.0);
    let cy = finite(t.center[1], 0.0, h, h / 2.0);
    let (dx, dy) = (cx - w / 2.0, cy - h / 2.0);
    if let Some(p) = e.params.get_mut("position") {
        let mv = |v: &mut ParamValue| {
            if let ParamValue::Vec2(v) = v {
                v.x += dx;
                v.y += dy;
            }
        };
        mv(&mut p.value);
        p.keyframes.iter_mut().for_each(|k| mv(&mut k.value));
    }
    Some(e)
}

// ---- import ----

/// A file the scene names: inside the scene's folder (or absolute), and there.
fn resolve(base: &Path, file: &str) -> Option<PathBuf> {
    let f = file.trim();
    if f.is_empty() || f.contains('\0') {
        return None;
    }
    let p = Path::new(f);
    Some(if p.is_absolute() { p.to_path_buf() } else { base.join(p) })
}

fn param(e: &mut [EffectInstance], effect: &str, id: &str, value: ParamValue) {
    if let Some(p) = e.iter_mut().find(|x| x.effect == effect).and_then(|x| x.param_mut(id)) {
        p.value = value;
        p.keyframes.clear();
    }
}

/// Sequence (and so track) names, de-duplicated, capped.
fn track_names(given: &[String], used: impl Iterator<Item = String>, fallback: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in given.iter().cloned().chain(used) {
        let n = short(n.trim(), 60);
        let n = if n.is_empty() { fallback.to_string() } else { n };
        if !out.contains(&n) && out.len() < MAX_TRACKS {
            out.push(n);
        }
    }
    out
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").filter(|x| !x.trim().is_empty()).ok_or_else(|| bad(CMD, "need `path` (an .overshorts file)"))?.to_string();
    let bytes = s.services.read_file(&path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(bad(CMD, format!("{path}: too large for a scene file")));
    }
    let sc: Scenes = serde_json::from_slice(&bytes).map_err(|e| bad(CMD, format!("{path}: not an OverShorts scene file ({e})")))?;
    if sc.format != FORMAT {
        return Err(bad(CMD, format!("{path}: not an OverShorts scene file (format `{}`)", short(&sc.format, 40))));
    }
    if sc.version > 1 {
        return Err(bad(CMD, format!("{path}: made by a newer OverShorts (version {}); update OverFilm", sc.version)));
    }
    let base = Path::new(&path).parent().map(Path::to_path_buf).unwrap_or_default();
    let width = (finite(sc.width, 64.0, 8192.0, 1080.0) as u32) & !1;
    let height = (finite(sc.height, 64.0, 8192.0, 1920.0) as u32) & !1;
    let rate = FrameRate::from_f64(finite(sc.fps, 10.0, 120.0, 60.0));
    let rate = if rate.num <= 0 || rate.frame_duration() <= Tick::ZERO { FrameRate::from_f64(60.0) } else { rate };
    let duration = finite(sc.duration, 0.1, MAX_SECONDS, 30.0);
    let name = {
        let n = short(sc.name.trim(), 80);
        if n.is_empty() { "OverShorts".to_string() } else { n }
    };
    let open = bool_p(p, "open").unwrap_or(true);

    let texts: Vec<&TextIn> = sc.texts.iter().take(MAX_TEXTS).collect();
    let pictures: Vec<&PictureIn> = sc.pictures.iter().take(MAX_PICTURES).collect();
    let sounds: Vec<&SoundIn> = sc.sounds.iter().take(MAX_SOUNDS).collect();
    let vnames = track_names(&sc.tracks.video, pictures.iter().map(|x| x.track.clone()).chain(texts.iter().map(|x| x.track.clone())), "OverShorts");
    let anames = track_names(&sc.tracks.audio, sounds.iter().map(|x| x.track.clone()), "OverShorts 소리");
    let vtrack = |n: &str| {
        let n = short(n.trim(), 60);
        vnames.iter().position(|x| *x == n).or_else(|| vnames.iter().position(|x| x == "OverShorts")).unwrap_or(0)
    };
    let atrack = |n: &str| {
        let n = short(n.trim(), 60);
        anames.iter().position(|x| *x == n).unwrap_or(0)
    };

    let mut missing: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    s.edit_group("Open OverShorts Short", |s| {
        let bin = s.edit("New Bin", |pr, _| Ok(pr.add_bin(&format!("OverShorts · {name}"), None)))?;
        // every file once
        let mut items: BTreeMap<String, ItemId> = BTreeMap::new();
        let files = pictures.iter().map(|x| x.file.as_str()).chain(sounds.iter().map(|x| x.file.as_str()));
        for f in files {
            let Some(full) = resolve(&base, f) else { continue };
            let key = full.to_string_lossy().into_owned();
            if items.contains_key(&key) || missing.contains(&key) || failed.contains(&key) {
                continue;
            }
            if !full.is_file() {
                missing.push(key);
                continue;
            }
            match crate::commands::import_streamed(s, &key, Some(bin)) {
                Ok(id) => {
                    items.insert(key, id);
                }
                Err(e) => {
                    log::warn!("OverShorts: {key}: {e}");
                    failed.push(key);
                }
            }
        }
        let item_of = |f: &str| resolve(&base, f).and_then(|full| items.get(&*full.to_string_lossy()).copied());
        let frame = Frame { width, height };
        let seq_label = s.prefs.labels.defaults.sequence;
        let seq = s.edit("OverShorts Sequence", |pr, st| {
            let settings =
                SequenceSettings { width, height, frame_rate: rate, preset: format!("{width}x{height} {}", rate.label()), ..SequenceSettings::default() };
            settings.validate().map_err(|e| bad(CMD, e))?;
            let id = pr.new_sequence(&name, settings, 0, 0, Some(bin));
            if let Some(it) = pr.item_mut(id) {
                it.label = seq_label;
            }
            let mut vtracks: Vec<Track> = vnames.iter().map(|n| Track::new(TrackId(pr.alloc_id()), TrackKind::Video, n.clone())).collect();
            let mut atracks: Vec<Track> = anames.iter().map(|n| Track::new(TrackId(pr.alloc_id()), TrackKind::Audio, n.clone())).collect();
            let end_all = secs(duration);
            for x in &pictures {
                let Some(item) = item_of(&x.file) else { continue };
                if let Some(ti) = picture_clip(pr, item, x, frame, rate, end_all) {
                    put(&mut vtracks, vtrack(&x.track), ti);
                }
            }
            let src = crate::graphics::graphic_source(pr, width, height, rate);
            for x in &texts {
                let start = rate.snap(secs(x.start));
                let end = rate.snap(secs(if x.end > 0.0 { x.end } else { duration })).min(rate.snap(end_all).max(rate.frame_duration()));
                if end <= start {
                    continue;
                }
                let dur = end - start;
                let Some(layer) = text_layer(x, frame, dur) else { continue };
                let Some(mut ti) = pr.make_track_item(src, TrackKind::Video, start, TimeRange::new(Tick::ZERO, dur), rate) else { continue };
                ti.name = short(x.text.lines().next().unwrap_or("글자"), 40);
                let mut layer = layer;
                filmcraft_project::resolve_auto_points(&mut layer, (width, height), (width, height));
                ti.effects.push(layer);
                ensure_layer_uids(&mut ti.effects);
                put(&mut vtracks, vtrack(&x.track), ti);
            }
            for x in &sounds {
                let Some(item) = item_of(&x.file) else { continue };
                for ti in sound_clips(pr, item, x, rate, end_all) {
                    put(&mut atracks, atrack(&x.track), ti);
                }
            }
            let mut markers = Vec::new();
            for m in sc.markers.iter().take(MAX_MARKERS) {
                let start = rate.snap(secs(m.start));
                let end = rate.snap(secs(m.end)).max(start);
                markers.push(Marker {
                    id: MarkerId(pr.alloc_id()),
                    start,
                    duration: end - start,
                    name: short(&m.name, 80),
                    comment: short(&m.comment, 400),
                    kind: MarkerKind::Comment,
                    color: Label::Cerulean,
                    url: String::new(),
                    author: String::new(),
                    custom_color: None,
                });
            }
            markers.sort_by_key(|m| m.start);
            let q = pr.sequence_mut(id).ok_or(EngineError::NoSequence)?;
            for t in vtracks.iter_mut().chain(atracks.iter_mut()) {
                t.sort();
            }
            q.video_tracks = vtracks;
            q.audio_tracks = atracks;
            q.markers = markers;
            q.check().map_err(EngineError::Other)?;
            if open {
                st.active_sequence = Some(id);
                if !st.open_sequences.contains(&id) {
                    st.open_sequences.push(id);
                }
            }
            st.selection.clear();
            Ok(id)
        })?;
        if open {
            s.events.push(crate::Event::OpenSequence(seq));
        }
        let clips: usize = s.project.sequence(seq).map(|q| q.all_tracks().map(|t| t.items.len()).sum()).unwrap_or(0);
        let mut msg = format!("OverShorts: \"{name}\" opened as a sequence ({clips} clips)");
        if !missing.is_empty() || !failed.is_empty() {
            msg.push_str(&format!(", {} file(s) not found or not opened", missing.len() + failed.len()));
        }
        s.toast(msg);
        Ok(json!({"sequence": seq.0, "bin": bin.0, "name": name, "clips": clips, "missing": missing, "failed": failed}))
    })
}

/// Add `ti` to track `i`, trimming its head where it would overlap the clip before (a track
/// holds one clip at a time; the file's order wins).
fn put(tracks: &mut [Track], i: usize, mut ti: TrackItem) {
    let Some(t) = tracks.get_mut(i) else { return };
    let last_end = t.items.iter().filter(|c| c.start <= ti.start).map(TrackItem::end).max().unwrap_or(Tick::MIN);
    if ti.start < last_end {
        let cut = last_end - ti.start;
        if ti.duration <= cut {
            return;
        }
        ti.start = last_end;
        ti.duration -= cut;
        ti.source_in += cut;
    }
    // and nothing after it may start inside it
    if let Some(next) = t.items.iter().filter(|c| c.start > ti.start).map(|c| c.start).min()
        && ti.end() > next
    {
        let cut = ti.end() - next;
        ti.duration -= cut;
        if ti.duration <= Tick::ZERO {
            return;
        }
    }
    t.items.push(ti);
}

/// The source picture size of an item (stills and videos).
fn picture_size(pr: &Project, item: ItemId) -> Option<(u32, u32)> {
    let (_, m, _) = pr.resolve_media(item)?;
    let v = m.info.video.as_ref()?;
    (v.width > 0 && v.height > 0).then_some((v.width, v.height))
}

fn picture_clip(pr: &mut Project, item: ItemId, x: &PictureIn, frame: Frame, rate: FrameRate, end_all: Tick) -> Option<TrackItem> {
    let (iw, ih) = picture_size(pr, item)?;
    let start = rate.snap(secs(x.start));
    let end = rate.snap(secs(x.end)).min(rate.snap(end_all).max(rate.frame_duration()));
    if end <= start {
        return None;
    }
    let is_still = pr.resolve_media(item).is_some_and(|(_, m, _)| m.info.kind == filmcraft_media::MediaKind::Still);
    let mut source_in = if is_still { Tick::ZERO } else { rate.snap(secs(x.source_in)) };
    let mut dur = end - start;
    if !is_still {
        // a video plays as long as it has frames
        let len = pr.item(item).map(|i| i.duration()).unwrap_or_default();
        if source_in >= len {
            source_in = Tick::ZERO;
        }
        dur = dur.min(len - source_in);
        if dur <= Tick::ZERO {
            return None;
        }
    }
    let mut ti = pr.make_track_item(item, TrackKind::Video, start, TimeRange::new(source_in, dur), rate)?;
    if !x.label.trim().is_empty() {
        ti.name = short(&x.label, 60);
    }
    let (w, h) = (frame.width as f64, frame.height as f64);
    let [rx, ry, rw, rh] = x.rect.unwrap_or([0.0, 0.0, w, h]);
    let rx = finite(rx, -w, 2.0 * w, 0.0);
    let ry = finite(ry, -h, 2.0 * h, 0.0);
    let rw = finite(rw, 1.0, 4.0 * w, w);
    let rh = finite(rh, 1.0, 4.0 * h, h);
    let (sx, sy) = (rw / iw as f64, rh / ih as f64);
    let k = match x.fit.as_str() {
        "cover" => sx.max(sy),
        "width" => sx,
        _ => sx.min(sy),
    };
    for e in &mut ti.effects {
        filmcraft_project::resolve_auto_points(e, (frame.width, frame.height), (iw, ih));
    }
    param(&mut ti.effects, "motion", "position", ParamValue::Vec2(Vec2::new(rx + rw / 2.0, ry + rh / 2.0)));
    param(&mut ti.effects, "motion", "scale", ParamValue::Float((k * 100.0).clamp(0.1, 10_000.0)));
    Some(ti)
}

fn sound_clips(pr: &mut Project, item: ItemId, x: &SoundIn, rate: FrameRate, end_all: Tick) -> Vec<TrackItem> {
    let len = pr.item(item).map(|i| i.duration()).unwrap_or_default();
    let mut out = Vec::new();
    if len <= Tick::ZERO {
        return out;
    }
    let start = rate.snap(secs(x.start));
    let stop = if x.end > 0.0 {
        rate.snap(secs(x.end))
    } else if x.looped {
        rate.snap(end_all)
    } else {
        start + len
    };
    let source_in = {
        let t = rate.snap(secs(x.source_in));
        if t >= len { Tick::ZERO } else { t }
    };
    let mut at = start;
    let mut from = source_in;
    // a loop repeats the file; capped so a tiny file can't make thousands of clips
    for _ in 0..if x.looped { 200 } else { 1 } {
        if at >= stop {
            break;
        }
        let dur = (len - from).min(stop - at);
        if dur < rate.frame_duration() {
            break;
        }
        let Some(mut ti) = pr.make_track_item(item, TrackKind::Audio, at, TimeRange::new(from, dur), rate) else { break };
        if !x.label.trim().is_empty() {
            ti.name = short(&x.label, 60);
        }
        volume(&mut ti, x, at);
        at += ti.duration;
        from = Tick::ZERO;
        out.push(ti);
    }
    out
}

/// The sound's volume on one clip starting at sequence time `at`: `levels` as keyframes (clip
/// time; the points just outside the clip keep the line right at its edges), else `gain`.
fn volume(ti: &mut TrackItem, x: &SoundIn, at: Tick) {
    let db = |v: f64| finite(v, -96.0, 15.0, 0.0);
    let Some(level) = ti.effects.iter_mut().find(|e| e.effect == "volume").and_then(|e| e.param_mut("level")) else { return };
    let pts: Vec<(Tick, f64)> = x.levels.iter().take(MAX_LEVELS).map(|[t, v]| (secs(*t), db(*v))).collect();
    if pts.is_empty() {
        level.value = ParamValue::Float(db(x.gain));
        level.keyframes.clear();
        return;
    }
    let (s, e) = (at, at + ti.duration);
    let value_at = |t: Tick| -> f64 {
        let i = pts.partition_point(|(pt, _)| *pt <= t);
        match (i.checked_sub(1).and_then(|j| pts.get(j)), pts.get(i)) {
            (Some((t0, v0)), Some((t1, v1))) if t1 > t0 => v0 + (v1 - v0) * ((t - *t0).0 as f64 / (*t1 - *t0).0 as f64),
            (Some((_, v)), _) | (None, Some((_, v))) => *v,
            (None, None) => 0.0,
        }
    };
    let mut keys: Vec<Keyframe> = vec![Keyframe::new(Tick::ZERO, ParamValue::Float(value_at(s)))];
    for (t, v) in pts.iter().filter(|(t, _)| *t > s && *t < e) {
        keys.push(Keyframe::new(*t - s, ParamValue::Float(*v)));
    }
    keys.push(Keyframe::new(ti.duration, ParamValue::Float(value_at(e))));
    keys.dedup_by(|b, a| a.time == b.time);
    level.value = ParamValue::Float(value_at(s));
    level.keyframes = keys;
}

#[cfg(test)]
#[path = "overfilm_shorts_tests.rs"]
mod tests;

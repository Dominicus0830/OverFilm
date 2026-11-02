//! Caption styles from Premiere Pro files the user already has (milestone M4.3).
//!
//! The user's own Premiere text styles become OverFilm [`CaptionPreset`]s:
//!
//! - **`.prtextstyle`** (Premiere's exported text / caption style): a small Premiere project in XML
//!   whose `Source Text` parameter holds the style as base64. Recent Premiere versions write a
//!   FlatBuffers table there (after an 8-byte length and the marker `44 33 22 11`); Premiere 12
//!   and older wrote UTF-16 JSON (`mTextParam`). Both are read.
//! - **`.prproj`** (gzip XML) and **Final Cut Pro XML** exported from Premiere: every text
//!   graphic's `Source Text` is read the same way, one style per different look.
//! - **`.mogrt`** (ZIP): a template made in Premiere carries the same `Source Text`; one made in
//!   After Effects only has `definition.json`, whose editable controls give the font, its size
//!   and the colours whose names say what they colour (fill, outline, background). The motion
//!   comes from the After Effects project inside ([`crate::aep`]): the text layer's scale,
//!   position, opacity and rotation keyframes; a per-character animator becomes the typewriter.
//!   When none can be read, it is guessed from the template's name (pop, slide, bounce,
//!   typewriter, fade).
//!
//! What is read from a Premiere text style: font (PostScript name), size, fill colour, outline
//! (colour, width, on/off), background box (colour, opacity, size, corner radius, on/off) and
//! the drop shadow (on/off). The layout of the FlatBuffers table was worked out from text styles
//! people share publicly; fields this module does not know are skipped. Nothing here is copied
//! from Adobe software, and no Adobe file is shipped: the user's own files are read when they
//! choose them.
//!
//! Every number in these files is treated as hostile: all reads are bounds-checked, sizes are
//! capped and recursion is bounded.

use serde_json::Value;

use crate::styles::{BoxStyle, CaptionPreset, Motion, Rgba, Stroke};

/// What was found in a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Imported {
    pub presets: Vec<CaptionPreset>,
    /// PostScript font names as written in the file (`Roboto-Bold`), one per preset (empty when
    /// the file names no font). The caller maps them to installed families.
    pub fonts: Vec<String>,
    /// Plain notes for the user (Korean): what could not be taken over.
    pub notes: Vec<String>,
}

/// The kinds of file [`import`] reads.
pub const EXTENSIONS: &[&str] = &["prtextstyle", "mogrt", "prproj", "xml"];

/// Largest file read, and the most any one ZIP entry or gzip stream may inflate to.
pub const MAX_FILE: usize = 512 << 20;
const MAX_INFLATE: usize = 64 << 20;
/// Most styles taken from one file.
const MAX_STYLES: usize = 200;
/// Most base64 blobs looked at in one document.
const MAX_BLOBS: usize = 5000;

/// Read caption styles from a Premiere file's bytes. `name` is the file name without its
/// extension (the preset name when the file has none of its own).
pub fn import(bytes: &[u8], name: &str) -> Result<Imported, String> {
    let name = clean_name(name);
    let mut out = Imported::default();
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        mogrt(bytes, &name, &mut out, 0)?;
    } else if bytes.starts_with(&[0x1f, 0x8b]) {
        let xml = gunzip(bytes)?;
        document(&String::from_utf8_lossy(&xml), &name, &mut out);
    } else {
        document(&String::from_utf8_lossy(bytes), &name, &mut out);
    }
    if out.presets.is_empty() {
        return Err(
            "이 파일에서 글자 스타일을 찾지 못했어요. Premiere의 .prtextstyle(텍스트 스타일 내보내기), .mogrt, .prproj, Premiere에서 내보낸 XML을 열어 주세요."
                .into(),
        );
    }
    Ok(out)
}

fn clean_name(n: &str) -> String {
    let n: String = n.chars().filter(|c| !c.is_control()).take(60).collect();
    let n = n.trim().to_string();
    if n.is_empty() { "Premiere 스타일".into() } else { n }
}

// ---------------------------------------------------------------------------------------------
// XML documents: .prtextstyle, .prproj (after gunzip), FCP XML, XML inside a .mogrt

/// Every text style in an XML document, named after the document's `StyleProjectItem`s when it
/// has them (a `.prtextstyle`), else `name`, `name 2`, ….
fn document(xml: &str, name: &str, out: &mut Imported) {
    let names = style_item_names(xml);
    let mut found: Vec<(PrStyle, String)> = Vec::new();
    for blob in base64_blobs(xml).into_iter().take(MAX_BLOBS) {
        let Some(st) = text_blob(&blob) else { continue };
        if found.iter().any(|(f, _)| f.same_look(&st)) {
            continue;
        }
        let n = match names.get(found.len()) {
            Some(n) => clean_name(n),
            None if found.is_empty() => name.to_string(),
            None => format!("{name} {}", found.len() + 1),
        };
        found.push((st, n));
        if found.len() >= MAX_STYLES {
            break;
        }
    }
    for (st, n) in found {
        let font = st.font.clone();
        let preset = st.preset(&n);
        out.fonts.push(font);
        out.presets.push(preset);
    }
}

/// `<StyleProjectItem …><ProjectItem><Name>…</Name>` in document order.
fn style_item_names(xml: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<StyleProjectItem") {
        rest = rest.get(i + 17..).unwrap_or_default();
        let end = rest.find("</StyleProjectItem>").unwrap_or(rest.len());
        let item = rest.get(..end).unwrap_or_default();
        if let Some(a) = item.find("<Name>")
            && let Some(body) = item.get(a + 6..)
            && let Some(b) = body.find("</Name>")
        {
            v.push(xml_unescape(body.get(..b).unwrap_or_default()));
        }
        if v.len() >= MAX_STYLES {
            break;
        }
    }
    v
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The text between tags that is long base64 (whitespace allowed), decoded.
fn base64_blobs(xml: &str) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    for seg in xml.split('<') {
        let Some(gt) = seg.find('>') else { continue };
        let text = seg.get(gt + 1..).unwrap_or_default().trim();
        if text.len() < 24 || !text.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=') || c.is_ascii_whitespace()) {
            continue;
        }
        if let Some(d) = base64(text) {
            v.push(d);
            if v.len() >= MAX_BLOBS {
                break;
            }
        }
    }
    v
}

/// Standard base64 (whitespace ignored, padding optional).
fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// The text style itself

/// What a Premiere text style holds that OverFilm uses.
#[derive(Clone, Debug, Default, PartialEq)]
struct PrStyle {
    font: String,
    size: Option<f32>,
    fill: Option<Rgba>,
    stroke: Option<(Rgba, f32)>,
    /// Colour with the opacity in alpha, padding, corner radius.
    background: Option<(Rgba, f32, f32)>,
    shadow: bool,
}

impl PrStyle {
    fn same_look(&self, o: &PrStyle) -> bool {
        self == o
    }

    fn preset(&self, name: &str) -> CaptionPreset {
        let base = CaptionPreset::default();
        let (_, style) = split_postscript(&self.font);
        CaptionPreset {
            name: name.to_string(),
            // the family is mapped to an installed one by the caller
            font: String::new(),
            font_style: if self.font.is_empty() { base.font_style.clone() } else { style },
            size: self.size.filter(|s| s.is_finite()).map_or(base.size, |s| s.clamp(8.0, 400.0)),
            fill: self.fill.unwrap_or([255, 255, 255, 255]),
            stroke: self.stroke.filter(|(_, w)| w.is_finite() && *w > 0.0).map(|(color, width)| Stroke { color, width: width.clamp(0.0, 100.0) }),
            stroke2: None,
            boxed: self.background.map(|(color, padding, radius)| BoxStyle {
                color,
                padding: if padding.is_finite() { padding.clamp(0.0, 200.0) } else { 0.0 },
                radius: if radius.is_finite() { radius.clamp(0.0, 200.0) } else { 0.0 },
            }),
            shadow: self.shadow,
            ..base
        }
    }
}

/// `Roboto-BoldItalic` → (`Roboto`, `BoldItalic`); `KomikaAxis` → (`KomikaAxis`, `Regular`).
pub fn split_postscript(ps: &str) -> (String, String) {
    let ps = ps.trim();
    match ps.rsplit_once('-') {
        Some((f, s)) if !f.is_empty() && !s.is_empty() => (f.to_string(), s.to_string()),
        _ => (ps.to_string(), "Regular".to_string()),
    }
}

/// A decoded `Source Text` value: FlatBuffers (current Premiere) or UTF-16 JSON (Premiere 12).
fn text_blob(d: &[u8]) -> Option<PrStyle> {
    if d.get(8..12) == Some(&[0x44, 0x33, 0x22, 0x11][..]) {
        return flat_style(d.get(12..)?);
    }
    legacy_json(d)
}

/// Bounds-checked FlatBuffers reads.
struct Fb<'a>(&'a [u8]);

#[derive(Clone, Copy)]
struct Table {
    pos: usize,
    vt: usize,
    vt_len: usize,
}

impl<'a> Fb<'a> {
    fn u8(&self, p: usize) -> Option<u8> {
        self.0.get(p).copied()
    }
    fn u16(&self, p: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.0.get(p..p.checked_add(2)?)?.try_into().ok()?))
    }
    fn u32(&self, p: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.0.get(p..p.checked_add(4)?)?.try_into().ok()?))
    }
    fn f32(&self, p: usize) -> Option<f32> {
        Some(f32::from_le_bytes(self.0.get(p..p.checked_add(4)?)?.try_into().ok()?))
    }
    /// The position an offset stored at `p` points to.
    fn follow(&self, p: usize) -> Option<usize> {
        let t = p.checked_add(self.u32(p)? as usize)?;
        (t < self.0.len()).then_some(t)
    }
    fn table(&self, pos: usize) -> Option<Table> {
        let soff = i32::from_le_bytes(self.0.get(pos..pos.checked_add(4)?)?.try_into().ok()?);
        let vt = usize::try_from(i64::try_from(pos).ok()?.checked_sub(i64::from(soff))?).ok()?;
        let vt_len = self.u16(vt)? as usize;
        if vt_len < 4 || !vt_len.is_multiple_of(2) || vt.checked_add(vt_len)? > self.0.len() {
            return None;
        }
        Some(Table { pos, vt, vt_len })
    }
    fn field(&self, t: Table, i: usize) -> Option<usize> {
        let o = 4 + 2 * i;
        if o + 2 > t.vt_len {
            return None;
        }
        let off = self.u16(t.vt + o)? as usize;
        if off == 0 { None } else { t.pos.checked_add(off) }
    }
    fn get_f32(&self, t: Table, i: usize) -> Option<f32> {
        self.f32(self.field(t, i)?).filter(|v| v.is_finite())
    }
    fn get_bool(&self, t: Table, i: usize) -> bool {
        self.field(t, i).and_then(|p| self.u8(p)).is_some_and(|v| v != 0)
    }
    fn get_table(&self, t: Table, i: usize) -> Option<Table> {
        self.table(self.follow(self.field(t, i)?)?)
    }
    /// The first element of a vector field (tables or strings): the position of its offset.
    fn first_of(&self, t: Table, i: usize) -> Option<usize> {
        let v = self.follow(self.field(t, i)?)?;
        if self.u32(v)? == 0 {
            return None;
        }
        Some(v + 4)
    }
    fn string_at(&self, p: usize) -> Option<String> {
        let s = self.follow(p)?;
        let n = (self.u32(s)? as usize).min(512);
        let from = s.checked_add(4)?;
        let b = self.0.get(from..from.checked_add(n)?)?;
        Some(String::from_utf8_lossy(b).into_owned())
    }
    /// A colour table: red, green, blue as bytes in fields 0, 1 and 2, each 255 when left out.
    fn colour(&self, t: Table) -> Rgba {
        let c = |i| self.field(t, i).and_then(|p| self.u8(p)).unwrap_or(255);
        [c(0), c(1), c(2), 255]
    }
}

/// The text-style table, as laid out by current Premiere versions:
///
/// - root table, field 0 → the text document table `T`;
/// - `T` 0: text runs (the first run's field 1 is its style: 1 size, 2 fill colour, 4 outline
///   colour, 5 outline on, 6 outline width); `T` 1: font PostScript names;
/// - `T` 11: drop shadow on (10 its colour, 12 opacity);
/// - `T` 17: background colour, 18 background on, 19 opacity (%), 20 size, 34 corner radius.
fn flat_style(b: &[u8]) -> Option<PrStyle> {
    let fb = Fb(b);
    let root = fb.table(fb.u32(0)? as usize)?;
    let t = fb.get_table(root, 0)?;
    let run = fb.first_of(t, 0).and_then(|p| fb.follow(p)).and_then(|p| fb.table(p));
    let style = run.and_then(|r| fb.get_table(r, 1));
    let font = fb.first_of(t, 1).and_then(|p| fb.string_at(p)).unwrap_or_default();
    let mut st = PrStyle { font: font.chars().filter(|c| !c.is_control()).take(80).collect(), ..PrStyle::default() };
    if let Some(s) = style {
        st.size = fb.get_f32(s, 1);
        st.fill = fb.get_table(s, 2).map(|c| fb.colour(c));
        if fb.get_bool(s, 5) {
            let colour = fb.get_table(s, 4).map_or([0, 0, 0, 255], |c| fb.colour(c));
            st.stroke = Some((colour, fb.get_f32(s, 6).unwrap_or(4.0)));
        }
    }
    if fb.get_bool(t, 18) {
        let mut colour = fb.get_table(t, 17).map_or([0, 0, 0, 255], |c| fb.colour(c));
        let opacity = fb.get_f32(t, 19).unwrap_or(100.0).clamp(0.0, 100.0);
        colour[3] = (opacity * 2.55).round() as u8;
        st.background = Some((colour, fb.get_f32(t, 20).unwrap_or(0.0), fb.get_f32(t, 34).unwrap_or(0.0)));
    }
    st.shadow = fb.get_bool(t, 11);
    // a table without a size and a font is something else that happens to be FlatBuffers
    (st.size.is_some() || !st.font.is_empty()).then_some(st)
}

/// Premiere 12 and older: UTF-16LE JSON after an 8-byte header.
fn legacy_json(d: &[u8]) -> Option<PrStyle> {
    let body = d.get(8..)?;
    let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let text = String::from_utf16_lossy(&units);
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let v: Value = serde_json::from_str(text.get(start..=end)?).ok()?;
    let tp = v.get("mTextParam")?;
    let ss = tp.get("mStyleSheet")?;
    let first = |k: &str| ss.get(k)?.get("mParamValues")?.get(0)?.get(1).cloned();
    let colour = |k: &str| -> Option<Rgba> {
        let n = first(k)?.as_u64()?;
        Some([((n >> 16) & 255) as u8, ((n >> 8) & 255) as u8, (n & 255) as u8, 255])
    };
    let mut st = PrStyle {
        font: first("mFontName").and_then(|f| f.as_str().map(str::to_string)).unwrap_or_default(),
        size: first("mFontSize").and_then(|f| f.as_f64()).map(|f| f as f32),
        fill: colour("mFillColor"),
        ..PrStyle::default()
    };
    if first("mStrokeVisible").and_then(|b| b.as_bool()).unwrap_or(false) {
        st.stroke = Some((colour("mStrokeColor").unwrap_or([0, 0, 0, 255]), first("mStrokeWidth").and_then(|f| f.as_f64()).unwrap_or(4.0) as f32));
    }
    st.shadow = tp.get("mShadowVisible").and_then(Value::as_bool).unwrap_or(false);
    (st.size.is_some() || !st.font.is_empty()).then_some(st)
}

// ---------------------------------------------------------------------------------------------
// .mogrt (ZIP)

fn mogrt(bytes: &[u8], name: &str, out: &mut Imported, depth: u32) -> Result<(), String> {
    let entries = unzip(bytes, depth == 0)?;
    let mut definition: Option<Value> = None;
    let mut aep: Option<&[u8]> = None;
    let mut inner_aep: Vec<(String, Vec<u8>)> = Vec::new();
    for (path, data) in &entries {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".aegraphic") {
            // the After Effects project, for its motion
            inner_aep = unzip(data, true).unwrap_or_default();
        } else if lower.ends_with(".aep") {
            aep = Some(data);
        } else if lower.ends_with("definition.json") {
            definition = serde_json::from_slice(data).ok();
        } else if data.starts_with(b"PK\x03\x04") && depth < 2 {
            // a Premiere-made template keeps its graphic in a nested package
            let _ = mogrt(data, name, out, depth + 1);
        } else if data.starts_with(&[0x1f, 0x8b]) {
            if let Ok(x) = gunzip(data) {
                document(&String::from_utf8_lossy(&x), name, out);
            }
        } else if lower.ends_with(".xml") || lower.ends_with(".prproj") || lower.ends_with(".prgraphic") || data.starts_with(b"<?xml") {
            document(&String::from_utf8_lossy(data), name, out);
        }
    }
    let aep = aep.or_else(|| inner_aep.iter().find(|(p, _)| p.to_ascii_lowercase().ends_with(".aep")).map(|(_, d)| d.as_slice()));
    if let Some(def) = &definition {
        let title = capsule_name(def).map(|t| clean_name(&t)).unwrap_or_else(|| name.to_string());
        if out.presets.is_empty() {
            if let Some((mut preset, font)) = from_definition(def, &title) {
                let moved = aep.and_then(|a| crate::aep::motion(a, frame_short(def)));
                apply_ae_motion(&mut preset, moved, &mut out.notes);
                out.presets.push(preset);
                out.fonts.push(font);
            }
        } else {
            // a Premiere-made template: keep its look, name it and guess its motion
            let motion = guess_motion(def);
            for p in &mut out.presets {
                if p.name == name {
                    p.name = title.clone();
                }
                if motion != Motion::None {
                    p.motion_in = motion;
                    p.motion_out = Motion::Fade;
                }
            }
        }
    }
    Ok(())
}

fn capsule_name(def: &Value) -> Option<String> {
    if let Some(s) = def.get("capsuleNameLocalized").and_then(|c| c.get("strDB")).and_then(|d| d.get(0)).and_then(|d| d.get("str")).and_then(Value::as_str)
        && !s.trim().is_empty()
    {
        return Some(s.to_string());
    }
    def.get("capsuleName").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string)
}

fn ui_name(c: &Value) -> String {
    c.get("uiName").and_then(|u| u.get("strDB")).and_then(|d| d.get(0)).and_then(|d| d.get("str")).and_then(Value::as_str).unwrap_or_default().to_lowercase()
}

fn any_of(s: &str, words: &[&str]) -> bool {
    words.iter().any(|w| s.contains(w))
}

const FILL_WORDS: &[&str] = &["fill", "text color", "text colour", "font color", "color text", "글자", "텍스트", "채우기", "문자"];
const STROKE_WORDS: &[&str] = &["stroke", "outline", "border", "테두리", "외곽", "선 색"];
const BOX_WORDS: &[&str] = &["background", "bg ", "bg_", "box", "배경", "박스", "상자"];
const SHADOW_WORDS: &[&str] = &["shadow", "그림자"];

/// An After Effects template's editable controls: font and size from the first text control
/// (sizes are scaled from the template's frame to 1080 lines), colours by their names.
fn from_definition(def: &Value, title: &str) -> Option<(CaptionPreset, String)> {
    let controls = def.get("clientControls")?.as_array()?;
    let short = frame_short(def);
    let mut preset = CaptionPreset { name: title.to_string(), ..CaptionPreset::default() };
    let mut font = String::new();
    let mut seen_text = false;
    let (mut fill, mut stroke, mut boxed, mut box_opacity) = (None, None, None, None);
    let (mut stroke_width, mut gradient) = (None, None);
    for c in controls.iter().take(500) {
        let kind = c.get("type").and_then(Value::as_u64).unwrap_or(0);
        let n = ui_name(c);
        match kind {
            6 if !seen_text => {
                seen_text = true;
                let fe = c.get("fonteditinfo");
                font = fe.and_then(|f| f.get("fontEditValue")).and_then(first_str).unwrap_or_default();
                if let Some(sz) = fe.and_then(|f| f.get("fontSizeEditValue")).and_then(first_f64).filter(|s| s.is_finite() && *s > 0.0) {
                    preset.size = ((sz * 1080.0 / short) as f32).clamp(8.0, 400.0);
                }
            }
            4 => {
                let Some(col) = c.get("value").and_then(float_colour) else { continue };
                if any_of(&n, SHADOW_WORDS) {
                    preset.shadow = true;
                } else if any_of(&n, STROKE_WORDS) {
                    stroke.get_or_insert(col);
                } else if any_of(&n, BOX_WORDS) {
                    boxed.get_or_insert(col);
                } else if any_of(&n, FILL_WORDS) || n.starts_with("text") || n.starts_with("main text") {
                    fill.get_or_insert(col);
                } else if n.contains("gradient") && n.contains("start") {
                    // a gradient text fill: its first colour
                    gradient.get_or_insert(col);
                }
            }
            2 => {
                let Some(v) = c.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()) else { continue };
                if any_of(&n, STROKE_WORDS) && any_of(&n, &["width", "size", "두께", "굵기"]) {
                    stroke_width = Some((v * 1080.0 / short) as f32);
                } else if any_of(&n, BOX_WORDS) && any_of(&n, &["opacity", "불투명", "투명"]) {
                    box_opacity = Some(v.clamp(0.0, 100.0) as f32);
                }
            }
            _ => {}
        }
    }
    if !seen_text {
        return None;
    }
    if let Some(f) = fill.or(gradient) {
        preset.fill = f;
    }
    // an outline only when the template has one
    preset.stroke = stroke.map(|s| Stroke { color: s, width: stroke_width.unwrap_or(6.0).clamp(0.0, 100.0) });
    if let Some(mut b) = boxed {
        if let Some(o) = box_opacity {
            b[3] = (o * 2.55).round() as u8;
        }
        preset.boxed = Some(BoxStyle { color: b, padding: 14.0, radius: 6.0 });
    }
    let (_, style) = split_postscript(&font);
    if !font.is_empty() {
        preset.font_style = style;
    }
    preset.motion_in = guess_motion(def);
    if preset.motion_in != Motion::None {
        preset.motion_out = Motion::Fade;
    }
    Some((preset, font))
}

/// The shorter side of the template's frame (1080 when it doesn't say).
fn frame_short(def: &Value) -> f64 {
    let frame = def
        .get("sourceInfoLocalized")
        .and_then(Value::as_object)
        .and_then(|m| m.values().next())
        .or_else(|| def.get("sourceInfo"))
        .and_then(|s| s.get("framesize"))
        .and_then(|f| f.get("size"));
    frame.and_then(|f| Some(f.get("x")?.as_f64()?.min(f.get("y")?.as_f64()?))).filter(|s| s.is_finite() && *s >= 16.0).unwrap_or(1080.0)
}

/// Put an After Effects template's own motion on its preset (else keep the motion guessed from
/// its name), and tell the user what came over.
fn apply_ae_motion(preset: &mut CaptionPreset, moved: Option<crate::aep::AeMotion>, notes: &mut Vec<String>) {
    let mut took = Vec::new();
    if let Some(m) = moved {
        if m.per_character {
            preset.motion_in = Motion::Typewriter;
            took.push("글자가 하나씩 나오는 움직임은 한 글자씩(타자기)으로");
        }
        if let Some(c) = m.custom {
            if !m.per_character {
                preset.motion_in = Motion::None;
            }
            preset.motion_out = Motion::None;
            preset.custom_motion = Some(c);
            took.push("크기·위치·투명도·회전 키프레임은 그대로");
        }
    }
    if took.is_empty() {
        notes.push(
            "After Effects로 만든 템플릿이라 글꼴·크기·이름으로 알 수 있는 색만 가져오고, 움직임은 이름을 보고 비슷한 OverFilm 모션으로 골랐어요.".into(),
        );
    } else {
        notes.push(format!("After Effects 템플릿의 움직임을 가져왔어요: {}. 효과(빛, 그라데이션, 글로우 등)와 표현식은 가져오지 못해요.", took.join(", ")));
    }
}

fn first_str(v: &Value) -> Option<String> {
    v.as_str().or_else(|| v.get(0)?.as_str()).filter(|s| !s.trim().is_empty()).map(|s| s.chars().take(80).collect())
}

fn first_f64(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.get(0)?.as_f64())
}

/// `[r, g, b, a]` in 0..1 (or 0..255 when any channel is above 1).
fn float_colour(v: &Value) -> Option<Rgba> {
    let a = v.as_array()?;
    let ch: Vec<f64> = a.iter().take(4).filter_map(Value::as_f64).filter(|x| x.is_finite()).collect();
    if ch.len() < 3 {
        return None;
    }
    let scale = if ch.iter().take(3).any(|x| *x > 1.0) { 1.0 } else { 255.0 };
    let byte = |x: f64| x.round().clamp(0.0, 255.0) as u8;
    let c = |i: usize| byte(ch.get(i).copied().unwrap_or(0.0) * scale);
    let alpha = ch.get(3).map_or(255, |a| byte(if *a > 1.0 { *a } else { a * 255.0 }));
    Some([c(0), c(1), c(2), alpha])
}

/// A motion from the template's and its controls' names.
fn guess_motion(def: &Value) -> Motion {
    let mut words = capsule_name(def).unwrap_or_default().to_lowercase().replace(['_', '-'], " ");
    if let Some(cs) = def.get("clientControls").and_then(Value::as_array) {
        for c in cs.iter().take(200) {
            words.push(' ');
            words.push_str(&ui_name(c).replace(['_', '-'], " "));
        }
    }
    if any_of(&words, &["typewriter", "typing", "type on", "타자", "한 글자"]) {
        Motion::Typewriter
    } else if any_of(&words, &["bounce", "통통", "바운스"]) {
        Motion::Bounce
    } else if any_of(&words, &["pop", "zoom", "scale in", "팝", "줌"]) {
        Motion::Pop
    } else if any_of(&words, &["slide", "sliding", "rise", "bottom to up", "bottomtoup", "슬라이드", "올라"]) {
        Motion::SlideUp
    } else if any_of(&words, &["fade", "페이드"]) {
        Motion::Fade
    } else {
        Motion::None
    }
}

// ---------------------------------------------------------------------------------------------
// ZIP and gzip, just enough to read a template

/// The entries of a ZIP archive (stored or deflated), at most 256, each inflated to at most
/// 64 MB. Media entries (pictures, video, sound, fonts) and After Effects projects are skipped.
fn unzip(b: &[u8], keep_ae: bool) -> Result<Vec<(String, Vec<u8>)>, String> {
    let fb = Fb(b);
    // end of central directory: the last "PK\5\6" in the final 64 KB + 22 bytes
    let from = b.len().saturating_sub(22 + 65535);
    let eocd = (from..b.len().saturating_sub(21)).rev().find(|&i| sl(b, i, 4) == Some(&b"PK\x05\x06"[..])).ok_or("ZIP 파일이 깨졌어요 (끝 표시 없음)")?;
    let count = fb.u16(eocd + 10).unwrap_or(0) as usize;
    let mut p = fb.u32(eocd + 16).ok_or("ZIP 파일이 깨졌어요")? as usize;
    let mut out = Vec::new();
    for _ in 0..count.min(4096) {
        if sl(b, p, 4) != Some(&b"PK\x01\x02"[..]) {
            break;
        }
        let at = |o: usize| p.saturating_add(o);
        let method = fb.u16(at(10)).unwrap_or(0);
        let csize = fb.u32(at(20)).unwrap_or(0) as usize;
        let full = fb.u32(at(24)).unwrap_or(0) as usize;
        let nlen = fb.u16(at(28)).unwrap_or(0) as usize;
        let elen = fb.u16(at(30)).unwrap_or(0) as usize;
        let clen = fb.u16(at(32)).unwrap_or(0) as usize;
        let local = fb.u32(at(42)).unwrap_or(0) as usize;
        let name = String::from_utf8_lossy(sl(b, at(46), nlen).unwrap_or_default()).into_owned();
        p = at(46 + nlen + elen + clen);
        let lower = name.to_ascii_lowercase();
        const SKIP: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".mov", ".mp4", ".wav", ".mp3", ".aif", ".ttf", ".otf", ".psd", ".ai"];
        let ae = lower.ends_with(".aep") || lower.ends_with(".aegraphic");
        if name.ends_with('/') || SKIP.iter().any(|s| lower.ends_with(s)) || (ae && !keep_ae) {
            continue;
        }
        if sl(b, local, 4) != Some(&b"PK\x03\x04"[..]) {
            continue;
        }
        let start = local
            .saturating_add(30)
            .saturating_add(fb.u16(local.saturating_add(26)).unwrap_or(0) as usize)
            .saturating_add(fb.u16(local.saturating_add(28)).unwrap_or(0) as usize);
        let Some(data) = sl(b, start, csize) else { continue };
        let bytes = match method {
            0 => data.to_vec(),
            8 => match miniz_oxide::inflate::decompress_to_vec_with_limit(data, full.clamp(1, MAX_INFLATE)) {
                Ok(v) => v,
                Err(_) => continue,
            },
            _ => continue,
        };
        out.push((name, bytes));
        if out.len() >= 256 {
            break;
        }
    }
    Ok(out)
}

fn sl(b: &[u8], from: usize, n: usize) -> Option<&[u8]> {
    b.get(from..from.checked_add(n)?)
}

/// A gzip stream (Premiere projects), inflated to at most 256 MB.
fn gunzip(b: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "gzip 파일이 깨졌어요".to_string();
    let flags = *b.get(3).ok_or_else(bad)?;
    let mut p = 10usize;
    if flags & 4 != 0 {
        let x = Fb(b).u16(p).ok_or_else(bad)? as usize;
        p = p.saturating_add(2 + x);
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            let z = b.get(p..).ok_or_else(bad)?.iter().position(|c| *c == 0).ok_or_else(bad)?;
            p = p.saturating_add(z + 1);
        }
    }
    if flags & 2 != 0 {
        p = p.saturating_add(2);
    }
    let data = b.get(p..).ok_or_else(bad)?;
    miniz_oxide::inflate::decompress_to_vec_with_limit(data, 256 << 20).map_err(|_| bad())
}

#[cfg(test)]
#[path = "premiere_tests.rs"]
mod tests;

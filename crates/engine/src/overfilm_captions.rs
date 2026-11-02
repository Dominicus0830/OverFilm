//! OverFilm: styled, animated captions (see `OVERFILM.md`). The styles and motion live in
//! `overfilm_captions::styles`; this module places them.
//!
//! - `overfilm.captions.stylize {track?, preset?, presets?, motionIn?, motionOut?, captions?}`
//!   turns the captions of a caption track (default C1) into graphic clips on a new video track
//!   "OverFilm 자막 · <preset>", one per caption, styled and keyframed by the preset, in one undo
//!   step. An earlier OverFilm caption track is replaced; the caption track stays (for SRT
//!   export) with its output turned off so the text is not drawn twice.
//! - `overfilm.captions.presets {presets?}` lists the built-in and the user's presets.
//! - `overfilm.captions.savePreset {preset: {...}, presets?}` adds or replaces a user preset.
//! - `overfilm.captions.importStyle {path, presets?}` takes the caption styles of a Premiere Pro
//!   file the user has (`.prtextstyle`, `.mogrt`, `.prproj` or Premiere's XML; see
//!   `overfilm_captions::premiere`) into the user's styles, fonts matched to installed families.
//! - `overfilm.captions.styleFromClip {name, clip?, presets?}` saves the look of a graphic text
//!   clip (the selected one by default), as restyled in Effect Controls, as a user style.
//!
//! - `overfilm.premiere.export {dir, name?, overlay=true, wait=false}` writes what Premiere Pro
//!   opens: `<name>.xml` (Final Cut Pro 7 XML of the sequence without the styled-caption track),
//!   `<name>.srt` (the caption track, for a Premiere caption track), `<name>-자막.mov` (the
//!   styled-caption track alone as ProRes 4444 with alpha, to lay on the top track at 00:00, in a
//!   background job) and `<name>-Premiere에서-여는-법.txt`.
//!
//! The user's presets are the `presets` path when given; else the user's own
//! `overfilm-caption-styles.json` (shared by every project, beside the OverFilm models) and, for
//! a saved project, `<project>.overfilm-caption-styles.json` beside it, which wins on a clash.
//! `savePreset` writes beside a saved project, the user's own file otherwise; `importStyle` and
//! `styleFromClip` write the user's own file, so a style taken once serves every project.

use filmcraft_project::graphic::ensure_layer_uids;
use filmcraft_project::{Track, TrackId, TrackKind};
use filmcraft_time::TimeRange;
use overfilm_captions::premiere;
use overfilm_captions::styles::{CaptionPreset, Frame, Motion, PresetFile, builtin_presets, caption_layer, preset_from_layer};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad, bool_p, has_seq, str_p};
use crate::{EngineError, Result, Session};

/// Name prefix of the video tracks this module makes.
pub const TRACK_PREFIX: &str = "OverFilm 자막";

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.captions.stylize",
            label: "Styled Captions (OverFilm)…",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"track":id|"C1"?,"preset":name?,"presets":path?,"motionIn":"none"|"fade"|"pop"|"slideUp"|"typewriter"|"bounce"?,"motionOut":…?,"captions":[id]?}"#,
            enabled: has_captions,
            run: stylize,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.premiere.export",
            label: "Export for Premiere Pro (OverFilm)…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r#"{"dir":path,"name":str?,"overlay":bool=true,"wait":bool=false}"#,
            enabled: has_seq,
            run: premiere_export,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.captions.presets",
            label: "List Caption Styles (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"presets":path?}"#,
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "overfilm.captions.savePreset",
            label: "Save Caption Style (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"preset":{name,…},"presets":path?}"#,
            enabled: always,
            run: save,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.captions.importStyle",
            label: "Import Premiere Caption Style (OverFilm)…",
            menu: &[],
            shortcut: None,
            params: r#"{"path":path (.prtextstyle, .mogrt, .prproj or a Premiere .xml),"presets":path?}"#,
            enabled: always,
            run: import_style,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.captions.styleFromClip",
            label: "Save Caption Style from Clip (OverFilm)",
            menu: &[],
            shortcut: None,
            params: r#"{"name":str,"clip":id?,"presets":path?}"#,
            enabled: always,
            run: style_from_clip,
            journal: true,
        },
    ]
}

fn has_captions(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if q.caption_tracks.iter().any(|t| !t.captions.is_empty()) { Ok(()) } else { Err("there are no captions; import an SRT first".into()) }
}

/// The `presets` path when given.
fn given_path(p: &Value) -> Option<String> {
    str_p(p, "presets").filter(|t| !t.trim().is_empty()).map(str::to_string)
}

/// The user's own styles, shared by every project.
fn mine_path() -> String {
    crate::overfilm_autocut::user_file("overfilm-caption-styles.json")
}

/// Where `savePreset` writes: `presets`, else beside a saved project, else the user's own file.
fn presets_path(s: &Session, p: &Value) -> String {
    if let Some(t) = given_path(p) {
        return t;
    }
    match s.path.as_deref() {
        Some(proj) => format!("{}.overfilm-caption-styles.json", proj.strip_suffix(".fcproj").unwrap_or(proj)),
        None => mine_path(),
    }
}

fn load_user(path: Option<&str>) -> Result<PresetFile> {
    let Some(path) = path else { return Ok(PresetFile::default()) };
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| EngineError::Other(format!("{path}: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PresetFile::default()),
        Err(e) => Err(EngineError::Other(format!("{path}: {e}"))),
    }
}

/// The files presets are read from, later ones winning: `presets` alone when given; else the
/// user's own file, then the project's.
fn preset_files(s: &Session, p: &Value) -> Vec<String> {
    if let Some(t) = given_path(p) {
        return vec![t];
    }
    let mut v = vec![mine_path()];
    let beside = presets_path(s, p);
    if !v.contains(&beside) {
        v.push(beside);
    }
    v
}

/// Built-ins, then the user's (a user preset of a built-in's name replaces it).
fn all_presets(s: &Session, p: &Value) -> Result<Vec<CaptionPreset>> {
    let mut v = builtin_presets();
    for f in preset_files(s, p) {
        // one unreadable file must not hide the others (or the built-ins)
        let Ok(file) = load_user(Some(&f)) else { continue };
        for u in file.presets.into_iter().take(500) {
            v.retain(|b| b.name != u.name);
            v.push(u);
        }
    }
    Ok(v)
}

/// Add or replace `presets` (by name) in the file at `path`.
fn store(path: &str, presets: &[CaptionPreset]) -> Result<usize> {
    let mut file = load_user(Some(path))?;
    for preset in presets {
        file.presets.retain(|x| x.name != preset.name);
        file.presets.push(preset.clone());
    }
    if let Some(dir) = std::path::Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(&file).map_err(|e| EngineError::Other(e.to_string()))?;
    std::fs::write(path, text).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(file.presets.len())
}

/// A name that is not a built-in style's (a built-in keeps its own look; the user's copy gets a
/// suffix).
fn not_builtin(name: &str, suffix: &str) -> String {
    let builtin = builtin_presets();
    let n = name.trim();
    if builtin.iter().any(|b| b.name == n) { format!("{n} ({suffix})") } else { n.to_string() }
}

/// The installed family and style for a PostScript font name (`NanumSquareB`, `Roboto-Bold`):
/// families compared without spaces, hyphens and case.
fn installed_font(ps: &str) -> Option<(String, String)> {
    let key = |t: &str| t.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    if !filmcraft_text::fonts::system_scanned() {
        filmcraft_text::fonts::scan_system();
    }
    let fams = filmcraft_text::fonts::families();
    let (fam, style) = premiere::split_postscript(ps);
    let whole = key(ps);
    // "Roboto-Bold" → Roboto / Bold; "NanumSquareB" (no hyphen) → the longest family it starts with
    let found = fams
        .iter()
        .find(|(f, _)| key(f) == key(&fam))
        .or_else(|| fams.iter().filter(|(f, _)| !key(f).is_empty() && whole.starts_with(&key(f))).max_by_key(|(f, _)| key(f).len()))?;
    let want = if key(&fam) == key(&found.0) { key(&style) } else { whole.get(key(&found.0).len()..).unwrap_or_default().to_string() };
    let style = found.1.iter().find(|s| key(s) == want).cloned().unwrap_or(if want.is_empty() || want == "regular" { "Regular".into() } else { style });
    Some((found.0.clone(), style))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let builtin: Vec<String> = builtin_presets().into_iter().map(|b| b.name).collect();
    let v = all_presets(s, p)?;
    Ok(json!({
        "presets": v.iter().map(|x| json!({"name": x.name, "builtin": builtin.contains(&x.name), "motionIn": x.motion_in, "motionOut": x.motion_out, "style": x})).collect::<Vec<_>>(),
        "file": presets_path(s, p),
    }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.captions.savePreset";
    let path = presets_path(s, p);
    let preset: CaptionPreset = serde_json::from_value(p.get("preset").cloned().unwrap_or(Value::Null)).map_err(|e| bad(CMD, format!("`preset`: {e}")))?;
    if preset.name.trim().is_empty() {
        return Err(bad(CMD, "the preset needs a `name`"));
    }
    let count = store(&path, std::slice::from_ref(&preset))?;
    Ok(json!({"file": path, "name": preset.name, "count": count}))
}

fn import_style(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.captions.importStyle";
    let path = str_p(p, "path")
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| bad(CMD, "need `path` (a .prtextstyle, .mogrt, .prproj or Premiere .xml file)"))?
        .to_string();
    let file = std::path::Path::new(&path);
    if file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).is_some_and(|e| e == "aep" || e == "aegraphic") {
        return Err(bad(CMD, "After Effects projects can't be read; open the .mogrt exported from them"));
    }
    if std::fs::metadata(file).is_ok_and(|m| m.len() > premiere::MAX_FILE as u64) {
        return Err(bad(CMD, "the file is too big for a caption style (over 512 MB)"));
    }
    let bytes = s.services.read_file(&path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let stem = file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut found = premiere::import(&bytes, &stem).map_err(|e| bad(CMD, e))?;
    let mut missing: Vec<String> = Vec::new();
    for (preset, ps) in found.presets.iter_mut().zip(found.fonts.iter()) {
        preset.name = not_builtin(&preset.name, "Premiere");
        if ps.trim().is_empty() {
            continue;
        }
        match installed_font(ps) {
            Some((family, style)) => {
                preset.font = family;
                preset.font_style = style;
            }
            None => {
                // kept, so it shows as missing (and is used once installed)
                preset.font = premiere::split_postscript(ps).0;
                if !missing.contains(ps) {
                    missing.push(ps.clone());
                }
            }
        }
    }
    let save_to = given_path(p).unwrap_or_else(mine_path);
    let count = store(&save_to, &found.presets)?;
    let names: Vec<&str> = found.presets.iter().map(|x| x.name.as_str()).collect();
    s.toast(format!("OverFilm: Premiere 자막 스타일 {}개를 가져왔어요 ({})", names.len(), names.join(", ")));
    Ok(json!({
        "file": save_to,
        "names": names,
        "count": count,
        "missingFonts": missing,
        "notes": found.notes,
        "presets": found.presets,
    }))
}

fn style_from_clip(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.captions.styleFromClip";
    let name = str_p(p, "name").map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(CMD, "the style needs a `name`"))?;
    let name = not_builtin(name, "내 스타일");
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let text_layer =
        |id: u64| q.find_item(filmcraft_project::ClipId(id)).and_then(|(_, it)| it.effects.iter().find(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER));
    let layer = match p.get("clip").and_then(Value::as_u64) {
        Some(id) => text_layer(id).ok_or_else(|| bad(CMD, "that clip has no text layer"))?,
        None => s
            .state
            .selection
            .iter()
            .find_map(|c| text_layer(c.0))
            .ok_or_else(|| bad(CMD, "select a caption (or another text graphic) on the timeline first"))?,
    };
    let frame = Frame { width: q.settings.width, height: q.settings.height };
    let preset = preset_from_layer(layer, &name, frame);
    let save_to = given_path(p).unwrap_or_else(mine_path);
    let count = store(&save_to, std::slice::from_ref(&preset))?;
    s.toast(format!("OverFilm: 자막 스타일 \"{name}\"을(를) 저장했어요"));
    Ok(json!({"file": save_to, "name": name, "count": count, "preset": preset}))
}

fn stylize(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.captions.stylize";
    let name = str_p(p, "preset").unwrap_or("기본");
    let mut preset = all_presets(s, p)?.into_iter().find(|x| x.name == name).ok_or_else(|| bad(CMD, format!("no caption style named {name}")))?;
    for (k, slot) in [("motionIn", &mut preset.motion_in), ("motionOut", &mut preset.motion_out)] {
        if let Some(m) = str_p(p, k) {
            *slot = Motion::parse(m).ok_or_else(|| bad(CMD, format!("{k}: unknown motion {m}")))?;
        }
    }
    let track = crate::captions::track_param(s, p).ok_or_else(|| bad(CMD, "no such caption track"))?;
    let only: Option<Vec<u64>> = p.get("captions").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect());
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let ct = q.caption_track(track).ok_or_else(|| bad(CMD, "no such caption track"))?;
    let caps: Vec<_> = ct.captions.iter().filter(|c| only.as_ref().is_none_or(|o| o.contains(&c.id.0))).cloned().collect();
    if caps.is_empty() {
        return Err(bad(CMD, "the caption track has no captions to style"));
    }
    let n = caps.len();
    let track_name = format!("{TRACK_PREFIX} · {}", preset.name);
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let clips = s.edit("OverFilm Styled Captions", move |pr, st| {
        let (w, h, rate) = {
            let q = pr.sequence(seq_id).ok_or(EngineError::NoSequence)?;
            (q.settings.width, q.settings.height, q.settings.frame_rate)
        };
        let src = crate::graphics::graphic_source(pr, w, h, rate);
        let frame = Frame { width: w, height: h };
        let mut items = Vec::with_capacity(caps.len());
        for c in &caps {
            let start = rate.snap(c.start);
            let dur = rate.snap(c.start + c.duration).max(start + rate.frame_duration()) - start;
            let Some(mut ti) = pr.make_track_item(src, TrackKind::Video, start, TimeRange::new(filmcraft_time::Tick::ZERO, dur), rate) else { continue };
            ti.name = c.text.lines().next().unwrap_or("자막").chars().take(40).collect();
            let mut layer = caption_layer(&preset, &c.text, c.speaker.as_deref(), dur, frame);
            filmcraft_project::resolve_auto_points(&mut layer, (w, h), (w, h));
            ti.effects.push(layer);
            ensure_layer_uids(&mut ti.effects);
            items.push(ti);
        }
        let track_id = TrackId(pr.alloc_id());
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        // replace an earlier styled-caption track
        q.video_tracks.retain(|t| !t.name.starts_with(TRACK_PREFIX));
        let mut t = Track::new(track_id, TrackKind::Video, track_name.clone());
        let ids: Vec<u64> = items.iter().map(|i| i.id.0).collect();
        // captions on one track never overlap; frame snapping can make neighbours touch, never cross
        let mut last_end = filmcraft_time::Tick::MIN;
        for mut it in items {
            if it.start < last_end {
                let cut = last_end - it.start;
                if it.duration <= cut {
                    continue;
                }
                it.start = last_end;
                it.duration -= cut;
            }
            last_end = it.end();
            t.items.push(it);
        }
        t.sort();
        q.video_tracks.push(t);
        if let Some(ct) = q.caption_track_mut(track) {
            ct.enabled = false;
        }
        q.check().map_err(EngineError::Other)?;
        st.selection.clear();
        Ok(ids)
    })?;
    s.toast(format!("OverFilm: {n} caption(s) styled as {name}"));
    Ok(json!({"clips": clips, "styled": n, "preset": name}))
}

/// File-name-safe version of a sequence name.
fn safe_name(n: &str) -> String {
    let s: String = n.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "sequence".into() } else { s.chars().take(80).collect() }
}

fn premiere_export(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "overfilm.premiere.export";
    let dir = str_p(p, "dir").filter(|d| !d.trim().is_empty()).ok_or_else(|| bad(CMD, "need `dir` (a folder)"))?.to_string();
    std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let name = safe_name(str_p(p, "name").map(str::to_string).or_else(|| s.project.item(seq_id).map(|i| i.name.clone())).as_deref().unwrap_or("sequence"));
    let at = |file: &str| std::path::Path::new(&dir).join(file).to_string_lossy().into_owned();
    let mut out = json!({"dir": dir});
    let mut notes: Vec<String> = Vec::new();

    // the sequence without the styled captions (Premiere would see them as offline graphics)
    let mut bare = (*s.project).clone();
    let styled = {
        let q = bare.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        let had = q.video_tracks.iter().any(|t| t.name.starts_with(TRACK_PREFIX) && !t.items.is_empty());
        q.video_tracks.retain(|t| !t.name.starts_with(TRACK_PREFIX));
        had
    };
    let xml = at(&format!("{name}.xml"));
    let opts = filmcraft_interchange::ExportOptions { relative_to: Some(dir.clone()), name: Some(name.clone()), ..Default::default() };
    let (bytes, _) =
        filmcraft_interchange::export(&bare, seq_id, filmcraft_interchange::Format::Fcp7Xml, &opts).map_err(|e| EngineError::Other(e.to_string()))?;
    s.services.write_file(&xml, &bytes).map_err(|e| EngineError::Other(format!("{xml}: {e}")))?;
    out["xml"] = json!(xml);
    notes.push(format!("1. Premiere Pro ▸ 파일 ▸ 가져오기 ▸ {name}.xml (편집 시퀀스가 그대로 열립니다)"));

    // captions as SRT
    let caption_track = s.active_sequence().and_then(|q| q.caption_tracks.iter().find(|t| !t.captions.is_empty()).map(|t| t.id));
    if let Some(tid) = caption_track {
        let srt = at(&format!("{name}.srt"));
        let spec = crate::commands::find("captions.export").ok_or_else(|| EngineError::Other("captions.export is missing".into()))?;
        (spec.run)(s, &json!({"path": srt, "format": "srt", "track": tid.0}))?;
        out["srt"] = json!(srt);
        notes.push(format!("2. 자막 트랙이 필요하면 {name}.srt 를 가져와 시퀀스에 끌어 놓습니다 (글자만, 스타일은 Premiere에서)"));
    }

    // the styled captions alone, with alpha
    if styled && bool_p(p, "overlay").unwrap_or(true) {
        let mut solo = (*s.project).clone();
        let q = solo.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        // from 00:00 (so it lines up when laid at the start) to the last caption
        let last = q.video_tracks.iter().filter(|t| t.name.starts_with(TRACK_PREFIX)).flat_map(|t| t.items.iter().map(|i| i.end())).max().unwrap_or_default();
        for t in q.video_tracks.iter_mut() {
            t.enabled = t.name.starts_with(TRACK_PREFIX);
        }
        for t in q.caption_tracks.iter_mut() {
            t.enabled = false;
        }
        let mov = at(&format!("{name}-자막.mov"));
        let params = json!({"format": "prores", "proresProfile": "4444", "alpha": true, "audio": false, "path": mov});
        let (_, mut settings) = crate::export_tools::settings_from_params(s, &params, CMD)?;
        settings.path = mov.clone();
        settings.include_audio = false;
        settings.range = Some(TimeRange::from_bounds(filmcraft_time::Tick::ZERO, last));
        let wait = bool_p(p, "wait").unwrap_or(false);
        let job = crate::export_tools::spawn_export(s, std::sync::Arc::new(solo), seq_id, settings, format!("Export {name}-자막.mov"), wait)?;
        if wait && let Some(e) = s.jobs.iter().find(|j| j.id == job).and_then(|j| j.to_json()["result"]["error"].as_str().map(str::to_string)) {
            return Err(EngineError::Other(format!("caption overlay export failed: {e}")));
        }
        out["overlay"] = json!(mov);
        out["job"] = json!(job);
        notes.push(format!("3. 모션 자막: {name}-자막.mov 를 가장 위 비디오 트랙의 00:00:00:00 에 놓습니다 (투명 배경 ProRes 4444, 마지막 자막까지)"));
    }
    let readme = at(&format!("{name}-Premiere에서-여는-법.txt"));
    let text = format!("OverFilm → Premiere Pro\n\n{}\n", notes.join("\n"));
    s.services.write_file(&readme, text.as_bytes()).map_err(|e| EngineError::Other(format!("{readme}: {e}")))?;
    out["readme"] = json!(readme);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn session_with_captions() -> Session {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let dir = crate::media_test_util::tmp_dir("overfilm-cap");
        let srt = dir.join("subs.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:02,000\n(화자1) 들어갑니다\n\n2\n00:00:02,500 --> 00:00:03,500\n(화자2) 좋아요\n한 번 더\n").unwrap();
        s.execute("file.import", json!({"paths": [srt.to_string_lossy()]})).unwrap();
        s
    }

    #[test]
    fn stylize_places_one_animated_graphic_per_caption_and_mutes_the_caption_track() {
        let mut s = session_with_captions();
        let r = s.execute("overfilm.captions.stylize", json!({"preset": "해설 두 사람"})).unwrap();
        assert_eq!(r["styled"], 2, "{r}");
        let q = s.active_sequence().unwrap();
        let t = q.video_tracks.iter().find(|t| t.name.starts_with(super::TRACK_PREFIX)).unwrap();
        assert_eq!(t.items.len(), 2);
        let layer = t.items[0].effects.iter().find(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER).unwrap();
        assert!(layer.params["opacity"].is_animated());
        assert!(!q.caption_tracks[0].enabled);
        // again with another style: the first styled track is replaced, not stacked
        s.execute("overfilm.captions.stylize", json!({"preset": "예능 노랑", "motionIn": "bounce"})).unwrap();
        let q = s.active_sequence().unwrap();
        assert_eq!(q.video_tracks.iter().filter(|t| t.name.starts_with(super::TRACK_PREFIX)).count(), 1);
        assert_eq!(s.history.undo.last().unwrap().0, "OverFilm Styled Captions");
    }

    #[test]
    fn user_presets_save_list_and_apply() {
        let mut s = session_with_captions();
        let dir = crate::media_test_util::tmp_dir("overfilm-cap-presets");
        let file = dir.join("styles.json").to_string_lossy().into_owned();
        let r = s.execute("overfilm.captions.savePreset", json!({"presets": file, "preset": {"name": "내 스타일", "size": 80.0, "motion_in": "pop"}})).unwrap();
        assert_eq!(r["count"], 1);
        let l = s.execute("overfilm.captions.presets", json!({"presets": file})).unwrap();
        assert!(l["presets"].as_array().unwrap().iter().any(|p| p["name"] == "내 스타일" && p["builtin"] == false));
        s.execute("overfilm.captions.stylize", json!({"presets": file, "preset": "내 스타일"})).unwrap();
    }

    #[test]
    fn premiere_bundle_has_xml_srt_and_an_alpha_overlay() {
        let mut s = session_with_captions();
        s.execute("overfilm.captions.stylize", json!({"preset": "박스"})).unwrap();
        // a short sequence keeps the overlay render quick
        let dir = crate::media_test_util::tmp_dir("overfilm-premiere");
        let r = s.execute("overfilm.premiere.export", json!({"dir": dir.to_string_lossy(), "name": "demo/cut", "wait": true})).unwrap();
        let xml = std::fs::read_to_string(r["xml"].as_str().unwrap()).unwrap();
        assert!(xml.contains("<xmeml"), "FCP7 XML");
        assert!(!xml.contains(super::TRACK_PREFIX), "styled captions are left out of the XML");
        let srt = std::fs::read_to_string(r["srt"].as_str().unwrap()).unwrap();
        assert!(srt.contains("들어갑니다"), "{srt}");
        let mov = r["overlay"].as_str().unwrap();
        assert!(mov.ends_with("demo_cut-자막.mov"), "{mov}");
        assert!(std::fs::metadata(mov).unwrap().len() > 1000);
        assert!(std::fs::read_to_string(r["readme"].as_str().unwrap()).unwrap().contains("00:00:00:00"));
        assert!(s.execute("overfilm.premiere.export", json!({})).is_err());
    }

    /// A Premiere text style (made by `overfilm_captions::premiere`'s tests): outlined yellow
    /// NanumSquare ExtraBold on a 60 % black box, with a shadow.
    const YELLOW_BOXED: &str = "RAEAAAAAAABEMyIRDAAAAAYACAAEAAAACAAAAFAAAABKACQABAAIAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAAAAAAAAAAAABAAFAAYABwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAAAATAAAACAAAACgAAAAAQAAAMQAAAABAAAAAABwQgAAkEEAABBBAQAAAAwAAAAIAAwABAAIAAgAAAAIAAAAIAAAAAIAAABBYQAAEgAYAAAABAAIAAAADAAQABQAAAAUAAAAAACQQhwAAAAwAAAAAQAAAAAAAEEKAAwAAAAEAAgAAAAMAAAA3AAAAAAAAAAKABAABAAIAAwAAAAMAAAAAAAAAAAAAAAAAAAAAQAAAAQAAAAVAAAATmFudW1TcXVhcmUtRXh0cmFCb2xkAAoAEAAEAAgADAAKAAAAAAAAAAAAAAAAAAAA";

    fn prtextstyle(dir: &std::path::Path, name: &str) -> String {
        let path = dir.join(format!("{name}.prtextstyle"));
        let xml = format!(
            "<?xml version=\"1.0\"?><PremiereData Version=\"3\"><StyleProjectItem><ProjectItem><Name>{name}</Name></ProjectItem></StyleProjectItem><ArbVideoComponentParam><Name>Source Text</Name><StartKeyframeValue Encoding=\"base64\">{YELLOW_BOXED}</StartKeyframeValue></ArbVideoComponentParam></PremiereData>"
        );
        std::fs::write(&path, xml).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_premiere_text_style_is_imported_listed_and_applied() {
        let mut s = session_with_captions();
        let dir = crate::media_test_util::tmp_dir("overfilm-cap-import");
        let file = dir.join("styles.json").to_string_lossy().into_owned();
        let r = s.execute("overfilm.captions.importStyle", json!({"path": prtextstyle(&dir, "내 예능 자막"), "presets": file})).unwrap();
        assert_eq!(r["names"], json!(["내 예능 자막"]), "{r}");
        let p = &r["presets"][0];
        assert_eq!(p["size"], 72.0);
        assert_eq!(p["box"]["color"], json!([0, 0, 0, 153]));
        // NanumSquare isn't installed here: kept by name, reported missing
        assert_eq!(r["missingFonts"], json!(["NanumSquare-ExtraBold"]));
        assert_eq!(p["font"], "NanumSquare");
        let l = s.execute("overfilm.captions.presets", json!({"presets": file})).unwrap();
        assert!(l["presets"].as_array().unwrap().iter().any(|p| p["name"] == "내 예능 자막" && p["builtin"] == false));
        let r = s.execute("overfilm.captions.stylize", json!({"presets": file, "preset": "내 예능 자막"})).unwrap();
        assert_eq!(r["styled"], 2);
        // a built-in's name is not taken over
        let r = s.execute("overfilm.captions.importStyle", json!({"path": prtextstyle(&dir, "박스"), "presets": file})).unwrap();
        assert_eq!(r["names"], json!(["박스 (Premiere)"]));
        assert!(s.execute("overfilm.captions.importStyle", json!({"path": dir.join("none.mogrt").to_string_lossy(), "presets": file})).is_err());
        assert!(s.execute("overfilm.captions.importStyle", json!({"path": file, "presets": file})).is_err());
        assert!(s.execute("overfilm.captions.importStyle", json!({})).is_err());
    }

    #[test]
    fn a_restyled_caption_clip_is_saved_as_a_style() {
        let mut s = session_with_captions();
        let dir = crate::media_test_util::tmp_dir("overfilm-cap-from-clip");
        let file = dir.join("styles.json").to_string_lossy().into_owned();
        assert!(s.execute("overfilm.captions.styleFromClip", json!({"name": "x", "presets": file})).is_err(), "nothing selected");
        let r = s.execute("overfilm.captions.stylize", json!({"preset": "예능 노랑"})).unwrap();
        let clip = r["clips"][0].as_u64().unwrap();
        s.state.selection = vec![filmcraft_project::ClipId(clip)];
        let r = s.execute("overfilm.captions.styleFromClip", json!({"name": "  노랑 복사  ", "presets": file})).unwrap();
        assert_eq!(r["name"], "노랑 복사");
        assert_eq!(r["preset"]["fill"], json!([255, 226, 70, 255]));
        assert_eq!(r["preset"]["motion_in"], "pop");
        let r = s.execute("overfilm.captions.styleFromClip", json!({"name": "기본", "clip": clip, "presets": file})).unwrap();
        assert_eq!(r["name"], "기본 (내 스타일)");
        assert!(s.execute("overfilm.captions.styleFromClip", json!({"name": "", "presets": file})).is_err());
        assert!(s.execute("overfilm.captions.styleFromClip", json!({"name": "y", "clip": 999999, "presets": file})).is_err());
    }

    #[test]
    fn bad_params_are_refused() {
        let mut s = session_with_captions();
        assert!(s.execute("overfilm.captions.stylize", json!({"preset": "없는 스타일"})).is_err());
        assert!(s.execute("overfilm.captions.stylize", json!({"motionIn": "spin"})).is_err());
        assert!(s.execute("overfilm.captions.stylize", json!({"track": "C9"})).is_err());
        assert!(s.execute("overfilm.captions.savePreset", json!({"presets": "/nonexistent-dir/x.json", "preset": {"name": ""}})).is_err());
        assert!(s.execute("overfilm.captions.savePreset", json!({"presets": "/tmp/x.json", "preset": 5})).is_err());
    }
}

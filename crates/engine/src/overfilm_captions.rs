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
//!
//! - `overfilm.premiere.export {dir, name?, overlay=true, wait=false}` writes what Premiere Pro
//!   opens: `<name>.xml` (Final Cut Pro 7 XML of the sequence without the styled-caption track),
//!   `<name>.srt` (the caption track, for a Premiere caption track), `<name>-자막.mov` (the
//!   styled-caption track alone as ProRes 4444 with alpha, to lay on the top track at 00:00, in a
//!   background job) and `<name>-Premiere에서-여는-법.txt`.
//!
//! The user's presets are `<project>.overfilm-caption-styles.json` beside a saved project, or the
//! `presets` path.

use filmcraft_project::graphic::ensure_layer_uids;
use filmcraft_project::{Track, TrackId, TrackKind};
use filmcraft_time::TimeRange;
use overfilm_captions::styles::{CaptionPreset, Frame, Motion, PresetFile, builtin_presets, caption_layer};
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
    ]
}

fn has_captions(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    let q = s.active_sequence().ok_or("no sequence is open")?;
    if q.caption_tracks.iter().any(|t| !t.captions.is_empty()) { Ok(()) } else { Err("there are no captions; import an SRT first".into()) }
}

fn presets_path(s: &Session, p: &Value) -> Option<String> {
    if let Some(t) = str_p(p, "presets").filter(|t| !t.trim().is_empty()) {
        return Some(t.to_string());
    }
    let proj = s.path.as_deref()?;
    Some(format!("{}.overfilm-caption-styles.json", proj.strip_suffix(".fcproj").unwrap_or(proj)))
}

fn load_user(path: Option<&str>) -> Result<PresetFile> {
    let Some(path) = path else { return Ok(PresetFile::default()) };
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| EngineError::Other(format!("{path}: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PresetFile::default()),
        Err(e) => Err(EngineError::Other(format!("{path}: {e}"))),
    }
}

/// Built-ins, then the user's (a user preset of a built-in's name replaces it).
fn all_presets(s: &Session, p: &Value) -> Result<Vec<CaptionPreset>> {
    let mut v = builtin_presets();
    for u in load_user(presets_path(s, p).as_deref())?.presets.into_iter().take(500) {
        v.retain(|b| b.name != u.name);
        v.push(u);
    }
    Ok(v)
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
    let path = presets_path(s, p).ok_or_else(|| bad(CMD, "save the project first, or give `presets` (a .json path)"))?;
    let preset: CaptionPreset = serde_json::from_value(p.get("preset").cloned().unwrap_or(Value::Null)).map_err(|e| bad(CMD, format!("`preset`: {e}")))?;
    if preset.name.trim().is_empty() {
        return Err(bad(CMD, "the preset needs a `name`"));
    }
    let mut file = load_user(Some(&path))?;
    file.presets.retain(|x| x.name != preset.name);
    file.presets.push(preset.clone());
    let text = serde_json::to_string_pretty(&file).map_err(|e| EngineError::Other(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"file": path, "name": preset.name, "count": file.presets.len()}))
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

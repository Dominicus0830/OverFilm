//! OverFilm One Go on a generated broadcast: play, a "break screen", play again.

use std::path::Path;
use std::sync::Arc;

use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{DemoScene, Generator, MediaSource};
use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::json;

use crate::Session;
use crate::media_test_util::{session_with, tmp_dir};

fn make_movie(path: &Path, shots: &[(Generator, i64)], w: u32, h: u32) {
    let rate = FrameRate::FPS_24;
    let mut p = Project::new("fixture");
    let seq = p.new_sequence("s", SequenceSettings { width: w, height: h, frame_rate: rate, ..Default::default() }, 1, 1, None);
    let mut sources: Vec<(ItemId, filmcraft_media::SharedSource)> = Vec::new();
    let mut at = Tick::ZERO;
    for (k, (generator, frames)) in shots.iter().enumerate() {
        let dur = rate.tick_of(*frames);
        let src = GeneratorSource::new(generator.clone(), w, h, rate, rate.tick_of(400));
        let clip = MediaClip {
            media: MediaRef::Generator(generator.clone()),
            info: src.info().clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        };
        let item = p.add_item(&format!("shot{k}"), Label::Iris, ItemKind::Media(Box::new(clip)), None);
        let mut v = p.make_track_item(item, TrackKind::Video, at, TimeRange::new(rate.tick_of(10 * k as i64), dur), rate).unwrap();
        for e in &mut v.effects {
            filmcraft_project::resolve_auto_points(e, (w, h), (w, h));
        }
        p.sequence_mut(seq).unwrap().video_tracks[0].items.push(v);
        sources.push((item, Arc::new(src)));
        at += dur;
    }
    let settings =
        filmcraft_export::ExportSettings { format: filmcraft_export::Format::ProRes, path: path.to_string_lossy().into_owned(), ..Default::default() };
    let provider = move |id: ItemId| sources.iter().find(|(i, _)| *i == id).map(|(_, s)| s.clone());
    filmcraft_export::export(&Arc::new(p), seq, &settings, &provider, &Default::default()).unwrap();
}

/// A broadcast with an 8 s break screen in the middle and its template learnt.
fn broadcast(name: &str) -> (Session, String) {
    let dir = tmp_dir(name);
    let path = dir.join("broadcast.mov");
    let play = Generator::Demo(DemoScene::OceanSunset);
    make_movie(&path, &[(play.clone(), 72), (Generator::BarsAndTone, 192), (play, 72)], 160, 90);
    let (mut s, _, _) = session_with(&[&path]);
    let tpl = dir.join("screens.json").to_string_lossy().into_owned();
    s.execute(
        "overfilm.screenTemplate.learn",
        json!({"templates": tpl, "screen": "standby", "name": "break", "rects": [[0.0, 0.0, 1.0, 1.0]], "time": Tick(6 * TICKS_PER_SECOND).0}),
    )
    .unwrap();
    (s, tpl)
}

fn states(v: &serde_json::Value) -> Vec<(String, String)> {
    v["steps"].as_array().unwrap().iter().map(|s| (s["step"].as_str().unwrap().to_string(), s["state"].as_str().unwrap().to_string())).collect()
}

#[test]
fn cuts_in_one_go_and_says_what_each_step_did() {
    let (mut s, tpl) = broadcast("overfilm-flow");
    let before = s.active_sequence().unwrap().duration();
    let r = s.execute("overfilm.flow.run", json!({"templates": tpl, "keepBefore": 1.0, "fps": 4, "captions": "off", "wait": true})).unwrap();
    assert_eq!(r["state"], "done", "{r}");
    assert_eq!(states(&r), vec![("autoCut".into(), "done".into()), ("applyCuts".into(), "done".into())], "{r}");
    assert!(r["steps"][1]["message"].as_str().unwrap().starts_with('1'), "{r}");
    let q = s.active_sequence().unwrap();
    let removed = (before - q.duration()).seconds();
    assert!((removed - 7.0).abs() <= 0.6, "{removed}");
    assert!(q.markers.iter().all(|m| !m.name.starts_with(crate::overfilm_autocut::MARK)), "markers used up");
    // the run stays readable afterwards, and another can start
    assert_eq!(s.execute("overfilm.flow.status", json!({})).unwrap()["state"], "done");
    assert!(s.is_enabled("overfilm.flow.run"));
    assert!(!s.is_enabled("overfilm.flow.stop"));
}

#[test]
fn marks_only_when_asked_and_runs_in_the_background() {
    let (mut s, tpl) = broadcast("overfilm-flow-bg");
    let before = s.active_sequence().unwrap().duration();
    let r = s.execute("overfilm.flow.run", json!({"templates": tpl, "keepBefore": 1.0, "fps": 4, "captions": "off", "applyCuts": false})).unwrap();
    assert_eq!(r["state"], "running", "{r}");
    assert_eq!(r["steps"][0]["state"], "running");
    assert!(r["steps"][0]["job"].is_u64(), "{r}");
    assert!(!s.is_enabled("overfilm.flow.run"), "one run at a time");
    let t0 = std::time::Instant::now();
    while super::active(&s) && t0.elapsed() < std::time::Duration::from_secs(60) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        s.poll_persistence();
    }
    let r = s.execute("overfilm.flow.status", json!({})).unwrap();
    assert_eq!(states(&r), vec![("autoCut".into(), "done".into())], "{r}");
    let q = s.active_sequence().unwrap();
    assert_eq!(q.duration(), before, "marked, not cut");
    assert_eq!(q.markers.iter().filter(|m| m.name.starts_with(crate::overfilm_autocut::MARK)).count(), 1);
}

#[test]
fn stop_cancels_the_running_step() {
    let (mut s, tpl) = broadcast("overfilm-flow-stop");
    let before = s.active_sequence().unwrap().duration();
    s.execute("overfilm.flow.run", json!({"templates": tpl, "fps": 8, "captions": "off"})).unwrap();
    let r = s.execute("overfilm.flow.stop", json!({})).unwrap();
    assert_eq!(r["state"], "stopped", "{r}");
    assert!(states(&r).iter().all(|(_, st)| st == "skipped"), "{r}");
    let t0 = std::time::Instant::now();
    while !s.overfilm_jobs.is_empty() && t0.elapsed() < std::time::Duration::from_secs(60) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        s.poll_persistence();
    }
    let q = s.active_sequence().unwrap();
    assert_eq!(q.duration(), before);
    assert!(q.markers.iter().all(|m| !m.name.starts_with(crate::overfilm_autocut::MARK)), "a stopped analysis marks nothing");
}

#[test]
fn what_can_be_known_at_the_start_is_refused_at_the_start() {
    let (mut s, tpl) = broadcast("overfilm-flow-refused");
    let err = |s: &mut Session, p: serde_json::Value| s.execute("overfilm.flow.run", p).unwrap_err().to_string();
    assert!(err(&mut s, json!({"autoCut": false, "kills": false, "captions": "off"})).contains("nothing to do"));
    assert!(err(&mut s, json!({"templates": tpl, "captions": "off", "render": true, "renderFormat": "doc"})).contains("not an export format"));
    assert!(err(&mut s, json!({"templates": tpl, "captions": "sometimes"})).contains("new, reuse or off"));
    assert!(err(&mut s, json!({"templates": tpl, "captions": "reuse"})).contains("no transcript"));
    // review off (a settings file that does not exist)
    s.overfilm_review.settings_path = Some(tmp_dir("overfilm-flow-review").join("claude-review.json"));
    assert!(err(&mut s, json!({"templates": tpl, "captions": "off", "reviewCuts": true})).contains("Claude Code review is off"));
    // no break-screen template: the first step's error comes back, nothing is left running
    let empty = tmp_dir("overfilm-flow-empty").join("none.json").to_string_lossy().into_owned();
    assert!(s.execute("overfilm.flow.run", json!({"templates": empty, "captions": "off", "wait": true})).is_err());
    assert!(s.overfilm_flow.is_none());
    assert_eq!(s.execute("overfilm.flow.status", json!({})).unwrap()["state"], "none");
    assert!(s.execute("overfilm.flow.stop", json!({})).is_err());
    // hostile numbers are clamped by the steps, never a panic
    let r = s.execute("overfilm.flow.run", json!({"templates": tpl, "fps": 1e300, "keepBefore": -5, "captions": "off", "wait": true}));
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn the_kill_scan_runs_between_the_cuts_and_the_captions_when_the_templates_have_name_plates() {
    let (mut s, tpl) = broadcast("overfilm-flow-kills");
    // no name plates or kill feed in the template: the step is left out, not failed
    let r =
        s.execute("overfilm.flow.run", json!({"templates": tpl, "keepBefore": 1.0, "fps": 4, "captions": "off", "applyCuts": false, "wait": true})).unwrap();
    assert!(states(&r).iter().all(|(k, _)| k != "kills"), "{r}");
    // name plates learnt (a strip at the top of a play frame): the scan runs after the cuts
    for team in [1, 2] {
        let x = if team == 1 { 0.0 } else { 0.6 };
        s.execute("overfilm.playerBar.learn", json!({"templates": tpl, "team": team, "rect": [x, 0.0, 0.4, 0.1], "time": Tick(TICKS_PER_SECOND).0})).unwrap();
    }
    let r = s.execute("overfilm.flow.run", json!({"templates": tpl, "keepBefore": 1.0, "fps": 4, "killFps": 1, "captions": "off", "wait": true})).unwrap();
    assert_eq!(r["state"], "done", "{r}");
    let keys: Vec<String> = states(&r).into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, vec!["autoCut", "applyCuts", "kills"], "{r}");
    assert_ne!(r["steps"][2]["state"], "failed", "{r}");
    // and it can be left out
    let r = s.execute("overfilm.flow.run", json!({"templates": tpl, "keepBefore": 1.0, "fps": 4, "kills": false, "captions": "off", "wait": true})).unwrap();
    assert!(states(&r).iter().all(|(k, _)| k != "kills"), "{r}");
}

#[test]
fn renders_the_finished_sequence_at_the_end_in_the_chosen_format() {
    let (mut s, tpl) = broadcast("overfilm-flow-render");
    let out = tmp_dir("overfilm-flow-render-out").join("finished.mov").to_string_lossy().into_owned();
    let r = s
        .execute(
            "overfilm.flow.run",
            json!({"templates": tpl, "keepBefore": 1.0, "fps": 4, "captions": "off", "render": true, "renderFormat": "prores", "renderPath": out, "wait": true}),
        )
        .unwrap();
    assert_eq!(r["state"], "done", "{r}");
    assert_eq!(states(&r).last(), Some(&("render".to_string(), "done".to_string())), "{r}");
    assert!(r["steps"][2]["message"].as_str().unwrap().contains("finished.mov"), "{r}");
    let len = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    assert!(len > 1000, "the file is written ({len} bytes)");
    // the Premiere hand-over writes the XML into the folder
    let dir = tmp_dir("overfilm-flow-render-premiere");
    let r = s
        .execute(
            "overfilm.flow.run",
            json!({"autoCut": false, "kills": false, "captions": "off", "render": true, "renderFormat": "premiere", "renderPath": dir.to_string_lossy(), "wait": true}),
        )
        .unwrap();
    assert_eq!(r["state"], "done", "{r}");
    assert!(std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).any(|e| e.path().extension().is_some_and(|x| x == "xml")), "{r}");
}

#[test]
fn render_parameters_follow_the_panel_choices() {
    let (s, _) = broadcast("overfilm-flow-render-params");
    let v = super::render_params(&s, &json!({"renderPath": "/x/out.mp4", "burnCaptions": false, "captionSidecar": "srt"}), "hevc");
    assert_eq!(v["format"], "hevc");
    assert_eq!(v["path"], "/x/out.mp4");
    assert_eq!(v["burnCaptions"], false);
    assert_eq!(v["captionSidecar"], "srt");
    let v = super::render_params(&s, &json!({"renderPath": "  "}), "h264");
    assert_eq!(v["burnCaptions"], true, "captions burnt in by default");
    assert!(!v["path"].as_str().unwrap().trim().is_empty(), "beside the project when no path is given");
    let v = super::render_params(&s, &json!({"renderPath": "/x"}), "premiere");
    assert_eq!(v, json!({"format": "premiere", "dir": "/x"}));
}

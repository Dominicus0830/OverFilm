//! Auto Cut on a generated broadcast: play, a "break screen" (another demo scene), play again.

use std::path::Path;
use std::sync::Arc;

use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{DemoScene, Generator, MediaSource};
use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, Project, SequenceSettings, TrackKind};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::json;

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

const PLAY: Generator = Generator::Demo(DemoScene::OceanSunset);

fn secs(t: i64) -> f64 {
    t as f64 / TICKS_PER_SECOND as f64
}

#[test]
fn marks_then_cuts_the_break_screen_up_to_the_lead_in() {
    let dir = tmp_dir("overfilm-autocut");
    let path = dir.join("broadcast.mov");
    // 3 s play, 8 s "break screen" (bars), 3 s play
    make_movie(&path, &[(PLAY, 72), (Generator::BarsAndTone, 192), (PLAY, 72)], 160, 90);
    let (mut s, _, _) = session_with(&[&path]);
    let tpl = dir.join("screens.json");
    let tpl_s = tpl.to_string_lossy().into_owned();
    // no templates yet: refused with a hint
    let e = s.execute("overfilm.autoCut", json!({"templates": tpl_s, "wait": true})).unwrap_err();
    assert!(e.to_string().contains("learn"), "{e}");
    let r = s
        .execute(
            "overfilm.screenTemplate.learn",
            json!({"templates": tpl_s, "screen": "standby", "name": "break", "rects": [[0.0, 0.0, 1.0, 1.0]], "time": Tick(6 * TICKS_PER_SECOND).0}),
        )
        .unwrap();
    assert!(r["selfScore"].as_f64().unwrap() > 0.99, "{r}");
    let l = s.execute("overfilm.screenTemplate.list", json!({"templates": tpl_s})).unwrap();
    assert_eq!(l["list"][0]["screen"], "standby");

    let n_undo = s.history.undo.len();
    let r = s.execute("overfilm.autoCut", json!({"templates": tpl_s, "wait": true, "keepBefore": 1.0, "fps": 4})).unwrap();
    let cuts = r["cuts"].as_array().unwrap();
    assert_eq!(cuts.len(), 1, "{r}");
    let (a, b) = (secs(cuts[0]["start"].as_i64().unwrap()), secs(cuts[0]["end"].as_i64().unwrap()));
    // the break screen runs 3 s – 11 s; cut until 1 s before its end
    assert!((a - 3.0).abs() <= 0.5 && (b - 10.0).abs() <= 0.5, "{a} {b} {r}");
    // marked, not cut
    assert_eq!(s.history.undo.len(), n_undo + 1);
    let q = s.active_sequence().unwrap();
    let marks: Vec<_> = q.markers.iter().filter(|m| m.name.starts_with(super::MARK)).collect();
    assert_eq!(marks.len(), 1);
    let before = q.duration();
    assert!(s.is_enabled("overfilm.autoCut.apply"));
    let r = s.execute("overfilm.autoCut.apply", json!({})).unwrap();
    assert_eq!(r["cuts"], 1);
    let q = s.active_sequence().unwrap();
    let removed = before - q.duration();
    assert!((removed.seconds() - (b - a)).abs() < 0.1, "{removed:?}");
    assert!(q.markers.iter().all(|m| !m.name.starts_with(super::MARK)), "the cut markers are used up");
    assert!(!s.is_enabled("overfilm.autoCut.apply"));
}

#[test]
fn bad_params_are_refused_without_panicking() {
    let dir = tmp_dir("overfilm-autocut-bad");
    let path = dir.join("b.mov");
    make_movie(&path, &[(PLAY, 24)], 64, 36);
    let (mut s, _, _) = session_with(&[&path]);
    let tpl = dir.join("t.json").to_string_lossy().into_owned();
    for p in [
        json!({"templates": tpl, "screen": "nope", "rects": [[0,0,1,1]]}),
        json!({"templates": tpl, "screen": "pause"}),
        json!({"templates": tpl, "screen": "pause", "rects": [[0,0]]}),
        json!({"templates": tpl, "screen": "pause", "rects": [[5,5,1,1]]}),
        json!({"templates": tpl, "screen": "pause", "rects": "x"}),
        json!({"templates": tpl, "screen": "pause", "rects": [[0,0,1,1]], "time": i64::MAX}),
    ] {
        assert!(s.execute("overfilm.screenTemplate.learn", p.clone()).is_err(), "{p}");
    }
    std::fs::write(&tpl, "{ not json").unwrap();
    assert!(s.execute("overfilm.autoCut", json!({"templates": tpl, "wait": true})).is_err());
    assert!(s.execute("overfilm.autoCut", json!({"templates": tpl, "mode": "boom"})).is_err());
    assert!(s.execute("overfilm.autoCut.apply", json!({})).is_err());
}

#[test]
fn frozen_picture_in_a_round_is_cut_with_a_dissolve() {
    let dir = tmp_dir("overfilm-autocut-pause");
    let path = dir.join("pause.mov");
    // 6 s setup (the leader stands in for the banner), 4 s play, 6 s frozen grey, 4 s play
    let grey = Generator::ColorMatte { color: [0.4, 0.4, 0.4, 1.0] };
    make_movie(&path, &[(Generator::BarsAndTone, 144), (PLAY, 96), (grey, 144), (PLAY, 96)], 160, 90);
    let (mut s, _, _) = session_with(&[&path]);
    let tpl = dir.join("screens.json").to_string_lossy().into_owned();
    s.execute(
        "overfilm.screenTemplate.learn",
        json!({"templates": tpl, "screen": "setup", "rects": [[0.0, 0.0, 1.0, 0.6]], "time": Tick(2 * TICKS_PER_SECOND).0}),
    )
    .unwrap();
    let before = s.active_sequence().unwrap().duration();
    let r = s.execute("overfilm.autoCut", json!({"templates": tpl, "wait": true, "keepBefore": 2.0, "fps": 4, "mode": "cut", "dissolveFrames": 6})).unwrap();
    let cuts = r["cuts"].as_array().unwrap();
    assert_eq!(cuts.len(), 2, "{r}");
    assert_eq!(cuts[0]["reason"], "waiting");
    assert_eq!(cuts[1]["reason"], "pause");
    assert_eq!(cuts[1]["dissolve"], true);
    let (a, b) = (secs(cuts[1]["start"].as_i64().unwrap()), secs(cuts[1]["end"].as_i64().unwrap()));
    assert!((a - 10.0).abs() <= 0.5 && (b - 16.0).abs() <= 0.5, "{a} {b}");
    let q = s.active_sequence().unwrap();
    let removed = (before - q.duration()).seconds();
    assert!((removed - (4.0 + 6.0)).abs() < 0.6, "{removed}");
    // one dissolve at the pause join on V1
    assert_eq!(q.video_tracks[0].transitions.len(), 1, "{:?}", q.video_tracks[0].transitions);
    assert_eq!(s.history.undo.last().unwrap().0, "OverFilm Auto Cut");
}

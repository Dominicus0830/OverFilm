//! Opening an OverShorts short: texts, pictures, sounds and markers become an editable sequence.

use std::path::Path;

use filmcraft_project::{ParamValue, TrackKind};
use filmcraft_time::Tick;
use serde_json::json;

use super::{fit_text, wrap};
use crate::Session;
use crate::media_test_util::tmp_dir;

fn write_png(path: &Path, w: u32, h: u32) {
    image::RgbaImage::from_pixel(w, h, image::Rgba([40, 120, 200, 255])).save(path).unwrap();
}

fn write_wav(path: &Path, seconds: f32) {
    std::fs::write(path, crate::voiceover::write_wav_f32(&vec![0.0; (48_000.0 * seconds) as usize], 48_000)).unwrap();
}

fn scene(dir: &Path) -> String {
    write_png(&dir.join("page.png"), 1080, 1920);
    write_png(&dir.join("짤.png"), 400, 200);
    write_wav(&dir.join("voice.wav"), 4.0);
    write_wav(&dir.join("bgm.wav"), 1.5);
    let file = dir.join("시험.overshorts");
    let v = json!({
        "format": "overshorts.scenes", "version": 1, "name": "시험 쇼츠",
        "width": 1080, "height": 1920, "fps": 60, "duration": 4.0,
        "tracks": {"video": ["장면", "자막", "제목"], "audio": ["TTS", "BGM"]},
        "texts": [
            {"track": "제목", "text": "힐러가 받은 힐량이 3천이었다는 이야기 들어 보셨나요 진짜로", "start": 0, "end": 4,
             "size": 84, "color": "#FFFFFF", "center": [540, 394], "maxWidth": 960, "maxLines": 2},
            {"track": "자막", "text": "힐러가 받은", "start": 0.1, "end": 1.0, "size": 74, "color": "#FFD43B",
             "stroke": "#000", "strokeWidth": 6, "center": [540, 1334]},
            {"track": "자막", "text": "겹치는 자막", "start": 0.5, "end": 1.5, "size": 74, "center": [540, 1334]},
            {"track": "자막", "text": "", "start": 2, "end": 3},
            {"track": "자막", "text": "망가진 숫자", "start": f64::MAX, "end": -5, "size": 1e300}
        ],
        "pictures": [
            {"track": "장면", "file": "page.png", "start": 0, "end": 2, "label": "글 머리"},
            {"track": "장면", "file": "짤.png", "start": 2, "end": 4, "rect": [0, 538, 1080, 729], "fit": "contain"},
            {"track": "장면", "file": "없는-파일.png", "start": 1, "end": 2}
        ],
        "sounds": [
            {"track": "TTS", "file": "voice.wav", "start": 0},
            {"track": "BGM", "file": "bgm.wav", "start": 0, "loop": true, "levels": [[0, -11], [1.0, -23], [3.0, -11]]}
        ],
        "markers": [{"name": "제목", "start": 0, "end": 2}, {"name": "짤", "start": 2, "end": 4}]
    });
    std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
    file.to_string_lossy().into_owned()
}

#[test]
fn a_short_opens_as_an_editable_vertical_sequence() {
    let dir = tmp_dir("overshorts");
    let path = scene(&dir);
    let mut s = Session::default();
    let r = s.execute("overfilm.shorts.import", json!({"path": path})).unwrap();
    assert_eq!(r["missing"].as_array().unwrap().len(), 1, "{r}");
    let q = s.active_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    let names: Vec<&str> = q.video_tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["장면", "자막", "제목"]);
    // pictures: the page and the reaction picture, fitted into its rectangle
    let scene = &q.video_tracks[0].items;
    assert_eq!(scene.len(), 2);
    let pic = &scene[1];
    let motion = pic.effects.iter().find(|e| e.effect == "motion").unwrap();
    assert_eq!(motion.param("scale").unwrap().value, ParamValue::Float(270.0));
    // captions: overlapping one trimmed, the empty one and the broken one dropped
    let caps = &q.video_tracks[1].items;
    assert_eq!(caps.len(), 2, "{caps:?}");
    assert!(caps[1].start >= caps[0].end());
    // the long title is wrapped onto two lines
    let title = &q.video_tracks[2].items[0];
    let layer = title.effects.iter().find(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER).unwrap();
    let ParamValue::Text(text) = &layer.param("text").unwrap().value else { panic!() };
    assert_eq!(text.lines().count(), 2, "{text}");
    // sounds: the voice once, the BGM repeated to the end with its ducking as keyframes
    assert_eq!(q.audio_tracks[0].items.len(), 1);
    let bgm = &q.audio_tracks[1].items;
    assert!(bgm.len() >= 3, "{}", bgm.len());
    assert_eq!(bgm.last().unwrap().end(), Tick::from_seconds_f64(4.0));
    let level = bgm[0].effects.iter().find(|e| e.effect == "volume").unwrap().param("level").unwrap();
    assert!(level.keyframes.len() >= 2);
    assert_eq!(q.markers.len(), 2);
    // one undo step takes it all back
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.sequences().next().is_none());
}

#[test]
fn not_a_scene_file_is_an_error_not_a_crash() {
    let dir = tmp_dir("overshorts-bad");
    let mut s = Session::default();
    for (n, body) in [("a.overshorts", "{"), ("b.overshorts", r#"{"format":"other"}"#), ("c.overshorts", r#"{"format":"overshorts.scenes","version":9}"#)] {
        let p = dir.join(n);
        std::fs::write(&p, body).unwrap();
        assert!(s.execute("overfilm.shorts.import", json!({"path": p.to_string_lossy()})).is_err(), "{n}");
    }
    assert!(s.execute("overfilm.shorts.import", json!({})).is_err());
}

#[test]
fn text_wraps_at_spaces_and_shrinks_to_fit() {
    assert_eq!(wrap("가나 다라", 10.0), ["가나 다라"]);
    assert_eq!(wrap("가나 다라", 2.5), ["가나", "다라"]);
    // a word wider than the line breaks between characters
    assert_eq!(wrap("가나다라", 2.0).len(), 2);
    let (text, size) = fit_text("가나다라마바사아자차카타파하 가나다라마바사아자차카타파하 가나다라", 84.0, 960.0, 2);
    assert!(text.lines().count() <= 3 && size < 84.0 && size >= 84.0 * 0.55);
}

#[test]
fn file_import_opens_a_short_too() {
    let dir = tmp_dir("overshorts-import");
    let path = scene(&dir);
    let mut s = Session::default();
    let r = s.execute("file.import", json!({"paths": [path]})).unwrap();
    assert_eq!(r["shorts"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(s.active_sequence().unwrap().video_tracks.len(), 3);
    assert!(s.active_sequence().unwrap().all_tracks().all(|t| t.kind == TrackKind::Video || t.kind == TrackKind::Audio));
}

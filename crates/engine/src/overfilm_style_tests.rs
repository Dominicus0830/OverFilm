use std::sync::Arc;

use filmcraft_project::{Caption, CaptionFormat, CaptionTrack, ClipId, TrackId};
use serde_json::json;

use super::*;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("overfilm-style-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("style.json").to_string_lossy().to_string()
}

#[test]
fn facts_read_the_rhythm_joins_effects_and_captions() {
    let mut s = demo();
    let seq = s.state.active_sequence.unwrap();
    let q = Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap();
    let mut ct = CaptionTrack::new(TrackId(990_000), "자막".into(), CaptionFormat::default());
    ct.captions.push(Caption {
        id: ClipId(990_001),
        start: Tick::ZERO,
        duration: Tick::from_seconds_f64(2.0),
        text: "겐지 나이스\n대박".into(),
        speaker: None,
        cue_id: None,
        settings: String::new(),
    });
    q.caption_tracks.push(ct);
    let q = s.active_sequence().unwrap().clone();
    let f = facts(&q, &|_| None);
    assert_eq!(f.length_ms, ms(q.duration()));
    // shots cover the sequence (each rounded down to the ms)
    let sum: i64 = f.shots_ms.iter().sum();
    assert!(f.length_ms - sum <= f.shots_ms.len() as i64 && sum <= f.length_ms, "{sum} vs {}", f.length_ms);
    assert_eq!(f.shots_ms.len(), f.cuts + 1);
    assert!(!f.segments_ms.is_empty());
    assert_eq!(f.sources_ms, 0, "unknown source lengths: no kept share");
    assert_eq!(f.captions, vec![CaptionFact { chars: 7, lines: 2, ms: 2000 }]);
}

#[test]
fn learn_saves_a_style_that_steers_the_reel_and_the_highlights() {
    let mut s = demo();
    let path = tmp("learn");
    let r = s.execute("overfilm.style.learn", json!({"style": path})).unwrap();
    assert_eq!(r["learntFrom"].as_array().unwrap().len(), 1, "the open sequence by default");
    assert!(r["summary"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().contains("샷 길이")), "{r}");
    let shown = s.execute("overfilm.style.show", json!({"style": path})).unwrap();
    assert_eq!(shown["summary"], r["summary"]);
    let st = saved(&s, &json!({"style": path})).unwrap();
    assert_eq!(st.sequences, 1);

    // a style with plain cuts: the reel uses no dissolve unless asked
    let mut plain = st.clone();
    plain.shot.count = 5;
    plain.transition_share = 0.0;
    plain.segment = overfilm_ai::style::Spread { p25: 4_000, median: 6_000, p75: 9_000, count: 5 };
    std::fs::write(&path, serde_json::to_string(&json!({"learntFrom": [], "style": plain})).unwrap()).unwrap();
    let seq = s.state.active_sequence.unwrap();
    {
        let q = Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap();
        for (i, a) in [2.0, 8.0].iter().enumerate() {
            q.markers.push(filmcraft_project::Marker {
                id: filmcraft_project::MarkerId(950_000 + i as u64),
                start: Tick::from_seconds_f64(*a),
                duration: Tick::from_seconds_f64(3.0),
                name: format!("OverFilm 하이라이트 {}", i + 1),
                comment: String::new(),
                kind: filmcraft_project::MarkerKind::Comment,
                color: filmcraft_project::Label::Green,
                url: String::new(),
                author: String::new(),
                custom_color: None,
            });
        }
    }
    s.execute("overfilm.highlights.reel", json!({"style": path})).unwrap();
    let q = s.active_sequence().unwrap();
    assert!(q.all_tracks().all(|t| t.transitions.is_empty()), "plain cuts like the style");
    // and the highlights run as long as the style's moments (overfilm.assist.suggest without `seconds`)
    assert_eq!(plain.highlight_length_ms(), Some(9_000));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn learn_refuses_what_it_cannot_read() {
    let mut s = demo();
    // an unsaved project and no path: the user's own style file
    let r = s.execute("overfilm.style.learn", json!({})).unwrap();
    assert!(r["style"].as_str().unwrap().ends_with("overfilm-style.json"), "{r}");
    let path = tmp("refuse");
    let r = s.execute("overfilm.style.learn", json!({"style": path, "sequences": [123_456_789]}));
    assert!(r.is_err());
    assert!(s.execute("overfilm.style.show", json!({"style": tmp("missing") + ".none"})).is_err());
    let mut empty = Session::default();
    assert!(empty.execute("overfilm.style.learn", json!({"style": path})).is_err(), "nothing to learn from");
}

#[test]
fn a_premiere_edit_brought_in_as_fcp7_xml_teaches_the_same_style() {
    let mut s = demo();
    let dir = std::env::temp_dir().join(format!("overfilm-style-{}-xml", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = dir.join("edit.xml").to_string_lossy().to_string();
    s.execute("file.exportInterchange", json!({"format": "xml", "path": xml})).unwrap();
    let original = facts(s.active_sequence().unwrap(), &|_| None);
    let before: Vec<ItemId> = s.project.items.keys().copied().collect();
    s.execute("file.import", json!({"paths": [xml]})).unwrap();
    let imported: Vec<ItemId> = s.project.items.keys().copied().filter(|i| !before.contains(i) && s.project.sequence(*i).is_some()).collect();
    assert_eq!(imported.len(), 1, "the XML brings one sequence");
    let path = dir.join("style.json").to_string_lossy().to_string();
    let r = s.execute("overfilm.style.learn", json!({"style": path, "sequences": [imported[0].0]})).unwrap();
    let q = s.project.sequence(imported[0]).unwrap();
    let back = facts(q, &|_| None);
    assert_eq!(back.cuts, original.cuts, "{r}");
    assert_eq!(back.shots_ms.len(), original.shots_ms.len());
    assert_eq!(back.transitions.len(), original.transitions.len());
    let _ = std::fs::remove_dir_all(&dir);
}

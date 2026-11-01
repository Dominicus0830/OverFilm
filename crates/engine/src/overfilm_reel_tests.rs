use std::sync::Arc;

use filmcraft_project::{ItemId, Label, Marker, MarkerId, MarkerKind};
use serde_json::json;

use super::*;

fn secs(t: Tick) -> f64 {
    t.0 as f64 / TICKS_PER_SECOND as f64
}

/// Times land on the frame grid (29.97 fps in the demo).
fn near(t: Tick, want: f64) -> bool {
    (secs(t) - want).abs() < 0.05
}

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn mark(s: &mut Session, name: &str, from: f64, to: f64) {
    let seq = s.state.active_sequence.unwrap();
    let p = Arc::make_mut(&mut s.project);
    let q = p.sequence_mut(seq).unwrap();
    let id = MarkerId(900_000 + q.markers.len() as u64);
    q.markers.push(Marker {
        id,
        start: Tick::from_seconds_f64(from),
        duration: Tick::from_seconds_f64(to - from),
        name: name.into(),
        comment: String::new(),
        kind: MarkerKind::Comment,
        color: Label::Green,
    });
    q.markers.sort_by_key(|m| m.start);
}

#[test]
fn the_reel_keeps_just_the_highlights_in_a_new_sequence() {
    let mut s = demo();
    let src = s.state.active_sequence.unwrap();
    let before = s.active_sequence().unwrap().clone();
    assert!(before.duration() > Tick::from_seconds_f64(12.0));
    mark(&mut s, "OverFilm 하이라이트 2", 8.0, 11.0);
    mark(&mut s, "OverFilm 하이라이트 1", 2.0, 5.0);
    // overlapping markers join into one moment
    mark(&mut s, "OverFilm 하이라이트 3", 9.0, 10.0);
    mark(&mut s, "OverFilm 컷: 무음", 6.0, 7.0);
    mark(&mut s, "OverFilm 유지 (Claude)", 6.5, 7.0);
    let items = s.project.items.len();
    let r = s.execute("overfilm.highlights.reel", json!({"dissolve": false})).unwrap();
    assert_eq!(r["moments"], 2, "{r}");
    let reel = ItemId(r["sequence"].as_u64().unwrap());
    assert_eq!(s.state.active_sequence, Some(reel), "the reel opens");
    let q = s.project.sequence(reel).unwrap();
    assert!(near(q.duration(), 6.0), "{r}");
    let marks: Vec<Tick> = q.markers.iter().filter(|m| m.name.starts_with(MARK_HIGHLIGHT)).map(|m| m.start).collect();
    assert_eq!(marks.len(), 3);
    assert!(near(marks[0], 0.0) && near(marks[1], 3.0) && near(marks[2], 4.0), "{marks:?}");
    assert!(q.markers.iter().all(|m| !m.name.starts_with("OverFilm 컷") && !m.name.starts_with("OverFilm 유지")), "no cut or keep markers in the reel");
    assert!(r["name"].as_str().unwrap().ends_with("하이라이트"), "{r}");
    // the source is unchanged
    assert_eq!(s.project.sequence(src).unwrap().duration(), before.duration());
    assert_eq!(s.project.sequence(src).unwrap().video_tracks, before.video_tracks);
    // a second reel gets its own name; one undo step removes it
    let r2 = s.execute("sequence.open", json!({"item": src.0})).and_then(|_| s.execute("overfilm.highlights.reel", json!({"open": false}))).unwrap();
    assert_ne!(r2["name"], r["name"]);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.items.len(), items);
}

#[test]
fn joins_get_a_dissolve_where_there_is_media() {
    let mut s = demo();
    mark(&mut s, "OverFilm 하이라이트 1", 2.0, 5.0);
    mark(&mut s, "OverFilm 하이라이트 2", 8.0, 11.0);
    let r = s.execute("overfilm.highlights.reel", json!({"dissolveFrames": 10})).unwrap();
    let q = s.active_sequence().unwrap();
    assert!(near(q.duration(), 6.0), "{r}");
    let trs: Vec<_> = q.all_tracks().flat_map(|t| t.transitions.iter()).collect();
    assert!(!trs.is_empty(), "the join gets a dissolve");
    assert!(trs.iter().all(|t| near(t.start + t.duration.mul_ratio(1, 2), 3.0)), "{trs:?}");
}

#[test]
fn without_highlights_it_is_disabled_and_refuses() {
    let mut s = demo();
    assert!((crate::find_command("overfilm.highlights.reel").unwrap().enabled)(&s).is_err());
    assert!(s.execute("overfilm.highlights.reel", json!({})).is_err());
    let mut empty = Session::default();
    assert!(empty.execute("overfilm.highlights.reel", json!({})).is_err());
}

#[test]
fn captions_follow_the_cut_even_on_a_locked_track() {
    use filmcraft_project::{Caption, CaptionFormat, CaptionTrack, ClipId, TrackId};
    let mut s = demo();
    mark(&mut s, "OverFilm 하이라이트 1", 2.0, 5.0);
    mark(&mut s, "OverFilm 하이라이트 2", 8.0, 11.0);
    let seq = s.state.active_sequence.unwrap();
    let q = Arc::make_mut(&mut s.project).sequence_mut(seq).unwrap();
    let mut ct = CaptionTrack::new(TrackId(990_000), "OverFilm 자막".into(), CaptionFormat::default());
    ct.locked = true;
    ct.sync_lock = false;
    for (i, (a, text)) in [(3.0, "보여"), (6.0, "컷"), (9.0, "나이스")].iter().enumerate() {
        ct.captions.push(Caption {
            id: ClipId(990_001 + i as u64),
            start: Tick::from_seconds_f64(*a),
            duration: Tick::from_seconds_f64(1.0),
            text: (*text).into(),
            speaker: None,
            cue_id: None,
            settings: String::new(),
        });
    }
    q.caption_tracks.push(ct);
    s.execute("overfilm.highlights.reel", json!({"dissolve": false})).unwrap();
    let q = s.active_sequence().unwrap();
    let ct = q.caption_tracks.iter().find(|c| c.name == "OverFilm 자막").unwrap();
    let got: Vec<(&str, bool)> = ct.captions.iter().map(|c| (c.text.as_str(), near(c.start, if c.text == "보여" { 1.0 } else { 4.0 }))).collect();
    assert_eq!(got, [("보여", true), ("나이스", true)]);
    assert!(ct.locked && !ct.sync_lock, "the track settings come back");
}

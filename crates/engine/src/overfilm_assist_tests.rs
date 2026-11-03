use serde_json::json;

use super::*;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn markers(s: &Session, prefix: &str) -> Vec<Marker> {
    s.active_sequence().unwrap().markers.iter().filter(|m| m.name.starts_with(prefix)).cloned().collect()
}

fn opts() -> AssistOptions {
    AssistOptions { silence: Some(SilenceParams::default()), highlights: Some(HighlightParams::default()), templates: None }
}

/// 60 s at -30 dB, 3 s of silence at 10 s, a roar at 40 s.
fn levels() -> Vec<f32> {
    let mut l = vec![-30.0; 600];
    for x in l.iter_mut().skip(100).take(30) {
        *x = -80.0;
    }
    for x in l.iter_mut().skip(400).take(40) {
        *x = -12.0;
    }
    l
}

#[test]
fn silences_and_highlights_become_markers_and_replace_earlier_ones() {
    let mut s = demo();
    let r = make(&mut s, &levels(), &opts()).unwrap();
    let quiet = markers(&s, MARK_SILENCE);
    assert_eq!(quiet.len(), 1, "{r}");
    assert_eq!(quiet[0].start, Tick::from_seconds_f64(10.2));
    assert_eq!(quiet[0].start + quiet[0].duration, Tick::from_seconds_f64(12.8));
    let high = markers(&s, MARK_HIGHLIGHT);
    assert_eq!(high.len(), 1, "{r}");
    assert!(high[0].start <= Tick::from_seconds_f64(40.0) && Tick::from_seconds_f64(44.0) <= high[0].start + high[0].duration);
    assert!(high[0].comment.contains("dB"), "{}", high[0].comment);
    assert_eq!(high[0].color, Label::Green);
    // the silence is an OverFilm cut marker: Apply removes it
    assert!((crate::find_command("overfilm.autoCut.apply").unwrap().enabled)(&s).is_ok());

    // again: replaced, not added; one undo step
    make(&mut s, &levels(), &opts()).unwrap();
    assert_eq!(markers(&s, MARK_SILENCE).len(), 1);
    assert_eq!(markers(&s, MARK_HIGHLIGHT).len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(markers(&s, MARK_HIGHLIGHT).len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(markers(&s, MARK_HIGHLIGHT).is_empty());
}

#[test]
fn applying_cuts_the_silence_with_the_other_cut_markers() {
    let mut s = demo();
    let before = s.active_sequence().unwrap().duration();
    make(&mut s, &levels(), &AssistOptions { highlights: None, ..opts() }).unwrap();
    s.execute("overfilm.autoCut.apply", json!({})).unwrap();
    let after = s.active_sequence().unwrap().duration();
    // the cut lands on the frame grid
    let removed = (before - after).0 as f64 / TICKS_PER_SECOND as f64;
    assert!((removed - 2.6).abs() < 0.05, "{removed}");
    assert!(markers(&s, MARK_SILENCE).is_empty());
}

#[test]
fn waiting_time_and_words_are_left_alone() {
    let mut s = demo();
    // Auto Cut already marked 0–20 s as waiting
    let seq = s.state.active_sequence.unwrap();
    let mut p = (*s.project).clone();
    p.sequence_mut(seq).unwrap().markers.push(Marker {
        id: MarkerId(999_999),
        start: Tick::ZERO,
        duration: Tick::from_seconds_f64(20.0),
        name: "OverFilm 컷: 대기".into(),
        comment: String::new(),
        kind: MarkerKind::Comment,
        color: Label::Rose,
        url: String::new(),
        author: String::new(),
        custom_color: None,
    });
    s.project = Arc::new(p);
    let mut l = levels();
    // a second roar inside the waiting time
    for x in l.iter_mut().skip(50).take(30) {
        *x = -10.0;
    }
    let r = make(&mut s, &l, &opts()).unwrap();
    assert!(markers(&s, MARK_SILENCE).is_empty(), "the silence lies in waiting time: {r}");
    let high = markers(&s, MARK_HIGHLIGHT);
    assert_eq!(high.len(), 1);
    assert!(high[0].start >= Tick::from_seconds_f64(20.0));
    assert_eq!(markers(&s, "OverFilm 컷: 대기").len(), 1, "Auto Cut's markers stay");
}

#[test]
fn the_command_runs_on_the_demo_and_refuses_bad_params() {
    let mut s = demo();
    let r = s.execute("overfilm.assist.suggest", json!({"wait": true, "count": 1_000_000, "thresholdDb": f64::MAX, "seconds": -5})).unwrap();
    assert!(r["silences"].is_array() && r["highlights"].as_array().unwrap().len() <= 100, "{r}");
    assert!(s.execute("overfilm.assist.suggest", json!({"silence": false, "highlights": false})).is_err());
    let mut empty = Session::default();
    assert!(empty.execute("overfilm.assist.suggest", json!({"wait": true})).is_err(), "no sequence");
}

/// On a real broadcast with its screen templates:
/// `OVERFILM_E2E_BROADCAST=match.mp4 OVERFILM_E2E_TEMPLATES=screens.json cargo test -p
/// filmcraft-engine -- --ignored overfilm_assist_e2e --nocapture`.
#[test]
#[ignore = "needs a broadcast recording and its screen templates"]
fn overfilm_assist_e2e_on_a_broadcast() {
    let (Some(path), Some(templates)) = (std::env::var_os("OVERFILM_E2E_BROADCAST"), std::env::var_os("OVERFILM_E2E_TEMPLATES")) else { return };
    let (mut s, _, _) = crate::media_test_util::session_with(&[std::path::Path::new(&path)]);
    let r = s.execute("overfilm.assist.suggest", json!({"wait": true, "templates": templates.to_string_lossy()})).unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap());
}

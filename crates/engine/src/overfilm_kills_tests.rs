use serde_json::json;

use super::*;
use crate::overfilm_assist::{AssistOptions, MARK_HIGHLIGHT};

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn names(s: &Session, prefix: &str) -> Vec<String> {
    s.active_sequence().unwrap().markers.iter().filter(|m| m.name.starts_with(prefix)).map(|m| m.name.clone()).collect()
}

fn d(at: i64, team: u8, player: u8) -> Death {
    Death { at, team, player }
}

#[test]
fn deaths_and_fights_become_markers_and_replace_earlier_ones() {
    let mut s = demo();
    let deaths = [d(1_000, 2, 4), d(3_000, 1, 1), d(6_000, 2, 2), d(30_000_000, 1, 5)];
    assert_eq!(mark(&mut s, &deaths, 10_000, 3).unwrap(), 1);
    assert_eq!(names(&s, MARK_KILL), ["OverFilm 처치: 2팀 4번", "OverFilm 처치: 1팀 1번", "OverFilm 처치: 2팀 2번", "OverFilm 처치: 1팀 5번"]);
    let q = s.active_sequence().unwrap();
    let f: Vec<_> = q.markers.iter().filter(|m| m.name.starts_with(MARK_FIGHT)).collect();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].start, Tick::ZERO, "5 s before the first death, not before the start");
    assert_eq!(f[0].start + f[0].duration, Tick::from_seconds_f64(9.0));
    assert!(f[0].comment.contains("3킬"), "{}", f[0].comment);
    // a death past the end sits at the end
    let last = q.markers.iter().filter(|m| m.name.starts_with(MARK_KILL)).map(|m| m.start).max().unwrap();
    assert_eq!(last, q.duration());
    // again: replaced, one undo step
    mark(&mut s, &deaths[..1], 10_000, 3).unwrap();
    assert_eq!(names(&s, MARK_KILL).len(), 1);
    assert!(names(&s, MARK_FIGHT).is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(names(&s, MARK_KILL).len(), 4);
}

#[test]
fn kills_lift_a_moment_into_the_highlights() {
    let mut s = demo();
    // flat sound: no highlight by itself
    let levels = vec![-30.0; 600];
    let opts = AssistOptions { silence: None, highlights: Some(Default::default()), templates: None };
    crate::overfilm_assist::make(&mut s, &levels, &opts).unwrap();
    assert!(names(&s, MARK_HIGHLIGHT).is_empty());
    mark(&mut s, &[d(30_000, 1, 1), d(31_000, 2, 1), d(33_000, 2, 2)], 10_000, 3).unwrap();
    let r = crate::overfilm_assist::make(&mut s, &levels, &opts).unwrap();
    assert_eq!(names(&s, MARK_HIGHLIGHT).len(), 1, "{r}");
    assert_eq!(r["highlights"][0]["kills"], 3);
    let q = s.active_sequence().unwrap();
    let h = q.markers.iter().find(|m| m.name.starts_with(MARK_HIGHLIGHT)).unwrap();
    assert!(h.comment.contains("처치 3"), "{}", h.comment);
}

#[test]
fn scan_and_learn_refuse_without_what_they_need() {
    let dir = std::env::temp_dir().join(format!("overfilm-kills-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("screens.json");
    let tp = path.to_string_lossy().to_string();
    let mut s = demo();
    let e = s.execute("overfilm.kills.scan", json!({"templates": tp, "wait": true})).unwrap_err().to_string();
    assert!(e.contains("player bars"), "{e}");
    assert!(s.execute("overfilm.playerBar.learn", json!({"templates": tp, "team": 3, "rect": [0, 0, 0.3, 0.05]})).is_err());
    assert!(s.execute("overfilm.playerBar.learn", json!({"templates": tp, "team": 1, "rect": [0, 0]})).is_err());
    assert!(s.execute("overfilm.playerBar.learn", json!({"templates": tp, "team": 1, "rect": [f64::NAN, 0, 0.3, 0.05]})).is_err());
    let mut empty = Session::default();
    assert!(empty.execute("overfilm.kills.scan", json!({"templates": tp})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// On a real broadcast: learn both bars at `OVERFILM_E2E_BAR_AT` seconds (everyone alive) from
/// `OVERFILM_E2E_BARS` ("x,y,w,h;x,y,w,h", team 1 then team 2) into a copy of the template file,
/// then scan. `OVERFILM_E2E_BROADCAST=match.mp4 OVERFILM_E2E_TEMPLATES=screens.json
/// OVERFILM_E2E_BARS=… OVERFILM_E2E_BAR_AT=1872 cargo test -p filmcraft-engine -- --ignored
/// overfilm_kills_e2e --nocapture`.
#[test]
#[ignore = "needs a broadcast recording and its screen templates"]
fn overfilm_kills_e2e_on_a_broadcast() {
    let (Some(path), Some(templates), Some(bars), Some(at)) = (
        std::env::var_os("OVERFILM_E2E_BROADCAST"),
        std::env::var_os("OVERFILM_E2E_TEMPLATES"),
        std::env::var("OVERFILM_E2E_BARS").ok(),
        std::env::var("OVERFILM_E2E_BAR_AT").ok().and_then(|v| v.parse::<f64>().ok()),
    ) else {
        return;
    };
    let copy = std::env::temp_dir().join("overfilm-kills-e2e.json");
    std::fs::copy(&templates, &copy).unwrap();
    let tp = copy.to_string_lossy().to_string();
    let (mut s, _, _) = crate::media_test_util::session_with(&[std::path::Path::new(&path)]);
    for (k, r) in bars.split(';').enumerate() {
        let rect: Vec<f64> = r.split(',').map(|v| v.trim().parse().unwrap()).collect();
        let r = s.execute("overfilm.playerBar.learn", json!({"templates": tp, "team": k + 1, "rect": rect, "time": Tick::from_seconds_f64(at).0})).unwrap();
        eprintln!("{r}");
    }
    let r = s.execute("overfilm.kills.scan", json!({"templates": tp, "wait": true})).unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap());
    let q = s.active_sequence().unwrap();
    for m in q.markers.iter().filter(|m| m.name.starts_with(MARK_KILL)) {
        eprintln!("{} {}", overfilm_vision::plan::clock(m.start), m.name);
    }
}

#[test]
fn templates_go_to_the_users_own_file_and_import_merges() {
    use overfilm_vision::{NRect, TemplateSet};
    let mut s = demo();
    // unsaved project, no path: the user's file
    let r = s.execute("overfilm.screenTemplate.list", json!({})).unwrap();
    assert!(r["templates"].as_str().unwrap().ends_with("overfilm-screens.json"), "{r}");
    // someone's template file with a player bar
    let dir = std::env::temp_dir().join(format!("overfilm-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let theirs = dir.join("owcs.json");
    let mut set = TemplateSet::default();
    set.bars.push(PlayerBar { team: 2, rect: NRect::new(0.7, 0.14, 0.27, 0.02), players: 5, alive_dark: vec![0.0; 5] });
    std::fs::write(&theirs, set.to_json()).unwrap();
    let mine = dir.join("mine.json").to_string_lossy().to_string();
    let r = s.execute("overfilm.screenTemplate.import", json!({"path": theirs.to_string_lossy(), "templates": mine})).unwrap();
    assert_eq!(r["bars"], 1);
    let r = s.execute("overfilm.screenTemplate.list", json!({"templates": mine})).unwrap();
    assert_eq!(r["bars"], json!([2]));
    assert!(s.execute("overfilm.screenTemplate.import", json!({"path": mine, "templates": mine})).is_err(), "not into itself");
    assert!(s.execute("overfilm.screenTemplate.import", json!({"path": dir.join("none.json").to_string_lossy(), "templates": mine})).is_err());
    std::fs::write(dir.join("bad.json"), "{not json").unwrap();
    assert!(s.execute("overfilm.screenTemplate.import", json!({"path": dir.join("bad.json").to_string_lossy(), "templates": mine})).is_err());

    // a saved project with an empty template file beside it still uses the user's file
    let proj = dir.join("p.fcproj");
    s.path = Some(proj.to_string_lossy().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("p.overfilm-screens.json"), "{\"templates\":[]}").unwrap();
    let r = s.execute("overfilm.screenTemplate.list", json!({})).unwrap();
    assert!(!r["templates"].as_str().unwrap().contains("p.overfilm-screens"), "{r}");
    // one with something in it wins
    std::fs::copy(dir.join("owcs.json"), dir.join("p.overfilm-screens.json")).unwrap();
    let r = s.execute("overfilm.screenTemplate.list", json!({})).unwrap();
    assert!(r["templates"].as_str().unwrap().contains("p.overfilm-screens"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

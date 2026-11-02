use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use overfilm_review::cli::Question;
use serde_json::json;

use super::*;

type Answer = Box<dyn Fn(&Question) -> std::result::Result<Value, ReviewError> + Send + Sync>;

/// Answers like Claude Code would, and keeps the questions.
struct Fake {
    asked: Mutex<Vec<Question>>,
    answer: Answer,
}

impl Reviewer for Fake {
    fn ask(&self, q: &Question, _: &AtomicBool) -> std::result::Result<Value, ReviewError> {
        lock(&self.asked).push(q.clone());
        (self.answer)(q)
    }
}

fn fake(answer: impl Fn(&Question) -> std::result::Result<Value, ReviewError> + Send + Sync + 'static) -> Arc<Fake> {
    Arc::new(Fake { asked: Mutex::new(Vec::new()), answer: Box::new(answer) })
}

fn settings_file() -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("overfilm-review-engine-{}-{n}", std::process::id())).join("claude-review.json")
}

/// The demo project with an OverFilm transcript on the first A1 clip (two people, a hero line),
/// review switched on and answered by `f`.
fn session(f: &Arc<Fake>) -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let a = &q.audio_tracks[0].items[0];
    let (item, sin) = (a.item, a.source_in);
    let o = sin.0 / TICKS_PER_MS;
    let w = |t: &str, a: i64, b: i64, sp: u32, c: f32| filmcraft_project::transcript::Word {
        text: t.into(),
        start: Tick((o + a) * TICKS_PER_MS),
        end: Tick((o + b) * TICKS_PER_MS),
        speaker: Some(sp),
        confidence: c,
    };
    let t = filmcraft_project::transcript::Transcript {
        language: "ko".into(),
        source: crate::overfilm_transcribe::SOURCE.into(),
        speakers: vec![
            filmcraft_project::transcript::Speaker { name: "화자1".into() },
            filmcraft_project::transcript::Speaker { name: crate::overfilm_transcribe::GAME_SPEAKER.into() },
        ],
        words: vec![
            w("자", 100, 300, 0, 0.99),
            w("겐지가", 400, 900, 0, 0.4),
            w("들어가고", 1000, 1600, 0, 0.95),
            w("류승룡검을", 2000, 2600, 1, 0.9),
            w("트레이서", 3000, 3600, 0, 0.95),
            w("따라갑니다", 3700, 4400, 0, 0.95),
        ],
    };
    Arc::make_mut(&mut s.project).transcripts.insert(item, Arc::new(t));
    s.overfilm_review.raw.insert(
        item,
        vec![RawChunk {
            first_word: 0,
            words: 6,
            raw: "자 갠지가 들어가고 류승룡검을 트레 따라갑니다".into(),
            text: "자 겐지가 들어가고 류승룡검을 트레이서 따라갑니다".into(),
        }],
    );
    s.overfilm_review.settings_path = Some(settings_file());
    s.overfilm_review.reviewer = Some(f.clone());
    s.execute("overfilm.review.settings", json!({"enabled": true})).unwrap();
    s
}

/// Sequence ms of the transcript word `text`.
fn word_ms(s: &Session, text: &str) -> (i64, i64) {
    let w = crate::transcript::sequence_words(s).into_iter().find(|w| w.text == text).unwrap();
    (ms(w.start), ms(w.end))
}

fn add_cut(s: &mut Session, a: i64, b: i64) -> u64 {
    s.edit("test", |pr, st| {
        let id = pr.alloc_id();
        let q = pr.sequence_mut(st.active_sequence.unwrap()).unwrap();
        q.markers.push(Marker {
            id: MarkerId(id),
            start: tick(a),
            duration: tick(b) - tick(a),
            name: crate::overfilm_assist::MARK_SILENCE.into(),
            comment: "조용함".into(),
            kind: MarkerKind::Comment,
            color: Label::Brown,
        });
        Ok(id)
    })
    .unwrap()
}

fn markers(s: &Session) -> Vec<Marker> {
    s.active_sequence().unwrap().markers.clone()
}

#[test]
fn off_by_default_and_nothing_is_asked() {
    let f = fake(|_| Ok(json!({})));
    let mut s = session(&f);
    s.execute("overfilm.review.settings", json!({"enabled": false})).unwrap();
    let (_, end) = word_ms(&s, "들어가고");
    add_cut(&mut s, end + 200, end + 5000);
    assert!(s.execute("overfilm.review.cuts", json!({"wait": true})).is_err());
    assert!(lock(&f.asked).is_empty());
    // a session that never set a settings file is off too
    let mut fresh = Session::default();
    fresh.execute("file.openDemoProject", json!({})).unwrap();
    assert!(!settings(&fresh).enabled);
    assert_eq!(fresh.execute("overfilm.review.settings", json!({})).unwrap()["enabled"], false);
}

#[test]
fn settings_persist_and_are_clamped() {
    let f = fake(|_| Ok(json!({})));
    let mut s = session(&f);
    let v = s.execute("overfilm.review.settings", json!({"auto": true, "timeoutSeconds": 99999, "model": "opus"})).unwrap();
    assert_eq!(v["auto"], true);
    assert_eq!(v["timeoutSecs"], 1800);
    let path = s.overfilm_review.settings_path.clone().unwrap();
    let back = Settings::load(&path);
    assert!(back.enabled && back.auto);
    assert_eq!(back.model, "opus");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn unclear_cut_partly_kept_clear_cut_not_asked() {
    // keep the 1.5 s after the word "들어가고", cut the rest
    let f = fake(|q| {
        let id = q.prompt.split("구간 id=").nth(1).unwrap().split('\n').next().unwrap().to_string();
        let start: i64 = q.prompt.split("start_ms=").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
        Ok(json!({"answers": [{"id": id, "decision": "partial", "keep": [{"start_ms": start, "end_ms": start + 1500}], "reason": "말끝 여운"}]}))
    });
    let mut s = session(&f);
    let (_, end) = word_ms(&s, "들어가고");
    let unclear = add_cut(&mut s, end + 200, end + 3000);
    let clear = add_cut(&mut s, end + 20_000, end + 25_000);
    let r = s.execute("overfilm.review.cuts", json!({"wait": true})).unwrap();
    assert_eq!(r["asked"], 1, "{r}");
    let asked = lock(&f.asked).clone();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].prompt.contains("시작 경계") && asked[0].prompt.contains("(앞) 화자1: 자 겐지가 들어가고"), "{}", asked[0].prompt);
    assert!(asked[0].prompt.contains("게임 음성: 류승룡검을"), "{}", asked[0].prompt);
    let ms_ = markers(&s);
    assert!(ms_.iter().all(|m| m.id.0 != unclear), "the unclear marker was replaced");
    let keep: Vec<&Marker> = ms_.iter().filter(|m| m.name == MARK_KEEP).collect();
    assert_eq!(keep.len(), 1);
    assert_eq!(keep[0].start, tick(end + 200));
    assert_eq!(keep[0].duration, tick(end + 1700) - tick(end + 200));
    let cut: Vec<&Marker> = ms_.iter().filter(|m| m.name == crate::overfilm_assist::MARK_SILENCE).collect();
    assert_eq!(cut.len(), 2, "{cut:?}");
    let piece = cut.iter().find(|m| m.id.0 != clear).unwrap();
    assert_eq!(piece.start, tick(end + 1700));
    assert_eq!(piece.start + piece.duration, tick(end + 3000));
    assert!(piece.comment.contains("Claude 검토: 일부만 자르기 – 말끝 여운"), "{}", piece.comment);
    let untouched = cut.iter().find(|m| m.id.0 == clear).unwrap();
    assert_eq!(untouched.comment, "조용함");
    // reviewed markers are not asked again; one undo puts the original back
    let r = s.execute("overfilm.review.cuts", json!({"wait": true})).unwrap();
    assert_eq!(r["asked"], 0, "{r}");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(markers(&s).iter().any(|m| m.id.0 == unclear));
}

#[test]
fn keep_answer_renames_the_marker_so_apply_leaves_it() {
    let f = fake(|q| {
        let id = q.prompt.split("구간 id=").nth(1).unwrap().split('\n').next().unwrap().to_string();
        Ok(json!({"answers": [{"id": id, "decision": "keep", "reason": "해설이 이어짐"}]}))
    });
    let mut s = session(&f);
    let (_, end) = word_ms(&s, "들어가고");
    let id = add_cut(&mut s, end + 100, end + 2000);
    s.execute("overfilm.review.cuts", json!({"wait": true})).unwrap();
    let m = markers(&s).into_iter().find(|m| m.id.0 == id).unwrap();
    assert_eq!(m.name, MARK_KEEP);
    assert!(m.comment.contains("원래 OverFilm 컷: 무음") && m.comment.contains("해설이 이어짐"), "{}", m.comment);
    assert!(s.execute("overfilm.autoCut.apply", json!({})).is_err(), "no cut markers are left to apply");
}

#[test]
fn failures_change_nothing() {
    let f = fake(|_| Err(ReviewError::NotLoggedIn));
    let mut s = session(&f);
    let (_, end) = word_ms(&s, "들어가고");
    add_cut(&mut s, end + 200, end + 3000);
    let before = markers(&s);
    let e = s.execute("overfilm.review.cuts", json!({"wait": true})).unwrap_err().to_string();
    assert!(e.contains("로그인"), "{e}");
    assert_eq!(markers(&s), before);
    // nonsense answers are dropped too
    let f = fake(|_| Ok(json!({"answers": [{"id": "nope", "decision": "cut", "reason": ""}, {"decision": 3}]})));
    let mut s = session(&f);
    add_cut(&mut s, end + 200, end + 3000);
    let before = markers(&s);
    let r = s.execute("overfilm.review.cuts", json!({"wait": true})).unwrap();
    assert_eq!(r["decided"].as_array().unwrap().len(), 0);
    assert_eq!(markers(&s), before);
}

fn caption_texts(s: &Session) -> Vec<String> {
    let q = s.active_sequence().unwrap();
    let t = q.caption_tracks.iter().find(|t| t.name == crate::overfilm_transcribe::CAPTION_TRACK).unwrap();
    t.captions.iter().map(|c| c.text.clone()).collect()
}

#[test]
fn captions_get_pure_recognition_and_fixes() {
    let f = fake(|q| {
        let answers: Vec<Value> = q
            .prompt
            .split("### 자막 id=")
            .skip(1)
            .map(|sec| {
                let id = sec.split(' ').next().unwrap().to_string();
                let current = sec.split("- 순수 인식").next().unwrap();
                if current.contains("겐지가") {
                    json!({"id": id, "decision": "fix", "text": "자 겐지가\n들어가고", "reason": "줄바꿈"})
                } else {
                    json!({"id": id, "decision": "keep", "reason": "맞음"})
                }
            })
            .collect();
        Ok(json!({"answers": answers}))
    });
    let mut s = session(&f);
    s.execute("overfilm.captions.fromTranscript", json!({})).unwrap();
    let before = caption_texts(&s);
    assert!(before.iter().any(|t| t.contains("겐지가")), "{before:?}");
    let r = s.execute("overfilm.review.captions", json!({"wait": true})).unwrap();
    let asked = lock(&f.asked).clone();
    assert_eq!(asked.len(), 1, "{r}");
    let p = &asked[0].prompt;
    assert!(p.contains("순수 인식(교정 전): 자 갠지가 들어가고"), "{p}");
    assert!(p.contains("자신 없는 단어: 겐지가"), "{p}");
    assert!(!p.contains("- 현재 자막:\n    류승룡"), "game voice is no caption: {p}");
    let after = caption_texts(&s);
    assert!(after.iter().any(|t| t.ends_with("자 겐지가\n들어가고")), "{after:?}");
    // the speaker label stays
    let fixed = after.iter().find(|t| t.contains("자 겐지가\n")).unwrap();
    assert_eq!(fixed.starts_with('('), before.iter().find(|t| t.contains("겐지가")).unwrap().starts_with('('));
    // asked once: not again
    let r = s.execute("overfilm.review.captions", json!({"wait": true})).unwrap();
    assert_eq!(r["asked"], 0, "{r}");
}

#[test]
fn caption_split_follows_word_times() {
    let f = fake(|q| {
        let id = q.prompt.split("자막 id=").nth(1).unwrap().split(' ').next().unwrap().to_string();
        Ok(json!({"answers": [{"id": id, "decision": "split", "parts": ["자 겐지가 들어가고", "트레이서 따라갑니다"], "reason": "두 문장"}]}))
    });
    let mut s = session(&f);
    // one long caption over the whole talk
    s.execute("overfilm.captions.fromTranscript", json!({})).unwrap();
    let (a, _) = word_ms(&s, "자");
    let (_, b) = word_ms(&s, "따라갑니다");
    s.edit("test", |pr, st| {
        let q = pr.sequence_mut(st.active_sequence.unwrap()).unwrap();
        let t = q.caption_tracks.iter_mut().find(|t| t.name == crate::overfilm_transcribe::CAPTION_TRACK).unwrap();
        t.captions.truncate(1);
        t.captions[0].start = tick(a);
        t.captions[0].duration = tick(b) - tick(a);
        t.captions[0].text = "자 겐지가 들어가고 트레이서 따라갑니다".into();
        t.captions[0].speaker = None;
        Ok(())
    })
    .unwrap();
    let q = s.active_sequence().unwrap();
    let c = q.caption_tracks.iter().find(|t| t.name == crate::overfilm_transcribe::CAPTION_TRACK).unwrap().captions[0].clone();
    s.execute("overfilm.review.captions", json!({"captions": [c.id.0], "lineChars": 40, "wait": true})).unwrap();
    let q = s.active_sequence().unwrap();
    let t = q.caption_tracks.iter().find(|t| t.name == crate::overfilm_transcribe::CAPTION_TRACK).unwrap();
    let parts: Vec<&Caption> = t.captions.iter().filter(|x| x.start >= c.start && x.start < c.start + c.duration).collect();
    assert_eq!(parts.len(), 2);
    let (_, gap_a) = word_ms(&s, "들어가고");
    let (gap_b, _) = word_ms(&s, "트레이서");
    let cut = ms(parts[1].start);
    assert!(cut >= gap_a && cut <= gap_b, "split at {cut} between {gap_a} and {gap_b}");
    assert_eq!(parts[0].start + parts[0].duration, parts[1].start);
    assert_eq!(parts[1].start + parts[1].duration, c.start + c.duration);
}

#[test]
fn auto_review_after_suggestions() {
    let f = fake(|q| {
        let ids: Vec<String> = q.prompt.split("구간 id=").skip(1).map(|x| x.split('\n').next().unwrap().to_string()).collect();
        Ok(json!({"answers": ids.iter().map(|id| json!({"id": id, "decision": "cut", "reason": "조용함"})).collect::<Vec<_>>()}))
    });
    let mut s = session(&f);
    s.execute("overfilm.review.settings", json!({"auto": true})).unwrap();
    let (_, end) = word_ms(&s, "들어가고");
    add_cut(&mut s, end + 200, end + 3000);
    after_cut_marks(&mut s);
    let t0 = std::time::Instant::now();
    while !s.overfilm_review.jobs.is_empty() && t0.elapsed() < std::time::Duration::from_secs(20) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        poll(&mut s);
    }
    assert_eq!(lock(&f.asked).len(), 1);
    assert!(markers(&s).iter().any(|m| m.comment.contains("Claude 검토: 자르기")));
}

#[test]
fn hostile_answers_do_not_panic() {
    let f = fake(|q| {
        let id = q.prompt.split("구간 id=").nth(1).map(|x| x.split('\n').next().unwrap_or("").to_string()).unwrap_or_default();
        Ok(
            json!({"answers": [{"id": id, "decision": "partial", "keep": [{"start_ms": i64::MIN, "end_ms": i64::MAX}, {"start_ms": 5, "end_ms": -5}], "reason": "x".repeat(10_000)}]}),
        )
    });
    let mut s = session(&f);
    let (_, end) = word_ms(&s, "들어가고");
    add_cut(&mut s, end + 200, end + 3000);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute("overfilm.review.cuts", json!({"all": true, "wait": true}))));
    assert!(r.unwrap().is_ok());
}

#[test]
fn one_go_reviews_cuts_and_captions_once_each() {
    // cuts: keep the silence; captions: keep them all
    let f = fake(|q| {
        let answers: Vec<Value> = if q.prompt.contains("구간 id=") {
            q.prompt.split("구간 id=").skip(1).map(|x| json!({"id": x.split('\n').next().unwrap(), "decision": "keep", "reason": "말 있음"})).collect()
        } else {
            q.prompt.split("### 자막 id=").skip(1).map(|x| json!({"id": x.split(' ').next().unwrap(), "decision": "keep", "reason": "맞음"})).collect()
        };
        Ok(json!({"answers": answers}))
    });
    let mut s = session(&f);
    // "on its own" too: the run must not ask a second time
    s.execute("overfilm.review.settings", json!({"auto": true, "batch": 20})).unwrap();
    let (_, end) = word_ms(&s, "들어가고");
    add_cut(&mut s, end - 300, end + 3000);
    let before = s.active_sequence().unwrap().duration();
    let r = s.execute("overfilm.flow.run", json!({"autoCut": false, "reviewCuts": true, "captions": "reuse", "reviewCaptions": true, "wait": true})).unwrap();
    assert_eq!(r["state"], "done", "{r}");
    let steps: Vec<&str> = r["steps"].as_array().unwrap().iter().map(|s| s["step"].as_str().unwrap()).collect();
    assert_eq!(steps, ["reviewCuts", "applyCuts", "captions", "reviewCaptions"], "{r}");
    assert!(r["steps"].as_array().unwrap().iter().all(|s| s["state"] == "done" || s["state"] == "skipped"), "{r}");
    let asked = lock(&f.asked).clone();
    assert_eq!(asked.len(), 2, "one cut question, one caption question: {:?}", asked.iter().map(|q| q.prompt.len()).collect::<Vec<_>>());
    // kept by Claude: nothing cut, the marker renamed
    assert_eq!(s.active_sequence().unwrap().duration(), before);
    assert!(markers(&s).iter().any(|m| m.name == MARK_KEEP));
    assert!(!caption_texts(&s).is_empty());
}

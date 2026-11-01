use std::sync::Arc;

use serde_json::json;

use super::*;

fn ow(words: &[(&str, i64, i64, Option<usize>, bool)], speakers: &[(&str, bool)]) -> overfilm_asr::Transcript {
    overfilm_asr::Transcript {
        language: "Korean".into(),
        device: "CPU".into(),
        chunks: vec![overfilm_asr::ChunkText {
            start_ms: 0,
            end_ms: 10_000,
            text: String::new(),
            words: words
                .iter()
                .map(|&(t, a, b, sp, game)| overfilm_asr::Word { text: t.into(), start_ms: a, end_ms: b, speaker: sp, game, ..Default::default() })
                .collect(),
        }],
        speakers: speakers.iter().map(|&(l, g)| overfilm_asr::Speaker { label: l.into(), game: g, ..Default::default() }).collect(),
    }
}

/// The demo project with an OverFilm transcript on the first A1 clip's media: 화자1 and 화자2
/// talk, a hero line (game voice) in between.
fn session() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let a = &q.audio_tracks[0].items[0];
    let (item, sin) = (a.item, a.source_in);
    let o = sin.0 / TICKS_PER_MS;
    let t = ow(
        &[
            ("겐지", o + 100, o + 400, Some(0), false),
            ("나이스", o + 450, o + 900, Some(0), false),
            ("류승룡검을", o + 1000, o + 1500, Some(0), true),
            ("받아라", o + 1500, o + 1900, Some(0), true),
            ("어", o + 2400, o + 2500, Some(1), false),
            ("그렇죠", o + 2600, o + 3100, Some(1), false),
        ],
        &[("화자1", false), ("화자2", false)],
    );
    let pt = to_project(&t);
    Arc::make_mut(&mut s.project).transcripts.insert(item, Arc::new(pt));
    s
}

#[test]
fn transcripts_keep_speakers_and_put_game_voice_on_its_own_speaker() {
    let t = to_project(&ow(
        &[("가", 0, 500, Some(0), false), ("나", 500, 900, Some(1), true), ("다", 1000, 1200, Some(5), false)],
        &[("화자1", false), ("겐지", true)],
    ));
    let names: Vec<&str> = t.speakers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["화자1", GAME_SPEAKER]);
    assert_eq!(t.words[0].start, Tick(0));
    assert_eq!(t.words[0].end, Tick(500 * TICKS_PER_MS));
    assert_eq!(t.speaker_name(&t.words[1]).as_deref(), Some(GAME_SPEAKER));
    assert_eq!(t.words[2].speaker, None, "an out-of-range speaker is dropped, not kept");
    assert_eq!(t.source, SOURCE);
    assert!(t.check().is_ok());
    // a game word of a person speaker adds the game speaker
    let t = to_project(&ow(&[("가", 0, 500, Some(0), true)], &[("화자1", false)]));
    assert_eq!(t.speaker_name(&t.words[0]).as_deref(), Some(GAME_SPEAKER));
}

#[test]
fn captions_from_the_transcript_drop_game_voice_and_label_people() {
    let mut s = session();
    let r = s.execute("overfilm.captions.fromTranscript", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    assert_eq!(q.caption_tracks.iter().filter(|t| t.name == CAPTION_TRACK).count(), 1);
    let t = q.caption_tracks.iter().find(|t| t.name == CAPTION_TRACK).unwrap();
    assert_eq!(r["captions"].as_u64().unwrap() as usize, t.captions.len());
    let text: Vec<&str> = t.captions.iter().map(|c| c.text.as_str()).collect();
    let all = text.join(" | ");
    assert!(all.contains("(화자1)") && all.contains("겐지 나이스"), "{all}");
    assert!(all.contains("(화자2)") && all.contains("그렇죠"), "{all}");
    assert!(!all.contains("받아라"), "game voice is dropped: {all}");
    assert_eq!(t.captions[0].speaker.as_deref(), Some("화자1"));
    for w in t.captions.windows(2) {
        assert!(w[0].start + w[0].duration <= w[1].start, "captions never overlap");
    }

    // game voice labelled, fillers cleaned; the earlier track is replaced, and one undo restores it
    s.execute("overfilm.captions.fromTranscript", json!({"gameVoice": "label", "cleanFillers": true})).unwrap();
    let q = s.active_sequence().unwrap();
    let tracks: Vec<_> = q.caption_tracks.iter().filter(|t| t.name == CAPTION_TRACK).collect();
    assert_eq!(tracks.len(), 1);
    let all: Vec<&str> = tracks[0].captions.iter().map(|c| c.text.as_str()).collect();
    let all = all.join(" | ");
    assert!(all.contains("(게임)") && all.contains("받아라"), "{all}");
    assert!(!all.contains("어 그렇죠"), "filler cleaned: {all}");
    s.execute("edit.undo", json!({})).unwrap();
    let all: String = s.active_sequence().unwrap().caption_tracks.iter().flat_map(|t| t.captions.iter().map(|c| c.text.clone())).collect();
    assert!(!all.contains("받아라"));

    // the styled-caption command takes the new track
    let tid = s.active_sequence().unwrap().caption_tracks.iter().find(|t| t.name == CAPTION_TRACK).unwrap().id;
    s.execute("overfilm.captions.stylize", json!({"track": tid.0, "preset": "해설 두 사람"})).unwrap();
}

#[test]
fn one_person_gets_no_labels() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let a = s.active_sequence().unwrap().audio_tracks[0].items[0].clone();
    let o = a.source_in.0 / TICKS_PER_MS;
    let t = to_project(&ow(&[("오늘은", o + 100, o + 500, Some(0), false), ("겐지", o + 500, o + 900, Some(0), false)], &[("화자1", false)]));
    Arc::make_mut(&mut s.project).transcripts.insert(a.item, Arc::new(t));
    s.execute("overfilm.captions.fromTranscript", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    let t = q.caption_tracks.iter().find(|t| t.name == CAPTION_TRACK).unwrap();
    assert_eq!(t.captions.len(), 1);
    assert_eq!(t.captions[0].text, "오늘은 겐지");
    assert_eq!(t.captions[0].speaker, None);
}

#[test]
fn bad_params_and_missing_transcripts_are_refused() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    assert!(s.execute("overfilm.captions.fromTranscript", json!({})).is_err(), "no transcript yet");
    let mut s = session();
    assert!(s.execute("overfilm.captions.fromTranscript", json!({"gameVoice": "loud"})).is_err());
    // huge or odd numbers are clamped, not trusted
    s.execute("overfilm.captions.fromTranscript", json!({"lineChars": u64::MAX, "maxLines": 0})).unwrap();
    if !cfg!(feature = "overfilm-qwen") {
        assert!(s.execute("overfilm.captions.generate", json!({})).is_err());
    }
    assert!(s.execute("overfilm.captions.generate", json!({"mode": "nope", "wait": true})).is_err());
}

/// End to end on a real recording (feature `overfilm-qwen`, models downloaded on first use):
/// `OVERFILM_E2E_MEDIA=clip.mp4 cargo test -p filmcraft-engine --features overfilm-qwen -- --ignored overfilm_e2e`.
#[cfg(feature = "overfilm-qwen")]
#[test]
#[ignore = "needs a Korean recording and the OverFilm models (several GB)"]
fn overfilm_e2e_recognises_a_real_recording() {
    let Some(path) = std::env::var_os("OVERFILM_E2E_MEDIA") else { return };
    let (mut s, _items, _seq) = crate::media_test_util::session_with(&[std::path::Path::new(&path)]);
    let r = s.execute("overfilm.captions.generate", json!({"wait": true})).unwrap();
    eprintln!("{r}");
    let q = s.active_sequence().unwrap();
    let t = q.caption_tracks.iter().find(|t| t.name == CAPTION_TRACK).unwrap();
    assert!(!t.captions.is_empty());
    if let Some(out) = std::env::var_os("OVERFILM_E2E_SRT") {
        s.execute("captions.export", json!({"path": out.to_string_lossy(), "track": t.id.0})).unwrap();
    }
}

#[test]
fn an_audio_track_picks_its_clips_with_their_stream_and_channels() {
    let mut s = session();
    const CMD: &str = "overfilm.captions.generate";
    // no track chosen: the usual targets, first stream, all channels
    let all = targets(&s, &json!({}), CMD).unwrap();
    assert!(!all.is_empty() && all.iter().all(|t| t.stream == 0 && t.channels.is_empty()));
    let q = s.active_sequence().unwrap();
    let a1 = q.audio_tracks[0].items[0].clone();
    let root = s.project.resolve_media(a1.item).map(|(r, _, _)| r).unwrap();
    // A1's clip plays the media's second stream, left channel only (a microphone track)
    let seq = s.state.active_sequence.unwrap();
    {
        let p = Arc::make_mut(&mut s.project);
        let q = p.sequence_mut(seq).unwrap();
        let it = &mut q.audio_tracks[0].items[0];
        it.audio_stream = 1;
        it.source_channels = vec![0];
    }
    let t = targets(&s, &json!({"audioTrack": 1}), CMD).unwrap();
    assert_eq!(t.first(), Some(&Target { item: root, stream: 1, channels: vec![0] }));
    // hostile or missing tracks are refused, not trusted
    for bad in [json!(0), json!(-1), json!("A1"), json!(u64::MAX), json!(999)] {
        assert!(targets(&s, &json!({"audioTrack": bad}), CMD).is_err(), "{bad}");
    }
    assert!(targets(&s, &json!({"audioTrack": null}), CMD).is_ok(), "null = all tracks");
}

#[test]
fn chosen_channels_are_mixed_and_odd_picks_fall_back_to_all() {
    let ch = vec![vec![1.0, 1.0], vec![0.0, 0.0]];
    assert_eq!(mono_of(&ch, &[]), vec![0.5, 0.5]);
    assert_eq!(mono_of(&ch, &[0]), vec![1.0, 1.0]);
    assert_eq!(mono_of(&ch, &[1]), vec![0.0, 0.0]);
    assert_eq!(mono_of(&ch, &[7, u16::MAX]), vec![0.5, 0.5], "channels the media does not have");
    assert!(mono_of(&[], &[0]).is_empty());
}

#[test]
fn captions_of_a_chosen_track_leave_other_media_out() {
    let mut s = session();
    let mut o = caption_options(&json!({}), "t").unwrap();
    o.only_items = Some(vec![ItemId(u64::MAX)]);
    assert!(place_captions(&mut s, &o).is_err(), "no words of the chosen media");
    let q = s.active_sequence().unwrap();
    let item = q.audio_tracks[0].items[0].item;
    o.only_items = Some(vec![s.project.resolve_media(item).map(|(r, _, _)| r).unwrap()]);
    assert!(place_captions(&mut s, &o).unwrap()["captions"].as_u64().unwrap() > 0);
}

//! OverFilm: Korean captions made inside the editor with the OverFilmSubs pipeline (see
//! `OVERFILM.md`): Qwen3-ASR with the Overwatch glossary as context, glossary correction,
//! speaker labels and in-game voice (hero lines, announcer) told apart.
//!
//! - `overfilm.captions.generate {items?, audioTrack?, mode="general"|"commentary", speakers=true,
//!   numSpeakers?, extraTerms?, gpu=true, gameVoice="drop"|"label"|"keep", lineChars=25,
//!   maxLines=1, leadMs=120, holdMs=400, readingCps=12, periods=true, cleanFillers=true, wait=false}` recognises the media of the sequence's audio
//!   clips (or `items`, or the Project panel selection) in a background job (feature
//!   `overfilm-qwen`; models are downloaded on first use into the folder OverFilmSubs uses), stores
//!   each item's transcript (Text panel ▸ Transcript; in-game voice is the speaker "게임 음성"),
//!   then lays the captions out as `overfilm.captions.fromTranscript` does. `audioTrack` (1 = A1)
//!   recognises only the clips on that audio track, each with the audio stream and channels the
//!   clip plays (a recording with the microphone on a track of its own, OBS-style), and captions
//!   only those clips' media. `numSpeakers: 1` skips telling voices apart (faster).
//!   `skipSilence` leaves quiet stretches of 2 s or more out of recognition; left out, it is on
//!   for media whose sound is a track of its own (several audio tracks, or sound only) and off
//!   for a video's only sound, where a soft voice under game sound could be skipped (turned on
//!   there, the item's report carries a `warning`). The time left
//!   is estimated from the average speed since recognition started (it moves in long steps).
//! - `overfilm.captions.fromTranscript {gameVoice?, lineChars?, maxLines?, leadMs?, holdMs?,
//!   readingCps?, periods?, cleanFillers?}` lays the sequence transcript out as Korean captions (two lines
//!   at most, breaks at natural Korean boundaries, a turn per speaker, `(화자1)` labels when two
//!   or more people talk; shown `leadMs` before the voice and `holdMs` after it, longer when the
//!   text needs time to read at `readingCps` letters a second) on the caption track "OverFilm 자막",
//!   replacing an earlier one. It works on any transcript (Whisper, imported, edited), and after
//!   cuts: the words are taken through the clips as they are now.

use std::sync::{Arc, Mutex};

use filmcraft_project::transcript::{Speaker, Transcript, Word};
use filmcraft_project::{CaptionFormat, CaptionTrack, ClipId, ItemId, TrackId};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use overfilm_captions::{Layout, TimedWord};
use overfilm_pipeline::speakers::GameVoice;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, has_seq, str_p, u64_p};
use crate::{EngineError, Result, Session};

/// Name of the caption track this module makes.
pub const CAPTION_TRACK: &str = "OverFilm 자막";
/// Speaker name of in-game voice in the stored transcripts.
pub const GAME_SPEAKER: &str = "게임 음성";
/// `Transcript::source` of the transcripts made here.
pub const SOURCE: &str = "overfilm-qwen3-asr";
/// Longest sound recognised per item (hours); longer media is refused rather than filling memory.
#[cfg(feature = "overfilm-qwen")]
const MAX_HOURS: i64 = 6;

const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "overfilm.captions.generate",
            label: "OverFilm Captions (Korean)…",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"items":[id]?,"mode":"general"|"commentary"="general","speakers":bool=true,"numSpeakers":n?,"extraTerms":str?,"gpu":bool=true,"skipSilence":bool?,"gameVoice":"drop"|"label"|"keep"="drop","lineChars":n=25,"maxLines":1|2=1,"leadMs":n=120,"holdMs":n=400,"readingCps":n=12,"periods":bool=true,"cleanFillers":bool=true,"wait":bool=false}"#,
            enabled: can_generate,
            run: generate,
            journal: true,
        },
        CommandSpec {
            id: "overfilm.captions.fromTranscript",
            label: "OverFilm Captions from Transcript",
            menu: &["Sequence", "OverFilm"],
            shortcut: None,
            params: r#"{"gameVoice":"drop"|"label"|"keep"?,"lineChars":n?,"maxLines":1|2?,"leadMs":n?,"holdMs":n?,"readingCps":n?,"periods":bool?,"cleanFillers":bool?}"#,
            enabled: has_words,
            run: from_transcript,
            journal: true,
        },
    ]
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn can_generate(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if cfg!(feature = "overfilm-qwen") { Ok(()) } else { Err("built without the `overfilm-qwen` feature".into()) }
}

fn has_words(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if crate::transcript::sequence_words(s).is_empty() { Err("the sequence has no transcript (OverFilm Captions or Transcribe first)".into()) } else { Ok(()) }
}

/// Caption options shared by both commands.
#[derive(Clone, Debug)]
pub struct CaptionOptions {
    pub game: GameVoice,
    pub layout: Layout,
    pub clean: bool,
    /// Caption only the words of these media items (an audio track was chosen); `None`: all.
    pub only_items: Option<Vec<ItemId>>,
}

fn caption_options(p: &Value, cmd: &str) -> Result<CaptionOptions> {
    let game = match str_p(p, "gameVoice") {
        Some(g) => GameVoice::parse(g).ok_or_else(|| bad(cmd, format!("gameVoice is drop, label or keep, not {g}")))?,
        None => GameVoice::Drop,
    };
    let chars = u64_p(p, "lineChars").map(|n| n.clamp(4, 60) as u32).unwrap_or(25);
    let lines = u64_p(p, "maxLines").map(|n| n.clamp(1, 2) as u32).unwrap_or(1);
    Ok(CaptionOptions {
        game,
        layout: Layout {
            max_line: chars * 2,
            max_lines: lines,
            lead_ms: u64_p(p, "leadMs").map_or(Layout::default().lead_ms, |v| v.min(1000) as i64),
            hold_ms: u64_p(p, "holdMs").map_or(Layout::default().hold_ms, |v| v.min(3000) as i64),
            reading_cps: u64_p(p, "readingCps").map_or(Layout::default().reading_cps, |v| v.min(40) as u32),
            periods: bool_p(p, "periods").unwrap_or(true),
            ..Layout::default()
        },
        clean: bool_p(p, "cleanFillers").unwrap_or(true),
        only_items: None,
    })
}

/// An OverFilm transcript (milliseconds from the item's first sample) as the project stores it:
/// media-time words, the speakers' labels, in-game voice as the speaker [`GAME_SPEAKER`].
pub fn to_project(t: &overfilm_asr::Transcript) -> Transcript {
    let mut speakers: Vec<Speaker> = t.speakers.iter().map(|s| Speaker { name: if s.game { GAME_SPEAKER.into() } else { s.label.clone() } }).collect();
    let mut game_index = speakers.iter().position(|s| s.name == GAME_SPEAKER);
    let mut words = Vec::new();
    for w in t.words() {
        let speaker = if w.game {
            let i = match game_index {
                Some(i) => i,
                None => {
                    speakers.push(Speaker { name: GAME_SPEAKER.into() });
                    let i = speakers.len() - 1;
                    game_index = Some(i);
                    i
                }
            };
            Some(i)
        } else {
            w.speaker.filter(|&i| i < speakers.len())
        };
        words.push(Word {
            text: w.text.clone(),
            start: Tick(w.start_ms.max(0).saturating_mul(TICKS_PER_MS)),
            end: Tick(w.end_ms.max(0).saturating_mul(TICKS_PER_MS)),
            speaker: speaker.and_then(|i| u32::try_from(i).ok()),
            confidence: w.conf.unwrap_or(1.0),
        });
    }
    let mut out = Transcript { language: "ko".into(), source: SOURCE.into(), speakers, words };
    out.normalize();
    out
}

/// Caption turns of sequence words: consecutive words of one speaker; labels when two or more
/// people (not game voice) speak.
pub fn turns(words: &[filmcraft_edit::transcript::SeqWord], game: GameVoice) -> Vec<(Option<String>, Vec<TimedWord>)> {
    let is_game = |w: &filmcraft_edit::transcript::SeqWord| w.speaker.as_deref() == Some(GAME_SPEAKER);
    let mut people: Vec<&str> = words.iter().filter(|w| !is_game(w)).filter_map(|w| w.speaker.as_deref()).collect();
    people.sort_unstable();
    people.dedup();
    let label_people = people.len() > 1;
    let ms = |t: Tick| t.0 / TICKS_PER_MS;
    let mut out: Vec<(Option<String>, Vec<TimedWord>)> = Vec::new();
    let mut key: Option<(Option<&str>, bool)> = None;
    for w in words {
        let g = is_game(w) && game != GameVoice::Keep;
        if g && game == GameVoice::Drop {
            key = None;
            continue;
        }
        let k = (w.speaker.as_deref(), g);
        if key != Some(k) {
            let label = if g {
                Some(overfilm_pipeline::speakers::GAME_LABEL.to_string())
            } else if label_people {
                w.speaker.clone()
            } else {
                None
            };
            out.push((label, Vec::new()));
            key = Some(k);
        }
        if let Some(last) = out.last_mut() {
            last.1.push(TimedWord { text: w.text.clone(), start_ms: ms(w.start), end_ms: ms(w.end) });
        }
    }
    if out.iter().all(|(_, w)| w.is_empty()) {
        out.clear();
    }
    out
}

/// What to recognise: a media item, the audio stream within it and the source channels to mix
/// (empty: all).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub item: ItemId,
    pub stream: usize,
    pub channels: Vec<u16>,
}

/// The targets of `p`: with `audioTrack` (1 = A1) the enabled clips on that audio track of the
/// active sequence, with the stream and channels each plays; else the usual targets
/// (`items`, the Project selection, the sequence's audio clips), first stream, all channels.
pub fn targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<Target>> {
    let Some(v) = p.get("audioTrack").filter(|v| !v.is_null()) else {
        return Ok(crate::transcript::targets(s, p).into_iter().map(|item| Target { item, stream: 0, channels: Vec::new() }).collect());
    };
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let n = v.as_u64().filter(|&n| n >= 1).ok_or_else(|| bad(cmd, "audioTrack is a track number: 1 for A1, 2 for A2…"))?;
    let track = usize::try_from(n - 1)
        .ok()
        .and_then(|i| q.audio_tracks.get(i))
        .ok_or_else(|| bad(cmd, format!("the sequence has no audio track A{n} (it has {})", q.audio_tracks.len())))?;
    let mut out: Vec<Target> = Vec::new();
    for it in track.items.iter().filter(|it| it.enabled) {
        let Some((root, _, _)) = s.project.resolve_media(it.item) else { continue };
        let t = Target { item: root, stream: it.audio_stream, channels: it.source_channels.clone() };
        // one transcript per media item: the first clip's stream and channels decide
        if !out.iter().any(|o| o.item == t.item) {
            out.push(t);
        }
    }
    if out.is_empty() {
        return Err(bad(cmd, format!("there are no clips with sound on A{n}")));
    }
    Ok(out)
}

/// Mono of the chosen source channels (all when `pick` is empty or names none of them).
pub fn mono_of(channels: &[Vec<f32>], pick: &[u16]) -> Vec<f32> {
    let chosen: Vec<Vec<f32>> = pick.iter().filter_map(|&c| channels.get(usize::from(c)).cloned()).collect();
    if chosen.is_empty() { filmcraft_speech::downmix(channels) } else { filmcraft_speech::downmix(&chosen) }
}

fn from_transcript(s: &mut Session, p: &Value) -> Result<Value> {
    let o = caption_options(p, "overfilm.captions.fromTranscript")?;
    place_captions(s, &o)
}

/// Lay the sequence transcript out as captions on the "OverFilm 자막" caption track.
fn place_captions(s: &mut Session, o: &CaptionOptions) -> Result<Value> {
    let mut words = crate::transcript::sequence_words(s);
    if let Some(only) = &o.only_items {
        words.retain(|w| only.contains(&w.item));
    }
    if words.is_empty() {
        return Err(EngineError::Other("the sequence has no transcript".into()));
    }
    let mut turns = turns(&words, o.game);
    if o.clean {
        turns = turns.into_iter().map(|(l, w)| (l, overfilm_pipeline::clean::clean(&w))).filter(|(_, w)| !w.is_empty()).collect();
    }
    let caps = overfilm_captions::layout_turns(&turns, &o.layout);
    if caps.is_empty() {
        return Err(EngineError::Other("no speech to caption (only in-game voice?)".into()));
    }
    let n = caps.len();
    let tid = s.edit_sequence("OverFilm Captions", move |q, ctx, st| {
        let tid = TrackId(ctx.alloc());
        let mut t = CaptionTrack::new(tid, CAPTION_TRACK.to_string(), CaptionFormat::default());
        let mut last_end = Tick::MIN;
        for c in &caps {
            let start = Tick(c.start_ms.max(0).saturating_mul(TICKS_PER_MS)).max(last_end);
            let end = Tick(c.end_ms.max(0).saturating_mul(TICKS_PER_MS));
            if end <= start {
                continue;
            }
            last_end = end;
            let text = c.text();
            let speaker = overfilm_captions::styles::split_speaker(&text).0;
            t.captions.push(filmcraft_project::Caption {
                id: ClipId(ctx.alloc()),
                start,
                duration: end - start,
                speaker,
                text,
                cue_id: None,
                settings: String::new(),
            });
        }
        q.caption_tracks.retain(|t| t.name != CAPTION_TRACK);
        q.caption_tracks.insert(0, t);
        st.caption_selection.clear();
        Ok(tid)
    })?;
    Ok(json!({"track": tid.0, "captions": n}))
}

/// A running recognition; [`poll`] stores the transcripts and places the captions.
pub struct PendingCaptions {
    pub job: u64,
    pub seq: ItemId,
    pub options: CaptionOptions,
    pub result: Arc<Mutex<Option<Vec<(ItemId, Transcript)>>>>,
    /// Each item's pure recognition (before the glossary correction), for Claude Code review.
    pub raw: Arc<Mutex<Vec<(ItemId, Vec<overfilm_review::captions::RawChunk>)>>>,
}

/// Apply finished recognitions. Called once per UI frame and after synchronous runs.
pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    let mut i = 0;
    while i < s.overfilm_caption_jobs.len() {
        let job = s.jobs.iter().find(|j| j.id == s.overfilm_caption_jobs[i].job);
        let finished = job.is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        let cancelled = job.is_some_and(|j| j.progress.cancel.load(Ordering::Relaxed));
        if !finished {
            i += 1;
            continue;
        }
        let pj = s.overfilm_caption_jobs.remove(i);
        let Some(done) = lock(&pj.result).take().filter(|_| !cancelled) else { continue };
        let raw = std::mem::take(&mut *lock(&pj.raw));
        let r = s.edit("OverFilm Transcribe", move |pr, _| {
            for (item, t) in done {
                pr.transcripts.insert(item, Arc::new(t));
            }
            Ok(())
        });
        if let Err(e) = r {
            s.error_toast("overfilm.captions.generate", format!("OverFilm Captions: {e}"));
            continue;
        }
        crate::overfilm_review::store_raw(s, raw);
        if s.state.active_sequence != Some(pj.seq) {
            s.toast("OverFilm Captions: transcripts saved; open the sequence and run OverFilm Captions from Transcript");
            continue;
        }
        match place_captions(s, &pj.options) {
            Ok(v) => {
                s.toast(format!("OverFilm Captions: {} caption(s) on the track \"{CAPTION_TRACK}\"", v["captions"]));
                crate::overfilm_review::after_captions(s);
            }
            Err(e) => s.error_toast("overfilm.captions.generate", format!("OverFilm Captions: {e}")),
        }
    }
}

#[cfg(not(feature = "overfilm-qwen"))]
fn generate(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("OverFilm recognition is not in this build (built without the `overfilm-qwen` feature)".into()))
}

#[cfg(feature = "overfilm-qwen")]
fn generate(s: &mut Session, p: &Value) -> Result<Value> {
    use overfilm_pipeline::pack::Mode;
    use overfilm_pipeline::recognise::{Request, ensure_models, recognise};
    const CMD: &str = "overfilm.captions.generate";
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let mut options = caption_options(p, CMD)?;
    let mode = match str_p(p, "mode") {
        Some(m) => Mode::parse(m).ok_or_else(|| bad(CMD, format!("mode is general or commentary, not {m}")))?,
        None => Mode::General,
    };
    if s.overfilm_caption_jobs.iter().any(|j| j.seq == seq) {
        return Err(EngineError::Other("OverFilm Captions is already running on this sequence".into()));
    }
    let mut req = Request::new(mode);
    req.extra_terms = str_p(p, "extraTerms").unwrap_or("").chars().take(2000).collect();
    req.gpu = bool_p(p, "gpu").unwrap_or(true);
    req.speakers = bool_p(p, "speakers").unwrap_or(true);
    req.num_speakers = u64_p(p, "numSpeakers").map(|n| n.clamp(1, 32) as usize);
    let user = overfilm_pipeline::user_glossary_dir();
    req.glossary_dir = user.is_dir().then_some(user);
    // one person: nothing to tell apart or name (the label is not shown), so it is skipped
    if req.speakers && req.num_speakers != Some(1) {
        req.known = overfilm_pipeline::voices::known(&overfilm_pipeline::voices::list());
    }
    let chosen = targets(s, p, CMD)?;
    if p.get("audioTrack").is_some_and(|v| !v.is_null()) {
        options.only_items = Some(chosen.iter().map(|t| t.item).collect());
    }
    // the media to recognise and how long each is
    let sr = filmcraft_speech::SAMPLE_RATE;
    let mut work = Vec::new();
    for Target { item, stream, channels } in chosen {
        let Some(it) = s.project.item(item) else { continue };
        let dur = match &it.kind {
            filmcraft_project::ItemKind::Media(m) => m.duration(),
            _ => continue,
        };
        let name = it.name.clone();
        let Some(src) = s.source(item) else { continue };
        if !src.info().has_audio() || stream >= src.info().audio_streams.len() {
            continue;
        }
        if dur > Tick(MAX_HOURS * 3600 * TICKS_PER_SECOND) {
            return Err(bad(CMD, format!("{name} is longer than {MAX_HOURS} hours; cut it into parts first")));
        }
        let len = usize::try_from(dur.to_units_floor(i64::from(sr)).max(0)).unwrap_or(0);
        // long quiet stretches: skipped on a separate voice track, kept in a video's only sound
        // unless asked (then with a warning in the report)
        let source = overfilm_pipeline::recognise::SoundSource::of(src.info().has_video(), src.info().audio_streams.len());
        let skip = bool_p(p, "skipSilence").unwrap_or_else(|| overfilm_pipeline::recognise::skip_silence_default(source));
        let warn = skip && source == overfilm_pipeline::recognise::SoundSource::Embedded;
        // teams, players and heroes read off the screen (overfilm.kills.scan), on this clip's clock
        let prior = crate::overfilm_lineup::prior_for(s, item);
        work.push((item, name, src, len, stream, channels, (skip, warn), prior));
    }
    if work.is_empty() {
        return Err(bad(CMD, "nothing to recognise (put clips with sound in the sequence, or pass `items`)"));
    }
    let total: u64 = work.iter().map(|w| w.3 as u64 / u64::from(sr)).sum::<u64>().max(1);
    let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    let job = crate::Job { id, label: "OverFilm Captions".into(), progress: Default::default(), result: Default::default() };
    job.progress.total.store(total, std::sync::atomic::Ordering::Relaxed);
    // recognition moves in long steps (a piece of sound at a time): a 15 s window would swing
    job.progress.set_steady_pace();
    let result: Arc<Mutex<Option<Vec<(ItemId, Transcript)>>>> = Arc::default();
    let report: Arc<Mutex<Value>> = Arc::default();
    let raw: Arc<Mutex<Vec<(ItemId, Vec<overfilm_review::captions::RawChunk>)>>> = Arc::default();
    let raw_out = raw.clone();
    let (prog, res, out, rep) = (job.progress.clone(), job.result.clone(), result.clone(), report.clone());
    let run = move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let models = overfilm_pipeline::models_dir();
        let stop = || prog.cancel.load(Ordering::Relaxed);
        let go = |status: &str| {
            *lock(&prog.status) = status.to_string();
            !prog.cancel.load(Ordering::Relaxed)
        };
        let r: std::result::Result<(Vec<(ItemId, Transcript)>, Vec<Value>), String> = (|| {
            ensure_models(&models, &mut |_, m| go(m))?;
            let mut done_secs = 0u64;
            let mut items = Vec::new();
            let mut reports = Vec::new();
            for (item, name, src, len, stream, channels, (skip, warn), prior) in &work {
                // the sound in one-minute pieces, so a long broadcast can be stopped
                let step = 60 * sr as usize;
                let mut audio = Vec::with_capacity(*len);
                let mut at = 0usize;
                while at < *len {
                    if stop() {
                        return Err("stopped".into());
                    }
                    let n = step.min(len - at);
                    let buf = src.audio_stream(*stream, at as i64, n, sr).map_err(|e| format!("{name}: {e}"))?;
                    let mono = mono_of(&buf.channels, channels);
                    if mono.is_empty() {
                        break;
                    }
                    audio.extend_from_slice(&mono);
                    at += n;
                    *lock(&prog.status) = format!("{name}: 소리 읽는 중 {}%", at * 100 / (*len).max(1));
                }
                if audio.is_empty() {
                    continue;
                }
                let secs = audio.len() as u64 / u64::from(sr);
                let req = Request { skip_silence: *skip, prior: prior.clone(), ..req.clone() };
                let rec = recognise(&audio, &req, &models, &mut |f, m| {
                    prog.done.store(done_secs + (f.clamp(0.0, 1.0) * secs as f32) as u64, Ordering::Relaxed);
                    *lock(&prog.status) = format!("{name}: {m}");
                    !prog.cancel.load(Ordering::Relaxed)
                })?;
                done_secs += secs;
                prog.done.store(done_secs, Ordering::Relaxed);
                let t = to_project(&rec.transcript);
                lock(&raw_out).push((*item, crate::overfilm_review::raw_chunks(&rec)));
                reports.push(json!({"item": item.0, "words": t.words.len(), "speakers": t.speakers.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
                    "voiceLines": rec.voice_lines, "corrections": rec.corrections.len(), "device": rec.device,
                    "stages": rec.transcript.stage_summary(), "skipSilence": skip, "silenceSkipped": rec.transcript.silence_skipped,
                    "matchNames": prior.as_ref().map(|p| p.teams.len() + p.players.len()).unwrap_or(0),
                    "warning": if *warn { overfilm_pipeline::recognise::SKIP_SILENCE_WARNING } else { "" }}));
                items.push((*item, t));
            }
            Ok((items, reports))
        })();
        let secs = t0.elapsed().as_secs_f64();
        let r = match r {
            Ok((items, reports)) => {
                *lock(&prog.status) = format!("{} clip(s) recognised in {secs:.0}s", items.len());
                *lock(&rep) = json!({"items": reports});
                *lock(&out) = Some(items);
                Ok(filmcraft_export::Report { path: String::new(), frames: 0, seconds: secs, bytes: 0, render_fps: 0.0, extra_files: Vec::new() })
            }
            Err(e) => {
                *lock(&prog.status) = if e == "stopped" || e == "취소됨" { "Stopped: nothing was changed".into() } else { e.clone() };
                Err(e)
            }
        };
        *lock(&res) = Some(r);
        prog.finished.store(true, Ordering::Relaxed);
    };
    let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
    s.jobs.push(job);
    s.overfilm_caption_jobs.push(PendingCaptions { job: id, seq, options, result, raw });
    let mut out = json!({"job": id, "seconds": total});
    if bool_p(p, "wait").unwrap_or(false) || cfg!(target_arch = "wasm32") {
        run();
        if let Value::Object(m) = lock(&report).clone() {
            for (k, v) in m {
                out[k] = v;
            }
        }
        poll(s);
    } else {
        std::thread::Builder::new().name("overfilm-captions".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
    }
    if let Some(Err(e)) = s.jobs.iter().find(|j| j.id == id).and_then(|j| lock(&j.result).clone()) {
        return Err(EngineError::Other(e));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "overfilm_transcribe_tests.rs"]
mod tests;

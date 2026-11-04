//! `OverFilmSubs`: the window. A navigation rail (새 자막, 최근, 목소리, 설정) and, under 새 자막,
//! four steps: drop a file, pick the kind of video, wait, then review and fix the result. The
//! look follows the built-in Windows 11 apps (see `ui::theme`).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
#![windows_subsystem = "windows"]

mod ui;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use eframe::egui;
use overfilm_subs::edit::{self, Opened, Preset, Recent};
use overfilm_subs::{export, job, mcp, notify, pack, review, voices};
use serde::{Deserialize, Serialize};

/// What the navigation rail shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    New,
    Recent,
    Voices,
    Glossary,
    Settings,
}

/// The steps under 새 자막.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Empty,
    Prep,
    Running,
    Review,
}

/// Background work shared with the window.
#[derive(Default)]
struct Shared {
    running: bool,
    progress: f32,
    message: String,
    result: Option<Result<job::Done, String>>,
}

/// What the window remembers between runs (`gui.json` beside the models).
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Prefs {
    #[serde(default)]
    preset: Preset,
    /// Caption shape, masking, numbers, game voice… (the input is not kept).
    options: job::Options,
    /// Where the last run's recognition ran ("GPU …" or "CPU").
    #[serde(default)]
    last_device: String,
    /// Seconds of work per minute of sound on the last run (for the time estimate).
    #[serde(default)]
    secs_per_audio_min: f32,
    /// Last export settings (format, frame rate, gap).
    #[serde(default)]
    export: export::Export,
    /// Desktop notification when captions are done.
    #[serde(default = "yes")]
    notify_done: bool,
    /// Sleep afterwards, or nothing.
    #[serde(default)]
    after_done: notify::After,
    /// The caption shape's defaults' version (see [`Prefs::upgrade`]).
    #[serde(default)]
    caption_shape: u32,
}

/// The caption shape's defaults' version: 2 is one line of up to 25 characters (before: two
/// lines of 16).
const CAPTION_SHAPE: u32 = 2;

fn yes() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        let mut options = job::Options::new(PathBuf::new());
        Preset::General.apply(&mut options);
        Self {
            preset: Preset::General,
            options,
            last_device: String::new(),
            secs_per_audio_min: 0.0,
            export: export::Export::default(),
            notify_done: true,
            after_done: notify::After::Nothing,
            caption_shape: CAPTION_SHAPE,
        }
    }
}

impl Prefs {
    /// Preferences saved before the caption shape's defaults changed hold the old defaults (the
    /// whole options are saved): two lines of 16 there become the new defaults once. Any other
    /// shape the user picked stays.
    fn upgrade(mut self) -> Self {
        if self.caption_shape < CAPTION_SHAPE {
            let o = &mut self.options;
            if o.line_chars == 16 && o.max_lines == 2 {
                o.line_chars = job::LINE_CHARS;
                o.max_lines = job::MAX_LINES;
            }
            self.caption_shape = CAPTION_SHAPE;
        }
        self
    }
}

fn prefs_path() -> PathBuf {
    edit::data_dir().join("gui.json")
}

fn recent_path() -> PathBuf {
    edit::data_dir().join("recent.json")
}

fn load_prefs() -> Prefs {
    let p = prefs_path();
    match std::fs::metadata(&p) {
        Ok(m) if m.len() < 1 << 20 => {
            std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str::<Prefs>(&s).ok()).map(Prefs::upgrade).unwrap_or_default()
        }
        _ => Prefs::default(),
    }
}

/// A finished result being reviewed.
struct Review {
    opened: Opened,
    /// The input the result was made from (for voice enrolment), when known.
    input: Option<PathBuf>,
    /// Options the captions were made with (re-used when speakers change).
    options: job::Options,
    issues: Vec<review::Issue>,
    ignored: HashSet<(usize, &'static str)>,
    filter: Option<&'static str>,
    selected: Option<usize>,
    /// Text box under the table for the selected caption.
    draft: String,
    /// Speaker being renamed and the name typed so far.
    renaming: Option<(usize, String)>,
    /// Scroll the table to the selected row on the next frame.
    jump: bool,
    undo: Vec<(Vec<overfilm_captions::Caption>, Option<overfilm_asr::Transcript>)>,
    summary: Option<String>,
    /// The export dialog is open.
    exporting: bool,
    /// Find and replace bar.
    find: FindBar,
    /// Saved versions, while the history dialog is open.
    history: Option<Vec<overfilm_subs::history::Version>>,
    /// Fixes made often enough to offer for the glossary.
    learned: Vec<overfilm_subs::learn::Pair>,
    /// Other end of a shift-click selection (with `selected`).
    sel_end: Option<usize>,
    /// Extra names for recognising a part again.
    redo_terms: String,
    /// A part being recognised again in the background.
    redo: Option<RedoJob>,
}

/// Recognising a part again: the span, progress, and the words when done.
struct RedoJob {
    from_ms: i64,
    to_ms: i64,
    state: Arc<Mutex<(f32, String, Option<Result<Vec<overfilm_asr::Word>, String>>)>>,
    cancel: Arc<AtomicBool>,
}

impl Review {
    /// The selected captions as a range of indices (one caption, or a shift-click span).
    fn selection(&self) -> Option<std::ops::RangeInclusive<usize>> {
        let a = self.selected?;
        let b = self.sel_end.unwrap_or(a);
        Some(a.min(b)..=a.max(b))
    }

    /// Start recognising the selected captions' time again, on `input`.
    fn start_redo(&mut self, input: PathBuf, ctx: &egui::Context) {
        let Some(sel) = self.selection() else { return };
        let (Some(first), Some(last)) = (self.opened.captions.get(*sel.start()), self.opened.captions.get(*sel.end())) else { return };
        let (from_ms, to_ms) = (first.start_ms, last.end_ms);
        let mut o = self.options.clone();
        o.input = input;
        let extra = self.redo_terms.trim();
        if !extra.is_empty() {
            o.extra_terms = if o.extra_terms.trim().is_empty() { extra.to_string() } else { format!("{}, {extra}", o.extra_terms) };
        }
        let d = mcp::user_glossary_dir();
        o.glossary_dir = d.is_dir().then_some(d);
        let state: Arc<Mutex<(f32, String, Option<Result<Vec<overfilm_asr::Word>, String>>)>> = Arc::new(Mutex::new((0.0, "시작".into(), None)));
        let cancel = Arc::new(AtomicBool::new(false));
        let (st, cn, ctx) = (state.clone(), cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = job::guarded(|| {
                job::recognise_span(&o, from_ms, to_ms, &mut |f, m| {
                    let mut s = st.lock().unwrap_or_else(PoisonError::into_inner);
                    s.0 = f;
                    s.1 = m.to_string();
                    ctx.request_repaint();
                    !cn.load(Ordering::Relaxed)
                })
            });
            st.lock().unwrap_or_else(PoisonError::into_inner).2 = Some(r);
            ctx.request_repaint();
        });
        self.redo = Some(RedoJob { from_ms, to_ms, state, cancel });
    }
}

/// The find and replace bar over the caption table.
#[derive(Default)]
struct FindBar {
    open: bool,
    find: String,
    with: String,
    whole_word: bool,
    /// Also add the replacement to the user glossary.
    learn: bool,
}

struct App {
    page: Page,
    step: Step,
    input: Option<PathBuf>,
    /// The input's audio tracks ("트랙 1 · 스테레오 · 48 kHz · aac") and the one to recognise.
    audio_tracks: Vec<String>,
    audio_track: usize,
    /// Whether the sound is inside the video or a track of its own (`None`: not read yet).
    sound_source: Option<overfilm_pipeline::recognise::SoundSource>,
    /// Skip long quiet stretches for this file: `None` follows the source (see
    /// [`overfilm_pipeline::recognise::skip_silence_default`]).
    skip_silence: Option<bool>,
    output: Option<PathBuf>,
    prefs: Prefs,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    started: Option<Instant>,
    /// Stage being shown and when it started (for the remaining time).
    stage_clock: Option<(usize, Instant, f32)>,
    review: Option<Review>,
    recent: Vec<Recent>,
    voices: Vec<voices::Voice>,
    enroll_name: String,
    enroll_file: Option<PathBuf>,
    enroll_from: String,
    enroll_to: String,
    enroll: Arc<Mutex<Option<Result<String, String>>>>,
    enrolling: Arc<AtomicBool>,
    models_ready: bool,
    models_dl: Arc<Mutex<Option<(f32, String, Option<Result<(), String>>)>>>,
    /// Message for the bottom of the current page (success or error).
    status: Option<Result<String, String>>,
    /// The 단어집 page.
    gloss: GlossPage,
    /// When the computer goes to sleep (after a finished run, if asked).
    sleep_at: Option<Instant>,
}

/// State of the 단어집 page: the user table as last read, the add form and the search.
#[derive(Default)]
struct GlossPage {
    entries: Option<Vec<overfilm_glossary::Entry>>,
    ko: String,
    misheard: String,
    note: String,
    search: String,
    /// Entry being edited: (id, ko, misheard, note).
    editing: Option<(String, String, String, String)>,
    builtin: Option<overfilm_glossary::Glossary>,
    /// Learned fixes waiting (read with the entries).
    pending: Vec<overfilm_subs::learn::Pair>,
}

impl App {
    fn new() -> Self {
        let dir = job::models_dir();
        Self {
            page: Page::New,
            step: Step::Empty,
            input: None,
            audio_tracks: Vec::new(),
            audio_track: 0,
            sound_source: None,
            skip_silence: None,
            output: None,
            prefs: load_prefs(),
            shared: Arc::default(),
            cancel: Arc::default(),
            started: None,
            stage_clock: None,
            review: None,
            recent: edit::load_recent(&recent_path()),
            voices: voices::list(),
            enroll_name: String::new(),
            enroll_file: None,
            enroll_from: String::new(),
            enroll_to: String::new(),
            enroll: Arc::default(),
            enrolling: Arc::default(),
            models_ready: overfilm_asr::catalogue::all().iter().all(|m| m.installed(&dir)),
            models_dl: Arc::default(),
            status: None,
            gloss: GlossPage::default(),
            sleep_at: None,
        }
    }

    fn save_prefs(&mut self) {
        let p = prefs_path();
        let r = (|| {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            let json = serde_json::to_string_pretty(&self.prefs).map_err(|e| e.to_string())?;
            std::fs::write(&p, json).map_err(|e| e.to_string())
        })();
        if let Err(e) = r {
            self.status = Some(Err(format!("설정을 저장하지 못했습니다: {e}")));
        }
    }

    /// Options for a run on the chosen file.
    fn options(&self) -> Option<job::Options> {
        let mut o = self.prefs.options.clone();
        o.input = self.input.clone()?;
        o.output = self.output.clone();
        o.only_voices = o.only_voices && o.speakers;
        o.audio_track = self.audio_track.min(self.audio_tracks.len().saturating_sub(1));
        o.skip_silence = self.skip_silence;
        let d = mcp::user_glossary_dir();
        o.glossary_dir = d.is_dir().then_some(d);
        Some(o)
    }

    fn choose_file(&mut self, p: PathBuf) {
        // a file that cannot be read shows its error when the run starts
        self.audio_tracks = overfilm_subs::media::audio_tracks(&p).unwrap_or_default();
        self.audio_track = 0;
        self.sound_source = overfilm_subs::media::sound_source(&p).ok();
        self.skip_silence = None;
        self.input = Some(p.clone());
        self.output = None;
        self.enroll_file = Some(p);
        self.page = Page::New;
        self.step = Step::Prep;
        self.status = None;
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Some(o) = self.options() else { return };
        self.save_prefs();
        {
            let mut s = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            *s = Shared { running: true, message: "시작".into(), ..Shared::default() };
        }
        self.cancel.store(false, Ordering::Relaxed);
        self.started = Some(Instant::now());
        self.stage_clock = None;
        self.step = Step::Running;
        self.status = None;
        let (shared, cancel, ctx) = (self.shared.clone(), self.cancel.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = job::guarded(|| {
                job::run(&o, &mut |f, m| {
                    let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
                    s.progress = f;
                    s.message = m.to_string();
                    ctx.request_repaint();
                    !cancel.load(Ordering::Relaxed)
                })
            });
            let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
            s.running = false;
            s.result = Some(r);
            ctx.request_repaint();
        });
    }

    fn finished(&mut self, done: job::Done) {
        self.models_ready = true;
        self.prefs.last_device = done.device.clone();
        if done.audio_seconds > 0 {
            self.prefs.secs_per_audio_min = done.seconds_taken as f32 * 60.0 / done.audio_seconds as f32;
        }
        self.save_prefs();
        let options = self.options().unwrap_or_else(|| self.prefs.options.clone());
        let summary = format!(
            "{} 분량을 {}에 만들었습니다 ({}). 단어집 교정 {}곳{}.",
            ui::widgets::duration(done.audio_seconds as f64),
            ui::widgets::duration(done.seconds_taken as f64),
            done.device,
            done.corrections.len(),
            if done.voice_lines > 0 { format!(", 게임 대사 {}줄 찾음", done.voice_lines) } else { String::new() }
        );
        let note = (!done.note.is_empty()).then(|| done.note.clone());
        match self.open_result(&done.srt, self.input.clone(), options) {
            Ok(()) => {
                if let Some(r) = &mut self.review {
                    let summary = match note {
                        Some(n) => format!("{summary} {n}"),
                        None => summary,
                    };
                    r.summary = Some(if done.stages.is_empty() { summary } else { format!("{summary}\n단계별 시간: {}", done.stages) });
                }
            }
            Err(e) => self.status = Some(Err(e)),
        }
    }

    /// Open a result for review (after a run, or from the recent list).
    fn open_result(&mut self, srt: &Path, input: Option<PathBuf>, options: job::Options) -> Result<(), String> {
        let opened = Opened::load(srt)?;
        let mut r = Review {
            opened,
            input: input.clone(),
            options,
            issues: Vec::new(),
            ignored: HashSet::new(),
            filter: None,
            selected: None,
            draft: String::new(),
            renaming: None,
            jump: false,
            undo: Vec::new(),
            summary: None,
            exporting: false,
            find: FindBar::default(),
            history: None,
            learned: Vec::new(),
            sel_end: None,
            redo_terms: String::new(),
            redo: None,
        };
        r.refresh_issues(self.prefs.options.mask_profanity);
        let entry = Recent {
            input: input.unwrap_or_else(|| srt.with_extension("")),
            srt: srt.to_path_buf(),
            preset: self.prefs.preset,
            captions: r.opened.captions.len(),
            people: r.opened.transcript.as_ref().map_or(0, overfilm_subs::speakers::people),
            open_issues: r.open_issues(),
        };
        if let Err(e) = edit::push_recent(&recent_path(), &mut self.recent, entry) {
            self.status = Some(Err(format!("최근 목록을 저장하지 못했습니다: {e}")));
        }
        self.review = Some(r);
        self.page = Page::New;
        self.step = Step::Review;
        Ok(())
    }

    fn start_enroll(&mut self, ctx: &egui::Context) {
        let Some(file) = self.enroll_file.clone() else {
            self.status = Some(Err("목소리를 들을 파일을 고르세요".into()));
            return;
        };
        let parse = |s: &str| {
            if s.trim().is_empty() { Ok(None) } else { voices::parse_time(s).map(Some).ok_or(format!("시간 형식이 아닙니다: {s} (예: 1:23)")) }
        };
        let (from, to) = match (parse(&self.enroll_from), parse(&self.enroll_to)) {
            (Ok(f), Ok(t)) => (f, t),
            (Err(e), _) | (_, Err(e)) => {
                self.status = Some(Err(e));
                return;
            }
        };
        let name = self.enroll_name.clone();
        let (out, busy, ctx) = (self.enroll.clone(), self.enrolling.clone(), ctx.clone());
        busy.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let r = job::guarded(|| voices::enroll(&name, &file, from, to)).map(|v| format!("목소리 '{}'를 등록했습니다 ({:.0}초 샘플)", v.name, v.seconds));
            *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
            busy.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }

    fn download_models(&mut self, ctx: &egui::Context) {
        let (state, ctx) = (self.models_dl.clone(), ctx.clone());
        *state.lock().unwrap_or_else(PoisonError::into_inner) = Some((0.0, "시작".into(), None));
        std::thread::spawn(move || {
            let r = job::guarded(|| {
                job::ensure_models(&job::models_dir(), &mut |f, m| {
                    if let Some(s) = state.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                        s.0 = f;
                        s.1 = m.to_string();
                    }
                    ctx.request_repaint();
                    true
                })
            });
            if let Some(s) = state.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                s.2 = Some(r);
            }
            ctx.request_repaint();
        });
    }

    /// A run ended: tell the user (notification, taskbar flash) and start the sleep countdown
    /// when asked. A cancelled run does neither: the user is at the desk.
    fn announce(&mut self, ctx: &egui::Context, ok: bool, text: &str) {
        let name = self.input.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if self.prefs.notify_done {
            let title = if ok { "자막을 다 만들었습니다" } else { "자막을 만들지 못했습니다" };
            let _ = notify::notify(title, &format!("{name}\n{text}"));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        if self.prefs.after_done == notify::After::Sleep {
            self.sleep_at = Some(Instant::now() + std::time::Duration::from_secs(notify::SLEEP_COUNTDOWN_SECS));
        }
    }

    /// Collect finished background work.
    fn poll(&mut self, ctx: &egui::Context) {
        let result = self.shared.lock().unwrap_or_else(PoisonError::into_inner).result.take();
        match result {
            Some(Ok(done)) => {
                let text = format!("자막 {}개", done.captions);
                self.finished(done);
                self.announce(ctx, true, &text);
            }
            Some(Err(e)) => {
                self.step = if self.input.is_some() { Step::Prep } else { Step::Empty };
                if !self.cancel.load(Ordering::Relaxed) {
                    self.announce(ctx, false, &e);
                }
                self.status = Some(Err(e));
            }
            None => {}
        }
        if let Some(r) = self.enroll.lock().unwrap_or_else(PoisonError::into_inner).take() {
            if r.is_ok() {
                self.enroll_name.clear();
                self.enroll_from.clear();
                self.enroll_to.clear();
            }
            self.status = Some(r);
            self.voices = voices::list();
        }
        let dl = self.models_dl.lock().unwrap_or_else(PoisonError::into_inner).as_ref().and_then(|s| s.2.clone());
        if let Some(r) = dl {
            *self.models_dl.lock().unwrap_or_else(PoisonError::into_inner) = None;
            match r {
                Ok(()) => self.models_ready = true,
                Err(e) => self.status = Some(Err(e)),
            }
        }
    }
}

impl Review {
    fn refresh_issues(&mut self, mask: bool) {
        let caps = &self.opened.captions;
        let mut issues = match pack::glossary(Some(mcp::user_glossary_dir().as_path()).filter(|d| d.is_dir()), mask) {
            Ok(g) => review::review(caps, &self.options.layout(), &pack::corrector(&g)),
            Err(_) => Vec::new(),
        };
        // talking over each other: captions labelled with two names
        for (k, (who, _)) in edit::caption_speakers(caps).iter().enumerate() {
            if who.as_deref().is_some_and(|w| w.contains('·')) && caps.get(k).is_some_and(|c| c.text().trim_start().starts_with('(')) {
                issues.push(review::Issue {
                    index: k + 1, kind: "speech_overlap", message: "두 사람이 동시에 말한 곳입니다".into(), suggestion: None
                });
            }
        }
        if let Some(t) = &self.opened.transcript {
            issues.extend(review::unsure(caps, t, review::UNSURE_BELOW));
        }
        issues.sort_by_key(|i| i.index);
        self.issues = issues;
    }

    fn open_issues(&self) -> usize {
        self.issues.iter().filter(|i| !self.ignored.contains(&(i.index, i.kind))).count()
    }

    fn remember(&mut self) {
        self.undo.push((self.opened.captions.clone(), self.opened.transcript.clone()));
        if self.undo.len() > 30 {
            self.undo.remove(0);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.poll(&ctx);
        let running = self.shared.lock().unwrap_or_else(PoisonError::into_inner).running;
        if !running && let Some(p) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf())) {
            self.choose_file(p);
        }
        ui::shell(self, root, &ctx);
    }
}

/// An installed Hangul face for the interface (Malgun Gothic on Windows); none is bundled.
fn korean_font() -> Option<egui::FontData> {
    let preferred = ["Malgun Gothic", "Apple SD Gothic Neo", "Noto Sans CJK KR", "Noto Sans KR", "NanumGothic", "Noto Sans CJK JP"];
    if let Some(f) = filmcraft_text::fonts::CRAFT_FONTS.iter().find(|f| f.scripts.contains(&"Kore")) {
        return Some(egui::FontData::from_static(f.bytes));
    }
    filmcraft_text::fonts::scan_system();
    let faces: Vec<_> = filmcraft_text::fonts::all_faces().into_iter().filter(|f| f.info.origin == "system" && !f.info.italic).collect();
    let covers = |f: &&std::sync::Arc<filmcraft_text::fonts::Face>| f.covers_text("자막추출한국어");
    let face = preferred
        .iter()
        .find_map(|n| faces.iter().filter(|f| f.info.family.eq_ignore_ascii_case(n)).filter(covers).min_by_key(|f| f.info.weight.abs_diff(400)))
        .or_else(|| faces.iter().filter(covers).min_by_key(|f| f.info.weight.abs_diff(400)))?;
    Some(egui::FontData::from_owned(face.data()?).tweak(egui::FontTweak::default())).map(|mut d| {
        d.index = face.info.index;
        d
    })
}

/// "Segoe UI Variable" (or "Segoe UI") for Latin text, as in the Windows apps, when installed.
fn latin_font() -> Option<egui::FontData> {
    let faces: Vec<_> = filmcraft_text::fonts::all_faces().into_iter().filter(|f| f.info.origin == "system" && !f.info.italic).collect();
    let face = ["Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI"]
        .iter()
        .find_map(|n| faces.iter().filter(|f| f.info.family.eq_ignore_ascii_case(n)).min_by_key(|f| f.info.weight.abs_diff(400)))?;
    let mut d = egui::FontData::from_owned(face.data()?);
    d.index = face.info.index;
    Some(d)
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("OverFilm 자막 추출")
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_drag_and_drop(true)
            // The OverFilm icon (the editor's, `assets/app-icon/`): window, taskbar and Alt-Tab.
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.filmcraft.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "OverFilm 자막 추출",
        opts,
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();
            let korean = korean_font();
            if let Some(f) = latin_font() {
                fonts.font_data.insert("latin".into(), std::sync::Arc::new(f));
                fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "latin".into());
            }
            if let Some(f) = korean {
                fonts.font_data.insert("korean".into(), std::sync::Arc::new(f));
                for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(fam).or_default().push("korean".into());
                }
            }
            cc.egui_ctx.set_fonts(fonts);
            ui::theme::install(&cc.egui_ctx);
            let mut app = App::new();
            // A file given on the command line (Windows "Open with", a file dropped on the exe):
            // an SRT opens for review, anything else is the input of a new run.
            if let Some(arg) = std::env::args_os().nth(1).map(PathBuf::from).filter(|p| p.is_file()) {
                if arg.extension().is_some_and(|e| e.eq_ignore_ascii_case("srt")) {
                    let o = app.prefs.options.clone();
                    if let Err(e) = app.open_result(&arg, None, o) {
                        app.status = Some(Err(e));
                    }
                } else {
                    app.choose_file(arg);
                }
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_caption_defaults_give_way_once() {
        let mut old = serde_json::to_value(Prefs::default()).unwrap();
        old["options"]["line_chars"] = 16.into();
        old["options"]["max_lines"] = 2.into();
        old.as_object_mut().unwrap().remove("caption_shape");
        let p = serde_json::from_value::<Prefs>(old.clone()).unwrap().upgrade();
        assert_eq!((p.options.line_chars, p.options.max_lines, p.caption_shape), (25, 1, CAPTION_SHAPE));
        // a shape the user picked stays
        old["options"]["line_chars"] = 20.into();
        let p = serde_json::from_value::<Prefs>(old.clone()).unwrap().upgrade();
        assert_eq!((p.options.line_chars, p.options.max_lines), (20, 2));
        // and two lines of 16 picked after the change stay too
        old["options"]["line_chars"] = 16.into();
        old["caption_shape"] = CAPTION_SHAPE.into();
        let p = serde_json::from_value::<Prefs>(old).unwrap().upgrade();
        assert_eq!((p.options.line_chars, p.options.max_lines), (16, 2));
        assert_eq!((Prefs::default().options.line_chars, Prefs::default().options.max_lines), (25, 1));
    }
}

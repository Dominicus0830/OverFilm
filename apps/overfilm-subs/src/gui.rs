//! `OverFilmSubs`: the window. Pick a video or audio file (or drop it on the window), choose the
//! kind of video, press the button; the SRT is saved next to the file.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]
#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use eframe::egui;
use overfilm_subs::speakers::GameVoice;
use overfilm_subs::{MEDIA_EXTENSIONS, job, mcp, pack, review};

#[derive(Default)]
struct Shared {
    running: bool,
    progress: f32,
    message: String,
    result: Option<Result<job::Done, String>>,
}

struct App {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    mode: pack::Mode,
    terms: String,
    mask: bool,
    digits: bool,
    gpu: bool,
    line_chars: u32,
    two_lines: bool,
    speakers: bool,
    /// 0 = count automatically
    num_speakers: usize,
    game_voice: GameVoice,
    per_speaker: bool,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    issues: Vec<(String, String)>,
    preview: Vec<String>,
    models_ready: bool,
    show_mcp: bool,
    status: Option<Result<String, String>>,
}

impl App {
    fn new() -> Self {
        let dir = job::models_dir();
        Self {
            input: None,
            output: None,
            mode: pack::Mode::General,
            terms: String::new(),
            mask: true,
            digits: true,
            gpu: true,
            line_chars: 16,
            two_lines: true,
            speakers: true,
            num_speakers: 0,
            game_voice: GameVoice::Drop,
            per_speaker: false,
            shared: Arc::default(),
            cancel: Arc::default(),
            issues: Vec::new(),
            preview: Vec::new(),
            models_ready: overfilm_asr::catalogue::all().iter().all(|m| m.installed(&dir)),
            show_mcp: false,
            status: None,
        }
    }

    fn options(&self) -> Option<job::Options> {
        let mut o = job::Options::new(self.input.clone()?);
        o.output = self.output.clone();
        o.mode = self.mode;
        o.extra_terms = self.terms.clone();
        o.mask_profanity = self.mask;
        o.digits = self.digits;
        o.gpu = self.gpu;
        o.line_chars = self.line_chars;
        o.max_lines = if self.two_lines { 2 } else { 1 };
        o.speakers = self.speakers;
        o.num_speakers = (self.num_speakers > 0).then_some(self.num_speakers);
        o.game_voice = self.game_voice;
        o.per_speaker = self.per_speaker;
        let d = mcp::user_glossary_dir();
        o.glossary_dir = d.is_dir().then_some(d);
        Some(o)
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Some(o) = self.options() else { return };
        {
            let mut s = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            *s = Shared { running: true, message: "시작".into(), ..Shared::default() };
        }
        self.issues.clear();
        self.preview.clear();
        self.cancel.store(false, Ordering::Relaxed);
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

    /// After a run: preview lines and review findings from the written SRT.
    fn load_result(&mut self, srt: &std::path::Path) {
        let Ok(caps) = job::read_srt(srt) else { return };
        self.preview = caps.iter().take(200).map(|c| format!("{}  {}", review::clock(c.start_ms), c.text().replace('\n', " / "))).collect();
        if let Ok(g) = pack::glossary(None, self.mask) {
            let o = overfilm_captions::Layout { max_line: self.line_chars * 2, max_lines: if self.two_lines { 2 } else { 1 }, ..Default::default() };
            self.issues = review::review(&caps, &o, &pack::corrector(&g))
                .into_iter()
                .take(300)
                .map(|i| {
                    (format!("#{} {}", i.index, i.message), caps.get(i.index.saturating_sub(1)).map(|c| c.text().replace('\n', " / ")).unwrap_or_default())
                })
                .collect();
        }
        self.models_ready = true;
    }

    fn file_row(&mut self, ui: &mut egui::Ui, running: bool) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("📂 영상/오디오 파일 열기")).clicked()
                && let Some(p) = rfd::FileDialog::new().add_filter("영상·오디오", MEDIA_EXTENSIONS).add_filter("모든 파일", &["*"]).pick_file()
            {
                self.input = Some(p);
                self.output = None;
            }
            match &self.input {
                Some(p) => ui.label(p.display().to_string()),
                None => ui.weak("파일을 고르거나 이 창에 끌어다 놓으세요"),
            };
        });
        if let Some(o) = self.options() {
            ui.horizontal(|ui| {
                ui.label(format!("저장: {}", o.srt_path().display()));
                if ui.add_enabled(!running, egui::Button::new("다른 곳에 저장…")).clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .add_filter("SRT 자막", &["srt"])
                        .set_file_name(o.srt_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                        .save_file()
                {
                    self.output = Some(p);
                }
            });
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("영상 종류");
            for m in [pack::Mode::General, pack::Mode::Commentary] {
                ui.radio_value(&mut self.mode, m, m.label());
            }
        });
        ui.horizontal(|ui| {
            ui.label("추가 이름·용어");
            ui.add(egui::TextEdit::singleline(&mut self.terms).hint_text("예: 스트리머 이름, 팀 이름 (쉼표로 구분)").desired_width(f32::INFINITY));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.mask, "욕설은 초성으로 (씨발 → ㅅㅂ)");
            ui.checkbox(&mut self.digits, "숫자는 숫자로 (오 시즌 → 5시즌)");
            ui.separator();
            ui.label("한 줄");
            ui.add(egui::DragValue::new(&mut self.line_chars).range(8..=30).suffix("자"));
            ui.checkbox(&mut self.two_lines, "최대 두 줄");
            ui.separator();
            ui.checkbox(&mut self.gpu, "그래픽카드(GPU) 사용")
                .on_hover_text("AMD·NVIDIA·Intel 그래픽카드로 빠르게 처리합니다. 없거나 문제가 생기면 자동으로 CPU를 씁니다.");
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.speakers, "목소리 구별 (화자1, 화자2…)").on_hover_text(
                "말하는 사람이 둘 이상이면 몇 명인지 세고, 자막마다 누가 말했는지 (화자1)처럼 붙입니다. 자막 하나에 두 사람 말이 섞이지 않습니다.",
            );
            ui.add_enabled_ui(self.speakers, |ui| {
                ui.label("사람 수");
                ui.add(
                    egui::DragValue::new(&mut self.num_speakers)
                        .range(0..=20)
                        .custom_formatter(|v, _| if v < 0.5 { "자동".into() } else { format!("{v:.0}명") }),
                )
                .on_hover_text("알면 정해 주세요. 자동이면 목소리로 셉니다.");
                ui.checkbox(&mut self.per_speaker, "사람마다 SRT 따로 저장");
            });
            ui.separator();
            ui.label("게임 음성(영웅 대사)");
            for g in [GameVoice::Drop, GameVoice::Label, GameVoice::Keep] {
                ui.radio_value(&mut self.game_voice, g, g.label());
            }
        });
    }

    fn results(&mut self, ui: &mut egui::Ui) {
        if self.preview.is_empty() {
            return;
        }
        ui.separator();
        ui.columns(2, |cols| {
            if let Some(c) = cols.get_mut(0) {
                c.strong(format!("자막 미리보기 ({}개 중 앞부분)", self.preview.len()));
                egui::ScrollArea::vertical().id_salt("preview").max_height(f32::INFINITY).show(c, |ui| {
                    for l in &self.preview {
                        ui.label(l);
                    }
                });
            }
            if let Some(c) = cols.get_mut(1) {
                c.strong(format!("검토할 곳 {}개", self.issues.len()));
                egui::ScrollArea::vertical().id_salt("issues").show(c, |ui| {
                    for (m, t) in &self.issues {
                        ui.label(egui::RichText::new(m).color(ui.visuals().warn_fg_color));
                        ui.weak(t);
                    }
                });
            }
        });
    }

    fn mcp_help(&mut self, ui: &mut egui::Ui) {
        let cli = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.join(if cfg!(windows) { "overfilm-subs.exe" } else { "overfilm-subs" })))
            .unwrap_or_default();
        let cfg = serde_json::json!({"mcpServers": {"overfilm-subs": {"command": cli, "args": ["mcp"]}}});
        let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
        ui.label("Claude Desktop의 설정 → 개발자 → 설정 편집(claude_desktop_config.json)에 아래를 넣고 다시 시작하면, 대화에서 자막 추출과 검토(오인식·욕설·읽기 속도·무음 구간 찾기, 고치기)를 시킬 수 있습니다.");
        ui.add(egui::TextEdit::multiline(&mut text.as_str()).code_editor().desired_width(f32::INFINITY));
        if ui.button("복사").clicked() {
            ui.ctx().copy_text(text);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        let (running, progress, message, result) = {
            let mut s = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            (s.running, s.progress, s.message.clone(), s.result.take())
        };
        if let Some(r) = result {
            match r {
                Ok(done) => {
                    self.load_result(&done.srt);
                    self.status = Some(Ok(format!(
                        "완료: 자막 {}개를 {}에 저장했습니다. ({}분 {}초 분량, {}분 {}초 걸림, {}, 단어집 교정 {}곳)",
                        done.captions,
                        done.srt.display(),
                        done.audio_seconds / 60,
                        done.audio_seconds % 60,
                        done.seconds_taken / 60,
                        done.seconds_taken % 60,
                        done.device,
                        done.corrections.len()
                    ) + &{
                        let who = job::speaker_summary(&done);
                        if who.is_empty() { String::new() } else { format!("\n{who}") }
                    }));
                }
                Err(e) => self.status = Some(Err(e)),
            }
        }
        let status = self.status.clone();
        if !running && let Some(p) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf())) {
            self.input = Some(p);
            self.output = None;
        }
        egui::Frame::central_panel(root.style()).show(root, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.heading("OverFilm 자막 추출");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(&mut self.show_mcp, "MCP 연결");
                });
            });
            ui.weak("오버워치 영상의 말소리를 한국어 자막(SRT)으로 만듭니다. 모두 이 컴퓨터에서 처리합니다.");
            if self.show_mcp {
                ui.group(|ui| self.mcp_help(ui));
            }
            ui.add_space(6.0);
            self.file_row(ui, running);
            ui.add_enabled_ui(!running, |ui| self.settings(ui));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let can = self.input.is_some() && !running;
                if ui.add_enabled(can, egui::Button::new(egui::RichText::new("자막 만들기").strong()).min_size(egui::vec2(140.0, 32.0))).clicked() {
                    self.start(ctx);
                }
                if running && ui.button("취소").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                if !self.models_ready && !running {
                    ui.weak(format!(
                        "처음 한 번 음성 인식 모델(약 {:.1}GB)을 받습니다.",
                        overfilm_asr::catalogue::all().iter().map(|m| m.size()).sum::<u64>() as f64 / 1e9
                    ));
                }
            });
            if running {
                ui.add(egui::ProgressBar::new(progress).show_percentage().text(format!("{:.0}%  {message}", progress * 100.0)));
            }
            match &status {
                Some(Ok(m)) if !running => {
                    ui.colored_label(egui::Color32::from_rgb(60, 160, 90), m);
                }
                Some(Err(e)) if !running => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                _ => {}
            }
            self.results(ui);
        });
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

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("OverFilm 자막 추출").with_inner_size([860.0, 620.0]).with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "OverFilm 자막 추출",
        opts,
        Box::new(|cc| {
            if let Some(f) = korean_font() {
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert("korean".into(), std::sync::Arc::new(f));
                for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(fam).or_default().push("korean".into());
                }
                cc.egui_ctx.set_fonts(fonts);
            }
            cc.egui_ctx.set_zoom_factor(1.15);
            Ok(Box::new(App::new()))
        }),
    )
}

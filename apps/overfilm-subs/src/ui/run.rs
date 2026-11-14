//! 새 자막 › 진행: the stage list, elapsed and remaining time, cancel, the text heard so far and
//! the queue of files; after a batch, the list of results.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Instant;

use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke, vec2};
use overfilm_subs::edit::{STAGES, stage};
use overfilm_subs::live;
use overfilm_subs::queue::State;

use super::widgets::{self, caption, pal, title};
use crate::{App, Step};

/// Chunks shown under "방금 인식한 말" (more while that many are being written at once).
const LIVE_ROWS: usize = 4;
/// Characters shown of a chunk still being written: the words being heard now.
const LIVE_TAIL: usize = 120;
/// Characters shown of a finished chunk (its last sentences; the whole text is in the result).
const DONE_TAIL: usize = 240;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let (running, progress, message) = {
        let s = app.shared.lock().unwrap_or_else(PoisonError::into_inner);
        (s.running, s.progress, s.message.clone())
    };
    if !running {
        if app.queue.is_batch() && app.queue.finished() {
            summary(app, ui);
        } else {
            app.step = if app.input.is_some() { Step::Prep } else { Step::Empty };
        }
        return;
    }
    let (now_stage, within) = stage(progress);
    let now = Instant::now();
    match app.stage_clock {
        Some((s, _, _)) if s == now_stage => {}
        _ => app.stage_clock = Some((now_stage, now, within)),
    }
    let elapsed = app.started.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
    // Remaining time of the current stage from its own speed (the long one is recognition).
    let remaining = app.stage_clock.and_then(|(_, t0, f0)| {
        let spent = now.duration_since(t0).as_secs_f64();
        let done = f64::from(within - f0);
        (done > 0.02 && spent > 3.0).then(|| spent / done * f64::from(1.0 - within))
    });
    let cancelling = app.cancel.load(Ordering::Relaxed);
    super::scroll_page(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        let name = app.input.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        caption(ui, format!("새 자막 / {name}"));
        let batch = app.queue.is_batch();
        let more = app.queue.next_waiting().is_some();
        let heading = match app.queue.position() {
            _ if cancelling => "멈추는 중".to_string(),
            Some((k, n)) if n > 1 => format!("자막 만드는 중 ({k}/{n})"),
            _ => "자막 만드는 중".to_string(),
        };
        title(ui, &heading);
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                let label = STAGES.get(now_stage).map_or("", |s| s.0);
                ui.label(RichText::new(format!("{label} {:.0}%", within * 100.0)).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let rest = remaining.map(|r| format!(" · 이 단계 약 {} 남음", widgets::duration(r))).unwrap_or_default();
                    caption(ui, format!("{} 지남{rest}", widgets::duration(elapsed)));
                });
            });
            widgets::progress_bar(ui, progress);
            ui.horizontal(|ui| {
                caption(ui, &message);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let all = if batch && more { "모두 취소" } else { "취소" };
                    if ui.add_enabled(!cancelling, egui::Button::new(all).min_size(vec2(0.0, 32.0))).clicked() {
                        app.stop_all = true;
                        app.cancel.store(true, Ordering::Relaxed);
                    }
                    if more && ui.add_enabled(!cancelling, egui::Button::new("이 파일 건너뛰기").min_size(vec2(0.0, 32.0))).clicked() {
                        app.cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
        });
        // the text first: it is what tells early whether a name keeps coming out wrong
        heard(app, ui, now_stage);
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        let skip_speakers = !app.prefs.options.speakers;
        for (k, (label, _)) in STAGES.iter().enumerate() {
            let skipped = k == 3 && skip_speakers;
            widgets::card(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    stage_mark(ui, rect, k.cmp(&now_stage), skipped);
                    ui.add_space(6.0);
                    let p = pal(ui);
                    let color = if k > now_stage || skipped { p.text2 } else { p.text };
                    ui.label(RichText::new(*label).color(color));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let note = match k.cmp(&now_stage) {
                            _ if skipped => "끔".to_string(),
                            std::cmp::Ordering::Less => "끝".to_string(),
                            std::cmp::Ordering::Equal => format!("{:.0}%", within * 100.0),
                            std::cmp::Ordering::Greater => "대기".to_string(),
                        };
                        caption(ui, note);
                    });
                });
            });
        }
        if batch {
            queue_list(app, ui, true);
        }
        ui.add_space(8.0);
        caption(ui, "창을 줄여 두어도 계속 진행합니다. 다른 파일을 끌어다 놓으면 대기열 끝에 들어갑니다.");
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
}

fn stage_mark(ui: &mut egui::Ui, rect: egui::Rect, order: std::cmp::Ordering, skipped: bool) {
    let p = pal(ui);
    let c = rect.center();
    match order {
        _ if skipped => {
            ui.painter().circle_stroke(c, 8.0, Stroke::new(1.0, p.text3));
        }
        std::cmp::Ordering::Less => {
            ui.painter().circle_filled(c, 9.0, p.ok);
            let s = Stroke::new(1.6, p.card);
            ui.painter().line_segment([c + vec2(-4.0, 0.0), c + vec2(-1.0, 3.0)], s);
            ui.painter().line_segment([c + vec2(-1.0, 3.0), c + vec2(4.5, -3.0)], s);
        }
        std::cmp::Ordering::Equal => {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            child.add(egui::Spinner::new().size(16.0).color(p.accent));
        }
        std::cmp::Ordering::Greater => {
            for s in egui::Shape::dashed_line(&circle(c, 8.0), Stroke::new(1.0, p.text3), 2.0, 2.5) {
                ui.painter().add(s);
            }
        }
    }
}

fn circle(c: egui::Pos2, r: f32) -> Vec<egui::Pos2> {
    (0..=32).map(|k| c + egui::Vec2::angled(std::f32::consts::TAU * k as f32 / 32.0) * r).collect()
}

/// "방금 인식한 말": each chunk's text while the recogniser writes it, so a name it keeps getting
/// wrong shows early (cancel, add it to the terms, start again).
fn heard(app: &App, ui: &mut egui::Ui, now_stage: usize) {
    let rows: Vec<live::LiveChunk> = app.live.lock().unwrap_or_else(PoisonError::into_inner).recent(LIVE_ROWS).into_iter().cloned().collect();
    if rows.is_empty() && now_stage != 2 {
        return;
    }
    super::widgets::section(ui, "방금 인식한 말");
    ui.spacing_mut().item_spacing.y = 6.0;
    widgets::card(ui, |ui| {
        let p = pal(ui);
        if rows.is_empty() {
            caption(ui, "첫 조각(최대 1분)을 인식하는 동안 여기에 글자가 나옵니다.");
            return;
        }
        for c in &rows {
            ui.horizontal_top(|ui| {
                let time = RichText::new(live::clock(c.start_ms)).monospace().small().color(p.text3);
                ui.add_sized(vec2(56.0, 18.0), egui::Label::new(time));
                let (text, color) = if c.done { (live::tail(&c.text, DONE_TAIL), p.text) } else { (live::tail(&c.text, LIVE_TAIL), p.text2) };
                ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
            });
        }
        caption(ui, "흐린 글자는 아직 인식 중이고, 단어집 교정 전입니다.");
    });
}

/// The files of the batch with where each one is; finished ones open for review.
fn queue_list(app: &mut App, ui: &mut egui::Ui, can_add: bool) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("대기열 {}개", app.queue.items.len())).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if can_add
                && widgets::subtle_button(ui, "파일 추가").clicked()
                && let Some(files) = rfd::FileDialog::new().add_filter("영상·오디오", overfilm_subs::MEDIA_EXTENSIONS).pick_files()
            {
                app.add_files(files);
            }
        });
    });
    super::status_bar(app, ui);
    let progress = app.shared.lock().unwrap_or_else(PoisonError::into_inner).progress;
    let (mut remove, mut open) = (None, None);
    for (k, it) in app.queue.items.iter().enumerate() {
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                let p = pal(ui);
                match &it.state {
                    State::Running => stage_mark(ui, rect, std::cmp::Ordering::Equal, false),
                    State::Done { .. } => stage_mark(ui, rect, std::cmp::Ordering::Less, false),
                    State::Waiting => stage_mark(ui, rect, std::cmp::Ordering::Greater, false),
                    State::Failed(_) => {
                        ui.painter().circle_filled(rect.center(), 9.0, p.bad);
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(12.0), p.card);
                    }
                    State::Skipped => stage_mark(ui, rect, std::cmp::Ordering::Greater, true),
                }
                ui.add_space(6.0);
                let right = 220.0_f32.min(ui.available_width() * 0.45);
                ui.allocate_ui_with_layout(vec2(ui.available_width() - right, 20.0), Layout::left_to_right(Align::Center), |ui| {
                    let color = if matches!(it.state, State::Waiting | State::Skipped) { p.text2 } else { p.text };
                    ui.add(egui::Label::new(RichText::new(it.name()).color(color)).truncate()).on_hover_text(it.input.display().to_string());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &it.state {
                    State::Waiting => {
                        if widgets::subtle_button(ui, "빼기").clicked() {
                            remove = Some(k);
                        }
                        caption(ui, "대기");
                    }
                    State::Running => {
                        caption(ui, format!("{:.0}%", progress.clamp(0.0, 1.0) * 100.0));
                    }
                    State::Done { captions, seconds_taken, .. } => {
                        if widgets::button(ui, "검토").clicked() {
                            open = Some(k);
                        }
                        caption(ui, format!("자막 {captions}개 · {}", widgets::duration(*seconds_taken as f64)));
                    }
                    State::Failed(e) => {
                        ui.label(RichText::new("실패").small().color(p.bad)).on_hover_text(e);
                    }
                    State::Skipped => {
                        caption(ui, "건너뜀");
                    }
                });
            });
        });
    }
    if let Some(k) = remove {
        app.queue.remove(k);
    }
    if let Some(k) = open {
        app.open_queued(k);
    }
}

/// After a batch: what came of each file.
fn summary(app: &mut App, ui: &mut egui::Ui) {
    super::scroll_page(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        caption(ui, "새 자막 / 대기열");
        let (done, failed, skipped) = app.queue.counts();
        title(ui, if failed == 0 && skipped == 0 { "대기열을 다 처리했습니다" } else { "대기열이 끝났습니다" });
        let mut line = format!("파일 {}개 중 {done}개의 자막을 만들었습니다", app.queue.items.len());
        if failed > 0 {
            line.push_str(&format!(", {failed}개는 실패했습니다(이름에 마우스를 올리면 까닭이 보입니다)"));
        }
        if skipped > 0 {
            line.push_str(&format!(", {skipped}개는 건너뛰었습니다"));
        }
        caption(ui, format!("{line}. 모두 '최근'에도 들어 있습니다."));
        // finished: new files start a new batch from 새 자막 (adding here would start nothing)
        queue_list(app, ui, false);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if widgets::accent_button(ui, "새 자막").clicked() {
                app.queue.items.clear();
                app.input = None;
                app.step = Step::Empty;
                app.status = None;
            }
            let failed_or_skipped: Vec<std::path::PathBuf> =
                app.queue.items.iter().filter(|i| matches!(i.state, State::Failed(_) | State::Skipped)).map(|i| i.input.clone()).collect();
            if !failed_or_skipped.is_empty() && widgets::button(ui, "남은 파일 다시 준비").clicked() {
                app.queue.items.clear();
                app.input = None;
                app.choose_files(failed_or_skipped);
            }
        });
    });
}

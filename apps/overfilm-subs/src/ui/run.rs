//! 새 자막 › 진행: the stage list, elapsed and remaining time, cancel.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Instant;

use eframe::egui::{self, Align, Layout, RichText, Sense, Stroke, vec2};
use overfilm_subs::edit::{STAGES, stage};

use super::widgets::{self, caption, pal, title};
use crate::App;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let (progress, message) = {
        let s = app.shared.lock().unwrap_or_else(PoisonError::into_inner);
        (s.progress, s.message.clone())
    };
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
        title(ui, if cancelling { "멈추는 중" } else { "자막 만드는 중" });
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
                    if ui.add_enabled(!cancelling, egui::Button::new("취소").min_size(vec2(0.0, 32.0))).clicked() {
                        app.cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
        });
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
        ui.add_space(8.0);
        caption(ui, "창을 줄여 두어도 계속 진행합니다.");
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

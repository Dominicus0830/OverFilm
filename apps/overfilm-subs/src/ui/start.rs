//! 새 자막 (empty: drop a file) and 최근.

use eframe::egui::{self, Align, Layout, RichText, Sense, vec2};
use overfilm_subs::MEDIA_EXTENSIONS;

use super::widgets::{self, Severity, accent_button, caption, pal, section, title};
use crate::App;

pub fn new_page(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    super::scroll_page(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        title(ui, "새 자막");
        models_bar(app, ui, ctx);
        super::status_bar(app, ui);
        drop_area(app, ui);
        if !app.recent.is_empty() {
            section(ui, "최근");
            recent_list(app, ui, 5);
        }
    });
}

pub fn recent_page(app: &mut App, ui: &mut egui::Ui) {
    super::scroll_page(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        title(ui, "최근");
        super::status_bar(app, ui);
        if app.recent.is_empty() {
            caption(ui, "아직 만든 자막이 없습니다. 새 자막에서 파일을 넣으면 여기에 쌓입니다.");
        } else {
            recent_list(app, ui, edit_max());
        }
    });
}

fn edit_max() -> usize {
    overfilm_subs::edit::RECENT_MAX
}

fn models_bar(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    if app.models_ready {
        return;
    }
    let dl = app.models_dl.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().map(|s| (s.0, s.1.clone()));
    match dl {
        Some((f, m)) => {
            widgets::info_bar(ui, Severity::Info, "모델을 받는 중입니다.", &m, |ui| {
                ui.label(format!("{:.0}%", f * 100.0));
            });
            widgets::progress_bar(ui, f);
        }
        None => {
            let gb = overfilm_asr::catalogue::all().iter().map(|m| m.size()).sum::<u64>() as f64 / 1e9;
            let mut go = false;
            widgets::info_bar(
                ui,
                Severity::Warn,
                "음성 인식 모델이 필요합니다.",
                &format!("처음 한 번 {gb:.1}GB를 받습니다. 미리 받아 두면 첫 자막을 바로 만들 수 있습니다."),
                |ui| go = widgets::button(ui, "지금 받기").clicked(),
            );
            if go {
                app.download_models(ctx);
            }
        }
    }
}

fn drop_area(app: &mut App, ui: &mut egui::Ui) {
    let p = pal(ui);
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let h = 220.0;
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    ui.painter().rect_filled(rect, egui::CornerRadius::same(super::theme::OVERLAY_RADIUS), if hovering { p.accent_soft } else { p.card });
    widgets::dashed_rect(ui, rect.shrink(0.5), if hovering { p.accent } else { p.text3 });
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(16.0)).layout(Layout::top_down(Align::Center)));
    child.add_space(54.0);
    child.label(RichText::new("영상이나 오디오 파일을 여기에 놓으세요").strong());
    child.label(RichText::new("mp4, mov, mkv, webm, mp3, m4a, wav 등. 소리는 이 컴퓨터 밖으로 나가지 않습니다.").small().color(p.text2));
    child.add_space(10.0);
    let pick = accent_button(&mut child, "파일 선택").clicked();
    child.label(RichText::new("여러 개를 고르거나 놓으면 같은 설정으로 차례로 만듭니다").small().color(p.text3));
    if (pick || r.clicked())
        && let Some(paths) = rfd::FileDialog::new().add_filter("영상·오디오", MEDIA_EXTENSIONS).add_filter("모든 파일", &["*"]).pick_files()
    {
        app.choose_files(paths);
    }
}

fn recent_list(app: &mut App, ui: &mut egui::Ui, max: usize) {
    let p = pal(ui);
    let mut open = None;
    let mut forget = None;
    egui::Frame::new().fill(p.card).stroke(egui::Stroke::new(1.0, p.stroke)).corner_radius(egui::CornerRadius::same(super::theme::CONTROL_RADIUS)).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (k, r) in app.recent.iter().take(max).enumerate() {
                if k > 0 {
                    let y = ui.cursor().top();
                    ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, p.stroke));
                }
                let exists = r.srt.is_file();
                let resp = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let name = r.input.file_name().or_else(|| r.srt.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.add(egui::Label::new(name).truncate()).on_hover_text(r.srt.display().to_string());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if !exists {
                                    caption(ui, "파일이 없습니다");
                                } else if r.open_issues > 0 {
                                    ui.label(RichText::new(format!("검토 {}곳 남음", r.open_issues)).small().color(p.warn));
                                } else {
                                    caption(ui, "검토 끝");
                                }
                                ui.add_space(16.0);
                                let who = if r.people > 1 { format!(" · 화자 {}명", r.people) } else { String::new() };
                                caption(ui, format!("자막 {}개{who}", r.captions));
                                ui.add_space(16.0);
                                caption(ui, r.preset.label());
                            });
                        });
                    })
                    .response;
                let resp = ui.interact(resp.rect, ui.scope_id().with(("recent", k)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(resp.rect, 0.0, egui::Color32::from_black_alpha(8));
                }
                if resp.clicked() {
                    open = Some(k);
                }
                resp.context_menu(|ui| {
                    if ui.button("목록에서 빼기").clicked() {
                        forget = Some(k);
                        ui.close();
                    }
                });
            }
        },
    );
    if let Some(k) = forget {
        if k < app.recent.len() {
            app.recent.remove(k);
        }
        if let Err(e) = overfilm_subs::edit::save_recent(&crate::recent_path(), &mut app.recent) {
            app.status = Some(Err(e));
        }
    }
    if let Some(k) = open
        && let Some(r) = app.recent.get(k).cloned()
    {
        if r.srt.is_file() {
            app.prefs.preset = r.preset;
            let mut o = app.prefs.options.clone();
            o.input = r.input.clone();
            if let Err(e) = app.open_result(&r.srt, Some(r.input.clone()).filter(|p| p.is_file()), o) {
                app.status = Some(Err(e));
            }
        } else if r.input.is_file() {
            app.choose_file(r.input);
        } else {
            app.status = Some(Err(format!("{}: 파일이 없습니다", r.srt.display())));
        }
    }
}

//! The window's screens. `shell` draws the navigation rail and the current page.

pub mod theme;
pub mod widgets;

mod glossary;
mod prep;
mod results;
mod run;
mod settings;
mod start;
mod voices;

use eframe::egui::{self, CornerRadius, Margin, Stroke};

use crate::{App, Page, Step};
use widgets::{Icon, nav_item, pal};

pub fn shell(app: &mut App, root: &mut egui::Ui, ctx: &egui::Context) {
    let p = pal(root);
    if let Some(at) = app.sleep_at {
        let left = at.saturating_duration_since(std::time::Instant::now()).as_secs();
        let mut choice = None;
        egui::Panel::top("sleep").frame(egui::Frame::new().fill(p.base).inner_margin(Margin::symmetric(12, 6))).show(root, |ui| {
            widgets::info_bar(
                ui,
                widgets::Severity::Info,
                &format!("{left}초 뒤 절전 모드로 들어갑니다."),
                "자막을 다 만들어서 설정대로 컴퓨터를 재웁니다.",
                |ui| {
                    if widgets::subtle_button(ui, "지금 절전").clicked() {
                        choice = Some(true);
                    }
                    if widgets::button(ui, "취소").clicked() {
                        choice = Some(false);
                    }
                },
            );
        });
        if left == 0 || choice == Some(true) {
            app.sleep_at = None;
            if let Err(e) = overfilm_subs::notify::sleep() {
                app.status = Some(Err(e));
            }
        } else if choice == Some(false) {
            app.sleep_at = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
    running_bar(app, root);
    // Review needs the width: the rail folds to icons there.
    let compact = app.page == Page::New && app.step == Step::Review;
    egui::Panel::left("nav")
        .exact_size(if compact { 52.0 } else { 220.0 })
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(p.base).inner_margin(Margin { left: 6, right: 6, top: 8, bottom: 8 }))
        .show(root, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut go = None;
            for (page, icon, label) in [
                (Page::New, Icon::Add, "새 자막"),
                (Page::Recent, Icon::Clock, "최근"),
                (Page::Voices, Icon::Mic, "목소리"),
                (Page::Glossary, Icon::Book, "단어집"),
            ] {
                if nav_item(ui, icon, label, app.page == page, compact) {
                    go = Some(page);
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                if nav_item(ui, Icon::Gear, "설정", app.page == Page::Settings, compact) {
                    go = Some(Page::Settings);
                }
            });
            if let Some(page) = go {
                app.page = page;
                app.status = None;
            }
        });
    let flush = app.page == Page::New && matches!(app.step, Step::Review | Step::Prep);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(p.layer)
                .stroke(Stroke::new(1.0, p.stroke))
                .corner_radius(CornerRadius { nw: theme::OVERLAY_RADIUS, ..CornerRadius::ZERO })
                .inner_margin(if flush { Margin::ZERO } else { Margin { left: 32, right: 32, top: 24, bottom: 16 } }),
        )
        .show(root, |ui| match app.page {
            Page::New => match app.step {
                Step::Empty => start::new_page(app, ui, ctx),
                Step::Prep => prep::page(app, ui, ctx),
                Step::Running => run::page(app, ui),
                Step::Review => results::page(app, ui, ctx),
            },
            Page::Recent => start::recent_page(app, ui),
            Page::Voices => voices::page(app, ui, ctx),
            Page::Glossary => glossary::page(app, ui),
            Page::Settings => settings::page(app, ui),
        });
}

/// While a run goes on and another page is open: where it is, and a way back to it.
fn running_bar(app: &mut App, root: &mut egui::Ui) {
    let (running, progress) = {
        let s = app.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        (s.running, s.progress)
    };
    if !running || (app.page == Page::New && app.step == Step::Running) {
        return;
    }
    let p = pal(root);
    let name = app.input.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let place = match app.queue.position() {
        Some((k, n)) if n > 1 => format!(" ({k}/{n})"),
        _ => String::new(),
    };
    let mut go = false;
    egui::Panel::top("running").frame(egui::Frame::new().fill(p.base).inner_margin(Margin::symmetric(12, 6))).show(root, |ui| {
        widgets::info_bar(ui, widgets::Severity::Info, &format!("자막 만드는 중{place} {:.0}%", progress.clamp(0.0, 1.0) * 100.0), &name, |ui| {
            go = widgets::button(ui, "진행 보기").clicked()
        });
    });
    if go {
        app.page = Page::New;
        app.step = Step::Running;
    }
    root.ctx().request_repaint_after(std::time::Duration::from_millis(1000));
}

/// The status message (last success or error) as an info bar.
pub fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    let mut close = false;
    match &app.status {
        Some(Ok(m)) => widgets::info_bar(ui, widgets::Severity::Ok, m, "", |ui| close = widgets::subtle_button(ui, "닫기").clicked()),
        Some(Err(e)) => {
            widgets::info_bar(ui, widgets::Severity::Error, "문제가 생겼습니다.", e, |ui| close = widgets::subtle_button(ui, "닫기").clicked())
        }
        None => {}
    }
    if close {
        app.status = None;
    }
}

/// Open the folder holding `path` in the file manager, with the file selected on Windows.
pub fn reveal(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    r.map(|_| ()).map_err(|e| format!("폴더를 열지 못했습니다: {e}"))
}

/// A scrolling page whose content keeps clear of the scroll bar.
pub fn scroll_page(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 0, right: 16, top: 0, bottom: 8 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    });
}

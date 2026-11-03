//! 목소리: the enrolled voices and the form to enrol one.

use std::sync::atomic::Ordering;

use eframe::egui::{self, RichText};
use overfilm_subs::voices;

use super::widgets::{self, accent_button, button, caption, section, setting, title};
use crate::App;

pub fn page(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    super::scroll_page(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        title(ui, "목소리");
        caption(
            ui,
            "스트리머·해설자처럼 자주 나오는 사람의 목소리를 등록해 두면, 다음 영상부터 '화자1' 대신 그 이름이 붙고 '등록한 목소리만 자막'을 쓸 수 있습니다. 소리는 저장하지 않고 목소리 특징만 저장합니다.",
        );
        ui.add_space(8.0);
        super::status_bar(app, ui);
        section(ui, &format!("등록한 목소리 {}명", app.voices.len()));
        if app.voices.is_empty() {
            widgets::card(ui, |ui| caption(ui, "아직 없습니다. 아래에서 등록하거나, 검토 화면에서 화자를 오른쪽 클릭해 '이 목소리 등록…'을 고르세요."));
        }
        let mut remove = None;
        for v in &app.voices {
            let desc = format!("{:.0}초 샘플 · {}", v.seconds, v.source);
            setting(ui, &v.name, &desc, false, |ui| {
                if button(ui, "지우기").clicked() {
                    remove = Some(v.name.clone());
                }
            });
        }
        if let Some(name) = remove {
            app.status = Some(voices::remove_in(&voices::dir(), &name).map(|()| format!("목소리 '{name}'를 지웠습니다.")));
            app.voices = voices::list();
        }
        section(ui, "목소리 등록");
        let busy = app.enrolling.load(Ordering::Relaxed);
        let file_label = app.enroll_file.as_ref().and_then(|p| p.file_name()).map_or("고르지 않음".to_string(), |n| n.to_string_lossy().into_owned());
        setting(ui, "들을 파일", &file_label, false, |ui| {
            if button(ui, "파일 선택").clicked()
                && let Some(p) = rfd::FileDialog::new().add_filter("영상·오디오", overfilm_subs::MEDIA_EXTENSIONS).pick_file()
            {
                app.enroll_file = Some(p);
            }
        });
        setting(ui, "구간", "그 사람만 말하는 10~30초. 검토 화면에서 열면 자동으로 골라 둡니다", false, |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.enroll_to).hint_text("1:45").desired_width(64.0));
            ui.label("~");
            ui.add(egui::TextEdit::singleline(&mut app.enroll_from).hint_text("1:23").desired_width(64.0));
        });
        setting(ui, "이름", "자막에 이 이름이 붙습니다", false, |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.enroll_name).hint_text("예: 아이치").desired_width(180.0));
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can = !busy && app.enroll_file.is_some() && !app.enroll_name.trim().is_empty();
            ui.add_enabled_ui(can, |ui| {
                if accent_button(ui, "등록").clicked() {
                    app.start_enroll(ctx);
                }
            });
            if busy {
                ui.spinner();
                ui.label(RichText::new("목소리를 듣는 중입니다").small());
            }
        });
    });
}

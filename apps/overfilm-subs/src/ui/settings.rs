//! 설정: processing device, models, Claude connection, about.

use eframe::egui::{self, RichText};
use overfilm_subs::job;

use super::widgets::{button, caption, pal, section, setting, title};
use crate::App;

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    super::scroll_page(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        title(ui, "설정");
        ui.add_space(8.0);
        super::status_bar(app, ui);

        section(ui, "처리 장치");
        let device =
            if app.prefs.last_device.is_empty() { "아직 실행하지 않았습니다".to_string() } else { format!("지난번: {}", app.prefs.last_device) };
        let before = app.prefs.options.gpu;
        setting(ui, "그래픽카드 사용", &device, false, |ui| {
            let label = if app.prefs.options.gpu { "자동 (GPU 우선)" } else { "항상 CPU" };
            craft_ui::ComboBox::from_id_salt("device").selected_text(label).width(170.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut app.prefs.options.gpu, true, "자동 (GPU 우선)");
                ui.selectable_value(&mut app.prefs.options.gpu, false, "항상 CPU");
            });
        });
        caption(ui, "자동이면 시작할 때 그래픽카드 계산이 CPU와 같은지 스스로 점검하고, 문제가 있으면 CPU로 처리합니다.");
        if before != app.prefs.options.gpu {
            app.save_prefs();
        }

        section(ui, "모델");
        let dir = job::models_dir();
        let all = overfilm_asr::catalogue::all();
        let gb = all.iter().map(|m| m.size()).sum::<u64>() as f64 / 1e9;
        let state = if app.models_ready { format!("받아 둠 · {gb:.1}GB") } else { format!("아직 받지 않음 · {gb:.1}GB") };
        setting(ui, &dir.display().to_string(), &state, false, |ui| {
            if button(ui, "폴더 열기").clicked() {
                let _ = std::fs::create_dir_all(&dir);
                if let Err(e) = super::reveal(&dir.join(".")) {
                    app.status = Some(Err(e));
                }
            }
        });
        caption(ui, "프로그램 옆에 models 폴더가 있으면 그것을 씁니다 (USB로 들고 다닐 때). 다른 곳을 쓰려면 환경 변수 OVERFILM_MODELS.");

        section(ui, "Claude");
        let cli = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.join(if cfg!(windows) { "overfilm-subs.exe" } else { "overfilm-subs" })))
            .unwrap_or_default();
        let cfg = serde_json::json!({"mcpServers": {"overfilm-subs": {"command": cli, "args": ["mcp"]}}});
        let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
        setting(ui, "Claude Desktop에 연결", "대화로 자막 추출·검토·고치기를 시킬 수 있습니다", false, |ui| {
            if button(ui, "설정 복사").clicked() {
                ui.ctx().copy_text(text.clone());
                app.status = Some(Ok("설정을 복사했습니다. Claude Desktop → 설정 → 개발자 → 설정 편집에 붙여 넣고 다시 시작하세요.".into()));
            }
        });
        let p = pal(ui);
        egui::Frame::new()
            .fill(p.card)
            .stroke(egui::Stroke::new(1.0, p.stroke))
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(egui::Margin { left: 52, right: 16, top: 10, bottom: 12 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                caption(ui, "Claude Desktop의 설정 → 개발자 → 설정 편집(claude_desktop_config.json)에 아래를 넣고 다시 시작합니다. 이미 다른 서버가 있으면 \"mcpServers\" 안에 overfilm-subs 부분만 더하세요.");
                ui.add_space(4.0);
                ui.add(egui::TextEdit::multiline(&mut text.as_str()).code_editor().desired_width(f32::INFINITY));
            });

        section(ui, "정보");
        setting(ui, "OverFilm 자막 추출", &format!("버전 {}", env!("CARGO_PKG_VERSION")), false, |ui| {
            ui.label(RichText::new("소리는 이 컴퓨터 밖으로 나가지 않습니다").small());
        });
    });
}

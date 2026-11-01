//! 설정: processing device, models, Claude connection, about.

use eframe::egui::{self, RichText};
use overfilm_subs::{claude, job};

use super::widgets::{accent_button, button, caption, pal, section, setting, subtle_button, title};
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
        let cli = claude::cli_path();
        for client in claude::Client::ALL {
            claude_card(app, ui, client, &cli);
        }
        let p = pal(ui);
        let text = claude::snippet(claude::Client::Desktop, &cli);
        egui::Frame::new()
            .fill(p.card)
            .stroke(egui::Stroke::new(1.0, p.stroke))
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(egui::Margin { left: 52, right: 16, top: 10, bottom: 12 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                caption(
                    ui,
                    "'연결'은 설정 파일에 overfilm-subs 한 줄만 더하고 다른 설정은 그대로 둡니다. 바꾸기 전 파일은 같은 폴더에 .bak으로 남습니다. 연결한 뒤 Claude를 다시 시작하세요. 직접 넣으려면 아래를 \"mcpServers\" 안에 붙여 넣습니다.",
                );
                ui.add_space(4.0);
                ui.add(egui::TextEdit::multiline(&mut text.as_str()).code_editor().desired_width(f32::INFINITY));
                if button(ui, "설정 복사").clicked() {
                    ui.ctx().copy_text(text.clone());
                    app.status = Some(Ok("설정을 복사했습니다.".into()));
                }
            });

        section(ui, "정보");
        setting(ui, "OverFilm 자막 추출", &format!("버전 {}", env!("CARGO_PKG_VERSION")), false, |ui| {
            ui.label(RichText::new("소리는 이 컴퓨터 밖으로 나가지 않습니다").small());
        });
    });
}

/// One Claude program: where it stands, and connect / disconnect.
fn claude_card(app: &mut App, ui: &mut egui::Ui, client: claude::Client, cli: &std::path::Path) {
    let (path, installed) = claude::config_path(client);
    let state = claude::state_of(&path, installed, cli);
    let what = match client {
        claude::Client::Desktop => "대화로 자막 추출·검토·고치기를 시킬 수 있습니다",
        claude::Client::Code => "터미널의 Claude Code에서 같은 도구를 씁니다",
    };
    let desc = match &state {
        claude::State::Missing => format!("{what} · 설치되어 있지 않은 것 같습니다"),
        claude::State::NotConnected => format!("{what} · 연결 안 됨"),
        claude::State::Connected => format!("연결됨 · {}", path.display()),
        claude::State::Elsewhere(p) => format!("다른 위치의 프로그램과 연결되어 있습니다 ({p}). 다시 연결하면 지금 이 프로그램으로 바꿉니다"),
        claude::State::Broken(e) => format!("설정 파일을 읽지 못해 손대지 않습니다: {e}"),
    };
    let mut act = None;
    setting(ui, &format!("{}에 연결", client.label()), &desc, false, |ui| match &state {
        claude::State::Connected => {
            if subtle_button(ui, "연결 해제").clicked() {
                act = Some(false);
            }
        }
        claude::State::Broken(_) => {
            if button(ui, "폴더 열기").clicked()
                && let Err(e) = super::reveal(&path)
            {
                app.status = Some(Err(e));
            }
        }
        claude::State::Elsewhere(_) => {
            if accent_button(ui, "다시 연결").clicked() {
                act = Some(true);
            }
        }
        claude::State::Missing | claude::State::NotConnected => {
            if accent_button(ui, "연결").clicked() {
                act = Some(true);
            }
        }
    });
    if let Some(on) = act {
        let r = if on { claude::connect(&path, client, cli) } else { claude::disconnect(&path) };
        app.status = Some(match r {
            Ok(()) if on => Ok(format!("{}에 연결했습니다. {}를 다시 시작하면 overfilm-subs 도구가 보입니다.", client.label(), client.label())),
            Ok(()) => Ok(format!("{} 연결을 해제했습니다.", client.label())),
            Err(e) => Err(e),
        });
    }
}

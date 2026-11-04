//! 새 자막 › 준비: the kind of video, the settings that follow from it, and the button.

use eframe::egui::{self, Align, Layout, Margin, RichText};
use overfilm_subs::edit::Preset;
use overfilm_subs::speakers::GameVoice;

use super::widgets::{self, accent_button, button, caption, expander, pal, radio_card, setting, subtle_button, toggle};
use crate::{App, Step};

const SAMPLE: &str = "워낙에 또 티원이 엄청나게 다양한 조합을 한다 해보다는";

pub fn page(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    let Some(input) = app.input.clone() else {
        app.step = Step::Empty;
        return;
    };
    let p = pal(ui);
    egui::Panel::bottom("prep_foot")
        .frame(egui::Frame::new().fill(p.layer).inner_margin(Margin::symmetric(24, 12)))
        .show_separator_line(true)
        .show(ui, |ui| footer(app, ui, ctx));
    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(Margin { left: 32, right: 32, top: 24, bottom: 16 })).show(ui, |ui| {
        super::scroll_page(ui, |ui| {
            caption(ui, "새 자막");
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if subtle_button(ui, "파일 바꾸기").clicked()
                        && let Some(f) = rfd::FileDialog::new().add_filter("영상·오디오", overfilm_subs::MEDIA_EXTENSIONS).pick_file()
                    {
                        app.choose_file(f);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let name = input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.add(egui::Label::new(RichText::new(name).text_style(egui::TextStyle::Heading).strong()).truncate());
                    });
                });
            });
            caption(ui, input.parent().map(|d| d.display().to_string()).unwrap_or_default());
            ui.add_space(12.0);
            super::status_bar(app, ui);
            let wide = ui.available_width() > 700.0;
            if wide {
                let side_w = 240.0;
                ui.horizontal_top(|ui| {
                    let lw = (ui.available_width() - side_w - 20.0).max(200.0);
                    ui.allocate_ui_with_layout(egui::vec2(lw, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(lw);
                        settings(app, ui);
                    });
                    ui.add_space(16.0);
                    ui.allocate_ui_with_layout(egui::vec2(side_w, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(side_w);
                        side(app, ui);
                    });
                });
            } else {
                settings(app, ui);
                side(app, ui);
            }
        });
    });
}

fn settings(app: &mut App, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.label(RichText::new("영상 종류").strong());
    for preset in Preset::ALL {
        let desc = if preset == Preset::Streamer && app.voices.is_empty() {
            "먼저 목소리를 등록하세요 (왼쪽 '목소리'). 등록한 사람의 말만 자막으로 만듭니다."
        } else {
            preset.description()
        };
        if radio_card(ui, app.prefs.preset == preset, preset.label(), desc).clicked() {
            app.prefs.preset = preset;
            preset.apply(&mut app.prefs.options);
        }
    }
    ui.add_space(14.0);
    ui.label(RichText::new("세부 설정").strong());
    if app.audio_tracks.len() > 1 {
        let (tracks, track) = (&app.audio_tracks, &mut app.audio_track);
        setting(ui, "오디오 트랙", "자막으로 만들 소리 (마이크만 따로 녹음한 트랙 등)", false, |ui| {
            let shown = tracks.get(*track).cloned().unwrap_or_default();
            craft_ui::ComboBox::from_id_salt("audio_track").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                for (i, t) in tracks.iter().enumerate() {
                    ui.selectable_value(track, i, t.as_str());
                }
            });
        });
    }
    silence_setting(app, ui);
    let o = &mut app.prefs.options;
    setting(ui, "추가 이름·용어", "단어집에 없는 이름만 쉼표로", false, |ui| {
        ui.add(egui::TextEdit::singleline(&mut o.extra_terms).hint_text("예: 스트리머 이름, 팀 이름").desired_width(240.0));
    });
    let open = expander(ui, "exp_speakers", "목소리 구별", "자막에 누가 말했는지 (화자1)처럼 붙입니다", |ui| {
        toggle(ui, &mut o.speakers);
    });
    if open {
        ui.add_enabled_ui(o.speakers, |ui| {
            let hint = if o.num_speakers == Some(1) {
                "1명: 목소리 구별을 건너뛰어 더 빨리 끝납니다"
            } else {
                "알면 정해 주세요. 가장 확실합니다"
            };
            setting(ui, "사람 수", hint, true, |ui| {
                let label = o.num_speakers.map_or("자동".to_string(), |n| format!("{n}명"));
                craft_ui::ComboBox::from_id_salt("num_speakers").selected_text(label).width(140.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut o.num_speakers, None, "자동");
                    for n in 1..=8 {
                        ui.selectable_value(&mut o.num_speakers, Some(n), format!("{n}명"));
                    }
                });
            });
            setting(ui, "사람마다 SRT 따로 저장", "Premiere에서 해설자별 자막 트랙", true, |ui| {
                toggle(ui, &mut o.per_speaker);
            });
            let names = if app.voices.is_empty() {
                "등록한 목소리가 없습니다".to_string()
            } else {
                format!("등록됨: {}", app.voices.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", "))
            };
            setting(ui, "등록한 목소리만 자막", &names, true, |ui| {
                ui.add_enabled_ui(!app.voices.is_empty(), |ui| toggle(ui, &mut o.only_voices));
            });
        });
    }
    let after = &mut app.prefs.after_done;
    let notify = &mut app.prefs.notify_done;
    setting(ui, "끝나면", if *notify { "알림을 띄웁니다" } else { "알림 없이" }, false, |ui| {
        craft_ui::ComboBox::from_id_salt("after_done").selected_text(after.label()).width(140.0).show_ui(ui, |ui| {
            for a in [overfilm_subs::notify::After::Nothing, overfilm_subs::notify::After::Sleep] {
                ui.selectable_value(after, a, a.label());
            }
        });
        ui.checkbox(notify, "알림");
    });
    if *after == overfilm_subs::notify::After::Sleep {
        caption(ui, "다 만들면 1분 동안 취소할 수 있게 기다린 뒤 컴퓨터를 절전 모드로 바꿉니다. 결과 파일은 그 전에 저장됩니다.");
    }
    let o = &mut app.prefs.options;
    setting(ui, "게임 음성", "영웅 대사, 아나운서", false, |ui| {
        craft_ui::ComboBox::from_id_salt("game_voice").selected_text(o.game_voice.label()).width(160.0).show_ui(ui, |ui| {
            for g in [GameVoice::Drop, GameVoice::Label, GameVoice::Keep] {
                ui.selectable_value(&mut o.game_voice, g, g.label());
            }
        });
    });
    let shape = format!(
        "{}자 · {} · {} · {}",
        o.line_chars,
        if o.max_lines >= 2 { "최대 두 줄" } else { "한 줄" },
        if o.mask_profanity { "욕설 초성" } else { "욕설 그대로" },
        if o.digits { "숫자로" } else { "숫자 그대로" }
    ) + if o.clean_fillers { " · 추임새 뺌" } else { "" };
    if expander(ui, "exp_shape", "자막 모양", &shape, |_| {}) {
        setting(ui, "한 줄 글자 수", "한글 기준", true, |ui| {
            ui.add(egui::DragValue::new(&mut o.line_chars).range(8..=30).suffix("자"));
        });
        let mut two = o.max_lines >= 2;
        setting(ui, "최대 두 줄", "끄면 한 줄짜리 자막만 만듭니다", true, |ui| {
            if toggle(ui, &mut two).changed() {
                o.max_lines = if two { 2 } else { 1 };
            }
        });
        setting(ui, "목소리보다 먼저", "자막이 말 시작보다 이만큼 먼저 뜹니다", true, |ui| {
            ui.add(egui::DragValue::new(&mut o.lead_ms).range(0..=1000).speed(10).suffix("ms"));
        });
        setting(ui, "말 끝난 뒤 남기기", "다음 자막이 바로 오면 그 전에 사라집니다", true, |ui| {
            ui.add(egui::DragValue::new(&mut o.hold_ms).range(0..=3000).speed(10).suffix("ms"));
        });
        setting(ui, "읽기 속도", "말이 빨라도 이 속도로 읽을 수 있게 조금 더 남깁니다 (0: 끔)", true, |ui| {
            ui.add(egui::DragValue::new(&mut o.reading_cps).range(0..=40).prefix("초당 ").suffix("자"));
        });
        setting(ui, "문장 끝에 마침표", "끄면 '했습니다.' → '했습니다' (물음표·느낌표는 그대로)", true, |ui| {
            toggle(ui, &mut o.periods);
        });
        setting(ui, "욕설은 초성으로", "씨발 → ㅅㅂ", true, |ui| {
            toggle(ui, &mut o.mask_profanity);
        });
        setting(ui, "숫자는 숫자로", "오 시즌 → 5시즌, 사십칠 퍼센트 → 47%", true, |ui| {
            toggle(ui, &mut o.digits);
        });
        setting(ui, "추임새 빼기", "어, 음, 그 그게 같은 군말과 말 더듬기를 자막에서 뺍니다", true, |ui| {
            toggle(ui, &mut o.clean_fillers);
        });
    }
}

/// Preview of the caption shape, and the last speed.
fn side(app: &mut App, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(RichText::new("미리보기").strong());
    let lines = preview_lines(&app.prefs.options);
    egui::Frame::new().fill(egui::Color32::from_rgb(16, 16, 16)).corner_radius(egui::CornerRadius::same(4)).inner_margin(Margin::symmetric(12, 14)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                for l in &lines {
                    ui.label(RichText::new(l).color(egui::Color32::WHITE).size(15.0).strong());
                }
            });
        },
    );
    caption(ui, "지금 설정으로 끊은 모습");
    ui.add_space(8.0);
    widgets::card(ui, |ui| {
        caption(ui, "처리 속도");
        if app.prefs.secs_per_audio_min > 0.0 {
            let ten = app.prefs.secs_per_audio_min * 10.0;
            ui.label(RichText::new(format!("소리 10분에 약 {}", widgets::duration(f64::from(ten)))).text_style(super::theme::subtitle()).strong());
            caption(ui, format!("지난번 실행 기준 ({})", app.prefs.last_device));
        } else {
            ui.label(RichText::new("처음 실행").strong());
            caption(ui, "한 번 돌리면 이 컴퓨터의 속도를 기억해 둡니다");
        }
    });
}

/// The sample sentence laid out with the current line settings.
fn preview_lines(o: &overfilm_subs::job::Options) -> Vec<String> {
    let words: Vec<overfilm_captions::TimedWord> = SAMPLE
        .split_whitespace()
        .enumerate()
        .map(|(k, w)| overfilm_captions::TimedWord { text: w.to_string(), start_ms: k as i64 * 300, end_ms: k as i64 * 300 + 250 })
        .collect();
    let layout = overfilm_captions::Layout { max_cue_ms: i64::MAX / 4, ..o.layout() };
    overfilm_captions::layout(&words, &layout).into_iter().next().map(|c| c.lines).unwrap_or_default()
}

fn footer(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if accent_button(ui, "자막 만들기").clicked() {
                app.start(ctx);
            }
            if button(ui, "취소").clicked() {
                app.input = None;
                app.step = Step::Empty;
            }
            let path = app.options().map(|o| o.srt_path()).unwrap_or_default();
            if subtle_button(ui, "바꾸기").clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .add_filter("SRT 자막", &["srt"])
                    .set_file_name(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                    .save_file()
            {
                app.output = Some(p);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let p = pal(ui);
                ui.add(egui::Label::new(RichText::new(format!("저장 위치: {}", path.display())).small().color(p.text2)).truncate())
                    .on_hover_text(path.display().to_string());
            });
        });
    });
}

/// Skip long quiet stretches: on by itself for a separate voice track, off for sound inside the
/// video; turning it on there shows a warning.
fn silence_setting(app: &mut App, ui: &mut egui::Ui) {
    use overfilm_pipeline::recognise::{SKIP_SILENCE_WARNING, SoundSource, skip_silence_default};
    let src = app.sound_source;
    let separate = src == Some(SoundSource::Separate);
    let auto = src.is_some_and(skip_silence_default);
    let mut on = app.skip_silence.unwrap_or(auto);
    let hint = match (separate, on) {
        (true, true) => "오디오 트랙이 분리되어 있어 켜 두었습니다 (권장). 2초 넘는 무음은 인식하지 않아 빨라집니다",
        (true, false) => "2초 넘는 무음을 인식하지 않아 빨라집니다. 분리된 트랙에서는 켜는 것을 권합니다",
        (false, false) => "영상에 섞인 소리라 꺼 두었습니다. 말소리만 따로 녹음된 트랙에서 쓰는 것을 권합니다",
        (false, true) => "영상에 섞인 소리에서 켰습니다",
    };
    let changed = setting(ui, "무음 구간 건너뛰기", hint, false, |ui| toggle(ui, &mut on).changed());
    if changed {
        app.skip_silence = (on != auto).then_some(on);
    }
    if on && !separate {
        widgets::info_bar(ui, widgets::Severity::Warn, "작은 말소리를 놓칠 수 있습니다", SKIP_SILENCE_WARNING, |_| {});
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_follows_line_length() {
        let mut o = overfilm_subs::job::Options::new(std::path::PathBuf::new());
        o.line_chars = 16;
        let two = preview_lines(&o);
        assert!(!two.is_empty() && two.len() <= 2, "{two:?}");
        o.max_lines = 1;
        o.line_chars = 30;
        assert_eq!(preview_lines(&o).len(), 1);
    }
}

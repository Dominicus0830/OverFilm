//! 새 자막 › 검토: command bar, speakers on the left, the caption table (number, start, end,
//! length, speaker, text) in the middle, findings grouped by kind on the right, and the selected
//! caption's text box underneath, as in subtitle editors.

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Margin, RichText, Sense, Stroke, vec2};
use overfilm_subs::{edit, export, review};

use super::widgets::{self, Severity, accent_button, button, caption, pal, subtle_button};
use crate::{App, Page, Review, Step};

const ROW_H: f32 = 30.0;

/// A finding kind as the window names it.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        "glossary" => "단어집·맞춤법",
        "unsure" => "자신 없는 단어",
        "fast" => "읽기 빠름",
        "short" => "너무 짧음",
        "speech_overlap" => "겹친 말",
        "overlap" => "시간 겹침",
        "lines" => "줄 수",
        "width" => "줄 길이",
        "repeat" => "반복",
        "empty" => "빈 자막",
        _ => "기타",
    }
}

const KIND_ORDER: [&str; 10] = ["glossary", "unsure", "fast", "short", "speech_overlap", "overlap", "width", "lines", "repeat", "empty"];

enum Action {
    Undo,
    FixAll,
    PerSpeaker,
    Reveal,
    New,
    Select(usize),
    Filter(Option<&'static str>),
    Apply(usize, String),
    Ignore(usize, &'static str),
    SaveDraft,
    Rename(usize, String),
    Merge(usize, usize),
    Game(usize, bool),
    Enroll(usize),
    NextIssue,
    Fillers(bool),
    Export,
    FindNext,
    ReplaceAll,
    RestoreVersion(u32),
    Learn(bool),
}

pub fn page(app: &mut App, ui: &mut egui::Ui, _ctx: &egui::Context) {
    let Some(r) = app.review.as_mut() else {
        app.step = Step::Empty;
        return;
    };
    let p = pal(ui);
    let mut actions = Vec::new();
    egui::Panel::top("cmd").frame(egui::Frame::new().fill(p.layer).inner_margin(Margin::symmetric(12, 6))).show(ui, |ui| {
        command_bar(r, ui, &mut actions);
    });
    egui::Panel::top("summary").frame(egui::Frame::new().inner_margin(Margin { left: 12, right: 12, top: 8, bottom: 0 })).show_separator_line(false).show(
        ui,
        |ui| {
            let mut close = false;
            if let Some(s) = &r.summary {
                widgets::info_bar(ui, Severity::Ok, &format!("자막 {}개를 만들었습니다.", r.opened.captions.len()), s, |ui| {
                    close = subtle_button(ui, "닫기").clicked()
                });
            } else {
                let name = r.opened.srt.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                widgets::info_bar(
                    ui,
                    Severity::Info,
                    &name,
                    &format!("자막 {}개. 고치면 바로 저장되고, 처음 고칠 때 .srt.bak을 남깁니다.", r.opened.captions.len()),
                    |_| {},
                );
            }
            if close {
                r.summary = None;
            }
            if let Some(l) = r.learned.first() {
                let mut act = None;
                widgets::info_bar(
                    ui,
                    Severity::Info,
                    &format!("'{}'을(를) '{}'(으)로 {}번 고쳤습니다.", l.from, l.to, l.count),
                    "단어집에 넣으면 다음 자막부터 자동으로 고칩니다.",
                    |ui| {
                        if subtle_button(ui, "묻지 않기").clicked() {
                            act = Some(false);
                        }
                        if accent_button(ui, "단어집에 넣기").clicked() {
                            act = Some(true);
                        }
                    },
                );
                if let Some(a) = act {
                    actions.push(Action::Learn(a));
                }
            }
            match &app.status {
                Some(Err(e)) => widgets::info_bar(ui, Severity::Error, "문제가 생겼습니다.", e, |_| {}),
                Some(Ok(m)) => widgets::info_bar(ui, Severity::Ok, m, "", |_| {}),
                None => {}
            }
            ui.add_space(6.0);
        },
    );
    if ui.ctx().input(|i| i.modifiers.command && (i.key_pressed(Key::F) || i.key_pressed(Key::H))) {
        r.find.open = true;
    }
    if r.find.open {
        egui::Panel::top("find").frame(egui::Frame::new().fill(p.card).inner_margin(Margin::symmetric(12, 8))).show(ui, |ui| {
            find_bar(r, ui, &mut actions);
        });
    }
    egui::Panel::left("speakers").exact_size(196.0).resizable(false).frame(egui::Frame::new().inner_margin(Margin::same(10))).show(ui, |ui| {
        speakers_pane(r, ui, &mut actions);
    });
    egui::Panel::right("issues").exact_size(236.0).resizable(false).frame(egui::Frame::new().inner_margin(Margin::same(10))).show(ui, |ui| {
        issues_pane(r, ui, &mut actions);
    });
    if r.selected.is_some() {
        egui::Panel::bottom("editor").frame(egui::Frame::new().fill(p.card).inner_margin(Margin::symmetric(14, 10))).show(ui, |ui| {
            editor(r, ui, &mut actions);
        });
    }
    egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
        table(r, ui, &mut actions);
    });
    if ui.ctx().input(|i| i.key_pressed(Key::F8)) {
        actions.push(Action::NextIssue);
    }
    if r.history.is_some() {
        history_dialog(r, ui.ctx(), &mut actions);
    }
    if r.exporting {
        export_dialog(r, &mut app.prefs.export, ui.ctx(), &mut actions);
    }
    for a in actions {
        run(app, a);
    }
}

fn command_bar(r: &mut Review, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if ui.add_enabled(!r.undo.is_empty(), egui::Button::new("실행 취소").frame(false).min_size(vec2(0.0, 32.0))).clicked() {
            actions.push(Action::Undo);
        }
        if subtle_button(ui, "기록").on_hover_text("저장할 때마다 남긴 기록에서 되돌리기").clicked() {
            r.history = Some(overfilm_subs::history::list(&r.opened.srt));
        }
        ui.separator();
        let glossary = r.issues.iter().filter(|i| i.kind == "glossary" && !r.ignored.contains(&(i.index, i.kind))).count();
        if ui.add_enabled(glossary > 0, egui::Button::new(format!("단어집대로 모두 고치기 ({glossary})")).frame(false).min_size(vec2(0.0, 32.0))).clicked()
        {
            actions.push(Action::FixAll);
        }
        if r.opened.transcript.is_some() {
            let mut clean = r.options.clean_fillers;
            if ui
                .checkbox(&mut clean, "추임새 빼기")
                .on_hover_text("어, 음, 그 그게 같은 군말과 말 더듬기를 자막에서 뺍니다. 단어 시간 파일로 자막을 다시 만들므로 손으로 고친 글자는 사라집니다.")
                .changed()
            {
                actions.push(Action::Fillers(clean));
            }
        }
        let people = r.opened.transcript.as_ref().map_or(0, overfilm_subs::speakers::people);
        if people > 1 && subtle_button(ui, "사람마다 SRT 저장").clicked() {
            actions.push(Action::PerSpeaker);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if subtle_button(ui, "새 자막").clicked() {
                actions.push(Action::New);
            }
            if subtle_button(ui, "폴더 열기").clicked() {
                actions.push(Action::Reveal);
            }
            if subtle_button(ui, "찾아 바꾸기").on_hover_text("Ctrl+H").clicked() {
                r.find.open = !r.find.open;
            }
            if subtle_button(ui, "내보내기").on_hover_text("VTT, ASS, 대본 TXT, 프레임에 맞춘 SRT").clicked() {
                r.exporting = true;
            }
        });
    });
}

/// Index of `label` among the transcript's speakers (for its colour).
fn speaker_index(r: &Review, label: &str) -> Option<usize> {
    r.opened.transcript.as_ref()?.speakers.iter().position(|s| s.label == label)
}

fn speaker_color(r: &Review, p: &super::theme::Palette, label: &str) -> Color32 {
    if label == "게임" || label == "게임 음성" {
        return p.game;
    }
    speaker_index(r, label).and_then(|k| p.speakers.get(k % p.speakers.len()).copied()).unwrap_or(p.text3)
}

fn speakers_pane(r: &mut Review, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let p = pal(ui);
    caption(ui, "화자");
    let Some(t) = r.opened.transcript.clone() else {
        caption(ui, "단어 시간 파일(.overfilm.json)이 없어 화자를 고칠 수 없습니다.");
        return;
    };
    if t.speakers.is_empty() {
        caption(ui, "화자를 나누지 않은 자막입니다.");
        return;
    }
    let longest = t.speakers.iter().filter(|s| !s.game).map(|s| s.seconds).fold(0.0_f32, f32::max);
    ui.spacing_mut().item_spacing.y = 2.0;
    for (k, s) in t.speakers.iter().enumerate() {
        let color = if s.game { p.game } else { p.speakers.get(k % p.speakers.len()).copied().unwrap_or(p.text3) };
        if let Some((idx, name)) = r.renaming.as_mut()
            && *idx == k
        {
            let resp = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()));
            resp.request_focus();
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                actions.push(Action::Rename(k, name.clone()));
            } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                r.renaming = None;
            }
            continue;
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(4), p.control_hover);
        }
        let sw = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0, rect.center().y), vec2(10.0, 10.0));
        ui.painter().rect_filled(sw, CornerRadius::same(2), color);
        let text_color = if s.game { p.text2 } else { p.text };
        ui.painter().text(egui::pos2(rect.left() + 26.0, rect.center().y), Align2::LEFT_CENTER, &s.label, FontId::proportional(14.0), text_color);
        let secs = if s.game { format!("{}단어", s.words) } else { widgets::duration(f64::from(s.seconds)) };
        ui.painter().text(egui::pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, secs, FontId::monospace(12.0), p.text2);
        let resp = resp.on_hover_text("오른쪽 클릭: 이름 바꾸기, 합치기, 목소리 등록");
        if resp.double_clicked() {
            r.renaming = Some((k, s.label.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("이름 바꾸기").clicked() {
                r.renaming = Some((k, s.label.clone()));
                ui.close();
            }
            ui.menu_button("다른 화자와 합치기", |ui| {
                for (j, o) in t.speakers.iter().enumerate().filter(|(j, _)| *j != k) {
                    if ui.button(&o.label).clicked() {
                        actions.push(Action::Merge(k, j));
                        ui.close();
                    }
                }
            });
            if !s.game && ui.button("이 목소리 등록…").clicked() {
                actions.push(Action::Enroll(k));
                ui.close();
            }
            ui.separator();
            let label = if s.game { "사람 목소리로 되돌리기" } else { "게임 음성으로 돌리기" };
            if ui.button(label).clicked() {
                actions.push(Action::Game(k, !s.game));
                ui.close();
            }
        });
        if !s.game && longest > 0.0 && s.seconds < 30.0 && s.seconds < longest * 0.1 {
            ui.label(RichText::new("말이 짧은 화자입니다. 다른 사람과 같은 목소리면 오른쪽 클릭으로 합치세요.").small().color(p.text2));
        }
    }
}

fn issues_pane(r: &Review, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let p = pal(ui);
    let open: Vec<&review::Issue> = r.issues.iter().filter(|i| !r.ignored.contains(&(i.index, i.kind))).collect();
    caption(ui, format!("검토할 곳 {}", open.len()));
    ui.spacing_mut().item_spacing.y = 2.0;
    let all = r.filter.is_none();
    if group_row(ui, &p, "모두 보기", "", r.opened.captions.len(), all).clicked() {
        actions.push(Action::Filter(None));
    }
    for kind in KIND_ORDER {
        let n = open.iter().filter(|i| i.kind == kind).count();
        if n == 0 {
            continue;
        }
        let desc = match kind {
            "glossary" => "이름 표기, 띄어쓰기",
            "unsure" => "잘못 들었을 수 있는 곳",
            "fast" => "초당 글자가 많음",
            "short" => "너무 빨리 사라짐",
            "speech_overlap" => "두 사람이 동시에 말함",
            "overlap" => "앞 자막과 시간이 겹침",
            "repeat" => "같은 말을 두 번 적음",
            _ => "",
        };
        if group_row(ui, &p, kind_label(kind), desc, n, r.filter == Some(kind)).clicked() {
            actions.push(Action::Filter(Some(kind)));
        }
    }
    ui.add_space(10.0);
    caption(ui, "F8 다음 곳 · Ctrl+Enter 글자 저장");
}

fn group_row(ui: &mut egui::Ui, p: &super::theme::Palette, title: &str, desc: &str, count: usize, selected: bool) -> egui::Response {
    let h = if desc.is_empty() { 34.0 } else { 46.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    if selected || resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), if selected { p.subtle } else { p.control_hover });
    }
    if selected {
        let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - 8.0), vec2(3.0, 16.0));
        ui.painter().rect_filled(bar, CornerRadius::same(2), p.accent);
    }
    let top = if desc.is_empty() { rect.center().y } else { rect.top() + 15.0 };
    ui.painter().text(egui::pos2(rect.left() + 12.0, top), Align2::LEFT_CENTER, title, FontId::proportional(14.0), p.text);
    if !desc.is_empty() {
        ui.painter().text(egui::pos2(rect.left() + 12.0, rect.top() + 33.0), Align2::LEFT_CENTER, desc, FontId::proportional(12.0), p.text2);
    }
    let badge = egui::Rect::from_center_size(egui::pos2(rect.right() - 20.0, top), vec2(28.0, 18.0));
    ui.painter().rect_filled(badge, CornerRadius::same(9), p.control);
    ui.painter().rect_stroke(badge, CornerRadius::same(9), Stroke::new(1.0, p.control_stroke), egui::StrokeKind::Inside);
    ui.painter().text(badge.center(), Align2::CENTER_CENTER, count.to_string(), FontId::proportional(12.0), p.text);
    resp
}

/// Caption rows shown under the current filter (0-based caption indices).
fn rows(r: &Review) -> Vec<usize> {
    match r.filter {
        None => (0..r.opened.captions.len()).collect(),
        Some(kind) => {
            let mut v: Vec<usize> =
                r.issues.iter().filter(|i| i.kind == kind && !r.ignored.contains(&(i.index, i.kind))).map(|i| i.index.saturating_sub(1)).collect();
            v.dedup();
            v
        }
    }
}

const COLS: [(&str, f32); 5] = [("#", 42.0), ("시작", 88.0), ("끝", 88.0), ("길이", 48.0), ("화자", 92.0)];

fn table(r: &mut Review, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let p = pal(ui);
    let labels = edit::caption_speakers(&r.opened.captions);
    let shown = rows(r);
    // header
    let (hrect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    ui.painter().rect_filled(hrect, 0.0, p.layer);
    let mut x = hrect.left();
    for (name, w) in COLS.iter().copied().chain(std::iter::once(("자막", 0.0))) {
        ui.painter().text(egui::pos2(x + 8.0, hrect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(12.0), p.text2);
        x += w;
        if w > 0.0 {
            ui.painter().vline(x, hrect.y_range(), Stroke::new(1.0, p.stroke));
        }
    }
    ui.painter().hline(hrect.x_range(), hrect.bottom(), Stroke::new(1.0, p.stroke));
    if shown.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| caption(ui, "이 종류는 모두 확인했습니다."));
        return;
    }
    let mut area = egui::ScrollArea::vertical().auto_shrink(false).id_salt("captions");
    if r.jump
        && let Some(pos) = r.selected.and_then(|s| shown.iter().position(|&k| k == s))
    {
        // bring a row chosen from the keyboard or a filter into view, a few rows from the top
        area = area.vertical_scroll_offset((pos as f32 - 3.0).max(0.0) * ROW_H);
    }
    r.jump = false;
    let r = &*r;
    area.show_rows(ui, ROW_H, shown.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for &k in shown.get(range).unwrap_or_default() {
            let Some(c) = r.opened.captions.get(k) else { continue };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            let selected = r.selected == Some(k);
            if selected {
                ui.painter().rect_filled(rect, 0.0, p.accent_soft);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, p.control_hover);
            }
            ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.stroke));
            let kinds: Vec<&'static str> = r.issues.iter().filter(|i| i.index == k + 1 && !r.ignored.contains(&(i.index, i.kind))).map(|i| i.kind).collect();
            let dur = (c.end_ms - c.start_ms).max(0) as f64 / 1000.0;
            let dur_color = if kinds.contains(&"short") {
                p.bad
            } else if kinds.contains(&"fast") {
                p.warn
            } else {
                p.text
            };
            let y = rect.center().y;
            let mono = FontId::monospace(12.5);
            let mut x = rect.left();
            let cells: [(String, Color32, &FontId); 4] = [
                ((k + 1).to_string(), p.text3, &mono),
                (short_clock(c.start_ms), p.text, &mono),
                (short_clock(c.end_ms), p.text, &mono),
                (format!("{dur:.2}"), dur_color, &mono),
            ];
            for ((text, color, font), (_, w)) in cells.into_iter().zip(COLS) {
                ui.painter().text(egui::pos2(x + 8.0, y), Align2::LEFT_CENTER, text, font.clone(), color);
                x += w;
            }
            let (who, text) = labels.get(k).cloned().unwrap_or_default();
            if let Some(who) = who {
                let mut sx = x + 8.0;
                for name in who.split('·') {
                    let sw = egui::Rect::from_center_size(egui::pos2(sx + 5.0, y), vec2(10.0, 10.0));
                    ui.painter().rect_filled(sw, CornerRadius::same(2), speaker_color(r, &p, name));
                    sx += 12.0;
                }
                let short = if who.contains('·') { who.replace("화자", "") } else { who.clone() };
                ui.painter().text(egui::pos2(sx + 4.0, y), Align2::LEFT_CENTER, short, FontId::proportional(13.0), p.text);
            }
            x += COLS[4].1;
            let tag = kinds.iter().find(|k| !matches!(**k, "short" | "fast")).map(|k| kind_label(k));
            let tag_w = if tag.is_some() { 96.0 } else { 0.0 };
            let clip = egui::Rect::from_min_max(egui::pos2(x + 8.0, rect.top()), egui::pos2(rect.right() - tag_w - 8.0, rect.bottom()));
            ui.painter().with_clip_rect(clip).text(egui::pos2(x + 8.0, y), Align2::LEFT_CENTER, text.replace('\n', " / "), FontId::proportional(14.0), p.text);
            if let Some(tag) = tag {
                ui.painter().text(egui::pos2(rect.right() - 10.0, y), Align2::RIGHT_CENTER, tag, FontId::proportional(12.0), p.warn);
            }
            if resp.clicked() {
                actions.push(Action::Select(k));
            }
        }
    });
}

/// `MM:SS.mmm` (hours only when there are any).
fn short_clock(ms: i64) -> String {
    let ms = ms.max(0);
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 { format!("{h}:{m:02}:{s:02}.{f:03}") } else { format!("{m:02}:{s:02}.{f:03}") }
}

fn editor(r: &mut Review, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let p = pal(ui);
    let Some(k) = r.selected else { return };
    let Some(c) = r.opened.captions.get(k).cloned() else { return };
    let issues: Vec<review::Issue> = r.issues.iter().filter(|i| i.index == k + 1 && !r.ignored.contains(&(i.index, i.kind))).cloned().collect();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("#{}", k + 1)).strong());
        caption(ui, format!("{} → {}", short_clock(c.start_ms), short_clock(c.end_ms)));
    });
    for i in &issues {
        ui.horizontal(|ui| {
            ui.label(RichText::new(kind_label(i.kind)).small().color(p.warn));
            ui.label(RichText::new(&i.message).small());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, "무시").clicked() {
                    actions.push(Action::Ignore(i.index, i.kind));
                }
                if let Some(s) = &i.suggestion
                    && accent_button(ui, "고치기").on_hover_text(s.replace('\n', " / ")).clicked()
                {
                    actions.push(Action::Apply(k, s.clone()));
                }
            });
        });
    }
    ui.horizontal(|ui| {
        let w = ui.available_width() - 120.0;
        let resp = ui.add(egui::TextEdit::multiline(&mut r.draft).desired_rows(2).desired_width(w).font(FontId::proportional(15.0)));
        let save_key = resp.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter));
        ui.vertical(|ui| {
            let changed = r.draft.trim() != c.text().trim();
            if ui.add_enabled(changed, egui::Button::new("글자 저장").min_size(vec2(100.0, 32.0))).clicked() || (changed && save_key) {
                actions.push(Action::SaveDraft);
            }
            if ui.add_enabled(changed, egui::Button::new("되돌리기").min_size(vec2(100.0, 32.0))).clicked() {
                r.draft = c.text();
            }
        });
    });
}

fn find_bar(r: &mut Review, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let hits = edit::find_in(&r.opened.captions, &r.find.find).len();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let f = ui.add(egui::TextEdit::singleline(&mut r.find.find).hint_text("찾을 글자").desired_width(180.0));
        if f.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            actions.push(Action::FindNext);
            f.request_focus();
        }
        ui.add(egui::TextEdit::singleline(&mut r.find.with).hint_text("바꿀 글자").desired_width(180.0));
        ui.checkbox(&mut r.find.whole_word, "단어 단위");
        ui.checkbox(&mut r.find.learn, "단어집에도 넣기").on_hover_text("다음 자막부터 자동으로 고쳐지도록 사용자 단어집에 '틀린 표기 → 맞는 표기'로 넣습니다");
        if ui.add_enabled(hits > 0, egui::Button::new("다음 찾기")).clicked() {
            actions.push(Action::FindNext);
        }
        if ui.add_enabled(hits > 0, egui::Button::new(format!("모두 바꾸기 ({hits})"))).clicked() {
            actions.push(Action::ReplaceAll);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if subtle_button(ui, "닫기").clicked() {
                r.find.open = false;
            }
        });
    });
}

/// Saved versions, newest first, each with a button to bring it back.
fn history_dialog(r: &mut Review, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let Some(list) = r.history.as_ref() else { return };
    let mut close = false;
    let resp = egui::Modal::new(egui::Id::unique("history")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.label(RichText::new("기록").text_style(super::theme::subtitle()).strong());
        caption(ui, "저장할 때마다 남긴 상태입니다. 되돌려도 지금 상태가 기록에 남으므로 언제든 다시 돌아올 수 있습니다.");
        ui.add_space(6.0);
        if list.is_empty() {
            caption(ui, "아직 기록이 없습니다. 이 파일을 한 번 고치면 그때부터 남습니다.");
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (k, v) in list.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(v.when()).monospace().size(12.5));
                    ui.label(&v.label);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if k == 0 {
                            caption(ui, "지금");
                        } else if button(ui, "되돌리기").clicked() {
                            actions.push(Action::RestoreVersion(v.id));
                        }
                        caption(ui, format!("자막 {}개", v.captions));
                    });
                });
            }
        });
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button(ui, "닫기").clicked() {
                close = true;
            }
        });
    });
    if close || resp.should_close() {
        r.history = None;
    }
}

/// Format, frame rate and gap, then a save dialog.
fn export_dialog(r: &mut Review, e: &mut export::Export, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let resp = egui::Modal::new(egui::Id::unique("export")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(RichText::new("내보내기").text_style(super::theme::subtitle()).strong());
        ui.add_space(8.0);
        caption(ui, "형식");
        for f in export::Format::ALL {
            ui.radio_value(&mut e.format, f, f.label());
        }
        ui.add_space(8.0);
        caption(ui, "프레임에 맞추기");
        let label = e.fps.map_or("맞추지 않음".to_string(), export::fps_label);
        craft_ui::ComboBox::from_id_salt("export_fps").selected_text(label).width(200.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut e.fps, None, "맞추지 않음");
            for f in export::FRAME_RATES {
                ui.selectable_value(&mut e.fps, Some(f), export::fps_label(f));
            }
        });
        ui.add_enabled_ui(e.fps.is_some(), |ui| {
            ui.horizontal(|ui| {
                ui.label("자막 사이 빈 프레임");
                ui.add(egui::DragValue::new(&mut e.gap_frames).range(0..=10).suffix("프레임"));
            });
        });
        caption(ui, "영상과 같은 프레임 수를 고르면 자막이 프레임 경계에서 바뀝니다. 빈 프레임은 자막이 붙어서 깜빡이는 것을 막습니다.");
        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if accent_button(ui, "저장할 곳 고르기…").clicked() {
                actions.push(Action::Export);
            }
            if button(ui, "닫기").clicked() {
                r.exporting = false;
            }
        });
    });
    if resp.should_close() {
        r.exporting = false;
    }
}

fn rebuild(r: &mut Review) -> Result<(), String> {
    let o = r.options.clone();
    r.opened.rebuild(&o)?;
    r.ignored.clear();
    r.selected = None;
    Ok(())
}

fn run(app: &mut App, a: Action) {
    let mask = app.prefs.options.mask_profanity;
    let Some(r) = app.review.as_mut() else { return };
    let result: Result<Option<String>, String> = match a {
        Action::Undo => match r.undo.pop() {
            Some((caps, t)) => r.opened.restore(caps, t).map(|()| None),
            None => Ok(None),
        },
        Action::FixAll => {
            r.remember();
            let corrector = overfilm_subs::pack::glossary(Some(overfilm_subs::mcp::user_glossary_dir().as_path()).filter(|d| d.is_dir()), mask)
                .map(|g| overfilm_subs::pack::corrector(&g));
            corrector.and_then(|c| r.opened.fix_all(&c)).map(|n| Some(format!("단어집대로 {n}곳을 고쳤습니다.")))
        }
        Action::PerSpeaker => r.opened.write_per_speaker(&r.options).map(|v| Some(format!("사람마다 SRT {}개를 저장했습니다.", v.len()))),
        Action::Reveal => super::reveal(&r.opened.srt).map(|()| None),
        Action::New => {
            app.review = None;
            app.input = None;
            app.step = Step::Empty;
            app.status = None;
            return;
        }
        Action::Select(k) => {
            r.selected = Some(k);
            r.draft = r.opened.captions.get(k).map(|c| c.text()).unwrap_or_default();
            return;
        }
        Action::Filter(f) => {
            r.filter = f;
            r.selected = None;
            return;
        }
        Action::Apply(k, text) => {
            r.remember();
            let res = r.opened.set_text(k, &text);
            r.draft = text;
            res.map(|()| None)
        }
        Action::Ignore(index, kind) => {
            r.ignored.insert((index, kind));
            return;
        }
        Action::SaveDraft => match r.selected {
            Some(k) => {
                r.remember();
                let d = r.draft.clone();
                let before = r.opened.captions.get(k).map(|c| c.text()).unwrap_or_default();
                let res = r.opened.set_text(k, &d);
                if res.is_ok() {
                    r.learned.extend(overfilm_subs::learn::record(&before, &d));
                }
                res.map(|()| None)
            }
            None => Ok(None),
        },
        Action::Learn(add) => {
            if r.learned.is_empty() {
                return;
            }
            let l = r.learned.remove(0);
            if add {
                overfilm_subs::learn::accept(&l).map(|()| Some(format!("'{}' → '{}'를 단어집에 넣었습니다.", l.from, l.to)))
            } else {
                overfilm_subs::learn::resolve(&l.from, &l.to, false).map(|()| None)
            }
        }
        Action::Rename(k, name) => {
            r.renaming = None;
            r.remember();
            match r.opened.transcript.as_mut() {
                Some(t) => {
                    edit::rename_speaker(t, k, &name).and_then(|()| rebuild(r)).map(|()| Some(format!("화자 이름을 '{}'(으)로 바꿨습니다.", name.trim())))
                }
                None => Ok(None),
            }
        }
        Action::Merge(from, into) => {
            r.remember();
            match r.opened.transcript.as_mut() {
                Some(t) => edit::merge_speakers(t, from, into).and_then(|()| rebuild(r)).map(|()| Some("화자를 합쳤습니다.".to_string())),
                None => Ok(None),
            }
        }
        Action::Game(k, game) => {
            r.remember();
            match r.opened.transcript.as_mut() {
                Some(t) => edit::set_game(t, k, game).and_then(|()| rebuild(r)).map(|()| None),
                None => Ok(None),
            }
        }
        Action::Enroll(k) => {
            let Some(t) = r.opened.transcript.as_ref() else { return };
            let label = t.speakers.get(k).map(|s| s.label.clone()).unwrap_or_default();
            app.enroll_name = if label.starts_with("화자") { String::new() } else { label };
            app.enroll_file = r.input.clone().filter(|p| p.is_file());
            match edit::solo_span(t, k) {
                Some((a, b)) => {
                    app.enroll_from = fmt_time(a);
                    app.enroll_to = fmt_time(b);
                    app.status = None;
                }
                None => {
                    app.enroll_from.clear();
                    app.enroll_to.clear();
                    app.status = Some(Err("이 화자가 혼자 5초 넘게 말한 곳이 없습니다. 구간을 직접 적어 주세요.".into()));
                }
            }
            app.page = Page::Voices;
            return;
        }
        Action::Fillers(on) => {
            r.remember();
            r.options.clean_fillers = on;
            app.prefs.options.clean_fillers = on;
            rebuild(r).map(|()| Some(if on { "추임새를 뺐습니다.".to_string() } else { "추임새를 다시 넣었습니다.".to_string() }))
        }
        Action::Export => {
            let e = app.prefs.export.clone();
            let stem = r.opened.srt.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let suggested = if e.format == export::Format::Srt && e.fps.is_some() {
                format!("{stem}.{}.srt", export::fps_label(e.fps.unwrap_or((30, 1))))
            } else {
                format!("{stem}.{}", e.format.extension())
            };
            let mut dlg = rfd::FileDialog::new().add_filter(e.format.extension().to_uppercase(), &[e.format.extension()]).set_file_name(suggested);
            if let Some(d) = r.opened.srt.parent() {
                dlg = dlg.set_directory(d);
            }
            match dlg.save_file() {
                Some(p) if p == r.opened.srt => Err("검토 중인 원본 SRT에는 덮어쓰지 않습니다. 다른 이름을 고르세요.".into()),
                Some(p) => {
                    r.exporting = false;
                    export::write(&p, &r.opened.captions, &e).map(|()| Some(format!("{}로 내보냈습니다.", p.display())))
                }
                None => return,
            }
        }
        Action::RestoreVersion(id) => {
            r.remember();
            r.history = None;
            r.selected = None;
            r.opened.restore_version(id).map(|()| Some(format!("기록 {id}번으로 되돌렸습니다.")))
        }
        Action::FindNext => {
            let hits = edit::find_in(&r.opened.captions, &r.find.find);
            let next = match r.selected {
                Some(s) => hits.iter().copied().find(|&k| k > s).or_else(|| hits.first().copied()),
                None => hits.first().copied(),
            };
            if let Some(k) = next {
                r.filter = None;
                r.selected = Some(k);
                r.jump = true;
                r.draft = r.opened.captions.get(k).map(|c| c.text()).unwrap_or_default();
            }
            return;
        }
        Action::ReplaceAll => {
            r.remember();
            let (find, with) = (r.find.find.clone(), r.find.with.clone());
            match r.opened.replace_all(&find, &with, r.find.whole_word) {
                Ok(n) => {
                    if let Some(k) = r.selected {
                        r.draft = r.opened.captions.get(k).map(|c| c.text()).unwrap_or_default();
                    }
                    let learned = if r.find.learn && n > 0 && !with.trim().is_empty() {
                        overfilm_subs::usergloss::add(with.trim(), &[find.trim().to_string()], "찾아 바꾸기에서")
                            .map(|_| " 단어집에도 넣었습니다.")
                            .unwrap_or(" 단어집에는 넣지 못했습니다.")
                    } else {
                        ""
                    };
                    Ok(Some(format!("{n}곳을 바꿨습니다.{learned}")))
                }
                Err(e) => Err(e),
            }
        }
        Action::NextIssue => {
            let shown = rows(r);
            let open: Vec<usize> =
                shown.iter().copied().filter(|&k| r.issues.iter().any(|i| i.index == k + 1 && !r.ignored.contains(&(i.index, i.kind)))).collect();
            let next = match r.selected {
                Some(s) => open.iter().copied().find(|&k| k > s).or_else(|| open.first().copied()),
                None => open.first().copied(),
            };
            if let Some(k) = next {
                r.selected = Some(k);
                r.jump = true;
                r.draft = r.opened.captions.get(k).map(|c| c.text()).unwrap_or_default();
            }
            return;
        }
    };
    r.refresh_issues(mask);
    let open = r.open_issues();
    let srt = r.opened.srt.clone();
    match result {
        Ok(Some(m)) => app.status = Some(Ok(m)),
        Ok(None) => {}
        Err(e) => app.status = Some(Err(e)),
    }
    if let Some(e) = app.recent.iter_mut().find(|e| e.srt == srt) {
        e.open_issues = open;
        let _ = edit::save_recent(&crate::recent_path(), &mut app.recent);
    }
}

/// `m:ss.s` for the voice form.
fn fmt_time(ms: u64) -> String {
    format!("{}:{:02}.{}", ms / 60_000, ms / 1000 % 60, ms % 1000 / 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_and_times() {
        assert_eq!(short_clock(20 * 60_000 + 10_640), "20:10.640");
        assert_eq!(short_clock(3_600_000 + 5), "1:00:00.005");
        assert_eq!(short_clock(-5), "00:00.000");
        assert_eq!(fmt_time(83_400), "1:23.4");
        assert_eq!(overfilm_subs::voices::parse_time(&fmt_time(83_400)), Some(83_400));
    }
}

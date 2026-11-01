//! 단어집: the user's own spellings (add, edit, remove), a search over the built-in pack, and the
//! fixes learned from hand edits waiting to be added.

use eframe::egui::{self, RichText};
use overfilm_subs::{learn, usergloss};

use super::widgets::{self, accent_button, button, caption, section, setting, subtle_button, title};
use crate::App;

enum Act {
    Add,
    Remove(String),
    Edit(String),
    SaveEdit,
    CancelEdit,
    Learn(learn::Pair, bool),
}

pub fn page(app: &mut App, ui: &mut egui::Ui) {
    if app.gloss.entries.is_none() {
        app.gloss.pending = learn::open(2);
        match usergloss::load() {
            Ok(e) => app.gloss.entries = Some(e),
            Err(e) => {
                app.gloss.entries = Some(Vec::new());
                app.status = Some(Err(e));
            }
        }
    }
    if app.gloss.builtin.is_none() {
        app.gloss.builtin = overfilm_subs::pack::glossary(None, true).ok();
    }
    let mut acts = Vec::new();
    super::scroll_page(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        title(ui, "단어집");
        let builtin = app.gloss.builtin.as_ref().map_or(0, |g| g.entries.len());
        caption(
            ui,
            format!(
                "영상에서 '틀린 표기'가 나오면 '맞는 표기'로 고칩니다. 영웅·선수·팀·용어 {builtin}개는 프로그램에 들어 있고, 여기에는 직접 더한 것만 보입니다. 다음 자막 추출과 '단어집대로 고치기'부터 적용됩니다."
            ),
        );
        ui.add_space(8.0);
        super::status_bar(app, ui);

        section(ui, "새 표기 더하기");
        let g = &mut app.gloss;
        setting(ui, "맞는 표기", "자막에 나와야 하는 글자", false, |ui| {
            ui.add(egui::TextEdit::singleline(&mut g.ko).hint_text("예: 정커퀸").desired_width(220.0));
        });
        setting(ui, "틀린 표기", "인식이 잘못 적는 글자, 쉼표로 여러 개", false, |ui| {
            ui.add(egui::TextEdit::singleline(&mut g.misheard).hint_text("예: 전국퀸, 정커 퀸").desired_width(220.0));
        });
        setting(ui, "메모", "나중에 알아보기 쉽게 (선택)", false, |ui| {
            ui.add(egui::TextEdit::singleline(&mut g.note).desired_width(220.0));
        });
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!g.ko.trim().is_empty(), |ui| {
                if accent_button(ui, "더하기").clicked() {
                    acts.push(Act::Add);
                }
            });
            caption(ui, "틀린 표기 없이 이름만 더하면 비슷하게 잘못 적은 이름을 가까운 것으로 고칩니다.");
        });

        let pending = g.pending.clone();
        if !pending.is_empty() {
            section(ui, "직접 고친 내용에서 찾은 후보");
            caption(ui, "검토 화면에서 같은 고침을 여러 번 했습니다. 단어집에 넣으면 다음부터 자동으로 고칩니다.");
            for p in pending.iter().take(30) {
                setting(ui, &format!("{} → {}", p.from, p.to), &format!("{}번 고침", p.count), false, |ui| {
                    if subtle_button(ui, "무시").clicked() {
                        acts.push(Act::Learn(p.clone(), false));
                    }
                    if button(ui, "넣기").clicked() {
                        acts.push(Act::Learn(p.clone(), true));
                    }
                });
            }
        }

        let entries = g.entries.clone().unwrap_or_default();
        section(ui, &format!("내가 더한 표기 {}개", entries.len()));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut g.search).hint_text("찾기 (내장 단어집도 함께)").desired_width(260.0));
        });
        let q = g.search.trim().to_string();
        let shown: Vec<&overfilm_glossary::Entry> =
            entries.iter().filter(|e| q.is_empty() || e.ko.contains(&q) || e.misheard.iter().any(|m| m.contains(&q)) || e.note.contains(&q)).collect();
        if entries.is_empty() {
            widgets::card(ui, |ui| caption(ui, "아직 없습니다. 위에서 더하거나, 검토 화면의 찾아 바꾸기에서 '단어집에도 넣기'를 켜세요."));
        }
        for e in shown {
            let editing = g.editing.as_ref().is_some_and(|x| x.0 == e.id);
            if editing {
                if let Some((_, ko, mis, note)) = g.editing.as_mut() {
                    widgets::card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(ko).desired_width(140.0));
                            ui.add(egui::TextEdit::singleline(mis).hint_text("틀린 표기, 쉼표로").desired_width(200.0));
                            ui.add(egui::TextEdit::singleline(note).hint_text("메모").desired_width(120.0));
                            if accent_button(ui, "저장").clicked() {
                                acts.push(Act::SaveEdit);
                            }
                            if subtle_button(ui, "취소").clicked() {
                                acts.push(Act::CancelEdit);
                            }
                        });
                    });
                }
                continue;
            }
            let desc = match (e.misheard.is_empty(), e.note.is_empty()) {
                (true, true) => "이름만 (비슷한 오타를 고침)".to_string(),
                (false, true) => format!("틀린 표기: {}", e.misheard.join(", ")),
                (true, false) => e.note.clone(),
                (false, false) => format!("틀린 표기: {} · {}", e.misheard.join(", "), e.note),
            };
            setting(ui, &e.ko, &desc, false, |ui| {
                if subtle_button(ui, "지우기").clicked() {
                    acts.push(Act::Remove(e.id.clone()));
                }
                if button(ui, "고치기").clicked() {
                    acts.push(Act::Edit(e.id.clone()));
                }
            });
        }
        if !q.is_empty()
            && let Some(b) = g.builtin.as_ref()
        {
            let hits = usergloss::search(b, &q, 20);
            section(ui, &format!("내장 단어집에서 찾은 것 {}개", hits.len()));
            if hits.is_empty() {
                caption(ui, "내장 단어집에는 없습니다.");
            }
            for e in hits {
                let mut desc = category_label(&e.category).to_string();
                if !e.aliases.is_empty() {
                    desc.push_str(&format!(" · 다른 이름: {}", e.aliases.join(", ")));
                }
                if !e.misheard.is_empty() {
                    desc.push_str(&format!(" · 고치는 표기: {}", e.misheard.join(", ")));
                }
                setting(ui, &e.ko, &desc, false, |ui| {
                    ui.label(RichText::new("내장").small());
                });
            }
        }
    });
    for a in acts {
        run(app, a);
    }
}

fn category_label(c: &str) -> &'static str {
    match c {
        "heroes" => "영웅",
        "abilities" => "기술",
        "maps" => "전장",
        "modes" => "모드",
        "players" => "선수",
        "teams" => "팀",
        "terms" => "용어",
        "slang" => "커뮤니티 표현",
        "netslang" => "인터넷·방송 말",
        "matchups" => "조합",
        "profanity" => "욕설 초성",
        "spelling" => "맞춤법",
        _ => "기타",
    }
}

fn run(app: &mut App, a: Act) {
    let g = &mut app.gloss;
    let r: Result<String, String> = match a {
        Act::Add => {
            let ko = g.ko.trim().to_string();
            let r = usergloss::add(&ko, &usergloss::split_list(&g.misheard), &g.note).map(|_| format!("'{ko}'을(를) 더했습니다."));
            if r.is_ok() {
                g.ko.clear();
                g.misheard.clear();
                g.note.clear();
            }
            r
        }
        Act::Remove(id) => usergloss::remove_from(&usergloss::file(), &id).map(|()| "지웠습니다.".to_string()),
        Act::Edit(id) => {
            if let Some(e) = g.entries.as_ref().and_then(|l| l.iter().find(|e| e.id == id)) {
                g.editing = Some((e.id.clone(), e.ko.clone(), e.misheard.join(", "), e.note.clone()));
            }
            return;
        }
        Act::CancelEdit => {
            g.editing = None;
            return;
        }
        Act::SaveEdit => {
            let Some((id, ko, mis, note)) = g.editing.take() else { return };
            let mut list = g.entries.clone().unwrap_or_default();
            match list.iter_mut().find(|e| e.id == id) {
                Some(e) if !ko.trim().is_empty() => {
                    e.ko = ko.trim().to_string();
                    e.misheard = usergloss::split_list(&mis);
                    e.note = note.trim().to_string();
                    usergloss::save(&list).map(|()| "고쳤습니다.".to_string())
                }
                Some(_) => Err("맞는 표기가 비어 있습니다".into()),
                None => Err("그런 단어가 없습니다".into()),
            }
        }
        Act::Learn(p, add) => {
            if add {
                learn::accept(&p).map(|()| format!("'{}' → '{}'를 단어집에 넣었습니다.", p.from, p.to))
            } else {
                learn::resolve(&p.from, &p.to, false).map(|()| "다시 묻지 않습니다.".to_string())
            }
        }
    };
    g.entries = None; // read again
    app.status = Some(r);
}

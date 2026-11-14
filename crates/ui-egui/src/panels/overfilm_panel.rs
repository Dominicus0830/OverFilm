//! The OverFilm panel (Window ▸ OverFilm, or Sequence ▸ OverFilm ▸ Cut and Caption in One Go):
//! the whole edit in one go (`overfilm.flow.run`: Auto Cut, Claude Code on the unclear cuts if
//! ticked, the cuts applied, the kill scan, OverFilm Captions, Claude Code on the captions if
//! ticked, the render if ticked) with each step's progress, and buttons for the other OverFilm
//! tools grouped in the order of the work. It docks like any panel.
//!
//! Automation ids: `overfilm.panel.<param>` for each control (`autoCut`, `templates`,
//! `keepBefore`, `applyCuts`, `reviewCuts`, `captions`, `audioTrack`, `mode`, `who`, `gameVoice`,
//! `maxLines`, `lineChars`, `reviewCaptions`, `kills`, `render`, `renderFormat`, `renderPath`,
//! `renderSplit`, `burnCaptions`), `overfilm.panel.run`, `overfilm.panel.stop`,
//! `overfilm.panel.step.<n>` for each step of the run, `overfilm.panel.reviewSettings`,
//! `overfilm.panel.tool.<command id>`,
//! `overfilm.panel.kill.<n>` for each kill in the kill list (click: go there),
//! `overfilm.panel.name.<player|hero>.<id>` and `overfilm.panel.name.<player|hero>.<id>.save` to
//! name a player or hero read off the kill feed, `overfilm.panel.heroName.<id>` and `….save` for
//! a hero on the name plates, and `overfilm.panel.preview.<plate|player|hero>.<id>` to show where
//! it is (the part of the frame, beside its name; the playhead moves there too).

use egui::{Rect, RichText, vec2};
use serde_json::{Value, json};

use super::overfilm::{Elems, check, combo, number, plain_error, text};
use crate::FilmcraftApp;

const PRE: &str = "overfilm.panel";

/// The other OverFilm tools, as buttons in the order of the work (menu commands: the dialogs
/// open as from the menu): get ready, cut, read the match, captions, hand over.
const TOOL_GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Get ready", &[("overfilm.screenTemplate.learn", "Learn Screen Template…")]),
    (
        "Cut",
        &[
            ("overfilm.autoCut", "Auto Cut…"),
            ("overfilm.assist.suggest", "Suggest Silences and Highlights"),
            ("overfilm.review.cuts", "Ask Claude Code About Cuts"),
            ("overfilm.autoCut.apply", "Apply Cut Markers"),
        ],
    ),
    ("Read the match", &[("overfilm.kills.scan", "Find Kills, Ultimates and Team Fights"), ("overfilm.highlights.reel", "Make Highlight Reel")]),
    (
        "Captions",
        &[
            ("overfilm.captions.generate", "Captions…"),
            ("overfilm.review.captions", "Ask Claude Code About Captions"),
            ("overfilm.captions.stylize", "Styled Captions…"),
        ],
    ),
    ("Export", &[("file.exportMedia", "Export Media…"), ("overfilm.premiere.export", "Export for Premiere Pro…")]),
];

/// The tools of all groups, in order.
fn tools() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    TOOL_GROUPS.iter().flat_map(|(_, list)| list.iter())
}

/// A group's heading in the user's words.
fn group_label(group: &str) -> &'static str {
    match group {
        "Get ready" => tl!("Get ready"),
        "Cut" => tl!("Cut"),
        "Read the match" => tl!("Read the match"),
        "Captions" => tl!("Captions"),
        "Export" => tl!("Export"),
        _ => "",
    }
}

/// The render formats the run offers (`file.exportMedia` formats, and the Premiere hand-over).
fn render_formats() -> Vec<(&'static str, String)> {
    vec![
        ("h264", tl!("MP4 (H.264)").to_string()),
        ("hevc", tl!("MP4 (H.265, smaller)").to_string()),
        ("av1", tl!("MP4 (AV1)").to_string()),
        ("prores", tl!("MOV (ProRes, for editing)").to_string()),
        ("dnxhr", tl!("MOV (DNxHR, for editing)").to_string()),
        ("gif", tl!("GIF").to_string()),
        ("wav", tl!("WAV (sound only)").to_string()),
        ("aac", tl!("M4A (sound only)").to_string()),
        ("premiere", tl!("Premiere Pro (XML + SRT)").to_string()),
    ]
}

fn state_id() -> egui::Id {
    egui::Id::unique("overfilm-panel-params")
}

/// The panel's choices (kept for the session), first filled from the template file in use.
fn defaults(app: &mut FilmcraftApp) -> Value {
    let templates = app.session.execute("overfilm.screenTemplate.list", json!({})).ok();
    json!({
        "autoCut": true,
        "templates": templates.as_ref().and_then(|t| t["templates"].as_str()).unwrap_or_default(),
        "keepBefore": 10.0,
        "applyCuts": "cut",
        "reviewCuts": false,
        "captions": "new",
        "audioTrack": "",
        "mode": "general",
        "who": "auto",
        "gameVoice": "drop",
        "maxLines": 2,
        "lineChars": 16,
        "reviewCaptions": false,
        "kills": true,
        "render": false,
        "renderFormat": "h264",
        "renderPath": "",
        "renderSplit": "match",
        "burnCaptions": true,
    })
}

/// The run's parameters from the panel's choices.
fn run_params(p: &Value) -> Value {
    let mut out = json!({
        "autoCut": p["autoCut"].as_bool().unwrap_or(true),
        "templates": p["templates"],
        "keepBefore": p["keepBefore"],
        "applyCuts": p["applyCuts"].as_str() != Some("mark"),
        "reviewCuts": p["reviewCuts"].as_bool().unwrap_or(false),
        "captions": p["captions"].as_str().unwrap_or("new"),
        "audioTrack": p["audioTrack"],
        "mode": p["mode"],
        "who": p["who"],
        "gameVoice": p["gameVoice"],
        "maxLines": p["maxLines"],
        "lineChars": p["lineChars"],
        "reviewCaptions": p["reviewCaptions"].as_bool().unwrap_or(false),
        "kills": p["kills"].as_bool().unwrap_or(true),
        "render": p["render"].as_bool().unwrap_or(false),
        "renderFormat": p["renderFormat"].as_str().unwrap_or("h264"),
        "renderPath": p["renderPath"],
        "renderSplit": p["renderSplit"].as_str().unwrap_or("match"),
        "burnCaptions": p["burnCaptions"].as_bool().unwrap_or(true),
    });
    if !out["render"].as_bool().unwrap_or(false)
        && let Some(m) = out.as_object_mut()
    {
        for k in ["renderFormat", "renderPath", "renderSplit", "burnCaptions"] {
            m.remove(k);
        }
    }
    if out["captions"] == "reuse" {
        // laid out from the transcript: nothing is recognised again
        if let Some(m) = out.as_object_mut() {
            for k in ["audioTrack", "mode", "who"] {
                m.remove(k);
            }
        }
    }
    super::overfilm::normalize(out)
}

/// A step of the run in the user's words.
fn step_label(step: &str) -> &'static str {
    match step {
        "autoCut" => tl!("Auto Cut (waiting and pauses)"),
        "reviewCuts" => tl!("Claude Code: unclear cuts"),
        "applyCuts" => tl!("Cut the marked parts"),
        "kills" => tl!("Kills, ultimates and players"),
        "captions" => tl!("Captions"),
        "reviewCaptions" => tl!("Claude Code: captions"),
        "render" => tl!("Render"),
        _ => "",
    }
}

fn state_label(state: &str) -> &'static str {
    match state {
        "pending" => tl!("Waiting"),
        "running" => tl!("Running"),
        "done" => tl!("Done"),
        "skipped" => tl!("Skipped"),
        "failed" => tl!("Failed"),
        _ => "",
    }
}

/// What the panel reads from files and the transcript, read again every second and a half
/// rather than on every frame (the review settings are a file; a transcript can be long).
#[derive(Clone, Default)]
struct Facts {
    at: f64,
    revision: u64,
    review_on: bool,
    has_words: bool,
    tools: Vec<(&'static str, bool)>,
    /// `overfilm.killFeed.list`: the kills and who is who.
    kills: Value,
    /// `overfilm.heroes.list`: the teams, players and hero portraits read off the name plates.
    lineup: Value,
}

fn facts(app: &mut FilmcraftApp, ctx: &egui::Context) -> Facts {
    let id = egui::Id::unique("overfilm-panel-facts");
    let now = ctx.input(|i| i.time);
    let revision = app.session.revision;
    if let Some(f) = ctx.data(|m| m.get_temp::<Facts>(id)).filter(|f| f.revision == revision && (now - f.at).abs() < 1.5) {
        return f;
    }
    let kills =
        if app.session.active_sequence().is_some() { app.session.execute("overfilm.killFeed.list", json!({})).unwrap_or_default() } else { Value::Null };
    let lineup = if app.session.active_sequence().is_some() { app.session.execute("overfilm.heroes.list", json!({})).unwrap_or_default() } else { Value::Null };
    let s = &app.session;
    let f = Facts {
        at: now,
        revision,
        review_on: filmcraft_engine::overfilm_review::settings(s).enabled,
        has_words: !filmcraft_engine::transcript::sequence_words(s).is_empty(),
        tools: tools().map(|(k, _)| (*k, s.is_enabled(k))).collect(),
        kills,
        lineup,
    };
    ctx.data_mut(|m| m.insert_temp(id, f.clone()));
    f
}

fn clock(secs: f64) -> String {
    let t = secs.max(0.0).round() as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}

pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let ctx = ui.ctx().clone();
    let mut p = match ctx.data(|m| m.get_temp::<Value>(state_id())) {
        Some(p) => p,
        None => defaults(app),
    };
    let mut elems: Elems = Vec::new();
    let status = filmcraft_engine::overfilm_flow::status(&app.session);
    let running = status["state"] == "running";
    let has_seq = app.session.active_sequence().is_some();
    let facts = facts(app, &ctx);
    let (review_on, has_words) = (facts.review_on, facts.has_words);
    let tracks: Vec<(String, String)> = app
        .session
        .active_sequence()
        .map(|q| {
            q.audio_tracks
                .iter()
                .enumerate()
                .filter(|(_, tr)| tr.items.iter().any(|c| c.enabled))
                .map(|(i, tr)| ((i + 1).to_string(), format!("A{} · {}", i + 1, tr.name)))
                .collect()
        })
        .unwrap_or_default();
    let (mut run, mut stop, mut settings, mut tool) = (false, false, false, None::<&'static str>);
    let (mut seek, mut name) = (None::<i64>, None::<(&'static str, u64, String)>);
    let mut hero_name = None::<(u64, String)>;
    let mut want_preview = None::<(&'static str, u64)>;
    let previews: Previews = ctx.data(|m| m.get_temp::<Previews>(previews_id())).unwrap_or_default();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 8.0))));
    let ui = &mut child;
    egui::ScrollArea::vertical().id_salt("overfilm-panel-scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.label(RichText::new(tl!("Cut and caption in one go")).strong().size(14.0));
        ui.label(RichText::new(tl!("Runs the ticked steps one after another in the background. Undo takes back each step on its own.")).color(t.text_dim));
        ui.add_space(6.0);
        ui.add_enabled_ui(!running, |ui| {
            // 1. Auto Cut
            check(ui, &mut elems, format!("{PRE}.autoCut"), tl!("Auto Cut waiting and pauses (tournament broadcasts)"), &mut p["autoCut"]);
            if p["autoCut"].as_bool().unwrap_or(false) {
                ui.indent("overfilm-panel-cut", |ui| {
                    egui::Grid::new("overfilm-panel-cut-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        text(
                            ui,
                            &mut elems,
                            format!("{PRE}.templates"),
                            tl!("Template file:"),
                            &mut p["templates"],
                            tl!("your OverFilm template file, or type a .json path"),
                        );
                        number(ui, &mut elems, format!("{PRE}.keepBefore"), tl!("Keep before the start:"), &mut p["keepBefore"], 0.0..=60.0, " s");
                        combo(
                            ui,
                            &mut elems,
                            format!("{PRE}.applyCuts"),
                            tl!("Result:"),
                            &mut p["applyCuts"],
                            &[("cut", tl!("Cut right away").to_string()), ("mark", tl!("Markers to check first").to_string())],
                        );
                    });
                });
            }
            ui.add_enabled_ui(review_on, |ui| {
                check(ui, &mut elems, format!("{PRE}.reviewCuts"), tl!("Then let Claude Code check the unclear cuts"), &mut p["reviewCuts"]);
            });
            // 2. Kills and players, so the captions get the names
            check(ui, &mut elems, format!("{PRE}.kills"), tl!("Read kills, ultimates and players first (names for the captions)"), &mut p["kills"]);
            ui.add_space(6.0);
            // 3. Captions
            egui::Grid::new("overfilm-panel-cap-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                let mut kinds = vec![("new", tl!("Recognise the speech").to_string())];
                if has_words {
                    kinds.push(("reuse", tl!("From the transcript already made (fast)").to_string()));
                }
                kinds.push(("off", tl!("No captions").to_string()));
                combo(ui, &mut elems, format!("{PRE}.captions"), tl!("Captions:"), &mut p["captions"], &kinds);
                if p["captions"] == "new" {
                    let mut opts: Vec<(&str, String)> = vec![("", tl!("All audio tracks").to_string())];
                    opts.extend(tracks.iter().map(|(k, l)| (k.as_str(), l.clone())));
                    combo(ui, &mut elems, format!("{PRE}.audioTrack"), tl!("Audio track:"), &mut p["audioTrack"], &opts);
                    combo(
                        ui,
                        &mut elems,
                        format!("{PRE}.mode"),
                        tl!("Video:"),
                        &mut p["mode"],
                        &[
                            ("general", tl!("Gameplay (heroes and game terms)").to_string()),
                            ("commentary", tl!("Tournament commentary (teams and players too)").to_string()),
                        ],
                    );
                    let counts: Vec<String> = (1..=10).map(|n| n.to_string()).collect();
                    let mut who: Vec<(&str, String)> = vec![("auto", tl!("Count them").to_string()), ("off", tl!("Don't tell them apart").to_string())];
                    who.extend(counts.iter().map(|n| (n.as_str(), if n == "1" { tl!("1 person (faster)").to_string() } else { tlf!("{n} people", n = n) })));
                    combo(ui, &mut elems, format!("{PRE}.who"), tl!("Speakers:"), &mut p["who"], &who);
                }
                if p["captions"] != "off" {
                    combo(
                        ui,
                        &mut elems,
                        format!("{PRE}.gameVoice"),
                        tl!("In-game voice:"),
                        &mut p["gameVoice"],
                        &[
                            ("drop", tl!("Leave out of the captions").to_string()),
                            ("label", tl!("Keep, marked (게임)").to_string()),
                            ("keep", tl!("Keep as speech").to_string()),
                        ],
                    );
                    number(ui, &mut elems, format!("{PRE}.maxLines"), tl!("Lines per caption:"), &mut p["maxLines"], 1.0..=2.0, "");
                    number(ui, &mut elems, format!("{PRE}.lineChars"), tl!("Characters per line:"), &mut p["lineChars"], 8.0..=30.0, "");
                }
            });
            if p["captions"] != "off" {
                ui.add_enabled_ui(review_on, |ui| {
                    check(ui, &mut elems, format!("{PRE}.reviewCaptions"), tl!("Then let Claude Code check the captions"), &mut p["reviewCaptions"]);
                });
            }
            ui.add_space(6.0);
            // 4. Render
            check(ui, &mut elems, format!("{PRE}.render"), tl!("Then render the finished video"), &mut p["render"]);
            if p["render"].as_bool().unwrap_or(false) {
                ui.indent("overfilm-panel-render", |ui| {
                    egui::Grid::new("overfilm-panel-render-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        combo(ui, &mut elems, format!("{PRE}.renderFormat"), tl!("File type:"), &mut p["renderFormat"], &render_formats());
                        text(
                            ui,
                            &mut elems,
                            format!("{PRE}.renderPath"),
                            tl!("Save to:"),
                            &mut p["renderPath"],
                            tl!("a folder or file; empty = beside the project"),
                        );
                        if p["renderFormat"] != "premiere" {
                            combo(
                                ui,
                                &mut elems,
                                format!("{PRE}.renderSplit"),
                                tl!("Matches:"),
                                &mut p["renderSplit"],
                                &[("match", tl!("One file per match (date-team1vsteam2)").to_string()), ("one", tl!("All in one file").to_string())],
                            );
                        }
                    });
                    if p["renderFormat"] != "premiere" && p["captions"] != "off" {
                        check(ui, &mut elems, format!("{PRE}.burnCaptions"), tl!("Burn the captions into the picture"), &mut p["burnCaptions"]);
                    }
                });
            }
        });
        if !review_on {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tl!("Claude Code review is off.")).color(t.text_dim));
                let r = ui.link(tl!("Claude Code Review Settings…"));
                elems.push((format!("{PRE}.reviewSettings"), r.rect, "Claude Code Review Settings…".into()));
                settings |= r.clicked();
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if running {
                let r = ui.button(tl!("Stop"));
                elems.push((format!("{PRE}.stop"), r.rect, "Stop".into()));
                stop = r.clicked();
            } else {
                let r = ui.add_enabled(has_seq, egui::Button::new(RichText::new(tl!("Run")).color(egui::Color32::WHITE)).fill(t.accent));
                elems.push((format!("{PRE}.run"), r.rect, "Run".into()));
                run = r.clicked();
                if !has_seq {
                    ui.label(RichText::new(tl!("Open a sequence first.")).color(t.text_dim));
                }
            }
        });
        // the run's steps
        if let Some(steps) = status["steps"].as_array().filter(|s| !s.is_empty()) {
            ui.add_space(8.0);
            ui.separator();
            for (n, st) in steps.iter().enumerate() {
                let state = st["state"].as_str().unwrap_or_default();
                let color = match state {
                    "failed" => t.danger,
                    "pending" | "skipped" => t.text_faint,
                    _ => t.text,
                };
                let mut line = format!("{}. {} · {}", n + 1, step_label(st["step"].as_str().unwrap_or_default()), state_label(state));
                if let Some(secs) = st["seconds"].as_f64() {
                    line.push_str(&format!(" ({})", clock(secs)));
                }
                let r = ui.label(RichText::new(&line).color(color)).rect;
                let mut detail = st["message"].as_str().unwrap_or_default().to_string();
                if state == "running" {
                    let frac = st["progress"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                    ui.add(egui::ProgressBar::new(frac).desired_height(6.0));
                    detail = format!("{:.0} % {}{}", frac * 100.0, st["status"].as_str().unwrap_or_default(), super::eta_suffix(st));
                }
                if !detail.is_empty() {
                    ui.label(RichText::new(&detail).color(if state == "failed" { t.danger } else { t.text_dim }).size(11.0));
                }
                elems.push((format!("{PRE}.step.{n}"), r, format!("{line} {detail}")));
            }
            let msg = status["message"].as_str().unwrap_or_default();
            if !msg.is_empty() && status["state"] != "failed" {
                ui.label(RichText::new(msg).color(t.text_dim));
            }
        }
        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new(tl!("Tools")).strong());
        egui::Grid::new("overfilm-panel-tools").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for (n, (group, list)) in TOOL_GROUPS.iter().enumerate() {
                ui.label(RichText::new(format!("{}. {}", n + 1, group_label(group))).color(t.text_dim));
                ui.horizontal_wrapped(|ui| {
                    for (id, label) in list.iter() {
                        let enabled = facts.tools.iter().find(|(k, _)| k == id).is_some_and(|(_, on)| *on);
                        let r = ui.add_enabled(enabled, egui::Button::new(crate::i18n::t(label)));
                        elems.push((format!("{PRE}.tool.{id}"), r.rect, (*label).to_string()));
                        if r.clicked() {
                            tool = Some(id);
                        }
                    }
                });
                ui.end_row();
            }
        });
        let mut look = Look { previews: &previews, want: &mut want_preview, seek: &mut seek };
        kill_list(ui, &mut elems, &facts.kills, &mut p, &mut name, &mut look, t);
        lineup_list(ui, &mut elems, &facts.lineup, &mut p, &mut hero_name, &mut look, t);
    });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if settings {
        let _ = crate::menus::invoke(app, &ctx, "overfilm.review.settings", json!({}));
    }
    if let Some(id) = tool
        && let Err(e) = crate::menus::invoke(app, &ctx, id, json!({}))
    {
        app.ui.status = plain_error(&e);
    }
    if let Some(time) = seek
        && let Err(e) = app.session.execute("playhead.set", json!({"time": time}))
    {
        app.ui.status = plain_error(&e.to_string());
    }
    if let Some((kind, id, n)) = name {
        match app.session.execute("overfilm.killFeed.name", json!({"kind": kind, "id": id, "name": n})) {
            Ok(_) => {
                // read the list again now, not in a second and a half
                ctx.data_mut(|m| m.remove::<Facts>(egui::Id::unique("overfilm-panel-facts")));
            }
            Err(e) => app.ui.status = plain_error(&e.to_string()),
        }
    }
    if let Some((kind, id)) = want_preview {
        let templates = p["templates"].as_str().unwrap_or_default().to_string();
        load_preview(app, &ctx, kind, id, &templates);
    }
    if let Some((id, n)) = hero_name {
        match app.session.execute("overfilm.heroes.name", json!({"id": id, "name": n})) {
            Ok(_) => ctx.data_mut(|m| m.remove::<Facts>(egui::Id::unique("overfilm-panel-facts"))),
            Err(e) => app.ui.status = plain_error(&e.to_string()),
        }
    }
    if stop && let Err(e) = app.session.execute("overfilm.flow.stop", json!({})) {
        app.ui.status = plain_error(&e.to_string());
    }
    if run {
        // a Claude Code step checks Claude Code first (install guide or settings when not ready)
        let rp = run_params(&p);
        if let Some(Err(e)) = crate::panels::overfilm_review::route(app, &ctx, "overfilm.flow.run", &rp) {
            app.ui.status = plain_error(&e);
        } else {
            match app.session.execute("overfilm.flow.run", rp) {
                Ok(_) => app.ui.status = tl!("OverFilm: started, see each step in the OverFilm panel").to_string(),
                Err(e) => app.ui.status = plain_error(&e.to_string()),
            }
        }
    }
    if running {
        // the steps move on in the background
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    ctx.data_mut(|m| m.insert_temp(state_id(), p));
}

/// The teams and players read off the screen, and the heroes on the name plates to name (the
/// markers "OverFilm 영웅 선택/교체" use the names, and so do OverFilm Captions).
fn lineup_list(
    ui: &mut egui::Ui,
    elems: &mut Elems,
    lineup: &Value,
    p: &mut Value,
    name: &mut Option<(u64, String)>,
    look: &mut Look,
    t: crate::theme::Tokens,
) {
    let heroes = lineup["heroes"].as_array().cloned().unwrap_or_default();
    let teams: Vec<&str> = lineup["teams"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let players: Vec<&str> = lineup["players"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if heroes.is_empty() && teams.is_empty() && players.is_empty() {
        return;
    }
    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new(tl!("Match on screen")).strong());
    if teams.len() == 2 {
        let r = ui.label(format!("{} vs {}", teams[0], teams[1]));
        elems.push((format!("{PRE}.match.teams"), r.rect, format!("{} vs {}", teams[0], teams[1])));
    }
    if !players.is_empty() {
        let line = players.join(", ");
        let r = ui.label(RichText::new(&line).color(t.text_dim).size(11.0));
        elems.push((format!("{PRE}.match.players"), r.rect, line));
    }
    if heroes.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(tl!("Name the heroes on the name plates")).id_salt("overfilm-panel-plate-heroes").show(ui, |ui| {
        ui.label(
            RichText::new(tl!("Each hero is numbered where it first appears (see the hero markers). Name it once: OverFilm knows the portrait again in later videos, and captions prefer the heroes in play."))
                .color(t.text_dim)
                .size(11.0),
        );
        egui::Grid::new("overfilm-panel-plate-heroes-grid").num_columns(5).spacing([8.0, 4.0]).show(ui, |ui| {
            for it in &heroes {
                let id = it["id"].as_u64().unwrap_or(0);
                look.show(ui, elems, "plate", id, it);
                name_label(ui, it, t);
                ui.label(RichText::new(it["first"].as_str().unwrap_or_default()).color(t.text_dim).size(11.0));
                let key = format!("plate.{id}");
                let mut v = p["names"][&key].as_str().unwrap_or_default().to_string();
                let r = ui.add(egui::TextEdit::singleline(&mut v).hint_text(tl!("hero name")).desired_width(120.0).min_size(vec2(120.0, 0.0)));
                elems.push((format!("{PRE}.heroName.{id}"), r.rect, v.clone()));
                let b = ui.add_enabled(!v.trim().is_empty(), egui::Button::new(tl!("Save")));
                elems.push((format!("{PRE}.heroName.{id}.save"), b.rect, "Save".into()));
                if b.clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !v.trim().is_empty()) {
                    *name = Some((id, v.trim().to_string()));
                    v.clear();
                }
                if !p["names"].is_object() {
                    p["names"] = json!({});
                }
                p["names"][&key] = json!(v);
                ui.end_row();
            }
        });
    });
}

/// The kills found on the sequence (kill feed: who killed whom; else the name plates' deaths),
/// each a link to its time, and the kill feed's players and heroes to name.
fn kill_list(
    ui: &mut egui::Ui,
    elems: &mut Elems,
    kills: &Value,
    p: &mut Value,
    name: &mut Option<(&'static str, u64, String)>,
    look: &mut Look,
    t: crate::theme::Tokens,
) {
    let Some(list) = kills["kills"].as_array().filter(|l| !l.is_empty()) else { return };
    ui.add_space(10.0);
    ui.separator();
    ui.label(RichText::new(tlf!("Kills ({n})", n = list.len())).strong());
    if kills["source"] == "plates" {
        ui.label(RichText::new(tl!("Who died, from the name plates. Learn the kill feed to see who killed whom.")).color(t.text_dim).size(11.0));
    }
    egui::ScrollArea::vertical().id_salt("overfilm-panel-kills").max_height(260.0).show(ui, |ui| {
        for (n, k) in list.iter().enumerate() {
            let line = format!("{}  {}", k["clock"].as_str().unwrap_or_default(), k["text"].as_str().unwrap_or_default());
            let color = if k["ult"].as_bool().unwrap_or(false) { t.accent } else { t.text };
            let mut r = ui.selectable_label(false, RichText::new(&line).color(color));
            elems.push((format!("{PRE}.kill.{n}"), r.rect, line.clone()));
            if let Some(c) = k["comment"].as_str().filter(|c| !c.is_empty()) {
                r = r.on_hover_text(c);
            }
            if r.clicked() {
                *look.seek = k["time"].as_i64();
            }
        }
    });
    let players = kills["players"].as_array().cloned().unwrap_or_default();
    let heroes = kills["heroes"].as_array().cloned().unwrap_or_default();
    if players.is_empty() && heroes.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(tl!("Name players and heroes")).id_salt("overfilm-panel-names").show(ui, |ui| {
        ui.label(
            RichText::new(tl!("See each one (or click a kill), then type the name. OverFilm knows the name or picture again in later videos. A hero named here or on the name plates is named in both; players are named from the plates' text."))
                .color(t.text_dim)
                .size(11.0),
        );
        for (kind, items) in [("player", &players), ("hero", &heroes)] {
            egui::Grid::new(format!("overfilm-panel-names-{kind}")).num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
                for it in items {
                    let id = it["id"].as_u64().unwrap_or(0);
                    look.show(ui, elems, if kind == "player" { "player" } else { "hero" }, id, it);
                    name_label(ui, it, t);
                    let key = format!("{kind}.{id}");
                    let mut v = p["names"][&key].as_str().unwrap_or_default().to_string();
                    let hint = if kind == "player" { tl!("player name") } else { tl!("hero name") };
                    let r = ui.add(egui::TextEdit::singleline(&mut v).hint_text(hint).desired_width(140.0).min_size(vec2(140.0, 0.0)));
                    elems.push((format!("{PRE}.name.{key}"), r.rect, v.clone()));
                    let b = ui.add_enabled(!v.trim().is_empty(), egui::Button::new(tl!("Save")));
                    elems.push((format!("{PRE}.name.{key}.save"), b.rect, "Save".into()));
                    if b.clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !v.trim().is_empty()) {
                        *name = Some((if kind == "player" { "player" } else { "hero" }, id, v.trim().to_string()));
                        v.clear();
                    }
                    if !p["names"].is_object() {
                        p["names"] = json!({});
                    }
                    p["names"][&key] = json!(v);
                    ui.end_row();
                }
            });
        }
    });
}

/// Pictures of what is being named, by "kind.id@time" (the time tells a new scan's from an old
/// one's).
type Previews = Vec<(String, egui::TextureHandle)>;

fn previews_id() -> egui::Id {
    egui::Id::unique("overfilm-panel-previews")
}

/// Most pictures kept.
const MAX_PREVIEWS: usize = 64;

fn preview_key(kind: &str, id: u64, it: &Value) -> String {
    format!("{kind}.{id}@{}", it["preview"]["time"].as_i64().unwrap_or(-1))
}

/// The previews shown so far and the one asked for, and a time to move the playhead to.
struct Look<'a> {
    previews: &'a Previews,
    want: &'a mut Option<(&'static str, u64)>,
    seek: &'a mut Option<i64>,
}

impl Look<'_> {
    /// The picture of `it` when it was looked at, else a button to look at it.
    fn show(&mut self, ui: &mut egui::Ui, elems: &mut Elems, kind: &'static str, id: u64, it: &Value) {
        let key = preview_key(kind, id, it);
        let aid = format!("{PRE}.preview.{kind}.{id}");
        match self.previews.iter().find(|(k, _)| *k == key) {
            Some((_, tex)) => {
                let size = tex.size_vec2();
                let h = if kind == "plate" { 56.0 } else { 36.0 };
                let w = (size.x * h / size.y.max(1.0)).min(220.0);
                let r = ui.add(egui::Image::new((tex.id(), vec2(w, h))).sense(egui::Sense::click())).on_hover_text(tl!("Click to go there"));
                elems.push((aid, r.rect, "preview".into()));
                if r.clicked() {
                    *self.want = Some((kind, id));
                }
            }
            None => {
                let r =
                    ui.add_enabled(!it["preview"].is_null(), egui::Button::new(tl!("See it"))).on_hover_text(tl!("Show it here and move the playhead there"));
                elems.push((aid, r.rect, "See it".into()));
                if r.clicked() {
                    *self.want = Some((kind, id));
                }
            }
        }
    }
}

/// The name, marked when it was taken from the other reading rather than typed.
fn name_label(ui: &mut egui::Ui, it: &Value, t: crate::theme::Tokens) {
    let shown = it["name"].as_str().unwrap_or_default();
    if it["auto"].as_bool().unwrap_or(false) {
        ui.label(RichText::new(tlf!("{name} (auto)", name = shown)).color(t.text_dim))
            .on_hover_text(tl!("Taken from the name plates or the kill feed. Type a name to correct it."));
    } else {
        ui.label(shown);
    }
}

/// Cut the picture of `kind` `id` out of its frame, keep it for the list and move the playhead
/// there.
fn load_preview(app: &mut FilmcraftApp, ctx: &egui::Context, kind: &'static str, id: u64, templates: &str) {
    let Some(k) = filmcraft_engine::overfilm_names::Kind::parse(kind) else { return };
    let params = if templates.is_empty() { json!({"kind": kind, "id": id}) } else { json!({"kind": kind, "id": id, "templates": templates}) };
    let templates = match app.session.execute("overfilm.names.preview", params) {
        Ok(v) => v["templates"].as_str().unwrap_or_default().to_string(),
        Err(e) => {
            app.ui.status = plain_error(&e.to_string());
            return;
        }
    };
    match filmcraft_engine::overfilm_names::preview_image(&app.session, k, id as usize, &templates) {
        Ok((px, w, h, time)) => {
            let key = format!("{kind}.{id}@{}", time.0);
            let tex = ctx.load_texture(format!("overfilm-preview-{key}"), egui::ColorImage::from_rgba_unmultiplied([w, h], &px), egui::TextureOptions::LINEAR);
            let mut list: Previews = ctx.data(|m| m.get_temp::<Previews>(previews_id())).unwrap_or_default();
            list.retain(|(k, _)| *k != key);
            if list.len() >= MAX_PREVIEWS {
                drop(list.remove(0));
            }
            list.push((key, tex));
            ctx.data_mut(|m| m.insert_temp(previews_id(), list));
            if let Err(e) = app.session.execute("playhead.set", json!({"time": time.0})) {
                app.ui.status = plain_error(&e.to_string());
            }
        }
        Err(e) => app.ui.status = plain_error(&e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_params_from_the_panel() {
        let p = json!({"autoCut": true, "templates": "s.json", "keepBefore": 10.0, "applyCuts": "mark", "reviewCuts": true,
                       "captions": "new", "audioTrack": "2", "mode": "commentary", "who": "1", "gameVoice": "drop", "reviewCaptions": false});
        assert_eq!(
            run_params(&p),
            json!({"autoCut": true, "templates": "s.json", "keepBefore": 10.0, "applyCuts": false, "reviewCuts": true, "captions": "new",
                   "audioTrack": 2, "mode": "commentary", "speakers": true, "numSpeakers": 1, "gameVoice": "drop", "reviewCaptions": false})
        );
        // from the transcript: no recognition settings; an empty template file is left to the default
        let p = json!({"autoCut": false, "templates": "", "captions": "reuse", "audioTrack": "1", "who": "3", "gameVoice": "label"});
        assert_eq!(
            run_params(&p),
            json!({"autoCut": false, "applyCuts": true, "reviewCuts": false, "captions": "reuse", "gameVoice": "label", "reviewCaptions": false})
        );
        assert_eq!(run_params(&Value::Null)["captions"], "new");
        // the caption shape set in the panel goes to the run
        let p = json!({"captions": "reuse", "maxLines": 1.0, "lineChars": 20.4});
        let r = run_params(&p);
        assert_eq!((r["maxLines"].clone(), r["lineChars"].clone()), (json!(1), json!(20)));
    }
}

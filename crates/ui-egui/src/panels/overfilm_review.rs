//! OverFilm: getting Claude Code ready and its review settings.
//!
//! - The settings dialog (Sequence ▸ OverFilm ▸ Claude Code Review Settings…,
//!   `overfilm.review.settings` without parameters): the on/off switch, review on its own, the
//!   model picked from the list the installed Claude Code offers (or typed), the `claude` path,
//!   time limit, the sign-in state with a Sign In button.
//! - Asking for a review (`overfilm.review.cuts`, `overfilm.review.captions`, `overfilm.flow.run`
//!   with a review step) checks first: Claude Code not installed opens the install guide; review
//!   off, not signed in or signed in with an API key opens the settings with what to do. A good
//!   check is remembered for ten minutes.
//!
//! Automation ids: `overfilm.review.<param>` for each control, `overfilm.review.modelPick`,
//! `overfilm.review.check`, `overfilm.review.login`, `overfilm.review.ok`,
//! `overfilm.review.cancel`; install guide `overfilm.review.install.copy<N>`,
//! `overfilm.review.install.docs`, `overfilm.review.install.recheck`,
//! `overfilm.review.install.close`.

use serde_json::{Value, json};

use crate::FilmcraftApp;

const COMMAND: &str = "overfilm.review.settings";
/// How long a good check is trusted (seconds).
const READY_FOR: f64 = 600.0;

#[derive(Clone)]
struct Draft {
    params: Value,
    /// Last check: the status message and whether it is ready.
    check: Option<(String, bool)>,
    /// Installed but not signed in with a Claude account: offer Sign In.
    needs_login: bool,
    /// The models to choose from: (id, label); the first is Claude Code's default.
    models: Vec<(String, String)>,
    /// The model is typed instead of picked.
    typed: bool,
    /// Why the dialog opened on its own (what to do).
    notice: String,
    error: String,
}

#[derive(Clone)]
struct Install {
    steps: Vec<(String, String)>,
    docs: String,
    message: String,
}

fn draft_id() -> egui::Id {
    egui::Id::unique("overfilm-review-draft")
}

fn install_id() -> egui::Id {
    egui::Id::unique("overfilm-review-install")
}

fn ready_id() -> egui::Id {
    egui::Id::unique("overfilm-review-ready-until")
}

/// The menu item without parameters opens the dialog with the saved settings; asking for a
/// review checks Claude Code first.
pub fn route(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == COMMAND && params.as_object().is_none_or(|m| m.is_empty()) {
        open_settings(app, ctx, String::new());
        return Some(Ok(json!({"dialog": "overfilm.review"})));
    }
    let asks = match id {
        "overfilm.review.cuts" | "overfilm.review.captions" => true,
        "overfilm.flow.run" => params["reviewCuts"].as_bool().unwrap_or(false) || params["reviewCaptions"].as_bool().unwrap_or(false),
        _ => false,
    };
    if !asks {
        return None;
    }
    preflight(app, ctx).err().map(Err)
}

/// Whether Claude Code can review now; when it cannot, the install guide or the settings open
/// with what to do, and the error says why.
pub fn preflight(app: &mut FilmcraftApp, ctx: &egui::Context) -> Result<(), String> {
    let now = ctx.input(|i| i.time);
    let enabled = app.session.execute(COMMAND, json!({})).ok().and_then(|v| v["enabled"].as_bool()).unwrap_or(false);
    if enabled && ctx.data(|m| m.get_temp::<f64>(ready_id())).is_some_and(|t| now < t) {
        return Ok(());
    }
    let status = app.session.execute("overfilm.review.status", json!({})).map(|v| v["status"].clone()).unwrap_or_default();
    let flag = |k: &str| status[k].as_bool().unwrap_or(false);
    if !flag("installed") {
        open_install(app, ctx, tl!("Claude Code is not installed on this computer. Install it, sign in, then check again.").to_string());
        return Err(tl!("Claude Code is not installed: follow the install guide").to_string());
    }
    if !flag("loggedIn") || !flag("subscription") {
        open_settings(app, ctx, tl!("Sign in to Claude Code with your Claude account (Sign In below), then choose a model.").to_string());
        return Err(tl!("Claude Code is not signed in: sign in in the review settings").to_string());
    }
    if !enabled {
        open_settings(app, ctx, tl!("Claude Code review is off. Turn it on and choose a model.").to_string());
        return Err(tl!("Claude Code review is off: turn it on in the review settings").to_string());
    }
    ctx.data_mut(|m| m.insert_temp(ready_id(), now + READY_FOR));
    Ok(())
}

fn models_of(app: &mut FilmcraftApp, cli_path: &str) -> Vec<(String, String)> {
    let v = app.session.execute("overfilm.review.models", json!({"cliPath": cli_path})).unwrap_or_default();
    v["models"]
        .as_array()
        .map(|a| a.iter().map(|m| (m["id"].as_str().unwrap_or_default().to_string(), m["label"].as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default()
}

/// Check Claude Code at `cli_path`: (message, ready, needs sign-in).
fn check_now(app: &mut FilmcraftApp, cli_path: &str) -> (String, bool, bool) {
    match app.session.execute("overfilm.review.status", json!({"cliPath": cli_path})) {
        Ok(v) => {
            let st = &v["status"];
            let flag = |k: &str| st[k].as_bool().unwrap_or(false);
            let ok = flag("loggedIn") && flag("subscription");
            (st["message"].as_str().unwrap_or_default().to_string(), ok, flag("installed") && !ok)
        }
        Err(e) => (e.to_string(), false, false),
    }
}

/// Open the settings dialog with the saved settings, the model list and a fresh check.
fn open_settings(app: &mut FilmcraftApp, ctx: &egui::Context, notice: String) {
    let current = app.session.execute(COMMAND, json!({})).unwrap_or_default();
    let path = current["cliPath"].as_str().unwrap_or_default().to_string();
    let models = models_of(app, &path);
    let model = current["model"].as_str().unwrap_or_default();
    let typed = !models.iter().any(|(id, _)| id == model);
    let (msg, ok, needs_login) = check_now(app, &path);
    let d = Draft { params: current, check: Some((msg, ok)), needs_login, models, typed, notice, error: String::new() };
    ctx.data_mut(|m| {
        m.insert_temp(draft_id(), Some(d));
        m.insert_temp::<Option<Install>>(install_id(), None);
    });
}

fn open_install(app: &mut FilmcraftApp, ctx: &egui::Context, message: String) {
    let v = app.session.execute("overfilm.review.install", json!({})).unwrap_or_default();
    let steps = v["steps"]
        .as_array()
        .map(|a| a.iter().map(|s| (s["text"].as_str().unwrap_or_default().to_string(), s["command"].as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default();
    let i = Install { steps, docs: v["docs"].as_str().unwrap_or_default().to_string(), message };
    ctx.data_mut(|m| {
        m.insert_temp(install_id(), Some(i));
        m.insert_temp::<Option<Draft>>(draft_id(), None);
    });
}

type Elems = Vec<(String, egui::Rect, String)>;

fn check(ui: &mut egui::Ui, elems: &mut Elems, key: &str, label: &str, v: &mut Value) {
    let mut b = v.as_bool().unwrap_or(false);
    let r = ui.checkbox(&mut b, label);
    *v = json!(b);
    elems.push((format!("overfilm.review.{key}"), r.rect, label.to_string()));
}

fn text(ui: &mut egui::Ui, elems: &mut Elems, key: &str, label: &str, v: &mut Value, hint: &str) {
    ui.label(label);
    let mut t = v.as_str().unwrap_or_default().to_string();
    let r = ui.add(egui::TextEdit::singleline(&mut t).hint_text(hint).desired_width(260.0));
    *v = json!(t);
    elems.push((format!("overfilm.review.{key}"), r.rect, t));
    ui.end_row();
}

fn number(ui: &mut egui::Ui, elems: &mut Elems, key: &str, label: &str, v: &mut Value, range: std::ops::RangeInclusive<f64>, suffix: &str) {
    ui.label(label);
    let mut n = v.as_f64().unwrap_or(*range.start());
    let r = ui.add(crate::widgets::number_drag(&mut n).range(range).suffix(suffix).speed(0.2).max_decimals(0));
    *v = json!(n.round().max(0.0) as u64);
    elems.push((format!("overfilm.review.{key}"), r.rect, format!("{n}")));
    ui.end_row();
}

/// The model row: a list of what the installed Claude Code offers, or a typed name.
fn model_row(ui: &mut egui::Ui, elems: &mut Elems, d: &mut Draft) {
    ui.label(tl!("Model:"));
    let current = d.params["model"].as_str().unwrap_or_default().to_string();
    let shown = if d.typed {
        tl!("Type a model name…").to_string()
    } else {
        d.models.iter().find(|(id, _)| *id == current).map(|(_, l)| l.clone()).unwrap_or_else(|| current.clone())
    };
    ui.vertical(|ui| {
        let r = craft_ui::ComboBox::from_id_salt("overfilm-review-model").width(260.0).selected_text(&shown).show_ui(ui, |ui| {
            for (id, label) in &d.models {
                if ui.selectable_label(!d.typed && *id == current, label).clicked() {
                    d.params["model"] = json!(id);
                    d.typed = false;
                }
            }
            if ui.selectable_label(d.typed, tl!("Type a model name…")).clicked() {
                d.typed = true;
            }
        });
        elems.push(("overfilm.review.modelPick".into(), r.response.rect, shown));
        if d.typed {
            let mut t = current;
            let r = ui.add(egui::TextEdit::singleline(&mut t).hint_text(tl!("full model name, e.g. claude-sonnet-5-5")).desired_width(260.0));
            d.params["model"] = json!(t);
            elems.push(("overfilm.review.model".into(), r.rect, t));
        }
    });
    ui.end_row();
}

/// The settings command's parameters from the dialog.
fn run_params(p: &Value) -> Value {
    json!({
        "enabled": p["enabled"].as_bool().unwrap_or(false),
        "auto": p["auto"].as_bool().unwrap_or(false),
        "cliPath": p["cliPath"].as_str().unwrap_or_default(),
        "model": p["model"].as_str().unwrap_or_default(),
        "timeoutSeconds": p["timeoutSecs"].as_u64().unwrap_or(180),
        "batch": p["batch"].as_u64().unwrap_or(6),
        "maxItems": p["maxItems"].as_u64().unwrap_or(60),
    })
}

pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    show_install(app, ctx);
    show_settings(app, ctx);
}

#[derive(Default)]
struct Clicks {
    close: bool,
    apply: bool,
    test: bool,
    login: bool,
}

fn settings_body(ui: &mut egui::Ui, elems: &mut Elems, d: &mut Draft, c: &mut Clicks) {
    ui.set_min_width(440.0);
    if !d.notice.is_empty() {
        ui.colored_label(ui.visuals().warn_fg_color, &d.notice);
        ui.add_space(4.0);
    }
    ui.label(tl!("When it is on, OverFilm asks the Claude Code on this computer about cuts and captions it is unsure of. It uses your Claude subscription (the account you signed in to Claude Code with), not an API key."));
    ui.add_space(6.0);
    check(ui, elems, "enabled", tl!("Let Claude Code review"), &mut d.params["enabled"]);
    check(ui, elems, "auto", tl!("Ask on its own after Auto Cut, Suggestions and OverFilm Captions"), &mut d.params["auto"]);
    ui.add_space(4.0);
    egui::Grid::new("overfilm-review-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        model_row(ui, elems, d);
        let p = &mut d.params;
        text(ui, elems, "cliPath", tl!("claude program:"), &mut p["cliPath"], tl!("empty: find it (claude.exe, claude.cmd)"));
        number(ui, elems, "timeoutSecs", tl!("Time limit per question:"), &mut p["timeoutSecs"], 10.0..=1800.0, tl!(" s"));
        number(ui, elems, "batch", tl!("Items per question:"), &mut p["batch"], 1.0..=20.0, "");
        number(ui, elems, "maxItems", tl!("Most items per review:"), &mut p["maxItems"], 1.0..=500.0, "");
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let r = ui.button(tl!("Check Claude Code"));
        c.test = r.clicked();
        elems.push(("overfilm.review.check".into(), r.rect, tl!("Check Claude Code").to_string()));
        if d.needs_login {
            let r = ui.button(tl!("Sign In…"));
            c.login = r.clicked();
            elems.push(("overfilm.review.login".into(), r.rect, tl!("Sign In…").to_string()));
        }
        if let Some((msg, ok)) = &d.check {
            let color = if *ok { ui.visuals().text_color() } else { ui.visuals().error_fg_color };
            ui.colored_label(color, msg);
        }
    });
    ui.weak(tl!("Review never cuts by itself: it changes the OverFilm cut markers (cut, keep, keep part) and the captions, in one undo step."));
    if !d.error.is_empty() {
        ui.colored_label(ui.visuals().error_fg_color, &d.error);
    }
    ui.separator();
    ui.horizontal(|ui| {
        let ok = ui.button(tl!("OK"));
        c.apply = ok.clicked();
        elems.push(("overfilm.review.ok".into(), ok.rect, tl!("OK").to_string()));
        let cancel = ui.button(tl!("Cancel"));
        c.close = cancel.clicked();
        elems.push(("overfilm.review.cancel".into(), cancel.rect, tl!("Cancel").to_string()));
    });
}

fn show_settings(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(Some(mut d)) = ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())) else { return };
    let mut elems: Elems = Vec::new();
    let mut c = Clicks::default();
    egui::Window::new(tl!("Claude Code Review (OverFilm)"))
        .id(egui::Id::unique("overfilm-review-dialog"))
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(crate::widgets::dialog_center(ctx))
        .show(ctx, |ui| settings_body(ui, &mut elems, &mut d, &mut c));
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if crate::widgets::escape_closes(ctx) {
        c.close = true;
    }
    let path = d.params["cliPath"].as_str().unwrap_or_default().to_string();
    if c.login {
        match app.session.execute("overfilm.review.login", json!({"cliPath": path})) {
            Ok(v) => d.check = Some((v["message"].as_str().unwrap_or_default().to_string(), false)),
            Err(e) => d.check = Some((e.to_string(), false)),
        }
    }
    if c.test {
        // the path being typed is the one checked; the list may change with it
        let (msg, ok, needs_login) = check_now(app, &path);
        d.check = Some((msg, ok));
        d.needs_login = needs_login;
        d.models = models_of(app, &path);
        if ok {
            ctx.data_mut(|m| m.remove::<f64>(ready_id()));
        }
    }
    if c.apply {
        match app.session.execute(COMMAND, run_params(&d.params)) {
            Ok(v) => {
                app.ui.status = if v["enabled"].as_bool().unwrap_or(false) {
                    tl!("Claude Code review is on").to_string()
                } else {
                    tl!("Claude Code review is off").to_string()
                };
                c.close = true;
            }
            Err(e) => d.error = e.to_string(),
        }
    }
    ctx.data_mut(|m| m.insert_temp(draft_id(), if c.close { None } else { Some(d) }));
}

fn show_install(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(Some(mut inst)) = ctx.data(|m| m.get_temp::<Option<Install>>(install_id())) else { return };
    let mut elems: Elems = Vec::new();
    let (mut close, mut recheck, mut docs) = (false, false, false);
    let mut copy: Option<String> = None;
    egui::Window::new(tl!("Install Claude Code (OverFilm)"))
        .id(egui::Id::unique("overfilm-review-install-dialog"))
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(crate::widgets::dialog_center(ctx))
        .show(ctx, |ui| {
            ui.set_min_width(460.0);
            ui.colored_label(ui.visuals().warn_fg_color, &inst.message);
            ui.add_space(6.0);
            for (n, (text, command)) in inst.steps.iter().enumerate() {
                ui.label(format!("{}. {text}", n + 1));
                if !command.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.code(command);
                        let r = ui.small_button(tl!("Copy"));
                        if r.clicked() {
                            copy = Some(command.clone());
                        }
                        elems.push((format!("overfilm.review.install.copy{}", n + 1), r.rect, command.clone()));
                    });
                }
                ui.add_space(4.0);
            }
            ui.weak(tl!("Claude Code review needs a Claude Pro or Max subscription. OverFilm never uses an API key."));
            ui.separator();
            ui.horizontal(|ui| {
                let r = ui.button(tl!("Check Again"));
                recheck = r.clicked();
                elems.push(("overfilm.review.install.recheck".into(), r.rect, tl!("Check Again").to_string()));
                let r = ui.button(tl!("Install Guide (web)"));
                docs = r.clicked();
                elems.push(("overfilm.review.install.docs".into(), r.rect, inst.docs.clone()));
                let r = ui.button(tl!("Close"));
                close = r.clicked();
                elems.push(("overfilm.review.install.close".into(), r.rect, tl!("Close").to_string()));
            });
        });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if crate::widgets::escape_closes(ctx) {
        close = true;
    }
    if let Some(c) = copy {
        ctx.copy_text(c);
        app.ui.status = tl!("Copied: paste it into PowerShell or Terminal").to_string();
    }
    if docs && !inst.docs.is_empty() {
        ctx.open_url(egui::OpenUrl::new_tab(inst.docs.clone()));
    }
    if recheck {
        let st = app.session.execute("overfilm.review.status", json!({})).map(|v| v["status"].clone()).unwrap_or_default();
        if st["installed"].as_bool().unwrap_or(false) {
            // next: sign in and choose a model
            open_settings(app, ctx, tl!("Claude Code is installed. Sign in if asked, turn review on and choose a model.").to_string());
            return;
        }
        inst.message = format!("{} {}", tl!("Still not found."), st["message"].as_str().unwrap_or_default());
    }
    ctx.data_mut(|m| m.insert_temp(install_id(), if close { None } else { Some(inst) }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_from_the_saved_settings() {
        let p = json!({"enabled": true, "auto": false, "cliPath": "C:\\claude.exe", "model": "", "timeoutSecs": 120, "batch": 4, "maxItems": 30});
        assert_eq!(
            run_params(&p),
            json!({"enabled": true, "auto": false, "cliPath": "C:\\claude.exe", "model": "", "timeoutSeconds": 120, "batch": 4, "maxItems": 30})
        );
        assert_eq!(run_params(&Value::Null)["enabled"], false);
    }
}

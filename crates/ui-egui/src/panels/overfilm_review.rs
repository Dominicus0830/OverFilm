//! OverFilm: the Claude Code review settings dialog (Sequence ▸ OverFilm ▸ Claude Code Review
//! Settings…, `overfilm.review.settings` without parameters): the on/off switch, review on its
//! own, the `claude` path, model, time limit, and a check that Claude Code is installed and
//! signed in with a Claude subscription.
//!
//! Automation ids: `overfilm.review.<param>` for each control, `overfilm.review.check`,
//! `overfilm.review.ok`, `overfilm.review.cancel`.

use serde_json::{Value, json};

use crate::FilmcraftApp;

const COMMAND: &str = "overfilm.review.settings";

#[derive(Clone)]
struct Draft {
    params: Value,
    /// Last check: the status message and whether it is ready.
    check: Option<(String, bool)>,
    error: String,
}

fn draft_id() -> egui::Id {
    egui::Id::unique("overfilm-review-draft")
}

/// The menu item without parameters opens the dialog with the saved settings.
pub fn route(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != COMMAND || !params.as_object().is_none_or(|m| m.is_empty()) {
        return None;
    }
    let current = app.session.execute(COMMAND, json!({})).unwrap_or_default();
    let d = Draft { params: current, check: None, error: String::new() };
    ctx.data_mut(|m| m.insert_temp(draft_id(), Some(d)));
    Some(Ok(json!({"dialog": "overfilm.review"})))
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
    let Some(Some(mut d)) = ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())) else { return };
    let mut elems: Elems = Vec::new();
    let (mut close, mut apply, mut test) = (false, false, false);
    egui::Window::new(tl!("Claude Code Review (OverFilm)"))
        .id(egui::Id::unique("overfilm-review-dialog"))
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(crate::widgets::dialog_center(ctx))
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label(tl!("When it is on, OverFilm asks the Claude Code on this computer about cuts and captions it is unsure of. It uses your Claude subscription (the account you signed in to Claude Code with), not an API key."));
            ui.add_space(6.0);
            check(ui, &mut elems, "enabled", tl!("Let Claude Code review"), &mut d.params["enabled"]);
            check(ui, &mut elems, "auto", tl!("Ask on its own after Auto Cut, Suggestions and OverFilm Captions"), &mut d.params["auto"]);
            ui.add_space(4.0);
            egui::Grid::new("overfilm-review-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                let p = &mut d.params;
                text(ui, &mut elems, "cliPath", tl!("claude program:"), &mut p["cliPath"], tl!("empty: find it (claude.exe, claude.cmd)"));
                text(ui, &mut elems, "model", tl!("Model:"), &mut p["model"], tl!("empty: Claude Code's default (e.g. sonnet, opus)"));
                number(ui, &mut elems, "timeoutSecs", tl!("Time limit per question:"), &mut p["timeoutSecs"], 10.0..=1800.0, tl!(" s"));
                number(ui, &mut elems, "batch", tl!("Items per question:"), &mut p["batch"], 1.0..=20.0, "");
                number(ui, &mut elems, "maxItems", tl!("Most items per review:"), &mut p["maxItems"], 1.0..=500.0, "");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let r = ui.button(tl!("Check Claude Code"));
                test = r.clicked();
                elems.push(("overfilm.review.check".into(), r.rect, tl!("Check Claude Code").to_string()));
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
                apply = ok.clicked();
                elems.push(("overfilm.review.ok".into(), ok.rect, tl!("OK").to_string()));
                let cancel = ui.button(tl!("Cancel"));
                close = cancel.clicked();
                elems.push(("overfilm.review.cancel".into(), cancel.rect, tl!("Cancel").to_string()));
            });
        });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if crate::widgets::escape_closes(ctx) {
        close = true;
    }
    if test {
        // the path being typed is the one checked
        match app.session.execute("overfilm.review.status", json!({"cliPath": d.params["cliPath"].as_str().unwrap_or_default()})) {
            Ok(v) => {
                let st = &v["status"];
                let ok = st["loggedIn"].as_bool().unwrap_or(false) && st["subscription"].as_bool().unwrap_or(false);
                d.check = Some((st["message"].as_str().unwrap_or_default().to_string(), ok));
            }
            Err(e) => d.check = Some((e.to_string(), false)),
        }
    }
    if apply {
        match app.session.execute(COMMAND, run_params(&d.params)) {
            Ok(v) => {
                app.ui.status = if v["enabled"].as_bool().unwrap_or(false) {
                    tl!("Claude Code review is on").to_string()
                } else {
                    tl!("Claude Code review is off").to_string()
                };
                close = true;
            }
            Err(e) => d.error = e.to_string(),
        }
    }
    ctx.data_mut(|m| m.insert_temp(draft_id(), if close { None } else { Some(d) }));
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

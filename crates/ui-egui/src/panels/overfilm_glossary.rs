//! OverFilm: the glossary window (OverFilm panel ▸ Tools ▸ Glossary…, or Sequence ▸ OverFilm ▸
//! Glossary…, `overfilm.glossary` without parameters). It shows the user's own words and the
//! built-in ones (searched), and a form with the table's columns to add a word or change one of
//! the user's. Saving goes through `overfilm.glossary.add` / `.remove`, into the same table
//! OverFilmSubs uses.
//!
//! Automation ids: `overfilm.glossary.<field>` for the form (`ko`, `misheard`, `aliases`, `old`,
//! `en`, `note`, `mask`, `inword`), `overfilm.glossary.save`, `overfilm.glossary.clear`,
//! `overfilm.glossary.search`, `overfilm.glossary.user.<id>.edit`,
//! `overfilm.glossary.user.<id>.remove`, `overfilm.glossary.builtin.<n>.use`,
//! `overfilm.glossary.close`.

use egui::RichText;
use serde_json::{Value, json};

use super::overfilm::Elems;
use crate::FilmcraftApp;

const COMMAND: &str = "overfilm.glossary";
const PRE: &str = "overfilm.glossary";
/// Built-in words shown at once (the search narrows them).
const SHOWN: u64 = 300;

#[derive(Clone, Default)]
struct Form {
    /// The user's word being changed; `None` adds one.
    id: Option<String>,
    ko: String,
    misheard: String,
    aliases: String,
    old: String,
    en: String,
    note: String,
    mask: bool,
    inword: bool,
}

impl Form {
    fn of(e: &Value) -> Self {
        let s = |k: &str| e[k].as_str().unwrap_or_default().to_string();
        let flag = |f: &str| e["flags"].as_array().is_some_and(|a| a.iter().any(|x| x == f));
        Form {
            id: Some(s("id")).filter(|i| !i.is_empty()),
            ko: s("ko"),
            misheard: joined(&e["misheard"]),
            aliases: joined(&e["aliases"]),
            old: joined(&e["old"]),
            en: s("en"),
            note: s("note"),
            mask: flag("mask"),
            inword: flag("inword"),
        }
    }

    fn params(&self) -> Value {
        let mut p = json!({
            "ko": self.ko.trim(), "misheard": self.misheard, "aliases": self.aliases, "old": self.old,
            "en": self.en.trim(), "note": self.note.trim(), "mask": self.mask, "inword": self.inword,
        });
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        p
    }
}

#[derive(Clone, Default)]
struct Draft {
    form: Form,
    /// The last answer of `overfilm.glossary`.
    list: Value,
    query: String,
    /// The query `list` answers.
    listed: String,
    message: String,
    error: String,
}

fn draft_id() -> egui::Id {
    egui::Id::unique("overfilm-glossary-draft")
}

/// `["a", "b"]` → `"a, b"`.
fn joined(v: &Value) -> String {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

fn category_label(c: &str) -> &'static str {
    match c {
        "heroes" => tl!("Hero"),
        "abilities" => tl!("Ability"),
        "maps" => tl!("Map"),
        "modes" => tl!("Mode"),
        "players" => tl!("Player"),
        "teams" => tl!("Team"),
        "terms" | "matchups" => tl!("Game term"),
        "slang" | "netslang" => tl!("Slang"),
        "profanity" => tl!("Profanity"),
        "spelling" => tl!("Spelling"),
        _ => tl!("Word"),
    }
}

/// The menu item and the panel button (no parameters) open the window.
pub fn route(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != COMMAND || !params.as_object().is_none_or(|m| m.is_empty()) {
        return None;
    }
    let mut d = ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())).flatten().unwrap_or_default();
    refresh(app, &mut d);
    ctx.data_mut(|m| m.insert_temp(draft_id(), Some(d)));
    Some(Ok(json!({"dialog": PRE})))
}

fn refresh(app: &mut FilmcraftApp, d: &mut Draft) {
    match app.session.execute(COMMAND, json!({"query": d.query.trim(), "limit": SHOWN})) {
        Ok(v) => d.list = v,
        Err(e) => d.error = e.to_string(),
    }
    d.listed = d.query.clone();
}

#[derive(Default)]
struct Clicks {
    save: bool,
    clear: bool,
    close: bool,
    edit: Option<Value>,
    remove: Option<String>,
    use_builtin: Option<Value>,
}

fn field(ui: &mut egui::Ui, elems: &mut Elems, key: &str, label: &str, v: &mut String, hint: &str) {
    ui.label(label);
    let r = ui.add(egui::TextEdit::singleline(v).hint_text(hint).desired_width(320.0));
    elems.push((format!("{PRE}.{key}"), r.rect, v.clone()));
    ui.end_row();
}

fn form_body(ui: &mut egui::Ui, elems: &mut Elems, f: &mut Form, c: &mut Clicks) {
    let heading = match &f.id {
        Some(id) => tlf!("Change a word ({id})", id = id),
        None => tl!("Add a word").to_string(),
    };
    ui.label(RichText::new(heading).strong());
    egui::Grid::new("overfilm-glossary-form").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        field(ui, elems, "ko", tl!("Right spelling:"), &mut f.ko, tl!("what the captions should show, e.g. 정커퀸"));
        field(ui, elems, "misheard", tl!("Wrong spellings:"), &mut f.misheard, tl!("what recognition writes instead, e.g. 전국퀸, 정커 킹"));
        field(ui, elems, "aliases", tl!("Also right:"), &mut f.aliases, tl!("short forms kept as they are, e.g. 정퀸"));
        field(ui, elems, "old", tl!("Former names:"), &mut f.old, tl!("old names still heard, e.g. 맥크리"));
        field(ui, elems, "en", tl!("English name:"), &mut f.en, tl!("optional; a player's handle as written"));
        field(ui, elems, "note", tl!("Note:"), &mut f.note, tl!("optional, for you"));
    });
    ui.horizontal_wrapped(|ui| {
        let r = ui.checkbox(&mut f.mask, tl!("Profanity: show the initial consonants"));
        elems.push((format!("{PRE}.mask"), r.rect, tl!("Profanity: show the initial consonants").to_string()));
        let r = ui.checkbox(&mut f.inword, tl!("Also inside longer words"));
        elems.push((format!("{PRE}.inword"), r.rect, tl!("Also inside longer words").to_string()));
    });
    ui.weak(tl!("Several spellings in one box: separate them with commas."));
    ui.horizontal(|ui| {
        let label = if f.id.is_some() { tl!("Save Changes") } else { tl!("Add") };
        let r = ui.add_enabled(!f.ko.trim().is_empty(), egui::Button::new(label));
        elems.push((format!("{PRE}.save"), r.rect, label.to_string()));
        c.save = r.clicked();
        let label = if f.id.is_some() { tl!("Cancel Change") } else { tl!("Clear the Form") };
        let r = ui.button(label);
        elems.push((format!("{PRE}.clear"), r.rect, label.to_string()));
        c.clear = r.clicked();
    });
}

fn user_rows(ui: &mut egui::Ui, elems: &mut Elems, list: &Value, editing: Option<&str>, c: &mut Clicks) {
    let words = list["user"].as_array().cloned().unwrap_or_default();
    ui.label(RichText::new(tlf!("My words ({n})", n = words.len())).strong());
    if words.is_empty() {
        ui.weak(tl!("None yet. Words added here or in OverFilmSubs show here."));
        return;
    }
    egui::ScrollArea::vertical().id_salt("overfilm-glossary-user").max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("overfilm-glossary-user-grid").num_columns(5).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new(tl!("Right spelling")).weak());
            ui.label(RichText::new(tl!("Wrong spellings")).weak());
            ui.label(RichText::new(tl!("Also right")).weak());
            ui.label(RichText::new(tl!("Note")).weak());
            ui.label("");
            ui.end_row();
            for w in &words {
                let id = w["id"].as_str().unwrap_or_default();
                let ko = w["ko"].as_str().unwrap_or_default();
                let r = if editing == Some(id) { ui.label(RichText::new(ko).strong()) } else { ui.label(ko) };
                ui.label(joined(&w["misheard"]));
                ui.label(joined(&w["aliases"]));
                ui.label(w["note"].as_str().unwrap_or_default());
                elems.push((format!("{PRE}.user.{id}"), r.rect, ko.to_string()));
                ui.horizontal(|ui| {
                    let e = ui.small_button(tl!("Change"));
                    elems.push((format!("{PRE}.user.{id}.edit"), e.rect, tl!("Change").to_string()));
                    if e.clicked() {
                        c.edit = Some(w.clone());
                    }
                    let x = ui.small_button(tl!("Remove"));
                    elems.push((format!("{PRE}.user.{id}.remove"), x.rect, tl!("Remove").to_string()));
                    if x.clicked() {
                        c.remove = Some(id.to_string());
                    }
                });
                ui.end_row();
            }
        });
    });
}

fn builtin_rows(ui: &mut egui::Ui, elems: &mut Elems, d: &mut Draft, c: &mut Clicks) {
    let total = d.list["builtinCount"].as_u64().unwrap_or(0);
    ui.label(RichText::new(tlf!("Built into the program ({n})", n = total)).strong());
    ui.horizontal(|ui| {
        ui.label(tl!("Search:"));
        let r = ui.add(egui::TextEdit::singleline(&mut d.query).hint_text(tl!("a spelling, right or wrong")).desired_width(220.0));
        elems.push((format!("{PRE}.search"), r.rect, d.query.clone()));
        let matches = d.list["builtinMatches"].as_u64().unwrap_or(0);
        let shown = d.list["builtin"].as_array().map_or(0, Vec::len);
        if (shown as u64) < matches {
            ui.weak(tlf!("first {shown} of {matches}", shown = shown, matches = matches));
        } else {
            ui.weak(tlf!("{matches} found", matches = matches));
        }
    });
    ui.weak(tl!("A built-in word that comes out wrong: Use as New Word, then fill in the wrong spelling."));
    let rows = d.list["builtin"].as_array().cloned().unwrap_or_default();
    // the rest of the window's height, leaving room for the Close row
    let height = (ui.available_height() - 44.0).max(280.0);
    egui::ScrollArea::vertical().id_salt("overfilm-glossary-builtin").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("overfilm-glossary-builtin-grid").num_columns(5).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new(tl!("Kind")).weak());
            ui.label(RichText::new(tl!("Right spelling")).weak());
            ui.label(RichText::new(tl!("Also right")).weak());
            ui.label(RichText::new(tl!("Wrong spellings")).weak());
            ui.label("");
            ui.end_row();
            for (n, w) in rows.iter().enumerate() {
                ui.label(category_label(w["category"].as_str().unwrap_or_default()));
                ui.label(w["ko"].as_str().filter(|k| !k.is_empty()).or_else(|| w["en"].as_str()).unwrap_or_default());
                let mut also = w["aliases"].as_array().cloned().unwrap_or_default();
                also.extend(w["old"].as_array().cloned().unwrap_or_default());
                ui.label(joined(&Value::Array(also)));
                ui.label(joined(&w["misheard"]));
                let r = ui.small_button(tl!("Use as New Word"));
                elems.push((format!("{PRE}.builtin.{n}.use"), r.rect, w["ko"].as_str().unwrap_or_default().to_string()));
                if r.clicked() {
                    c.use_builtin = Some(w.clone());
                }
                ui.end_row();
            }
        });
    });
}

pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(Some(mut d)) = ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())) else { return };
    let mut elems: Elems = Vec::new();
    let mut c = Clicks::default();
    let mut open = true;
    egui::Window::new(tl!("Glossary (OverFilm)"))
        .id(egui::Id::unique("overfilm-glossary-dialog"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([760.0, 620.0])
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(crate::widgets::dialog_center(ctx))
        .show(ctx, |ui| {
            ui.set_min_width(640.0);
            ui.label(tl!("Captions change a wrong spelling into the right one. Your words are added to the built-in ones and used from the next OverFilm Captions and Claude Code review on. OverFilmSubs uses the same words."));
            ui.add_space(6.0);
            form_body(ui, &mut elems, &mut d.form, &mut c);
            if !d.message.is_empty() {
                ui.label(&d.message);
            }
            if !d.error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &d.error);
            }
            ui.separator();
            user_rows(ui, &mut elems, &d.list, d.form.id.as_deref(), &mut c);
            ui.separator();
            builtin_rows(ui, &mut elems, &mut d, &mut c);
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                let r = ui.button(tl!("Close"));
                elems.push((format!("{PRE}.close"), r.rect, tl!("Close").to_string()));
                c.close = r.clicked();
                if let Some(f) = d.list["file"].as_str() {
                    ui.weak(tlf!("Saved in {file}", file = f));
                }
            });
        });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if !open || crate::widgets::escape_closes(ctx) {
        c.close = true;
    }
    if c.save {
        match app.session.execute("overfilm.glossary.add", d.form.params()) {
            Ok(v) => {
                let ko = v["entry"]["ko"].as_str().unwrap_or_default().to_string();
                d.message = if d.form.id.is_some() { tlf!("Saved '{ko}'.", ko = ko) } else { tlf!("Added '{ko}'.", ko = ko) };
                d.error.clear();
                d.form = Form::default();
                refresh(app, &mut d);
            }
            Err(e) => d.error = super::overfilm::plain_error(&e.to_string()),
        }
    }
    if c.clear {
        d.form = Form::default();
    }
    if let Some(w) = c.edit {
        d.form = Form::of(&w);
        d.message.clear();
    }
    if let Some(w) = c.use_builtin {
        // a new word with the right spelling filled in; the wrong one is what the user adds
        let mut f = Form::of(&w);
        f.id = None;
        f.misheard.clear();
        f.note.clear();
        d.form = f;
        d.message = tl!("Fill in the wrong spelling, then Add.").to_string();
    }
    if let Some(id) = c.remove {
        match app.session.execute("overfilm.glossary.remove", json!({"id": id})) {
            Ok(_) => {
                if d.form.id.as_deref() == Some(id.as_str()) {
                    d.form = Form::default();
                }
                d.message = tl!("Removed.").to_string();
                d.error.clear();
                refresh(app, &mut d);
            }
            Err(e) => d.error = super::overfilm::plain_error(&e.to_string()),
        }
    }
    if d.query != d.listed {
        refresh(app, &mut d);
    }
    ctx.data_mut(|m| m.insert_temp(draft_id(), if c.close { None } else { Some(d) }));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Form;

    #[test]
    fn the_form_round_trips_a_word() {
        let w = json!({"id": "user2", "ko": "정커퀸", "misheard": ["전국퀸", "정커 킹"], "aliases": [], "old": ["퀸"], "en": "", "note": "탱커", "flags": ["inword"]});
        let f = Form::of(&w);
        assert_eq!(f.misheard, "전국퀸, 정커 킹");
        assert!(f.inword && !f.mask);
        let p = f.params();
        assert_eq!(p["id"], "user2");
        assert_eq!(p["old"], "퀸");
        assert!(Form::default().params().get("id").is_none());
    }
}

//! OverFilm dialogs (see `OVERFILM.md`): the Sequence ▸ OverFilm menu items open these when
//! invoked without parameters; with parameters the engine command runs directly.
//!
//! - **OverFilm Captions (Korean)…** (`overfilm.captions.generate`): video kind, speakers, game
//!   voice, line length, extra terms.
//! - **Styled Captions (OverFilm)…** (`overfilm.captions.stylize`): caption track, style, motion.
//! - **Auto Cut Waiting and Pauses (OverFilm)…** (`overfilm.autoCut`): template file, time kept
//!   before the start, mark or cut.
//! - **Learn Screen Template (OverFilm)…** (`overfilm.screenTemplate.learn`): the Program frame at
//!   the playhead; drag on it to mark the parts that always look the same on that screen (green),
//!   switch to "ignore" to mark what changes (red: the clock, a camera). Choosing a team's name
//!   plates instead learns where they are (`overfilm.playerBar.learn`, the first green rectangle
//!   around the team's plates, on a frame where the whole team is alive), for kill detection.
//!
//! Automation ids: `overfilm.<dialog>.ok`, `.cancel` and `overfilm.<dialog>.<param>` for each
//! control (`<dialog>` = captions, style, autoCut, learn); `overfilm.learn.frame` is the picture.

use serde_json::{Value, json};

use crate::FilmcraftApp;

type Elems = Vec<(String, egui::Rect, String)>;

#[derive(Clone)]
struct Draft {
    /// The engine command OK runs.
    command: &'static str,
    params: Value,
    /// Captions dialog: none. Style: presets and caption tracks. Auto Cut / Learn: the template
    /// file's current list.
    info: Value,
    error: String,
    /// Learn: the Program frame and its size, the rectangle being dragged (frame 0..1).
    frame: Option<(egui::TextureHandle, [usize; 2])>,
    drag: Option<egui::Pos2>,
}

fn draft_id() -> egui::Id {
    egui::Id::unique("overfilm-dialog-draft")
}

const COMMANDS: &[&str] = &["overfilm.captions.generate", "overfilm.captions.stylize", "overfilm.autoCut", "overfilm.screenTemplate.learn"];

fn prefix(command: &str) -> &'static str {
    match command {
        "overfilm.captions.generate" => "overfilm.captions",
        "overfilm.captions.stylize" => "overfilm.style",
        "overfilm.autoCut" => "overfilm.autoCut",
        _ => "overfilm.learn",
    }
}

/// Menu entry points: the OverFilm commands without parameters open their dialog.
pub fn route(app: &mut FilmcraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let command = COMMANDS.iter().copied().find(|c| *c == id)?;
    if !params.as_object().is_none_or(|m| m.is_empty()) {
        return None;
    }
    if let Some(Err(e)) = filmcraft_engine::find_command(id).map(|c| (c.enabled)(&app.session)) {
        app.ui.status = plain_error(&e);
        return Some(Err(e));
    }
    let s = &mut app.session;
    let templates = s.execute("overfilm.screenTemplate.list", json!({})).ok();
    let templates_path = templates.as_ref().and_then(|t| t["templates"].as_str()).unwrap_or_default().to_string();
    let (params, info) = match command {
        "overfilm.captions.generate" => (
            json!({"mode": "general", "speakers": true, "numSpeakers": 0, "gameVoice": "drop", "lineChars": 16, "maxLines": 2,
                   "cleanFillers": false, "gpu": true, "extraTerms": ""}),
            Value::Null,
        ),
        "overfilm.captions.stylize" => {
            let presets = s.execute("overfilm.captions.presets", json!({})).ok().map(|v| v["presets"].clone()).unwrap_or_default();
            let tracks: Vec<Value> = s
                .active_sequence()
                .map(|q| q.caption_tracks.iter().map(|t| json!({"id": t.id.0, "name": t.name, "captions": t.captions.len()})).collect())
                .unwrap_or_default();
            // the OverFilm caption track when there is one, else the first
            let track = tracks
                .iter()
                .find(|t| t["name"] == filmcraft_engine::overfilm_transcribe::CAPTION_TRACK)
                .or_else(|| tracks.first())
                .map(|t| t["id"].clone())
                .unwrap_or(Value::Null);
            (json!({"track": track, "preset": "기본", "motionIn": "", "motionOut": ""}), json!({"presets": presets, "tracks": tracks}))
        }
        "overfilm.autoCut" => {
            (json!({"templates": templates_path, "keepBefore": 10.0, "fps": 2.0, "frozenPauses": true, "mode": "mark"}), templates.unwrap_or_default())
        }
        _ => {
            let frame = s.render_program(0.5).map(|img| {
                let rgba = img.over_black_rgba8();
                let size = [img.w, img.h];
                (ctx.load_texture("overfilm-learn-frame", egui::ColorImage::from_rgba_unmultiplied(size, &rgba), Default::default()), size)
            });
            let d = Draft {
                command,
                params: json!({"screen": "setup", "name": "", "templates": templates_path, "rects": [], "ignore": [], "drawIgnore": false}),
                info: templates.unwrap_or_default(),
                error: if frame.is_none() { tl!("No picture at the playhead: move it onto the broadcast first.").to_string() } else { String::new() },
                frame,
                drag: None,
            };
            ctx.data_mut(|m| m.insert_temp(draft_id(), Some(d)));
            return Some(Ok(json!({"dialog": prefix(command)})));
        }
    };
    let d = Draft { command, params, info, error: String::new(), frame: None, drag: None };
    ctx.data_mut(|m| m.insert_temp(draft_id(), Some(d)));
    Some(Ok(json!({"dialog": prefix(command)})))
}

fn combo(ui: &mut egui::Ui, elems: &mut Elems, id: String, label: &str, v: &mut Value, options: &[(&str, String)]) {
    ui.label(label);
    let cur = v.as_str().unwrap_or_default().to_string();
    let shown = options.iter().find(|(k, _)| *k == cur).map(|(_, l)| l.clone()).unwrap_or_else(|| cur.clone());
    let r = craft_ui::ComboBox::from_id_salt(id.clone())
        .selected_text(shown.clone())
        .show_ui(ui, |ui| {
            for (k, l) in options {
                if ui.selectable_label(cur == *k, l.as_str()).clicked() {
                    *v = json!(k);
                }
            }
        })
        .response;
    elems.push((id, r.rect, shown));
    ui.end_row();
}

fn check(ui: &mut egui::Ui, elems: &mut Elems, id: String, label: &str, v: &mut Value) {
    let mut b = v.as_bool().unwrap_or(false);
    let r = ui.checkbox(&mut b, label);
    *v = json!(b);
    elems.push((id, r.rect, label.to_string()));
}

fn number(ui: &mut egui::Ui, elems: &mut Elems, id: String, label: &str, v: &mut Value, range: std::ops::RangeInclusive<f64>, suffix: &str) {
    ui.label(label);
    let mut n = v.as_f64().unwrap_or(*range.start());
    // counts are whole numbers; seconds and rates keep one decimal
    let whole = v.is_u64() || v.is_i64();
    let drag = crate::widgets::number_drag(&mut n).range(range).suffix(suffix);
    let r = ui.add(if whole { drag.speed(0.05).max_decimals(0) } else { drag.speed(0.1).max_decimals(1) });
    if whole {
        n = n.round();
    }
    *v = if whole { json!(n as u64) } else { json!(n) };
    elems.push((id, r.rect, format!("{n}")));
    ui.end_row();
}

fn text(ui: &mut egui::Ui, elems: &mut Elems, id: String, label: &str, v: &mut Value, hint: &str) {
    ui.label(label);
    let mut t = v.as_str().unwrap_or_default().to_string();
    let r = ui.add(egui::TextEdit::singleline(&mut t).hint_text(hint).desired_width(260.0));
    *v = json!(t);
    elems.push((id, r.rect, t));
    ui.end_row();
}

fn motions() -> Vec<(&'static str, String)> {
    vec![
        ("", tl!("Style's own").to_string()),
        ("none", tl!("None").to_string()),
        ("fade", tl!("Fade").to_string()),
        ("pop", tl!("Pop").to_string()),
        ("slideUp", tl!("Slide Up").to_string()),
        ("typewriter", tl!("Typewriter").to_string()),
        ("bounce", tl!("Bounce").to_string()),
    ]
}

fn norm_rect(a: egui::Pos2, b: egui::Pos2) -> [f32; 4] {
    let (x0, x1) = (a.x.min(b.x).clamp(0.0, 1.0), a.x.max(b.x).clamp(0.0, 1.0));
    let (y0, y1) = (a.y.min(b.y).clamp(0.0, 1.0), a.y.max(b.y).clamp(0.0, 1.0));
    [x0, y0, x1 - x0, y1 - y0]
}

fn learn_body(ui: &mut egui::Ui, elems: &mut Elems, d: &mut Draft) {
    let pre = "overfilm.learn";
    egui::Grid::new("overfilm-learn").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        let p = &mut d.params;
        combo(
            ui,
            elems,
            format!("{pre}.screen"),
            tl!("Screen:"),
            &mut p["screen"],
            &[
                ("standby", tl!("Break screen (countdown)").to_string()),
                ("setup", tl!("Round setup (PREPARE TO ATTACK…)").to_string()),
                ("pause", tl!("Pause notice").to_string()),
                ("live", tl!("Game HUD (in play)").to_string()),
                ("bar1", tl!("Team 1 name plates (kills)").to_string()),
                ("bar2", tl!("Team 2 name plates (kills)").to_string()),
            ],
        );
        if p["screen"].as_str().is_some_and(|v| v.starts_with("bar")) {
            if p["players"].is_null() {
                p["players"] = json!(5);
            }
            number(ui, elems, format!("{pre}.players"), tl!("Players:"), &mut p["players"], 1.0..=12.0, "");
        }
        text(ui, elems, format!("{pre}.name"), tl!("Name:"), &mut p["name"], tl!("e.g. PREPARE TO ATTACK"));
        text(ui, elems, format!("{pre}.templates"), tl!("Template file:"), &mut p["templates"], tl!("your OverFilm template file, or type a .json path"));
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let mut ignore = d.params["drawIgnore"].as_bool().unwrap_or(false);
        for (v, key, l) in [(false, "match", tl!("Drag: what always looks the same")), (true, "ignore", tl!("Drag: what changes (ignore)"))] {
            let r = ui.radio(ignore == v, l);
            elems.push((format!("{pre}.draw.{key}"), r.rect, l.to_string()));
            if r.clicked() {
                ignore = v;
            }
        }
        d.params["drawIgnore"] = json!(ignore);
    });
    let Some((tex, size)) = d.frame.clone() else { return };
    let w = ui.available_width().clamp(320.0, 720.0);
    let h = w * size[1] as f32 / size[0].max(1) as f32;
    let r = ui.add(egui::Image::new((tex.id(), egui::vec2(w, h))).sense(egui::Sense::click_and_drag()));
    elems.push((format!("{pre}.frame"), r.rect, "frame".into()));
    let to_norm = |p: egui::Pos2| egui::pos2((p.x - r.rect.min.x) / r.rect.width().max(1.0), (p.y - r.rect.min.y) / r.rect.height().max(1.0));
    if r.drag_started()
        && let Some(p) = r.interact_pointer_pos()
    {
        d.drag = Some(to_norm(p));
    }
    let current = match (d.drag, r.interact_pointer_pos()) {
        (Some(a), Some(b)) => Some(norm_rect(a, to_norm(b))),
        _ => None,
    };
    if r.drag_stopped() {
        if let Some(n) = current.filter(|n| n[2] > 0.005 && n[3] > 0.005) {
            let key = if d.params["drawIgnore"].as_bool().unwrap_or(false) { "ignore" } else { "rects" };
            if let Some(a) = d.params[key].as_array_mut()
                && a.len() < 16
            {
                a.push(json!(n.iter().map(|v| (v * 1000.0).round() / 1000.0).collect::<Vec<_>>()));
            }
        }
        d.drag = None;
    }
    let painter = ui.painter_at(r.rect);
    let to_screen = |n: &[f32]| {
        egui::Rect::from_min_size(
            egui::pos2(r.rect.min.x + n.first().copied().unwrap_or(0.0) * r.rect.width(), r.rect.min.y + n.get(1).copied().unwrap_or(0.0) * r.rect.height()),
            egui::vec2(n.get(2).copied().unwrap_or(0.0) * r.rect.width(), n.get(3).copied().unwrap_or(0.0) * r.rect.height()),
        )
    };
    let nums = |v: &Value| v.as_array().map(|a| a.iter().filter_map(Value::as_f64).map(|f| f as f32).collect::<Vec<f32>>()).unwrap_or_default();
    for (key, color) in [("rects", egui::Color32::from_rgb(80, 220, 120)), ("ignore", egui::Color32::from_rgb(240, 80, 80))] {
        for v in d.params[key].as_array().cloned().unwrap_or_default() {
            painter.rect_stroke(to_screen(&nums(&v)), 0.0, egui::Stroke::new(2.0, color), egui::StrokeKind::Outside);
        }
    }
    if let Some(n) = current {
        painter.rect_stroke(to_screen(&n), 0.0, egui::Stroke::new(1.0, egui::Color32::WHITE), egui::StrokeKind::Outside);
    }
    ui.horizontal(|ui| {
        let n = d.params["rects"].as_array().map_or(0, Vec::len);
        let m = d.params["ignore"].as_array().map_or(0, Vec::len);
        ui.weak(tlf!("{n} area(s) to match, {m} to ignore", n, m));
        let r = ui.button(tl!("Remove Last"));
        elems.push((format!("{pre}.undoRect"), r.rect, "Remove Last".into()));
        if r.clicked() {
            let key = if d.params["drawIgnore"].as_bool().unwrap_or(false) { "ignore" } else { "rects" };
            if let Some(a) = d.params[key].as_array_mut() {
                a.pop();
            }
        }
        let r = ui.button(tl!("Clear"));
        elems.push((format!("{pre}.clear"), r.rect, "Clear".into()));
        if r.clicked() {
            d.params["rects"] = json!([]);
            d.params["ignore"] = json!([]);
        }
    });
    if let Some(list) = d.info["list"].as_array().filter(|l| !l.is_empty()) {
        let names: Vec<String> =
            list.iter().map(|t| format!("{} ({})", t["name"].as_str().unwrap_or_default(), screen_label(t["screen"].as_str().unwrap_or_default()))).collect();
        ui.weak(tlf!("Already learnt: {names}", names = names.join(", ")));
    }
    if d.params["rects"].as_array().is_none_or(Vec::is_empty) {
        ui.weak(tl!("Drag on the picture to draw at least one green box, then OK."));
    }
}

/// A screen kind in the dialog's words.
fn screen_label(screen: &str) -> &'static str {
    match screen {
        "standby" => tl!("Break screen (countdown)"),
        "setup" => tl!("Round setup (PREPARE TO ATTACK…)"),
        "pause" => tl!("Pause notice"),
        "live" => tl!("Game HUD (in play)"),
        _ => "",
    }
}

/// An engine error in the user's words: the known OverFilm ones are said plainly (and translated);
/// anything else keeps its text without the "invalid parameters for `…`:" lead.
fn plain_error(e: &str) -> String {
    const KNOWN: &[(&str, &str)] = &[
        ("give `rects`", "Drag on the picture to draw at least one green box, then OK."),
        ("`rect` is [x, y, w, h]", "Draw one green box around the team's name plates."),
        (
            "no break-screen, setup or pause template",
            "No break-screen, setup or pause template yet: import a template file, or learn one from a frame of that screen.",
        ),
        ("has no player bars", "No name plates learnt yet: learn each team's name plates first (Learn Screen Template, Team 1/2 name plates)."),
        ("every plate looks dark", "Every plate looks dark: learn them on a frame where the whole team is alive."),
        ("flat colour", "That box is one flat colour: pick a part with text or graphics."),
        ("empty or outside the frame", "The box is empty or outside the picture: draw it again."),
        ("no video clip at that time", "No picture at the playhead: move it onto the broadcast first."),
        ("has no media", "That clip has no media (offline?)."),
        ("has no video", "That clip has no picture."),
        ("can't decode the frame", "Couldn't read that frame of the video."),
        ("already running", "It is already running: see the progress at the bottom."),
        ("no video clip with media to analyse", "No video to analyse: put the broadcast on V1, or select its clips."),
        ("nothing to recognise", "No clips with sound in the sequence."),
        ("hours; cut it into parts", "The recording is too long: cut it into parts first."),
        ("not in this build", "This build has no OverFilm speech recognition."),
        ("has no captions to style", "That caption track has no captions."),
        ("has no transcript", "The sequence has no transcript yet: make the captions first."),
        ("no speech to caption", "No speech to caption (only in-game voice?)."),
        ("that is the template file itself", "That is the template file itself."),
        ("No such file", "The file is not there."),
    ];
    if let Some((_, msg)) = KNOWN.iter().find(|(k, _)| e.contains(k)) {
        return crate::i18n::t(msg).to_string();
    }
    match e.strip_prefix("invalid parameters for `").and_then(|r| r.split_once("`: ")) {
        Some((_, rest)) => rest.to_string(),
        None => e.to_string(),
    }
}

/// "Import Templates…": bring someone else's template file into this one.
fn import_button(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, d: &mut Draft, import: &mut bool) {
    let r = ui.button(tl!("Import Templates…"));
    elems.push((format!("{pre}.import"), r.rect, "Import Templates…".into()));
    if r.clicked() {
        *import = true;
        d.params["refresh"] = json!(true);
    }
}

/// Whether the template list has something Auto Cut can cut by.
fn has_break_templates(info: &Value) -> bool {
    info["list"].as_array().is_some_and(|l| l.iter().any(|t| matches!(t["screen"].as_str(), Some("standby" | "setup" | "pause"))))
}

pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(Some(mut d)) = ctx.data(|m| m.get_temp::<Option<Draft>>(draft_id())) else { return };
    let pre = prefix(d.command);
    let title = match d.command {
        "overfilm.captions.generate" => tl!("OverFilm Captions (Korean)"),
        "overfilm.captions.stylize" => tl!("Styled Captions (OverFilm)"),
        "overfilm.autoCut" => tl!("Auto Cut Waiting and Pauses (OverFilm)"),
        _ => tl!("Learn Screen Template (OverFilm)"),
    };
    let mut elems: Elems = Vec::new();
    let (mut close, mut apply, mut import) = (false, false, false);
    // an import finished: read the template list again
    if d.params["refresh"].as_bool() == Some(true) && !app.native_dialog_open() {
        d.params["refresh"] = json!(false);
        let t = d.params["templates"].as_str().filter(|t| !t.is_empty()).map(|t| json!({"templates": t})).unwrap_or(json!({}));
        if let Ok(v) = app.session.execute("overfilm.screenTemplate.list", t) {
            d.info = v;
        }
    }
    egui::Window::new(title)
        .id(egui::Id::unique(("overfilm-dialog", pre)))
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(crate::widgets::dialog_center(ctx))
        .show(ctx, |ui| {
            ui.set_min_width(if d.command == "overfilm.screenTemplate.learn" { 640.0 } else { 380.0 });
            match d.command {
                "overfilm.captions.generate" => {
                    egui::Grid::new("overfilm-captions").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        let p = &mut d.params;
                        combo(
                            ui,
                            &mut elems,
                            format!("{pre}.mode"),
                            tl!("Video:"),
                            &mut p["mode"],
                            &[
                                ("general", tl!("Gameplay (heroes and game terms)").to_string()),
                                ("commentary", tl!("Tournament commentary (teams and players too)").to_string()),
                            ],
                        );
                        combo(
                            ui,
                            &mut elems,
                            format!("{pre}.gameVoice"),
                            tl!("In-game voice:"),
                            &mut p["gameVoice"],
                            &[
                                ("drop", tl!("Leave out of the captions").to_string()),
                                ("label", tl!("Keep, marked (게임)").to_string()),
                                ("keep", tl!("Keep as speech").to_string()),
                            ],
                        );
                        number(ui, &mut elems, format!("{pre}.numSpeakers"), tl!("Speakers (0 = count them):"), &mut p["numSpeakers"], 0.0..=12.0, "");
                        number(ui, &mut elems, format!("{pre}.lineChars"), tl!("Characters per line:"), &mut p["lineChars"], 8.0..=30.0, "");
                        number(ui, &mut elems, format!("{pre}.maxLines"), tl!("Lines per caption:"), &mut p["maxLines"], 1.0..=2.0, "");
                        text(ui, &mut elems, format!("{pre}.extraTerms"), tl!("Extra names and terms:"), &mut p["extraTerms"], tl!("comma separated"));
                    });
                    let p = &mut d.params;
                    check(ui, &mut elems, format!("{pre}.speakers"), tl!("Tell speakers apart (화자1, 화자2…)"), &mut p["speakers"]);
                    check(ui, &mut elems, format!("{pre}.cleanFillers"), tl!("Leave out fillers and stutters (어, 음…)"), &mut p["cleanFillers"]);
                    check(ui, &mut elems, format!("{pre}.gpu"), tl!("Use the GPU"), &mut p["gpu"]);
                    ui.weak(tl!("Runs in the background. The models (about 6 GB) are downloaded the first time, into the folder OverFilmSubs uses."));
                }
                "overfilm.captions.stylize" => {
                    egui::Grid::new("overfilm-style").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        let tracks: Vec<(String, String)> = d.info["tracks"]
                            .as_array()
                            .map(|a| {
                                a.iter().map(|t| (t["id"].to_string(), format!("{} ({})", t["name"].as_str().unwrap_or_default(), t["captions"]))).collect()
                            })
                            .unwrap_or_default();
                        let mut track = json!(d.params["track"].to_string());
                        let opts: Vec<(&str, String)> = tracks.iter().map(|(k, l)| (k.as_str(), l.clone())).collect();
                        combo(ui, &mut elems, format!("{pre}.track"), tl!("Caption track:"), &mut track, &opts);
                        d.params["track"] = track.as_str().and_then(|t| t.parse::<u64>().ok()).map(Value::from).unwrap_or(Value::Null);
                        let presets: Vec<String> =
                            d.info["presets"].as_array().map(|a| a.iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
                        let opts: Vec<(&str, String)> = presets.iter().map(|n| (n.as_str(), n.clone())).collect();
                        combo(ui, &mut elems, format!("{pre}.preset"), tl!("Style:"), &mut d.params["preset"], &opts);
                        let m = motions();
                        combo(ui, &mut elems, format!("{pre}.motionIn"), tl!("Motion in:"), &mut d.params["motionIn"], &m);
                        combo(ui, &mut elems, format!("{pre}.motionOut"), tl!("Motion out:"), &mut d.params["motionOut"], &m);
                    });
                    ui.weak(tl!(
                        "Each caption becomes a graphic clip on a new video track; fill, outline, position and keyframes stay editable in Effect Controls."
                    ));
                }
                "overfilm.autoCut" => {
                    egui::Grid::new("overfilm-autocut").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        let p = &mut d.params;
                        text(
                            ui,
                            &mut elems,
                            format!("{pre}.templates"),
                            tl!("Template file:"),
                            &mut p["templates"],
                            tl!("your OverFilm template file, or type a .json path"),
                        );
                        number(ui, &mut elems, format!("{pre}.keepBefore"), tl!("Keep before the start:"), &mut p["keepBefore"], 0.0..=60.0, " s");
                        number(ui, &mut elems, format!("{pre}.fps"), tl!("Frames checked per second:"), &mut p["fps"], 0.5..=8.0, "");
                        combo(
                            ui,
                            &mut elems,
                            format!("{pre}.mode"),
                            tl!("Result:"),
                            &mut p["mode"],
                            &[("mark", tl!("Markers to check first").to_string()), ("cut", tl!("Cut right away").to_string())],
                        );
                    });
                    check(ui, &mut elems, format!("{pre}.frozenPauses"), tl!("A frozen picture in a round is a pause"), &mut d.params["frozenPauses"]);
                    let n = d.info["list"].as_array().map_or(0, Vec::len);
                    ui.weak(tlf!("{n} screen template(s) in the file. Learn them first: Sequence ▸ OverFilm ▸ Learn Screen Template.", n));
                    import_button(ui, &mut elems, pre, &mut d, &mut import);
                    if !has_break_templates(&d.info) {
                        ui.colored_label(
                            app.tokens.danger,
                            tl!("No break-screen, setup or pause template yet: import a template file, or learn one from a frame of that screen."),
                        );
                    }
                }
                _ => {
                    learn_body(ui, &mut elems, &mut d);
                    import_button(ui, &mut elems, pre, &mut d, &mut import);
                }
            }
            if !d.error.is_empty() {
                ui.colored_label(app.tokens.danger, &d.error);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let c = ui.button(tl!("Cancel"));
                elems.push((format!("{pre}.cancel"), c.rect, "Cancel".into()));
                close |= c.clicked();
                let ready = match d.command {
                    "overfilm.autoCut" => has_break_templates(&d.info),
                    "overfilm.screenTemplate.learn" => d.params["rects"].as_array().is_some_and(|a| !a.is_empty()) && d.frame.is_some(),
                    _ => true,
                };
                let o = ui.add_enabled(ready, egui::Button::new(egui::RichText::new(tl!("OK")).color(egui::Color32::WHITE)).fill(app.tokens.accent));
                elems.push((format!("{pre}.ok"), o.rect, "OK".into()));
                apply |= o.clicked();
            });
        });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if crate::widgets::escape_closes(ctx) {
        close = true;
    }
    if import {
        let into = d.params["templates"].as_str().unwrap_or_default().to_string();
        app.pick_ui(crate::native_dialogs::FileDialog::open_file("JSON", &["json"]), move |app, paths| {
            let Some(path) = paths.into_iter().next() else { return };
            let mut p = json!({"path": path});
            if !into.is_empty() {
                p["templates"] = json!(into);
            }
            app.ui.status = match app.session.execute("overfilm.screenTemplate.import", p) {
                Ok(v) => tlf!("Imported {n} template(s) and {b} player bar(s)", n = v["added"], b = v["bars"]),
                Err(e) => plain_error(&e.to_string()),
            };
        });
    }
    if apply {
        let (command, p) = learn_call(&d).unwrap_or((d.command, run_params(&d)));
        match app.session.execute(command, p) {
            Ok(v) => {
                app.ui.status = match command {
                    "overfilm.playerBar.learn" => tlf!("Learnt team {team}'s name plates", team = v["team"]),
                    "overfilm.screenTemplate.learn" => {
                        tlf!("Learnt \"{name}\" ({n} templates in the file)", name = v["name"].as_str().unwrap_or_default(), n = v["count"])
                    }
                    "overfilm.captions.stylize" => tlf!("{n} styled caption(s)", n = v["styled"]),
                    _ => tl!("Started: see the progress at the bottom").to_string(),
                };
                close = true;
            }
            Err(e) => d.error = plain_error(&e.to_string()),
        }
    }
    ctx.data_mut(|m| m.insert_temp(draft_id(), if close { None } else { Some(d) }));
}

/// Learning a team's name plates instead of a screen template: the first green rectangle is the
/// bar around the team's plates.
fn learn_call(d: &Draft) -> Option<(&'static str, Value)> {
    let team = match d.params["screen"].as_str()? {
        "bar1" => 1,
        "bar2" => 2,
        _ => return None,
    };
    let mut p = json!({"team": team, "players": d.params["players"].as_f64().unwrap_or(5.0).round().clamp(1.0, 12.0) as u64});
    if let Some(r) = d.params["rects"].as_array().and_then(|a| a.first()) {
        p["rect"] = r.clone();
    }
    if let Some(t) = d.params["templates"].as_str().filter(|t| !t.is_empty()) {
        p["templates"] = json!(t);
    }
    Some(("overfilm.playerBar.learn", p))
}

/// The command's parameters from the dialog's (empty choices left out).
fn run_params(d: &Draft) -> Value {
    let mut p = d.params.clone();
    if let Some(m) = p.as_object_mut() {
        m.remove("drawIgnore");
        m.remove("refresh");
        m.remove("players");
        m.retain(|_, v| !(v.is_null() || v.as_str().is_some_and(str::is_empty)));
        if m.get("numSpeakers").and_then(Value::as_f64).is_some_and(|n| n < 1.0) {
            m.remove("numSpeakers");
        }
        for k in ["numSpeakers", "lineChars", "maxLines"] {
            if let Some(n) = m.get(k).and_then(Value::as_f64) {
                m.insert(k.into(), json!(n.round().max(0.0) as u64));
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_drop_empty_choices_and_round_counts() {
        let d = Draft {
            command: "overfilm.captions.generate",
            params: json!({"numSpeakers": 0.0, "lineChars": 15.6, "maxLines": 2.0, "extraTerms": "", "mode": "general", "drawIgnore": false}),
            info: Value::Null,
            error: String::new(),
            frame: None,
            drag: None,
        };
        assert_eq!(run_params(&d), json!({"lineChars": 16, "maxLines": 2, "mode": "general"}));
        let r = norm_rect(egui::pos2(0.5, 0.9), egui::pos2(0.1, 1.4));
        for (a, b) in r.iter().zip([0.1, 0.9, 0.4, 0.1]) {
            assert!((a - b).abs() < 1e-5, "{r:?}");
        }
    }

    #[test]
    fn name_plates_learn_a_player_bar() {
        let mut d = Draft {
            command: "overfilm.screenTemplate.learn",
            params: json!({"screen": "bar2", "players": 4.6, "rects": [[0.7, 0.14, 0.27, 0.02], [0.1, 0.1, 0.1, 0.1]], "ignore": [], "templates": "s.json", "name": ""}),
            info: Value::Null,
            error: String::new(),
            frame: None,
            drag: None,
        };
        let (cmd, p) = learn_call(&d).unwrap();
        assert_eq!(cmd, "overfilm.playerBar.learn");
        assert_eq!(p, json!({"team": 2, "players": 5, "rect": [0.7, 0.14, 0.27, 0.02], "templates": "s.json"}));
        d.params["screen"] = json!("setup");
        assert!(learn_call(&d).is_none());
    }
}

//! The few Windows parts the screens are built from: settings cards (title, description, one
//! control on the right), toggle switches, radio cards, info bars, the navigation rail, accent
//! buttons and small line icons.

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::theme::{self, Palette};

pub fn pal(ui: &Ui) -> Palette {
    theme::palette(ui.ctx())
}

/// Page title (28 px).
pub fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).text_style(egui::TextStyle::Heading).strong());
}

/// Section header above a group of cards (body, semibold).
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).strong());
}

/// Caption-size secondary text.
pub fn caption(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    let p = pal(ui);
    ui.label(RichText::new(text.into()).small().color(p.text2))
}

/// A card: the card fill, a 1 px stroke, 4 px corners.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A settings card: title and description on the left, the control on the right. `indent`
/// makes it a child row of an expander above it.
pub fn setting<R>(ui: &mut Ui, title: &str, description: &str, indent: bool, control: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(if indent { 0 } else { theme::CONTROL_RADIUS }))
        .inner_margin(Margin { left: if indent { 52 } else { 16 }, right: 16, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(if indent { 32.0 } else { 44.0 });
            ui.horizontal(|ui| {
                let right = 260.0_f32.min(ui.available_width() * 0.5);
                ui.allocate_ui_with_layout(vec2(ui.available_width() - right, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.label(title);
                    if !description.is_empty() {
                        caption(ui, description);
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), control).inner
            })
            .inner
        })
        .inner
}

/// An expander header card: like [`setting`] with a chevron; returns whether it is open.
pub fn expander(ui: &mut Ui, id: &str, title: &str, description: &str, control: impl FnOnce(&mut Ui)) -> bool {
    let key = ui.make_persistent_id(id);
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(key, false));
    let r = setting(ui, title, description, false, |ui| {
        let r = chevron(ui, open);
        control(ui);
        r
    });
    if r.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(key, open));
    }
    open
}

fn chevron(ui: &mut Ui, open: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let p = pal(ui);
    if r.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(theme::CONTROL_RADIUS), p.subtle);
    }
    let c = rect.center();
    let (a, b, d) = if open {
        (pos2(c.x - 4.0, c.y + 2.0), pos2(c.x, c.y - 2.0), pos2(c.x + 4.0, c.y + 2.0))
    } else {
        (pos2(c.x - 4.0, c.y - 2.0), pos2(c.x, c.y + 2.0), pos2(c.x + 4.0, c.y - 2.0))
    };
    let s = Stroke::new(1.3, p.text2);
    ui.painter().line_segment([a, b], s);
    ui.painter().line_segment([b, d], s);
    r
}

/// A Windows toggle switch with "켬"/"끔" on its left.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let p = pal(ui);
    let (rect, mut r) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let enabled = ui.is_enabled();
    let t = ui.ctx().animate_bool_responsive(r.id, *on);
    let painter = ui.painter();
    let radius = CornerRadius::same(10);
    if *on {
        painter.rect_filled(rect, radius, if enabled { p.accent } else { p.text3 });
    } else {
        painter.rect_stroke(rect, radius, Stroke::new(1.0, p.text2), StrokeKind::Inside);
    }
    let knob = if r.hovered() { 7.0 } else { 6.0 };
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    painter.circle_filled(pos2(x, rect.center().y), knob, if *on { p.on_accent } else { p.text2 });
    ui.label(if *on { "켬" } else { "끔" });
    r
}

/// A radio card: the whole card is the target.
pub fn radio_card(ui: &mut Ui, selected: bool, title: &str, description: &str) -> egui::Response {
    let p = pal(ui);
    let inner = egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, if selected { p.control_stroke } else { p.stroke }))
        .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                radio_mark(ui, rect, selected);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(title);
                    caption(ui, description);
                });
            });
        });
    let r = ui.interact(inner.response.rect, ui.scope_id().with(("radio_card", title)), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(inner.response.rect, CornerRadius::same(theme::CONTROL_RADIUS), Color32::from_black_alpha(6));
    }
    r
}

pub fn radio_mark(ui: &Ui, rect: Rect, selected: bool) {
    let p = pal(ui);
    let c = rect.center();
    if selected {
        ui.painter().circle_filled(c, 10.0, p.accent);
        ui.painter().circle_filled(c, 4.5, p.on_accent);
    } else {
        ui.painter().circle_stroke(c, 9.5, Stroke::new(1.0, p.text2));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Ok,
    Warn,
    Error,
}

/// An info bar: coloured background, an icon, bold title and text; `actions` on the right.
pub fn info_bar(ui: &mut Ui, sev: Severity, title: &str, text: &str, actions: impl FnOnce(&mut Ui)) {
    let p = pal(ui);
    let (bg, fg) = match sev {
        Severity::Info => (p.card, p.accent),
        Severity::Ok => (p.ok_bg, p.ok),
        Severity::Warn => (p.warn_bg, p.warn),
        Severity::Error => (p.card, p.bad),
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 8.0, fg);
                let mark = match sev {
                    Severity::Ok => "✓",
                    Severity::Info => "i",
                    _ => "!",
                };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, mark, egui::FontId::proportional(11.0), bg);
                ui.add_space(4.0);
                let right = 220.0_f32.min(ui.available_width() * 0.4);
                ui.allocate_ui_with_layout(vec2(ui.available_width() - right, 0.0), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
                    ui.label(RichText::new(title).strong());
                    if !text.is_empty() {
                        ui.label(RichText::new(text).color(p.text2));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), actions);
            });
        });
}

/// The accent (primary) button.
pub fn accent_button(ui: &mut Ui, text: &str) -> egui::Response {
    let p = pal(ui);
    ui.add(egui::Button::new(RichText::new(text).color(p.on_accent)).fill(p.accent).stroke(Stroke::new(1.0, p.accent)).min_size(vec2(96.0, 32.0)))
}

/// A standard button.
pub fn button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(text).min_size(vec2(0.0, 32.0)))
}

/// A borderless command-bar button.
pub fn subtle_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(text).frame(false).min_size(vec2(0.0, 32.0)))
}

/// A thin progress bar (3 px, rounded).
pub fn progress_bar(ui: &mut Ui, fraction: f32) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), p.control_stroke);
    let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
    let done = Rect::from_min_size(rect.min, vec2(rect.width() * f, rect.height()));
    ui.painter().rect_filled(done, CornerRadius::same(2), p.accent);
}

/// Small line icons for the navigation rail.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Add,
    Clock,
    Mic,
    Book,
    Gear,
}

pub fn icon(ui: &Ui, rect: Rect, which: Icon, color: Color32) {
    let s = Stroke::new(1.4, color);
    let c = rect.center();
    let painter = ui.painter();
    match which {
        Icon::Add => {
            painter.line_segment([pos2(c.x - 6.0, c.y), pos2(c.x + 6.0, c.y)], s);
            painter.line_segment([pos2(c.x, c.y - 6.0), pos2(c.x, c.y + 6.0)], s);
        }
        Icon::Clock => {
            painter.circle_stroke(c, 6.5, s);
            painter.line_segment([c, pos2(c.x, c.y - 4.0)], s);
            painter.line_segment([c, pos2(c.x + 3.0, c.y + 1.5)], s);
        }
        Icon::Mic => {
            let body = Rect::from_center_size(pos2(c.x, c.y - 2.0), vec2(5.0, 9.0));
            painter.rect_stroke(body, CornerRadius::same(3), s, StrokeKind::Middle);
            let arc: Vec<Pos2> = (0..=12)
                .map(|k| {
                    let a = std::f32::consts::PI * (k as f32 / 12.0);
                    pos2(c.x - 5.0 * a.cos(), c.y + 1.0 + 4.5 * a.sin())
                })
                .collect();
            painter.add(egui::Shape::line(arc, s));
            painter.line_segment([pos2(c.x, c.y + 5.5), pos2(c.x, c.y + 7.5)], s);
        }
        Icon::Book => {
            // an open book: two pages meeting at the spine
            let l = [
                pos2(c.x, c.y - 4.5),
                pos2(c.x - 3.5, c.y - 6.0),
                pos2(c.x - 7.0, c.y - 5.0),
                pos2(c.x - 7.0, c.y + 5.0),
                pos2(c.x - 3.5, c.y + 4.0),
                pos2(c.x, c.y + 5.5),
            ];
            let r = [
                pos2(c.x, c.y - 4.5),
                pos2(c.x + 3.5, c.y - 6.0),
                pos2(c.x + 7.0, c.y - 5.0),
                pos2(c.x + 7.0, c.y + 5.0),
                pos2(c.x + 3.5, c.y + 4.0),
                pos2(c.x, c.y + 5.5),
            ];
            painter.add(egui::Shape::line(l.to_vec(), s));
            painter.add(egui::Shape::line(r.to_vec(), s));
            painter.line_segment([pos2(c.x, c.y - 4.5), pos2(c.x, c.y + 5.5)], s);
        }
        Icon::Gear => {
            painter.circle_stroke(c, 2.5, s);
            for k in 0..8 {
                let a = std::f32::consts::TAU * (k as f32 / 8.0);
                let d = Vec2::angled(a);
                painter.line_segment([c + d * 4.5, c + d * 6.8], s);
            }
            painter.circle_stroke(c, 4.6, s);
        }
    }
}

/// One item of the navigation rail; returns true when clicked.
pub fn nav_item(ui: &mut Ui, which: Icon, label: &str, selected: bool, compact: bool) -> bool {
    let p = pal(ui);
    let w = if compact { 40.0 } else { ui.available_width() };
    let (rect, r) = ui.allocate_exact_size(vec2(w, 36.0), Sense::click());
    let r = r.on_hover_text(label);
    if selected || r.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(theme::CONTROL_RADIUS), if selected { p.subtle } else { p.control_hover });
    }
    if selected {
        let bar = Rect::from_min_size(pos2(rect.left(), rect.center().y - 8.0), vec2(3.0, 16.0));
        ui.painter().rect_filled(bar, CornerRadius::same(2), p.accent);
    }
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
    icon(ui, icon_rect, which, p.text);
    if !compact {
        ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(14.0), p.text);
    }
    r.clicked()
}

/// A dashed rounded outline (the drop area).
pub fn dashed_rect(ui: &Ui, rect: Rect, color: Color32) {
    let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    for s in egui::Shape::dashed_line(&pts, Stroke::new(1.0, color), 6.0, 4.0) {
        ui.painter().add(s);
    }
}

/// `mm:ss` (or `h:mm:ss`) for a duration in seconds.
pub fn duration(secs: f64) -> String {
    let s = if secs.is_finite() && secs > 0.0 { secs.round() as u64 } else { 0 };
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(0.0), "0:00");
        assert_eq!(duration(65.4), "1:05");
        assert_eq!(duration(3_880.0), "1:04:40");
        assert_eq!(duration(f64::NAN), "0:00");
        assert_eq!(duration(-3.0), "0:00");
    }
}

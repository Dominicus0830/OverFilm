//! The few Windows parts the screens are built from: settings cards (title, description, one
//! control on the right), toggle switches, radio cards, info bars, the navigation rail, accent
//! buttons and small line icons.

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::theme::{self, Palette};

pub fn pal(ui: &Ui) -> Palette {
    theme::palette(ui.ctx())
}

/// Page title (20 px, bold).
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
    // turns over (down when closed, up when open) in `MOTION` seconds
    let t = ui.ctx().animate_bool_with_time(r.id, open, theme::MOTION);
    icon_turned(ui, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), Icon::ChevronDown, p.text2, t * std::f32::consts::PI);
    r
}

/// An on/off switch with "켬"/"끔" beside it: a 4 px rectangle with a square knob (no pill,
/// `docs/design.md`).
pub fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let p = pal(ui);
    let (rect, mut r) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let enabled = ui.is_enabled();
    let t = ui.ctx().animate_bool_responsive(r.id, *on);
    let painter = ui.painter();
    let radius = CornerRadius::same(theme::CONTROL_RADIUS);
    if *on {
        painter.rect_filled(rect, radius, if enabled { p.accent } else { p.text3 });
    } else {
        painter.rect_filled(rect, radius, p.control);
        painter.rect_stroke(rect, radius, Stroke::new(1.0, if r.hovered() { p.text2 } else { p.control_stroke }), StrokeKind::Inside);
    }
    let knob = 12.0;
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    let knob_rect = Rect::from_center_size(pos2(x, rect.center().y), vec2(knob, knob));
    painter.rect_filled(knob_rect, CornerRadius::same(2), if *on { p.on_accent } else { p.text2 });
    if r.has_focus() {
        painter.rect_stroke(rect.expand(2.0), radius, Stroke::new(1.0, p.accent), StrokeKind::Outside);
    }
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

/// An info bar: a 3 px colour bar on the left (no round icon), bold title and text; `actions`
/// on the right. Only ok and warning tint the background, because they mean something.
pub fn info_bar(ui: &mut Ui, sev: Severity, title: &str, text: &str, actions: impl FnOnce(&mut Ui)) {
    let p = pal(ui);
    let (bg, fg) = match sev {
        Severity::Info => (p.card, p.accent),
        Severity::Ok => (p.ok_bg, p.ok),
        Severity::Warn => (p.warn_bg, p.warn),
        Severity::Error => (p.card, p.bad),
    };
    let frame = egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, p.stroke))
        .corner_radius(CornerRadius::same(theme::CONTROL_RADIUS))
        .inner_margin(Margin { left: 16, right: 12, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
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
    let r = frame.response.rect;
    let bar = Rect::from_min_max(pos2(r.left() + 1.0, r.top() + 1.0), pos2(r.left() + 4.0, r.bottom() - 1.0));
    ui.painter().rect_filled(bar, CornerRadius { nw: theme::CONTROL_RADIUS, sw: theme::CONTROL_RADIUS, ne: 0, se: 0 }, fg);
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

/// A thin progress bar (4 px, 2 px corners).
pub fn progress_bar(ui: &mut Ui, fraction: f32) {
    let p = pal(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 4.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), p.control_stroke);
    let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
    let done = Rect::from_min_size(rect.min, vec2(rect.width() * f, rect.height()));
    ui.painter().rect_filled(done, CornerRadius::same(2), p.accent);
}

/// Line icons, the same set as the editor's (`crates/ui-egui/src/icons.rs`, `docs/design.md`):
/// drawn in code on a 16 x 16 grid with a 1.25 stroke, one colour, no third-party artwork.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Add,
    Clock,
    Mic,
    Book,
    Gear,
    ChevronDown,
}

/// A pen on the 16 x 16 icon grid inside `rect`, optionally turned by `turn` radians about the
/// centre (the expander chevron).
struct Pen16<'a> {
    painter: &'a egui::Painter,
    rect: Rect,
    turn: f32,
    stroke: Stroke,
}

impl Pen16<'_> {
    fn s(&self) -> f32 {
        self.rect.width().min(self.rect.height()) / 16.0
    }
    fn p(&self, x: f32, y: f32) -> Pos2 {
        let v = vec2(x - 8.0, y - 8.0) * self.s();
        let (sin, cos) = self.turn.sin_cos();
        self.rect.center() + vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
    }
    fn line(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| self.p(*x, *y)).collect();
        self.painter.add(egui::Shape::line(v, self.stroke));
    }
    fn circle(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_stroke(self.p(x, y), r * self.s(), self.stroke);
    }
    fn arc(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
        let pts: Vec<(f32, f32)> = (0..=20)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / 20.0).to_radians();
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.line(&pts);
    }
}

pub fn icon(ui: &Ui, rect: Rect, which: Icon, color: Color32) {
    icon_turned(ui, rect, which, color, 0.0);
}

fn icon_turned(ui: &Ui, rect: Rect, which: Icon, color: Color32, turn: f32) {
    let s = rect.width().min(rect.height()) / 16.0;
    let pen = Pen16 { painter: ui.painter(), rect, turn, stroke: Stroke::new((1.25 * s).max(1.0), color) };
    match which {
        Icon::Add => {
            pen.line(&[(8.0, 3.0), (8.0, 13.0)]);
            pen.line(&[(3.0, 8.0), (13.0, 8.0)]);
        }
        Icon::Clock => {
            pen.circle(8.0, 8.0, 6.0);
            pen.line(&[(8.0, 4.5), (8.0, 8.0), (10.5, 9.5)]);
        }
        Icon::Mic => {
            pen.line(&[(6.0, 2.0), (10.0, 2.0), (10.0, 10.0), (6.0, 10.0), (6.0, 2.0)]);
            pen.arc(8.0, 8.0, 4.5, 0.0, 180.0);
            pen.line(&[(8.0, 12.5), (8.0, 14.5)]);
        }
        Icon::Book => {
            // an open book: two pages meeting at the spine
            pen.line(&[(8.0, 4.0), (5.0, 2.8), (1.5, 3.5), (1.5, 12.5), (5.0, 11.8), (8.0, 13.0)]);
            pen.line(&[(8.0, 4.0), (11.0, 2.8), (14.5, 3.5), (14.5, 12.5), (11.0, 11.8), (8.0, 13.0)]);
            pen.line(&[(8.0, 4.0), (8.0, 13.0)]);
        }
        Icon::Gear => {
            pen.circle(8.0, 8.0, 2.2);
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                pen.line(&[(8.0 + 4.0 * a.cos(), 8.0 + 4.0 * a.sin()), (8.0 + 6.0 * a.cos(), 8.0 + 6.0 * a.sin())]);
            }
            pen.circle(8.0, 8.0, 4.2);
        }
        Icon::ChevronDown => pen.line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
    }
}

/// The selected item's marker on the navigation rail: a 3 px bar that glides to the item chosen
/// (`MOTION` seconds) instead of jumping. Each [`nav_item`] that is selected leaves its place.
pub fn nav_indicator(ui: &Ui) {
    let p = pal(ui);
    let key = egui::Id::unique("overfilm-subs-nav-selected");
    let Some((x, y)) = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(key)) else {
        return;
    };
    let y = ui.ctx().animate_value_with_time(key.with("y"), y, theme::MOTION);
    let bar = Rect::from_min_size(pos2(x, y - 8.0), vec2(3.0, 16.0));
    ui.painter().rect_filled(bar, CornerRadius::same(2), p.accent);
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
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("overfilm-subs-nav-selected"), (rect.left(), rect.center().y)));
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

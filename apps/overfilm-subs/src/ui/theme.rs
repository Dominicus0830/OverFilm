//! Windows 11 look for egui: the colours, corner radii (controls 4, windows and menus 8) and type
//! ramp (caption 12, body 14, subtitle 20, title 28) of the built-in Windows apps, in light and
//! dark, following the system theme.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, Visuals};

/// Colours of one theme.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Window background behind the navigation.
    pub base: Color32,
    /// Page area (one layer up from the base).
    pub layer: Color32,
    /// Cards and list rows.
    pub card: Color32,
    /// Buttons, boxes.
    pub control: Color32,
    pub control_hover: Color32,
    pub stroke: Color32,
    pub control_stroke: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub text3: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub accent_soft: Color32,
    pub subtle: Color32,
    pub warn_bg: Color32,
    pub warn: Color32,
    pub ok_bg: Color32,
    pub ok: Color32,
    pub bad: Color32,
    /// Speaker colours, in speaker order.
    pub speakers: [Color32; 6],
    pub game: Color32,
}

pub const LIGHT: Palette = Palette {
    base: Color32::from_rgb(0xf3, 0xf3, 0xf3),
    layer: Color32::from_rgb(0xf9, 0xf9, 0xf9),
    card: Color32::from_rgb(0xfd, 0xfd, 0xfd),
    control: Color32::from_rgb(0xff, 0xff, 0xff),
    control_hover: Color32::from_rgb(0xf6, 0xf6, 0xf6),
    stroke: Color32::from_rgb(0xe5, 0xe5, 0xe5),
    control_stroke: Color32::from_rgb(0xd1, 0xd1, 0xd1),
    text: Color32::from_rgb(0x1a, 0x1a, 0x1a),
    text2: Color32::from_rgb(0x5d, 0x5d, 0x5d),
    text3: Color32::from_rgb(0x8a, 0x8a, 0x8a),
    accent: Color32::from_rgb(0x00, 0x5f, 0xb8),
    on_accent: Color32::WHITE,
    accent_soft: Color32::from_rgb(0xdd, 0xea, 0xf7),
    subtle: Color32::from_rgb(0xea, 0xea, 0xea),
    warn_bg: Color32::from_rgb(0xff, 0xf4, 0xce),
    warn: Color32::from_rgb(0x9d, 0x5d, 0x00),
    ok_bg: Color32::from_rgb(0xdf, 0xf6, 0xdd),
    ok: Color32::from_rgb(0x0f, 0x7b, 0x0f),
    bad: Color32::from_rgb(0xc4, 0x2b, 0x1c),
    speakers: [
        Color32::from_rgb(0x3a, 0x7c, 0xc3),
        Color32::from_rgb(0x8a, 0x63, 0xc7),
        Color32::from_rgb(0xc0, 0x57, 0x7a),
        Color32::from_rgb(0x5c, 0x9a, 0x3a),
        Color32::from_rgb(0xb8, 0x86, 0x1b),
        Color32::from_rgb(0x2a, 0x9d, 0x8f),
    ],
    game: Color32::from_rgb(0x8a, 0x8a, 0x8a),
};

pub const DARK: Palette = Palette {
    base: Color32::from_rgb(0x20, 0x20, 0x20),
    layer: Color32::from_rgb(0x27, 0x27, 0x27),
    card: Color32::from_rgb(0x2b, 0x2b, 0x2b),
    control: Color32::from_rgb(0x2d, 0x2d, 0x2d),
    control_hover: Color32::from_rgb(0x35, 0x35, 0x35),
    stroke: Color32::from_rgb(0x1c, 0x1c, 0x1c),
    control_stroke: Color32::from_rgb(0x3d, 0x3d, 0x3d),
    text: Color32::WHITE,
    text2: Color32::from_rgb(0xc5, 0xc5, 0xc5),
    text3: Color32::from_rgb(0x9a, 0x9a, 0x9a),
    accent: Color32::from_rgb(0x60, 0xcd, 0xff),
    on_accent: Color32::BLACK,
    accent_soft: Color32::from_rgb(0x1f, 0x3a, 0x4a),
    subtle: Color32::from_rgb(0x32, 0x32, 0x32),
    warn_bg: Color32::from_rgb(0x43, 0x35, 0x19),
    warn: Color32::from_rgb(0xfc, 0xe1, 0x00),
    ok_bg: Color32::from_rgb(0x39, 0x3d, 0x1b),
    ok: Color32::from_rgb(0x6c, 0xcb, 0x5f),
    bad: Color32::from_rgb(0xff, 0x99, 0xa4),
    speakers: [
        Color32::from_rgb(0x6a, 0xa8, 0xe8),
        Color32::from_rgb(0xb3, 0x93, 0xea),
        Color32::from_rgb(0xe4, 0x8a, 0xa8),
        Color32::from_rgb(0x8c, 0xc4, 0x6a),
        Color32::from_rgb(0xe3, 0xbd, 0x5a),
        Color32::from_rgb(0x5e, 0xc8, 0xb8),
    ],
    game: Color32::from_rgb(0x8a, 0x8a, 0x8a),
};

/// The palette for the theme egui is showing now.
pub fn palette(ctx: &egui::Context) -> Palette {
    if ctx.theme() == Theme::Dark { DARK } else { LIGHT }
}

/// The subtitle and caption text styles (20 and 12 px).
pub fn subtitle() -> TextStyle {
    TextStyle::Name("Subtitle".into())
}

pub const CONTROL_RADIUS: u8 = 4;
pub const OVERLAY_RADIUS: u8 = 8;

/// Install both themes and the type ramp; egui then follows the system theme.
pub fn install(ctx: &egui::Context) {
    for (theme, p) in [(Theme::Light, LIGHT), (Theme::Dark, DARK)] {
        ctx.set_visuals_of(theme, visuals(theme, &p));
    }
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
            (TextStyle::Heading, FontId::new(28.0, FontFamily::Proportional)),
            (subtitle(), FontId::new(20.0, FontFamily::Proportional)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 32.0;
        s.spacing.combo_height = 320.0;
        s.spacing.menu_margin = egui::Margin::same(4);
    });
}

fn visuals(theme: Theme, p: &Palette) -> Visuals {
    let mut v = if theme == Theme::Dark { Visuals::dark() } else { Visuals::light() };
    let ctl = CornerRadius::same(CONTROL_RADIUS);
    v.panel_fill = p.base;
    v.window_fill = p.layer;
    v.window_stroke = Stroke::new(1.0, p.control_stroke);
    v.window_corner_radius = CornerRadius::same(OVERLAY_RADIUS);
    v.menu_corner_radius = CornerRadius::same(OVERLAY_RADIUS);
    v.extreme_bg_color = p.control;
    v.text_edit_bg_color = Some(p.control);
    v.faint_bg_color = p.card;
    v.code_bg_color = p.control;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.bad;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.text2);
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    for (w, fill) in [
        (&mut v.widgets.noninteractive, p.card),
        (&mut v.widgets.inactive, p.control),
        (&mut v.widgets.hovered, p.control_hover),
        (&mut v.widgets.active, p.subtle),
        (&mut v.widgets.open, p.control_hover),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.corner_radius = ctl;
        w.expansion = 0.0;
        w.fg_stroke.color = p.text;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    v.widgets.noninteractive.fg_stroke.color = p.text;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.control_stroke);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.control_stroke);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.control_stroke);
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.control_stroke);
    v
}

//! The dark violet theme — build plan phase 3, drawn to the mockup.
//!
//! §9 names the risk plainly: "egui will not look like a Mac app by default,
//! and the mockup is fairly polished… Accept it will read as a *designed* app
//! rather than a *native* one — which for a cross-platform tool is a defensible
//! identity, not a failure." So this commits to a look rather than chasing
//! three platforms' idea of a control.
//!
//! Every colour is named here and nowhere else, which is the one structural
//! thing worth copying from `cad_app/src/theme.rs`: a widget that reaches for a
//! literal is a widget that will be wrong the next time the palette moves.

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, Visuals};

// Surfaces, darkest first.
pub const VOID: Color32 = Color32::from_rgb(0x0B, 0x09, 0x14);
pub const PAPER: Color32 = Color32::from_rgb(0x10, 0x0D, 0x1C);
pub const CHROME: Color32 = Color32::from_rgb(0x16, 0x12, 0x24);
pub const PANEL: Color32 = Color32::from_rgb(0x1A, 0x16, 0x2A);
pub const RAISED: Color32 = Color32::from_rgb(0x22, 0x1D, 0x36);
pub const HEADER: Color32 = Color32::from_rgb(0x26, 0x21, 0x3C);
pub const LINE: Color32 = Color32::from_rgb(0x2E, 0x27, 0x46);

// Text.
pub const INK: Color32 = Color32::from_rgb(0xE8, 0xE3, 0xF5);
pub const INK_DIM: Color32 = Color32::from_rgb(0x92, 0x8A, 0xB0);
pub const INK_FAINT: Color32 = Color32::from_rgb(0x6B, 0x63, 0x88);

// Accent.
pub const VIOLET: Color32 = Color32::from_rgb(0x8B, 0x5C, 0xF6);
pub const VIOLET_BRIGHT: Color32 = Color32::from_rgb(0xA7, 0x8B, 0xFA);
pub const VIOLET_DEEP: Color32 = Color32::from_rgb(0x6D, 0x45, 0xC8);
pub const BLUE: Color32 = Color32::from_rgb(0x53, 0x8D, 0xF5);
pub const DANGER: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);
pub const PDF_RED: Color32 = Color32::from_rgb(0xE2, 0x43, 0x4C);

// The chequerboard behind a locked image — the convention every editor uses
// for "nothing here". Light and near-white rather than mid-grey: it sits on a
// white page, and a dark chequer would read as content of its own.
pub const CHEQUER_LIGHT: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF4);
pub const CHEQUER_DARK: Color32 = Color32::from_rgb(0xDC, 0xDC, 0xE2);

// Drawing.
pub const MARKUP: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x8A);
pub const SELECTED: Color32 = Color32::from_rgb(0x4C, 0xC9, 0xF0);
pub const SNAP: Color32 = Color32::from_rgb(0x5E, 0xEA, 0xD4);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    visuals.panel_fill = PAPER;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = VOID;
    visuals.faint_bg_color = RAISED;
    visuals.override_text_color = Some(INK);
    visuals.hyperlink_color = VIOLET_BRIGHT;
    visuals.error_fg_color = DANGER;
    visuals.warn_fg_color = Color32::from_rgb(0xF5, 0xC4, 0x6B);
    visuals.window_stroke = Stroke::new(1.0, LINE);

    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, INK_DIM);

    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, INK_DIM);

    visuals.widgets.hovered.bg_fill = RAISED;
    visuals.widgets.hovered.weak_bg_fill = RAISED;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, VIOLET_DEEP);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, INK);

    visuals.widgets.active.bg_fill = VIOLET;
    visuals.widgets.active.weak_bg_fill = VIOLET;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, VIOLET);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.open.bg_fill = RAISED;
    visuals.widgets.open.weak_bg_fill = RAISED;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, INK);

    visuals.selection.bg_fill = VIOLET_DEEP.gamma_multiply(0.55);
    visuals.selection.stroke = Stroke::new(1.0, VIOLET_BRIGHT);

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(7);
    }

    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
    });
}

/// A rounded tile with a violet-to-blue wash — the shape the logo and every
/// Tool Wizard icon share.
pub fn icon_tile(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let radius = CornerRadius::same(9);
    painter.rect_filled(rect, radius, bottom);
    // A lighter band across the top two-thirds reads as a wash without needing
    // a real gradient behind rounded corners.
    let upper = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + rect.height() * 0.62));
    painter.rect_filled(upper, radius, top.gamma_multiply(0.85));
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, top.gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
}

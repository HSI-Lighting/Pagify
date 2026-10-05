//! The app's palette — dark (the original, build plan phase 3) and light (the
//! compact-UI restyle, drawn to `pagify_pdf_compact_light_theme`), switched at
//! runtime from the View tab. See `../docs/COMPACT_UI_RESTYLE.md` §1.
//!
//! §9 of the original build plan names the risk plainly: "egui will not look
//! like a Mac app by default, and the mockup is fairly polished… Accept it
//! will read as a *designed* app rather than a *native* one — which for a
//! cross-platform tool is a defensible identity, not a failure." So this
//! commits to a look rather than chasing three platforms' idea of a control.
//!
//! Every colour is named here and nowhere else — the one structural thing
//! worth keeping from `cad_app/src/theme.rs`: a widget that reaches for a
//! literal is a widget that will be wrong the next time the palette moves.
//! That now includes "moves because someone flipped the toggle", not only
//! "moves because the palette itself changed" — which is why these are
//! functions reading a runtime flag rather than `const`s: a `const` cannot be
//! conditional on anything, and a toggle is nothing but a condition.

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, Visuals};
use std::sync::atomic::{AtomicBool, Ordering};

pub use pagify_shell::appearance::Mode;

/// `false` = Dark, `true` = Light. A bool, not the `Mode` enum itself —
/// `AtomicBool` is in `std::sync::atomic`; a two-variant enum would need a
/// `u8` round-trip through `AtomicU8` for the exact same two states.
static IS_LIGHT: AtomicBool = AtomicBool::new(false);

pub fn mode() -> Mode {
    if IS_LIGHT.load(Ordering::Relaxed) { Mode::Light } else { Mode::Dark }
}

/// Flips the active palette and remembers it for next launch. Does **not**
/// repaint by itself — `apply` runs once a frame from `PagifyApp::ui`'s own
/// top, the same as every other per-frame recomputation in that function
/// (`resolved_zoom`, for one), so the next frame already reflects this
/// without `set_mode` needing a `&egui::Context` of its own to call `apply`
/// with. `Verb` dispatch (`fn act`) has no such context to hand it.
pub fn set_mode(new_mode: Mode) {
    IS_LIGHT.store(new_mode == Mode::Light, Ordering::Relaxed);
    pagify_shell::appearance::save(new_mode);
}

pub fn toggle() {
    set_mode(if mode() == Mode::Light { Mode::Dark } else { Mode::Light });
}

/// Reads the remembered mode off disk into the live flag — called once, from
/// `PagifyApp::new`, before that tab's first `ui` frame runs `apply`. A
/// missing or corrupt file leaves the flag at its default, Dark: the same
/// "never fail to start over a preferences file" rule
/// `pagify_shell::recent::Recent::load` already follows, and the right
/// default for not surprising anyone who upgrades without ever touching the
/// toggle.
pub fn load_persisted_mode() {
    IS_LIGHT.store(pagify_shell::appearance::load() == Mode::Light, Ordering::Relaxed);
}

mod dark {
    use egui::Color32;

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
    pub const WARN: Color32 = Color32::from_rgb(0xF5, 0xC4, 0x6B);
    pub const PDF_RED: Color32 = Color32::from_rgb(0xE2, 0x43, 0x4C);

    // The chequerboard behind a locked image — the convention every editor
    // uses for "nothing here". Light and near-white rather than mid-grey in
    // *both* themes: it sits on a white page regardless of app theme, and a
    // dark chequer would read as content of its own.
    pub const CHEQUER_LIGHT: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF4);
    pub const CHEQUER_DARK: Color32 = Color32::from_rgb(0xDC, 0xDC, 0xE2);

    // Drawing.
    pub const MARKUP: Color32 = Color32::from_rgb(0xFF, 0x5C, 0x8A);
    pub const SELECTED: Color32 = Color32::from_rgb(0x4C, 0xC9, 0xF0);
    pub const SNAP: Color32 = Color32::from_rgb(0x5E, 0xEA, 0xD4);
}

/// Built from `pagify_pdf_compact_light_theme/code.html`'s own Tailwind
/// config (lines 13–27) and default `slate` scale — see
/// `../docs/COMPACT_UI_RESTYLE.md` §1. The violet accent is split across
/// three shades the same way the dark palette is, by *role* rather than by
/// carrying the dark palette's own hex values over: `VIOLET` is the brand
/// colour as seen *on* a light surface (text, icons — the mockup's own
/// `text-brand-600`), `VIOLET_DEEP` is the saturated fill a light-on-top
/// label sits on (the mockup's `bg-brand-600`/`700` buttons), `VIOLET_BRIGHT`
/// the lighter, more vivid step between them. A handful of constants the
/// mockup's CSS never names (`WARN`, `BLUE`, `DANGER`, `PDF_RED`, `MARKUP`,
/// `SELECTED`, `SNAP`) are chosen to keep roughly the same contrast against
/// these lighter surfaces that the dark palette's own versions keep against
/// its darker ones, since no mockup page exercises them.
mod light {
    use egui::Color32;

    pub const VOID: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    pub const PAPER: Color32 = Color32::from_rgb(0xF8, 0xFA, 0xFC);
    pub const CHROME: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    pub const PANEL: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    pub const RAISED: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF0);
    pub const HEADER: Color32 = Color32::from_rgb(0xF1, 0xF5, 0xF9);
    pub const LINE: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF0);

    pub const INK: Color32 = Color32::from_rgb(0x0F, 0x17, 0x2A);
    pub const INK_DIM: Color32 = Color32::from_rgb(0x47, 0x55, 0x69);
    pub const INK_FAINT: Color32 = Color32::from_rgb(0x94, 0xA3, 0xB8);

    pub const VIOLET: Color32 = Color32::from_rgb(0x7C, 0x3A, 0xED);
    pub const VIOLET_BRIGHT: Color32 = Color32::from_rgb(0x8B, 0x5C, 0xF6);
    pub const VIOLET_DEEP: Color32 = Color32::from_rgb(0x6D, 0x28, 0xD9);
    pub const BLUE: Color32 = Color32::from_rgb(0x2E, 0x5E, 0xD6);
    pub const DANGER: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
    pub const WARN: Color32 = Color32::from_rgb(0xB4, 0x5B, 0x09);
    pub const PDF_RED: Color32 = Color32::from_rgb(0xC2, 0x29, 0x2E);

    pub const CHEQUER_LIGHT: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF4);
    pub const CHEQUER_DARK: Color32 = Color32::from_rgb(0xDC, 0xDC, 0xE2);

    pub const MARKUP: Color32 = Color32::from_rgb(0xDB, 0x27, 0x77);
    pub const SELECTED: Color32 = Color32::from_rgb(0x02, 0x88, 0xD1);
    pub const SNAP: Color32 = Color32::from_rgb(0x0D, 0x94, 0x88);
}

macro_rules! themed {
    ($fn_name:ident, $const_name:ident) => {
        pub fn $fn_name() -> Color32 {
            if mode() == Mode::Light { light::$const_name } else { dark::$const_name }
        }
    };
}

themed!(void, VOID);
themed!(paper, PAPER);
themed!(chrome, CHROME);
themed!(panel, PANEL);
themed!(raised, RAISED);
themed!(header, HEADER);
themed!(line, LINE);
themed!(ink, INK);
themed!(ink_dim, INK_DIM);
themed!(ink_faint, INK_FAINT);
themed!(violet, VIOLET);
themed!(violet_bright, VIOLET_BRIGHT);
themed!(violet_deep, VIOLET_DEEP);
themed!(blue, BLUE);
themed!(danger, DANGER);
themed!(warn, WARN);
themed!(pdf_red, PDF_RED);
themed!(chequer_light, CHEQUER_LIGHT);
themed!(chequer_dark, CHEQUER_DARK);
themed!(markup, MARKUP);
themed!(selected, SELECTED);
themed!(snap, SNAP);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = if mode() == Mode::Light { Visuals::light() } else { Visuals::dark() };

    visuals.panel_fill = paper();
    visuals.window_fill = panel();
    visuals.extreme_bg_color = void();
    visuals.faint_bg_color = raised();
    visuals.override_text_color = Some(ink());
    visuals.hyperlink_color = violet_bright();
    visuals.error_fg_color = danger();
    visuals.warn_fg_color = warn();
    visuals.window_stroke = Stroke::new(1.0, line());
    // Sharp rectangles throughout, not egui's own rounded defaults —
    // reported from use after the compact restyle made the ambient rounding
    // (`widget.corner_radius` below) much more visible than it used to be.
    visuals.window_corner_radius = CornerRadius::ZERO;
    visuals.menu_corner_radius = CornerRadius::ZERO;

    visuals.widgets.noninteractive.bg_fill = panel();
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, line());
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, ink_dim());

    visuals.widgets.inactive.bg_fill = raised();
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    // **Reported from use**: an unchecked checkbox ("Pages", "Ortho" in the
    // titlebar) was indistinguishable from plain text — egui draws its small
    // box using exactly this stroke, and `Stroke::NONE` left it with no
    // visible edge at all, so there was nothing on screen to suggest it
    // could be clicked. A button's own `bg_fill` still carries it; a
    // checkbox's unchecked box has nothing else to.
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, line());
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, ink_dim());

    visuals.widgets.hovered.bg_fill = raised();
    visuals.widgets.hovered.weak_bg_fill = raised();
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, violet_deep());
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, ink());

    visuals.widgets.active.bg_fill = violet();
    visuals.widgets.active.weak_bg_fill = violet();
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, violet());
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.open.bg_fill = raised();
    visuals.widgets.open.weak_bg_fill = raised();
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, line());
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, ink());

    visuals.selection.bg_fill = violet_deep().gamma_multiply(0.55);
    visuals.selection.stroke = Stroke::new(1.0, violet_bright());

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::ZERO;
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
    let radius = CornerRadius::ZERO;
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

// `IS_LIGHT` is one process-wide flag, and `cargo test` runs `#[test]`
// functions on separate threads by default, across every module in this
// binary — any test anywhere in `pagify_app` that calls `set_mode`/`toggle`
// needs this, not just this module's own tests below, or two such tests
// running concurrently would flip the same flag out from under each other.
// `pub(crate)` rather than private, for exactly that reason. Held for a
// whole test's body, not just around a single read/write, so nothing is ever
// racing a sibling test rather than just the production code it means to
// exercise.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing that must never happen: a leftover call somewhere still
    /// reading a dark-mode colour by name while the rest of the app is light,
    /// or vice versa. Every themed accessor has to move together.
    #[test]
    fn every_themed_colour_changes_between_modes() {
        let _guard = TEST_LOCK.lock().unwrap();
        set_mode(Mode::Dark);
        let before = [
            void(), paper(), chrome(), panel(), raised(), header(), line(), ink(), ink_dim(),
            ink_faint(), violet(), violet_bright(), violet_deep(), blue(), danger(), warn(),
            pdf_red(), markup(), selected(), snap(),
        ];
        set_mode(Mode::Light);
        let after = [
            void(), paper(), chrome(), panel(), raised(), header(), line(), ink(), ink_dim(),
            ink_faint(), violet(), violet_bright(), violet_deep(), blue(), danger(), warn(),
            pdf_red(), markup(), selected(), snap(),
        ];
        assert_ne!(before, after, "flipping the mode should change every one of these");
        set_mode(Mode::Dark);
    }

    /// `chequer_light`/`chequer_dark` are the one pair that's deliberately
    /// the *same* in both modes — the locked-image chequerboard sits on a
    /// white page regardless of app theme, so it is not part of the "every
    /// colour changes" test above.
    #[test]
    fn the_chequerboard_stays_the_same_in_both_modes() {
        let _guard = TEST_LOCK.lock().unwrap();
        set_mode(Mode::Dark);
        let dark = (chequer_light(), chequer_dark());
        set_mode(Mode::Light);
        let light = (chequer_light(), chequer_dark());
        assert_eq!(dark, light);
        set_mode(Mode::Dark);
    }

    #[test]
    fn toggle_flips_and_set_mode_is_absolute() {
        let _guard = TEST_LOCK.lock().unwrap();
        set_mode(Mode::Dark);
        toggle();
        assert_eq!(mode(), Mode::Light);
        toggle();
        assert_eq!(mode(), Mode::Dark);
        set_mode(Mode::Light);
        set_mode(Mode::Light);
        assert_eq!(mode(), Mode::Light);
        set_mode(Mode::Dark);
    }
}

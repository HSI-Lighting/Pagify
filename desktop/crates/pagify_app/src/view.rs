//! `PagifyApp`'s own view methods: the zoom this tab reads at, turning pages,
//! rotating the page.

use crate::ZoomMode;
use pagify_shell::verbs::{PageTarget, ZoomTarget};
use pdf_core::Rotation;

impl crate::PagifyApp {
    /// **Reported from use**: the same file at the same zoom looked smaller
    /// in Pagify than in another reader. A PDF point is 1/72 inch, but the
    /// on-screen-pixels-per-inch nearly every PDF viewer, browser and the
    /// desktop itself has agreed on since Windows' own 96-DPI default is
    /// 96, not 72 — so "100%" meaning "one point, one egui unit" (what
    /// `resolved_zoom` returning `1.0` used to turn into directly) rendered
    /// at 72/96 = 75% of what "100%" means everywhere else. This is the
    /// correction, applied once in `draw_pages` to convert the *logical*
    /// zoom (what gets stored, stepped, and shown as a percentage) into the
    /// actual on-screen size — not folded into `resolved_zoom` itself,
    /// because `ZoomMode::Factor`'s own stored number, the percentage
    /// label, and the pinch-to-zoom math that reads and writes that same
    /// stored number all still need the *un*-corrected value to stay
    /// internally consistent with each other.
    pub(crate) const DISPLAY_DPI_SCALE: f32 = 96.0 / 72.0;

    /// The zoom ceiling every interactive path (toolbar buttons, `+`/`-`,
    /// pinch, Ctrl-scroll) is clamped to — 6400%, matching the range
    /// professional PDF/CAD viewers offer. **Reported from use**: on a
    /// vector-heavy AutoCAD-exported page, Pagify could not reach the same
    /// close-up detail another viewer could. Three call sites each clamped
    /// to a literal `16.0` (1600%) independently of this one; raised and
    /// unified here rather than in each of them, since any one left behind
    /// would silently reintroduce the lower ceiling for its own path.
    pub(crate) const MAX_ZOOM: f32 = 64.0;

    /// The *logical* zoom — what `ZoomMode::Factor` stores, what the
    /// percentage display reads, and what a pinch or a step multiplies.
    /// `draw_pages` converts this to an actual on-screen size once, near its
    /// own top — see [`Self::DISPLAY_DPI_SCALE`] for why those two numbers
    /// are not the same thing, and `Width`/`Fit` below for why computing a
    /// fit against `available` (an on-screen size) needs to divide that
    /// factor back out before handing back a *logical* one.
    pub(crate) fn resolved_zoom(&self) -> f32 {
        let (w, h) = self.page_extent(self.tab().zoom_settle.zoom_basis);
        let available = (self.tab().canvas_pt - egui::vec2(24.0, 24.0)).max(egui::vec2(1.0, 1.0));
        match self.tab().zoom {
            ZoomMode::Factor(f) => f,
            ZoomMode::Width => {
                ((available.x / w) / Self::DISPLAY_DPI_SCALE).clamp(0.05, Self::MAX_ZOOM)
            }
            ZoomMode::Fit => ((available.x / w).min(available.y / h) / Self::DISPLAY_DPI_SCALE)
                .clamp(0.05, Self::MAX_ZOOM),
        }
    }

    pub(crate) fn go_to(&mut self, target: PageTarget) {
        let Some(page_count) = self.tab().doc.as_ref().map(|d| d.page_count) else {
            self.say_error("nothing open.");
            return;
        };
        let last = page_count.saturating_sub(1);
        let current_page = self.tab().page;
        let wanted = match target {
            PageTarget::Number(n) => n - 1,
            PageTarget::Next => current_page.saturating_add(1),
            PageTarget::Previous => current_page.saturating_sub(1),
            PageTarget::First => 0,
            PageTarget::Last => last,
        };
        if wanted > last {
            self.say_error(format!("there are only {page_count} pages."));
            return;
        }
        self.tab_mut().page = wanted;
        self.tab_mut().zoom_settle.zoom_basis = wanted;
        let scroll_pt = self.tab().doc.as_ref().map(|d| d.strip.scroll_to(wanted)).unwrap_or(0.0);
        self.tab_mut().scroll_pt = scroll_pt;
        self.tab_mut().scroll_to_pt = Some(scroll_pt);
        self.tab_mut().zoom_settle.settling = 3;
    }

    pub(crate) fn set_zoom(&mut self, target: ZoomTarget) {
        // Choosing Fit or Width fits the page being read now: see `zoom_basis`.
        if matches!(target, ZoomTarget::Fit | ZoomTarget::Width) {
            let page = self.tab().page;
            self.tab_mut().zoom_settle.zoom_basis = page;
        }
        let current = self.resolved_zoom();
        // Every `Factor` built here is clamped at the point it's built —
        // including `ZoomTarget::Factor` itself, the typed `zoom N%` command,
        // which this previously let through uncapped while the button/key/
        // pinch paths below were independently capped lower (at a literal
        // `16.0`, not `Self::MAX_ZOOM`). One inconsistent cap in three places
        // is worse than one cap in one place.
        self.tab_mut().zoom = match target {
            ZoomTarget::Factor(f) => ZoomMode::Factor(f.clamp(0.05, Self::MAX_ZOOM)),
            ZoomTarget::In => ZoomMode::Factor((current * 1.25).clamp(0.05, Self::MAX_ZOOM)),
            ZoomTarget::Out => ZoomMode::Factor((current / 1.25).clamp(0.05, Self::MAX_ZOOM)),
            ZoomTarget::Actual => ZoomMode::Factor(1.0),
            ZoomTarget::Fit => ZoomMode::Fit,
            ZoomTarget::Width => ZoomMode::Width,
        };
    }

    pub(crate) fn rotate_view(&mut self, degrees: i32) {
        let quarters = |r: Rotation| match r {
            Rotation::None => 0,
            Rotation::Clockwise90 => 1,
            Rotation::Clockwise180 => 2,
            Rotation::Clockwise270 => 3,
        };
        let turned = (quarters(self.tab_mut().rotation) + degrees.rem_euclid(360) / 90) % 4;
        self.tab_mut().rotation = match turned {
            0 => Rotation::None,
            1 => Rotation::Clockwise90,
            2 => Rotation::Clockwise180,
            _ => Rotation::Clockwise270,
        };
        // **The frames the pages are drawn in turn with them.** Reported from
        // use, with screenshots, as a regression: "when i rotate the pages it
        // changes the ratio of the page". Only the raster was turned; the strip
        // still laid every page out, and was drawing it, in the frame of the
        // page upright — so a quarter turn squeezed the page into a frame of
        // the wrong proportions.
        let sideways = self.tab_mut().rotation.swaps_axes();
        let page = self.tab_mut().page;
        if let Some(doc) = &mut self.tab_mut().doc {
            doc.rendered_is_stale();
            doc.strip = doc.strip.retarget(doc.strip.layout(), sideways);
        }
        // The rows changed height, so wherever the view was is now somewhere
        // else; keep the page that was being read in view.
        let top = self.tab_mut().doc.as_ref().and_then(|d| d.strip.top_of(page));
        self.tab_mut().scroll_to_pt = top;
        self.tab_mut().zoom_settle.settling = 3;
    }
}

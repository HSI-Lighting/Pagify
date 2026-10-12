//! Where a page lands on a sheet of paper — the one answer both the print
//! window's preview and the printer job are drawn from.
//!
//! Pure: device pixels in, device pixels out, no Windows types. The preview
//! and the real job calling the same [`place`] is what makes "what you see is
//! what prints" a property of the code rather than something two copies of the
//! arithmetic have to keep agreeing on.

use pdf_core::document::Rotation;

/// A sheet as the printer reports it, all in device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paper {
    /// The whole sheet (`PHYSICALWIDTH` / `PHYSICALHEIGHT`).
    pub physical: (u32, u32),
    /// The unprintable margin to the printable area's top-left
    /// (`PHYSICALOFFSETX` / `PHYSICALOFFSETY`).
    pub offset: (u32, u32),
    /// The printable area (`HORZRES` / `VERTRES`). GDI's (0, 0) is its top-left.
    pub printable: (u32, u32),
    /// Pixels per inch (`LOGPIXELSX` / `LOGPIXELSY`).
    pub dpi: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Scale up or down to fill the printable area, aspect kept.
    #[default]
    Page,
    /// 72 points to the inch, whatever the sheet.
    ActualSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub fit: Fit,
    /// Turn a page a quarter turn when that fills the sheet better.
    pub rotate_to_fit: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { fit: Fit::Page, rotate_to_fit: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// `None` or `Clockwise90`: what to render the page with.
    pub rotation: Rotation,
    /// Where the rendered page goes, relative to the printable area's origin.
    pub dest: PxRect,
    /// Actual size overflowed the printable area, so part of the page is lost.
    pub clipped: bool,
}

/// The page's natural size in device pixels when turned by `rotation`.
fn natural(paper: &Paper, page_pt: (f32, f32), rotation: Rotation) -> (f32, f32) {
    let (w, h) = match rotation {
        Rotation::Clockwise90 | Rotation::Clockwise270 => (page_pt.1, page_pt.0),
        _ => page_pt,
    };
    (w * paper.dpi.0 / 72.0, h * paper.dpi.1 / 72.0)
}

/// How well `rotation` suits the sheet: bigger is better. Fit scores the scale
/// it reaches; actual size scores minus what spills over the edge.
fn score(paper: &Paper, page_pt: (f32, f32), fit: Fit, rotation: Rotation) -> f32 {
    let (nw, nh) = natural(paper, page_pt, rotation);
    let (pw, ph) = (paper.printable.0 as f32, paper.printable.1 as f32);
    match fit {
        Fit::Page => (pw / nw).min(ph / nh),
        Fit::ActualSize => -((nw - pw).max(0.0) + (nh - ph).max(0.0)),
    }
}

/// Centred when it fits an axis, against the printable origin when it does not.
fn centred(inner: i32, outer: i32) -> i32 {
    if inner <= outer {
        (outer - inner) / 2
    } else {
        0
    }
}

pub fn place(paper: &Paper, page_pt: (f32, f32), options: &Options) -> Placement {
    let (pw, ph) = (paper.printable.0 as i32, paper.printable.1 as i32);
    let empty = Placement {
        rotation: Rotation::None,
        dest: PxRect { x: 0, y: 0, w: 0, h: 0 },
        clipped: false,
    };
    if pw <= 0 || ph <= 0 || !(page_pt.0 > 0.0) || !(page_pt.1 > 0.0) || !(paper.dpi.0 > 0.0) || !(paper.dpi.1 > 0.0) {
        return empty;
    }

    // A quarter turn has to be *clearly* better: a square page, or a tie, stays
    // upright rather than flipping on rounding noise.
    let upright = score(paper, page_pt, options.fit, Rotation::None);
    let turned = score(paper, page_pt, options.fit, Rotation::Clockwise90);
    let rotation = if options.rotate_to_fit && turned > upright + upright.abs() * 1e-5 + 1e-6 {
        Rotation::Clockwise90
    } else {
        Rotation::None
    };

    let (nw, nh) = natural(paper, page_pt, rotation);
    let (w, h) = match options.fit {
        Fit::Page => {
            let s = (pw as f32 / nw).min(ph as f32 / nh);
            // The clamp keeps rounding from putting the page a pixel over.
            (((nw * s).round() as i32).clamp(1, pw), ((nh * s).round() as i32).clamp(1, ph))
        }
        Fit::ActualSize => ((nw.round() as i32).max(1), (nh.round() as i32).max(1)),
    };
    Placement {
        rotation,
        dest: PxRect { x: centred(w, pw), y: centred(h, ph), w, h },
        clipped: w > pw || h > ph,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A4 at 300 dpi with no margin, portrait or landscape.
    fn a4(landscape: bool) -> Paper {
        let (w, h) = if landscape { (3508, 2480) } else { (2480, 3508) };
        Paper { physical: (w, h), offset: (0, 0), printable: (w, h), dpi: (300.0, 300.0) }
    }
    const A4_PT: (f32, f32) = (595.0, 842.0);
    const A4_LANDSCAPE_PT: (f32, f32) = (842.0, 595.0);

    fn fit(rotate: bool) -> Options {
        Options { fit: Fit::Page, rotate_to_fit: rotate }
    }
    fn actual(rotate: bool) -> Options {
        Options { fit: Fit::ActualSize, rotate_to_fit: rotate }
    }

    #[test]
    fn a_page_the_shape_of_the_sheet_fills_it_without_turning() {
        let p = place(&a4(false), A4_PT, &fit(true));
        assert_eq!(p.rotation, Rotation::None);
        assert!(p.dest.w >= 2479 && p.dest.h >= 3507, "{p:?}");
        assert!(!p.clipped);
    }

    #[test]
    fn a_landscape_page_on_portrait_paper_is_turned_to_fill_it() {
        let turned = place(&a4(false), A4_LANDSCAPE_PT, &fit(true));
        let upright = place(&a4(false), A4_LANDSCAPE_PT, &fit(false));
        assert_eq!(turned.rotation, Rotation::Clockwise90);
        assert_eq!(upright.rotation, Rotation::None);
        assert!(turned.dest.w * turned.dest.h > upright.dest.w * upright.dest.h * 3 / 2, "{turned:?} vs {upright:?}");
    }

    #[test]
    fn a_portrait_page_on_landscape_paper_is_turned_too() {
        assert_eq!(place(&a4(true), A4_PT, &fit(true)).rotation, Rotation::Clockwise90);
    }

    #[test]
    fn a_square_page_is_never_turned() {
        assert_eq!(place(&a4(false), (500.0, 500.0), &fit(true)).rotation, Rotation::None);
        assert_eq!(place(&a4(true), (500.0, 500.0), &fit(true)).rotation, Rotation::None);
    }

    #[test]
    fn fit_centres_on_the_axis_with_room_to_spare() {
        // A tall thin page on A4: full height, centred across.
        let p = place(&a4(false), (200.0, 842.0), &fit(false));
        assert_eq!((p.dest.y, p.dest.h), (0, 3508));
        assert_eq!(p.dest.x, (2480 - p.dest.w) / 2);
        assert!(p.dest.x > 0);
    }

    #[test]
    fn a_small_page_is_scaled_up_to_fill_the_sheet() {
        let p = place(&a4(false), (297.5, 421.0), &fit(false));
        assert!(p.dest.w >= 2479 && p.dest.h >= 3507, "{p:?}");
    }

    #[test]
    fn actual_size_is_72_points_to_the_inch_and_centred_when_it_fits() {
        // 5 x 7 in on A4 at 300 dpi → 1500 x 2100 px.
        let p = place(&a4(false), (360.0, 504.0), &actual(false));
        assert_eq!((p.dest.w, p.dest.h), (1500, 2100));
        assert_eq!((p.dest.x, p.dest.y), ((2480 - 1500) / 2, (3508 - 2100) / 2));
        assert!(!p.clipped);
    }

    #[test]
    fn actual_size_that_overflows_is_anchored_at_the_origin_and_flagged() {
        // A0 portrait (2384 x 3370 pt) on A4.
        let p = place(&a4(false), (2384.0, 3370.0), &actual(false));
        assert_eq!((p.dest.x, p.dest.y), (0, 0));
        assert!(p.clipped);
    }

    #[test]
    fn actual_size_turns_a_page_only_when_that_stops_it_overflowing() {
        // 500 x 300 pt = 2083 x 1250 px: too wide for a 1500-wide sheet upright,
        // 1250 x 2083 fits when turned.
        let sheet = Paper { physical: (1500, 2500), offset: (0, 0), printable: (1500, 2500), dpi: (300.0, 300.0) };
        let turned = place(&sheet, (500.0, 300.0), &actual(true));
        assert_eq!(turned.rotation, Rotation::Clockwise90);
        assert!(!turned.clipped);
        assert_eq!(place(&sheet, (500.0, 300.0), &actual(false)).rotation, Rotation::None);
    }

    #[test]
    fn the_unprintable_margin_does_not_move_the_origin() {
        // GDI's (0, 0) is the printable area's corner, so placement is relative
        // to it whatever the margin is.
        let mut sheet = a4(false);
        sheet.offset = (100, 80);
        sheet.printable = (2280, 3348);
        let p = place(&sheet, A4_PT, &fit(false));
        assert!(p.dest.x >= 0 && p.dest.y >= 0);
        assert!(p.dest.x + p.dest.w <= 2280 && p.dest.y + p.dest.h <= 3348, "{p:?}");
    }

    #[test]
    fn non_square_printer_resolution_still_keeps_the_pages_shape() {
        let sheet = Paper { physical: (1200, 3000), offset: (0, 0), printable: (1200, 3000), dpi: (300.0, 600.0) };
        let p = place(&sheet, (400.0, 400.0), &fit(false));
        // 400 pt square is 1667 x 3333 device px: twice as tall as wide.
        assert_eq!(p.dest.h, 2 * p.dest.w, "{p:?}");
    }

    #[test]
    fn fit_never_leaves_the_printable_area() {
        for (w, h) in [(595.0, 842.0), (842.0, 595.0), (3.0, 4000.0), (4000.0, 3.0), (1190.55, 841.89), (2384.0, 3370.0)] {
            for dpi in [150.0f32, 300.0, 600.0, 1200.0] {
                let sheet = Paper {
                    physical: ((8.27 * dpi) as u32, (11.69 * dpi) as u32),
                    offset: (0, 0),
                    printable: ((8.0 * dpi) as u32, (11.4 * dpi) as u32),
                    dpi: (dpi, dpi),
                };
                for rotate in [false, true] {
                    let p = place(&sheet, (w, h), &fit(rotate));
                    assert!(p.dest.x >= 0 && p.dest.y >= 0, "{w}x{h}@{dpi}: {p:?}");
                    assert!(p.dest.x + p.dest.w <= sheet.printable.0 as i32, "{w}x{h}@{dpi}: {p:?}");
                    assert!(p.dest.y + p.dest.h <= sheet.printable.1 as i32, "{w}x{h}@{dpi}: {p:?}");
                    assert!(!p.clipped);
                }
            }
        }
    }

    #[test]
    fn nonsense_in_gives_an_empty_placement_not_a_panic() {
        let zero = Paper { physical: (0, 0), offset: (0, 0), printable: (0, 0), dpi: (0.0, 0.0) };
        assert_eq!(place(&zero, A4_PT, &fit(true)).dest.w, 0);
        assert_eq!(place(&a4(false), (0.0, 842.0), &fit(true)).dest.w, 0);
        assert_eq!(place(&a4(false), (f32::NAN, 842.0), &fit(true)).dest.w, 0);
    }
}

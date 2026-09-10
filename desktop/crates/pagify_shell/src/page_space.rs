//! The one place a coordinate is flipped.
//!
//! Three spaces, and two of them disagree about which way *y* goes:
//!
//! | Space        | Unit        | Origin       | y    |
//! |--------------|-------------|--------------|------|
//! | PDF page     | points      | bottom-left  | up   |
//! | Pagify app   | page points | top-left     | down |
//! | `cad_kernel` | drawing units | arbitrary  | up   |
//!
//! `pdf_core` already flips once, at the PDFium boundary, and says so. Adding
//! `cad_kernel` invites a *second* flip, and two flips is a bug factory: the
//! failure is silent, only some objects appear mirrored, and it surfaces weeks
//! later. Build plan §5.1 settles it — app space is canonical for the markup
//! layer, and this module is the only code in either crate allowed to convert.
//!
//! ## Why the kernel's origin is the page's bottom-left
//!
//! The kernel's origin is nominally arbitrary, so it is worth spending: pinning
//! it to the page's bottom-left in points makes kernel space *identical* to PDF
//! page space. Committing markup to the PDF is then a copy, not a conversion,
//! and the flip below stays the only one in the codebase. Choosing anything
//! else would buy nothing and reintroduce the second flip at save time.

use cad_kernel::Vec2;

/// A point in Pagify's app space: page points, top-left origin, y increasing
/// downwards. Distinct from [`Vec2`] on purpose — the compiler should refuse to
/// mix the two, because a mixed pair is exactly the bug this module exists to
/// prevent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppPoint {
    pub x: f64,
    pub y: f64,
}

impl AppPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        AppPoint { x, y }
    }
}

/// The conversion for one page. Constructed from the page's height, because the
/// flip is meaningless without it — taking it here is what stops a caller
/// converting with a height it merely assumed.
#[derive(Debug, Clone, Copy)]
pub struct PageSpace {
    height_pt: f64,
}

impl PageSpace {
    pub const fn new(height_pt: f64) -> Self {
        PageSpace { height_pt }
    }

    /// The page height this conversion is pinned to, in points.
    pub const fn height_pt(&self) -> f64 {
        self.height_pt
    }

    /// App space → kernel space.
    pub fn to_kernel(&self, p: AppPoint) -> Vec2 {
        Vec2::new(p.x, self.height_pt - p.y)
    }

    /// Kernel space → app space.
    pub fn from_kernel(&self, v: Vec2) -> AppPoint {
        AppPoint::new(v.x, self.height_pt - v.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the plan asks for on day one: the conversion is an
    /// involution, so a round trip returns the point it started from.
    ///
    /// Asserted at two strengths, because floating point gives two different
    /// answers and only saying the weaker one would hide the stronger:
    ///
    /// - **On the page, the round trip is exact.** `h - (h - y)` for
    ///   `0 <= y <= h` cancels without loss, measured at zero drift across
    ///   every page height here. This is the case that matters and it is
    ///   asserted as exact equality, so any future change that costs us
    ///   exactness fails loudly rather than quietly widening a tolerance.
    /// - **Off the page it is exact to 1.1e-13**, which is where a mark
    ///   dragged past an edge lives. That is four orders of magnitude finer
    ///   than `cad_kernel`'s own `EPS` of 1e-9 — below the resolution at which
    ///   the kernel considers two points distinct at all, so it cannot produce
    ///   a visible or a logical difference.
    #[test]
    fn round_trip_is_the_identity() {
        for height in [1.0_f64, 72.0, 792.0, 841.89, 14_400.0] {
            let space = PageSpace::new(height);
            let mut on_page = 0;

            for xi in -20..=20 {
                for yi in -20..=20 {
                    let start = AppPoint::new(xi as f64 * 37.5, yi as f64 * 41.25);
                    let back = space.from_kernel(space.to_kernel(start));

                    assert_eq!(back.x, start.x, "x must never be touched");

                    if (0.0..=height).contains(&start.y) {
                        assert_eq!(
                            back, start,
                            "a point on the page must round trip exactly \
                             (height {height})"
                        );
                        on_page += 1;
                    } else {
                        assert!(
                            (back.y - start.y).abs() < cad_kernel::EPS,
                            "off-page round trip drifted past the kernel's own \
                             tolerance at {start:?}, height {height}: got {back:?}"
                        );
                    }
                }
            }

            assert!(on_page > 0, "height {height} exercised no on-page points");
        }
    }

    /// The flip is real in both directions, not an accidental identity — a test
    /// that only asserted the round trip would pass if both were no-ops.
    #[test]
    fn the_flip_actually_flips() {
        let space = PageSpace::new(792.0);

        // Top-left in app space is the origin in kernel space.
        assert_eq!(space.to_kernel(AppPoint::new(0.0, 0.0)), Vec2::new(0.0, 792.0));
        assert_eq!(space.to_kernel(AppPoint::new(0.0, 792.0)), Vec2::new(0.0, 0.0));
        assert_eq!(space.from_kernel(Vec2::new(0.0, 0.0)), AppPoint::new(0.0, 792.0));

        // x is never touched. Only y disagrees between the two spaces.
        let p = AppPoint::new(123.5, 400.0);
        assert_eq!(space.to_kernel(p).x, p.x);
    }

    /// Ordering inverts: a mark *above* another in app space is *below* it in
    /// kernel space. This is the assertion that would have caught a silent
    /// second flip, because a doubly-flipped pipeline preserves order.
    #[test]
    fn vertical_order_inverts() {
        let space = PageSpace::new(792.0);
        let higher_on_screen = AppPoint::new(0.0, 100.0);
        let lower_on_screen = AppPoint::new(0.0, 700.0);

        assert!(higher_on_screen.y < lower_on_screen.y);
        assert!(space.to_kernel(higher_on_screen).y > space.to_kernel(lower_on_screen).y);
    }
}

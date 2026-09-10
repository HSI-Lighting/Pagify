//! Image pixels to page points, in one place.
//!
//! This is the same discipline the crate already applies to the top-left flip,
//! and for the same reason: a conversion done in two places is a conversion
//! done two different ways eventually, and the failure is silent — a text layer
//! that sits a few points off the ink it describes still searches correctly and
//! feels broken to select.
//!
//! Recognition happens on a *rendered* page, so there are two transformations
//! stacked: a scale (pixels per point), and whatever rotation the page was
//! rendered at. Both are undone here and nowhere else.

use crate::document::{Rect, Rotation};
use crate::ocr::LineBox;

/// How a page was rasterised, and therefore how to get back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageImage {
    pub page_width_pt: f32,
    pub page_height_pt: f32,
    /// Pixels per point. `dpi / 72.0` — so 300 dpi is 4.1667.
    pub scale: f32,
    pub rotation: Rotation,
}

impl PageImage {
    pub fn new(page_width_pt: f32, page_height_pt: f32, dpi: f32, rotation: Rotation) -> Self {
        PageImage { page_width_pt, page_height_pt, scale: dpi / 72.0, rotation }
    }

    /// The pixel dimensions this page renders to.
    pub fn image_size(&self) -> (u32, u32) {
        let (w, h) = if self.rotation.swaps_axes() {
            (self.page_height_pt, self.page_width_pt)
        } else {
            (self.page_width_pt, self.page_height_pt)
        };
        (
            (w * self.scale).round().max(1.0) as u32,
            (h * self.scale).round().max(1.0) as u32,
        )
    }

    /// Page points → image pixels.
    pub fn to_image(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale;
        match self.rotation {
            Rotation::None => (x * s, y * s),
            // Turning the page clockwise sends its top-left corner to the
            // image's top-right.
            Rotation::Clockwise90 => ((self.page_height_pt - y) * s, x * s),
            Rotation::Clockwise180 => {
                ((self.page_width_pt - x) * s, (self.page_height_pt - y) * s)
            }
            Rotation::Clockwise270 => (y * s, (self.page_width_pt - x) * s),
        }
    }

    /// Image pixels → page points.
    pub fn to_page(&self, x: f32, y: f32) -> (f32, f32) {
        let s = if self.scale.abs() < 1e-9 { 1.0 } else { self.scale };
        let (x, y) = (x / s, y / s);
        match self.rotation {
            Rotation::None => (x, y),
            Rotation::Clockwise90 => (y, self.page_height_pt - x),
            Rotation::Clockwise180 => (self.page_width_pt - x, self.page_height_pt - y),
            Rotation::Clockwise270 => (self.page_width_pt - y, x),
        }
    }

    /// A detected line, as a rectangle on the page.
    ///
    /// The quad's four corners are each converted and then bounded, rather than
    /// the bounding box being converted: under rotation those are different
    /// answers, and only the first one is right.
    pub fn line_to_page(&self, line: &LineBox) -> Rect {
        let corners: Vec<(f32, f32)> =
            line.quad.iter().map(|(x, y)| self.to_page(*x, *y)).collect();

        Rect {
            left: corners.iter().map(|c| c.0).fold(f32::MAX, f32::min),
            top: corners.iter().map(|c| c.1).fold(f32::MAX, f32::min),
            right: corners.iter().map(|c| c.0).fold(f32::MIN, f32::max),
            bottom: corners.iter().map(|c| c.1).fold(f32::MIN, f32::max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(rotation: Rotation) -> PageImage {
        PageImage::new(612.0, 792.0, 300.0, rotation)
    }

    /// The property test the plan asks for, across every rotation and a range
    /// of scales — because a conversion that is right at one dpi and wrong at
    /// another is the kind that ships.
    #[test]
    fn page_to_image_and_back_is_the_identity() {
        for rotation in [
            Rotation::None,
            Rotation::Clockwise90,
            Rotation::Clockwise180,
            Rotation::Clockwise270,
        ] {
            for dpi in [72.0f32, 150.0, 200.0, 300.0, 400.0] {
                let mapping = PageImage::new(612.0, 792.0, dpi, rotation);

                for px in [0.0f32, 1.0, 100.0, 305.5, 611.0, 612.0] {
                    for py in [0.0f32, 1.0, 100.0, 396.25, 791.0, 792.0] {
                        let (ix, iy) = mapping.to_image(px, py);
                        let (bx, by) = mapping.to_page(ix, iy);

                        assert!(
                            (bx - px).abs() < 0.01 && (by - py).abs() < 0.01,
                            "{rotation:?} at {dpi} dpi: ({px}, {py}) -> ({ix}, {iy}) -> ({bx}, {by})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_quarter_turn_swaps_the_image_dimensions() {
        let upright = page(Rotation::None).image_size();
        let turned = page(Rotation::Clockwise90).image_size();
        assert_eq!(turned, (upright.1, upright.0));
    }

    #[test]
    fn the_top_left_of_an_upright_page_is_the_top_left_of_the_image() {
        let (x, y) = page(Rotation::None).to_image(0.0, 0.0);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6);
    }

    #[test]
    fn turning_the_page_clockwise_sends_its_top_left_to_the_image_top_right() {
        let mapping = page(Rotation::Clockwise90);
        let (width, _) = mapping.image_size();
        let (x, y) = mapping.to_image(0.0, 0.0);

        assert!((x - width as f32).abs() < 1.0, "x was {x}, image is {width} wide");
        assert!(y.abs() < 1e-6, "y was {y}");
    }

    #[test]
    fn a_line_box_converts_by_its_corners_not_its_bounding_box() {
        // Under rotation those are different answers. A skewed quad whose
        // bounding box is converted lands in the wrong place, and the error
        // grows with the skew — which is exactly the photographed page the quad
        // exists for.
        let mapping = page(Rotation::Clockwise90);
        let skewed = LineBox {
            quad: [(100.0, 100.0), (400.0, 130.0), (400.0, 180.0), (100.0, 150.0)],
            confidence: 0.9,
        };

        let rect = mapping.line_to_page(&skewed);
        // Every corner must land inside the page.
        assert!(rect.left >= -1.0 && rect.right <= 612.0 + 1.0, "{rect:?}");
        assert!(rect.top >= -1.0 && rect.bottom <= 792.0 + 1.0, "{rect:?}");
        assert!(rect.right > rect.left && rect.bottom > rect.top, "degenerate: {rect:?}");
    }

    #[test]
    fn a_zero_scale_does_not_produce_infinities() {
        let broken = PageImage { page_width_pt: 612.0, page_height_pt: 792.0, scale: 0.0, rotation: Rotation::None };
        let (x, y) = broken.to_page(100.0, 100.0);
        assert!(x.is_finite() && y.is_finite());
    }
}

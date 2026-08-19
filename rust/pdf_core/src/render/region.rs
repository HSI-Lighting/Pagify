//! Resolving a crop rectangle in page points into a pixel region to render.
//!
//! ## Why this is not the on-screen render path
//!
//! The on-screen path is sized by its destination: Kotlin measures the view,
//! allocates a bitmap, and the bitmap's dimensions decide the render size (`zoom`
//! only picks the cache entry). That is right for the screen, where the
//! destination is the constraint.
//!
//! An export is the other way round. The crop rectangle and an explicit scale
//! decide the size, because the whole point of exporting is that the output
//! resolution is *independent of the screen* — a 4× capture of a region is
//! sharper than the display could ever show. Reusing the screen path would tie
//! the two together again and cap every export at screen resolution.
//!
//! ## Why this is not a screenshot
//!
//! Because the region is re-rendered from the document, nothing that is not in
//! the document can appear in the output: no notification, no dialog of ours, no
//! status bar. That is not a filter applied afterwards — those pixels never exist
//! in the first place. See roadmap decision 4.8.

use crate::document::{PageSize, Rect};
use crate::error::{PdfError, Result};
use crate::render::bitmap::{MAX_DIMENSION_PX, MAX_PIXELS};

/// A crop resolved against a page size and an export scale.
///
/// Everything here is in pixels and ready to hand to the renderer: the whole page
/// is drawn at `page_width` × `page_height` with its top-left corner placed at
/// `(-offset_x, -offset_y)` in a `width` × `height` bitmap, so only the crop lands
/// inside the bitmap and the rest falls off its edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionPixels {
    /// The whole page at the export scale — what the renderer is told to draw.
    pub page_width: u32,
    pub page_height: u32,
    /// Top-left of the crop within that page image.
    pub offset_x: i32,
    pub offset_y: i32,
    /// Size of the output bitmap.
    pub width: u32,
    pub height: u32,
    /// The scale actually used. Equal to the requested one unless the render
    /// ceiling forced it down — see [`clamp_scale`].
    pub scale: f32,
}

/// The largest page image we will ask the renderer to lay out.
///
/// Not an allocation — only the crop is allocated — but the page image's
/// dimensions still cross the FFI as `i32`, and a small crop of a large page at a
/// large scale can put a startling number there. This keeps the arithmetic
/// nowhere near the edge of the type.
const MAX_PAGE_IMAGE_PX: f32 = 1_000_000.0;

impl RegionPixels {
    /// Resolve a crop in page points (top-left origin, y down — decision 4.4).
    ///
    /// The crop is normalised and clipped to the page first, so a rectangle
    /// dragged right-to-left or overshooting the edge is a valid request rather
    /// than an error: the user's finger leaving the page is not a fault.
    pub fn resolve(page: PageSize, crop: Rect, scale: f32) -> Result<Self> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(PdfError::InvalidArgument(format!(
                "export scale must be a positive number, got {scale}"
            )));
        }
        if !(crop.left.is_finite()
            && crop.top.is_finite()
            && crop.right.is_finite()
            && crop.bottom.is_finite())
        {
            return Err(PdfError::InvalidArgument(
                "crop rectangle has a non-finite edge".to_string(),
            ));
        }

        let left = crop.left.min(crop.right).max(0.0);
        let right = crop.left.max(crop.right).min(page.width_pt);
        let top = crop.top.min(crop.bottom).max(0.0);
        let bottom = crop.top.max(crop.bottom).min(page.height_pt);

        if right - left <= 0.0 || bottom - top <= 0.0 {
            return Err(PdfError::InvalidArgument(format!(
                "crop {crop:?} does not overlap a {}x{} pt page",
                page.width_pt, page.height_pt
            )));
        }

        let scale = clamp_scale(page, right - left, bottom - top, scale);

        // Rounding the *edges* rather than the width is what keeps framing stable
        // across scales: 1× and 4× of the same crop then cover the same points,
        // instead of drifting by whatever the width happened to round to.
        let x0 = (left * scale).round();
        let x1 = (right * scale).round();
        let y0 = (top * scale).round();
        let y1 = (bottom * scale).round();

        Ok(RegionPixels {
            page_width: (page.width_pt * scale).round().max(1.0) as u32,
            page_height: (page.height_pt * scale).round().max(1.0) as u32,
            offset_x: x0 as i32,
            offset_y: y0 as i32,
            width: (x1 - x0).max(1.0) as u32,
            height: (y1 - y0).max(1.0) as u32,
            scale,
        })
    }

    pub fn byte_len(&self) -> usize {
        self.width as usize * self.height as usize * crate::render::BYTES_PER_PIXEL
    }
}

/// Reduce a scale until the region it produces fits the render ceiling.
///
/// Clamping rather than failing is deliberate. The alternative is an export that
/// refuses at 4× on a large page, which reads as a bug to anyone holding the
/// device: they asked for a picture of a region, and the region is small. Handing
/// back the sharpest image that fits is the answer that matches the request.
///
/// The result is never raised, only lowered, so a modest export is untouched.
fn clamp_scale(page: PageSize, crop_width_pt: f32, crop_height_pt: f32, requested: f32) -> f32 {
    let mut scale = requested;

    // Each side of the output bitmap.
    scale = scale.min(MAX_DIMENSION_PX as f32 / crop_width_pt);
    scale = scale.min(MAX_DIMENSION_PX as f32 / crop_height_pt);

    // Its area, which bites long before either side does on a square-ish crop.
    let area_limit = (MAX_PIXELS as f32 / (crop_width_pt * crop_height_pt)).sqrt();
    scale = scale.min(area_limit);

    // And the page image the renderer lays the crop out within.
    scale = scale.min(MAX_PAGE_IMAGE_PX / page.width_pt.max(1.0));
    scale = scale.min(MAX_PAGE_IMAGE_PX / page.height_pt.max(1.0));

    // A crop can be smaller than a point; never let the clamp reach zero.
    scale.max(f32::MIN_POSITIVE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a4() -> PageSize {
        PageSize {
            width_pt: 595.0,
            height_pt: 842.0,
        }
    }

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn a_crop_at_one_times_is_its_own_size_in_points() {
        let region = RegionPixels::resolve(a4(), rect(100.0, 200.0, 300.0, 500.0), 1.0).unwrap();
        assert_eq!((region.width, region.height), (200, 300));
        assert_eq!((region.offset_x, region.offset_y), (100, 200));
        assert_eq!((region.page_width, region.page_height), (595, 842));
    }

    #[test]
    fn scale_multiplies_the_output_but_not_the_framing() {
        let crop = rect(100.0, 200.0, 300.0, 500.0);
        let one = RegionPixels::resolve(a4(), crop, 1.0).unwrap();
        let four = RegionPixels::resolve(a4(), crop, 4.0).unwrap();

        assert_eq!((four.width, four.height), (one.width * 4, one.height * 4));
        // Same fraction of the page image, so the same content is framed.
        assert_eq!(
            four.offset_x as f32 / four.page_width as f32,
            one.offset_x as f32 / one.page_width as f32,
        );
        assert_eq!(
            four.offset_y as f32 / four.page_height as f32,
            one.offset_y as f32 / one.page_height as f32,
        );
    }

    #[test]
    fn a_rectangle_dragged_backwards_is_normalised_rather_than_rejected() {
        let dragged = RegionPixels::resolve(a4(), rect(300.0, 500.0, 100.0, 200.0), 1.0).unwrap();
        let forwards = RegionPixels::resolve(a4(), rect(100.0, 200.0, 300.0, 500.0), 1.0).unwrap();
        assert_eq!(dragged, forwards);
    }

    #[test]
    fn a_crop_overshooting_the_page_is_clipped_to_it() {
        let region = RegionPixels::resolve(a4(), rect(-50.0, -50.0, 900.0, 900.0), 1.0).unwrap();
        assert_eq!((region.offset_x, region.offset_y), (0, 0));
        assert_eq!((region.width, region.height), (595, 842));
    }

    #[test]
    fn a_crop_entirely_off_the_page_is_an_error_not_an_empty_bitmap() {
        let err = RegionPixels::resolve(a4(), rect(700.0, 900.0, 800.0, 1000.0), 1.0);
        assert!(matches!(err, Err(PdfError::InvalidArgument(_))));
    }

    #[test]
    fn a_zero_width_crop_is_an_error() {
        assert!(matches!(
            RegionPixels::resolve(a4(), rect(100.0, 200.0, 100.0, 500.0), 1.0),
            Err(PdfError::InvalidArgument(_))
        ));
    }

    #[test]
    fn a_scale_that_is_not_a_positive_number_is_rejected() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(
                matches!(
                    RegionPixels::resolve(a4(), rect(0.0, 0.0, 10.0, 10.0), bad),
                    Err(PdfError::InvalidArgument(_))
                ),
                "scale {bad} should have been rejected"
            );
        }
    }

    #[test]
    fn a_non_finite_crop_edge_is_rejected() {
        assert!(matches!(
            RegionPixels::resolve(a4(), rect(0.0, 0.0, f32::NAN, 10.0), 1.0),
            Err(PdfError::InvalidArgument(_))
        ));
    }

    #[test]
    fn an_enormous_scale_is_clamped_instead_of_allocating_half_a_gigabyte() {
        // A whole A4 page at 100× would be 59500 x 84200 px — about 20 GB.
        let region = RegionPixels::resolve(a4(), rect(0.0, 0.0, 595.0, 842.0), 100.0).unwrap();

        assert!(region.scale < 100.0, "the scale should have been reduced");
        assert!(region.width <= MAX_DIMENSION_PX && region.height <= MAX_DIMENSION_PX);
        assert!(
            u64::from(region.width) * u64::from(region.height) <= MAX_PIXELS,
            "{}x{} exceeds the area ceiling",
            region.width,
            region.height,
        );
        // And what it does produce is still a real image, not a token one.
        assert!(region.width > 1000);
    }

    #[test]
    fn a_modest_export_is_left_exactly_as_asked() {
        let region = RegionPixels::resolve(a4(), rect(10.0, 10.0, 210.0, 110.0), 4.0).unwrap();
        assert_eq!(region.scale, 4.0);
        assert_eq!((region.width, region.height), (800, 400));
    }

    #[test]
    fn a_tiny_crop_of_a_huge_page_still_bounds_the_page_image() {
        let poster = PageSize {
            width_pt: 20_000.0,
            height_pt: 20_000.0,
        };
        let region = RegionPixels::resolve(poster, rect(0.0, 0.0, 4.0, 4.0), 10_000.0).unwrap();
        assert!(region.page_width <= MAX_PAGE_IMAGE_PX as u32);
        assert!(region.page_height <= MAX_PAGE_IMAGE_PX as u32);
    }

    #[test]
    fn a_sub_point_crop_still_produces_at_least_one_pixel() {
        let region = RegionPixels::resolve(a4(), rect(10.0, 10.0, 10.2, 10.2), 1.0).unwrap();
        assert_eq!((region.width, region.height), (1, 1));
    }
}

//! A capture that spans whatever is on screen, rather than one page.
//!
//! The reader lays pages out in a column with gaps between them, so a box dragged
//! around something interesting very often crosses a join: the bottom of one page
//! and the top of the next, or a spread read across two. A capture that stopped at
//! the page it started on would answer a question nobody asked.
//!
//! ## Who owns the layout
//!
//! The caller does. It reports each page's share as a [`Tile`] — which part of
//! that page, and where that part belongs in the picture — because only the caller
//! knows where a page sits on a screen. Deriving it here would mean a second copy
//! of the reader's layout arithmetic, kept in step by hope.
//!
//! ## Units
//!
//! `crop` is in **page points**, in that page's own space. `dest` is in **capture
//! units**, with the origin at the picture's top-left; multiplying by `scale`
//! gives pixels. Markup is in capture units too, for the reason above: a mark
//! drawn across a join belongs to neither page.

use serde::{Deserialize, Serialize};

use crate::document::{Color, Rect};
use crate::error::{PdfError, Result};
use crate::render::bitmap::{MAX_DIMENSION_PX, MAX_PIXELS};

/// One page's contribution to a capture.
///
/// `rename_all` on a struct renames its *fields*, which is what is wanted here —
/// unlike on an enum, where it renames the variants and the fields inside them
/// need `rename_all_fields` as well. Getting that distinction wrong once already
/// sent `quarter_turns` to a decoder expecting `quarterTurns`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub page_index: usize,
    /// The part of the page to draw, in that page's points.
    pub crop: Rect,
    /// Where it belongs in the picture, in capture units.
    pub dest: Rect,
}

/// What to capture, and how sharply.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportRequest {
    pub tiles: Vec<Tile>,
    /// Size of the picture in capture units.
    pub width: f32,
    pub height: f32,
    /// Capture units to pixels. Lowered if it would breach the render ceiling.
    pub scale: f32,
    /// Shows wherever no page reaches — between pages, and past their edges.
    pub background: Color,
    pub render_annotations: bool,
    pub render_form_data: bool,
}

/// A tile resolved into pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedTile {
    pub left: i32,
    pub top: i32,
    /// Points-to-pixels for this page, derived from how big its share came out.
    ///
    /// Per tile rather than shared: pages in one document are not all the same
    /// size, and the reader draws them all to the same width, so each page is at
    /// its own scale. Deriving it from the destination is what keeps a tile the
    /// size the layout said, whatever the page's own dimensions are.
    pub scale: f32,
}

/// The picture's pixel size, and the transform into it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportPlan {
    pub width: u32,
    pub height: u32,
    /// The scale actually used — the requested one unless the ceiling lowered it.
    pub scale: f32,
}

impl ViewportPlan {
    pub fn resolve(request: &ViewportRequest) -> Result<Self> {
        if !request.scale.is_finite() || request.scale <= 0.0 {
            return Err(PdfError::InvalidArgument(format!(
                "export scale must be a positive number, got {}",
                request.scale
            )));
        }
        if !request.width.is_finite()
            || !request.height.is_finite()
            || request.width <= 0.0
            || request.height <= 0.0
        {
            return Err(PdfError::InvalidArgument(format!(
                "a capture must have a positive size, got {}x{}",
                request.width, request.height
            )));
        }

        let scale = clamp_scale(request.width, request.height, request.scale);

        Ok(ViewportPlan {
            width: (request.width * scale).round().max(1.0) as u32,
            height: (request.height * scale).round().max(1.0) as u32,
            scale,
        })
    }

    /// Where a tile lands, or `None` if none of it is inside the picture.
    pub fn place(&self, tile: &Tile) -> Option<PlacedTile> {
        let left = (tile.dest.left * self.scale).round();
        let top = (tile.dest.top * self.scale).round();
        let right = (tile.dest.right * self.scale).round();
        let bottom = (tile.dest.bottom * self.scale).round();

        let width = right - left;
        let height = bottom - top;
        if width < 1.0 || height < 1.0 {
            return None;
        }
        // Wholly off one side. Callers list the pages that *might* contribute and
        // let this decide, so this is an ordinary outcome rather than a mistake.
        if right <= 0.0 || bottom <= 0.0 || left >= self.width as f32 || top >= self.height as f32 {
            return None;
        }

        let crop_width = tile.crop.right - tile.crop.left;
        if crop_width <= 0.0 || !crop_width.is_finite() {
            return None;
        }

        Some(PlacedTile {
            left: left as i32,
            top: top as i32,
            // From the destination, not from the requested scale: the tile has to
            // come out the size the layout said, and its page's points-per-unit is
            // whatever makes that true.
            scale: width / crop_width,
        })
    }
}

/// Reduce a scale until the picture fits the render ceiling.
///
/// Clamping rather than refusing, for the same reason as a single-page capture:
/// someone asking for a sharper picture of what is on screen should get the
/// sharpest one that fits, not an error.
fn clamp_scale(width: f32, height: f32, requested: f32) -> f32 {
    let mut scale = requested;
    scale = scale.min(MAX_DIMENSION_PX as f32 / width);
    scale = scale.min(MAX_DIMENSION_PX as f32 / height);
    scale = scale.min((MAX_PIXELS as f32 / (width * height)).sqrt());
    scale.max(f32::MIN_POSITIVE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    fn request(tiles: Vec<Tile>) -> ViewportRequest {
        ViewportRequest {
            tiles,
            width: 400.0,
            height: 300.0,
            scale: 2.0,
            background: Color {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            render_annotations: true,
            render_form_data: true,
        }
    }

    fn tile(page_index: usize, crop: Rect, dest: Rect) -> Tile {
        Tile {
            page_index,
            crop,
            dest,
        }
    }

    #[test]
    fn the_picture_is_the_requested_size_in_pixels() {
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();
        assert_eq!((plan.width, plan.height), (800, 600));
        assert_eq!(plan.scale, 2.0);
    }

    #[test]
    fn a_tile_lands_where_the_layout_put_it() {
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();
        let placed = plan
            .place(&tile(
                0,
                rect(0.0, 0.0, 100.0, 100.0),
                rect(50.0, 20.0, 150.0, 120.0),
            ))
            .expect("inside");

        assert_eq!((placed.left, placed.top), (100, 40));
        // 100 units wide at 2x is 200 px, from a 100 pt crop: 2 px per point.
        assert_eq!(placed.scale, 2.0);
    }

    #[test]
    fn each_tile_gets_its_own_scale_because_pages_are_not_all_one_size() {
        // The reader draws every page to the same width, so an A3 page and an A5
        // page on screen are at very different points-per-pixel. A shared scale
        // would stretch one of them.
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();

        let wide = plan
            .place(&tile(
                0,
                rect(0.0, 0.0, 842.0, 100.0),
                rect(0.0, 0.0, 400.0, 50.0),
            ))
            .expect("inside");
        let narrow = plan
            .place(&tile(
                1,
                rect(0.0, 0.0, 420.0, 100.0),
                rect(0.0, 50.0, 400.0, 100.0),
            ))
            .expect("inside");

        assert!(
            narrow.scale > wide.scale * 1.9,
            "{} vs {}",
            narrow.scale,
            wide.scale
        );
    }

    #[test]
    fn a_tile_that_starts_above_the_picture_keeps_its_negative_offset() {
        // The common case: the top page is scrolled halfway off. Its tile has to
        // be placed with a negative top and clipped when it is drawn, not moved.
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();
        let placed = plan
            .place(&tile(
                0,
                rect(0.0, 0.0, 100.0, 100.0),
                rect(0.0, -60.0, 100.0, 40.0),
            ))
            .expect("partly inside");

        assert_eq!(placed.top, -120);
    }

    #[test]
    fn a_tile_entirely_outside_the_picture_is_dropped() {
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();
        assert!(plan
            .place(&tile(
                0,
                rect(0.0, 0.0, 10.0, 10.0),
                rect(0.0, 400.0, 100.0, 500.0)
            ))
            .is_none());
        assert!(plan
            .place(&tile(
                0,
                rect(0.0, 0.0, 10.0, 10.0),
                rect(-200.0, 0.0, -100.0, 50.0)
            ))
            .is_none());
    }

    #[test]
    fn a_tile_thinner_than_a_pixel_is_dropped_rather_than_rendered_at_zero() {
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();
        assert!(plan
            .place(&tile(
                0,
                rect(0.0, 0.0, 10.0, 10.0),
                rect(0.0, 0.0, 0.1, 50.0)
            ))
            .is_none());
    }

    #[test]
    fn a_tile_with_an_empty_crop_is_dropped() {
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();
        assert!(plan
            .place(&tile(
                0,
                rect(50.0, 0.0, 50.0, 10.0),
                rect(0.0, 0.0, 100.0, 50.0)
            ))
            .is_none());
    }

    #[test]
    fn an_enormous_scale_is_clamped_instead_of_allocating_gigabytes() {
        let mut wanted = request(vec![]);
        wanted.scale = 200.0;

        let plan = ViewportPlan::resolve(&wanted).unwrap();
        assert!(plan.scale < 200.0);
        assert!(plan.width <= MAX_DIMENSION_PX && plan.height <= MAX_DIMENSION_PX);
        assert!(u64::from(plan.width) * u64::from(plan.height) <= MAX_PIXELS);
        assert!(plan.width > 1000, "clamped so far it is useless");
    }

    #[test]
    fn a_capture_with_no_size_is_rejected() {
        for (width, height) in [(0.0, 300.0), (400.0, 0.0), (f32::NAN, 300.0)] {
            let mut wanted = request(vec![]);
            wanted.width = width;
            wanted.height = height;
            assert!(
                matches!(
                    ViewportPlan::resolve(&wanted),
                    Err(PdfError::InvalidArgument(_))
                ),
                "{width}x{height} should have been rejected",
            );
        }
    }

    #[test]
    fn a_scale_that_is_not_a_positive_number_is_rejected() {
        for bad in [0.0, -2.0, f32::INFINITY] {
            let mut wanted = request(vec![]);
            wanted.scale = bad;
            assert!(matches!(
                ViewportPlan::resolve(&wanted),
                Err(PdfError::InvalidArgument(_))
            ));
        }
    }

    #[test]
    fn the_wire_form_is_what_the_app_sends() {
        // Pinned as a literal, because both sides have to agree and a shared
        // builder would let them agree on the wrong thing.
        let json = r#"{"pageIndex":3,"crop":{"left":0.0,"top":10.0,"right":595.0,"bottom":400.0},"dest":{"left":0.0,"top":-20.0,"right":300.0,"bottom":170.0}}"#;
        let decoded: Tile = serde_json::from_str(json).expect("decode");

        assert_eq!(
            decoded,
            tile(
                3,
                rect(0.0, 10.0, 595.0, 400.0),
                rect(0.0, -20.0, 300.0, 170.0),
            ),
        );
    }

    #[test]
    fn two_tiles_meeting_at_a_join_leave_no_gap_between_them() {
        // A capture across a page boundary is the whole point of this module. If
        // rounding put a one-pixel line of background between the two halves it
        // would be visible on every such capture.
        let plan = ViewportPlan::resolve(&request(vec![])).unwrap();

        let upper = plan
            .place(&tile(
                0,
                rect(0.0, 700.0, 595.0, 842.0),
                rect(0.0, 0.0, 400.0, 95.5),
            ))
            .expect("upper");
        let lower = plan
            .place(&tile(
                1,
                rect(0.0, 0.0, 595.0, 142.0),
                rect(0.0, 95.5, 400.0, 191.0),
            ))
            .expect("lower");

        let upper_bottom = upper.top + (95.5f32 * plan.scale).round() as i32;
        assert_eq!(
            upper_bottom, lower.top,
            "the tiles do not meet: {upper_bottom} vs {}",
            lower.top,
        );
    }
}

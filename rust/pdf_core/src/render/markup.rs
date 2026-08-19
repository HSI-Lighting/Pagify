//! Drawing on a capture.
//!
//! ## Where the shapes live, and why here
//!
//! The wet stroke — the line that follows a finger, redrawn every frame — stays
//! in the UI and never touches this crate. What arrives here is the *committed*
//! shape, which is the split decision 4.7 settles: ephemeral interaction state on
//! the platform side, committed state in the core. A drag emits an event per
//! frame and cannot queue behind a render lock; a commit happens once and can.
//!
//! ## Coordinates
//!
//! Shapes are in **page points, top-left origin** — the same space as annotations
//! and as the capture's crop (decision 4.4), never in the capture's pixels.
//! Pixel-space shapes would be wrong the moment the export scale changed, and the
//! sheet lets exactly that happen: re-rendering the same capture at 4× has to put
//! the markup in the same place on the page, not a quarter of the way into it.

use serde::{Deserialize, Serialize};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Stroke as SkStroke, Transform,
};

use crate::document::{Color, Point, Rect};
use crate::error::{PdfError, Result};
use crate::render::bitmap::{Bitmap, PixelOrder};
use crate::render::region::RegionPixels;

/// One committed mark on a capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Markup {
    pub shape: Shape,
    pub color: Color,
    /// Stroke width in page points, so a mark keeps its weight relative to the
    /// page whatever resolution the capture is exported at.
    pub width_pt: f32,
}

/// What was drawn.
///
/// `rename_all_fields` as well as `rename_all`: the first renames the variants,
/// the second the fields inside them. Missing the second is what once made the
/// app send `quarter_turns` against a decoder expecting `quarterTurns`, and only
/// on the variants that happened to have a two-word field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Shape {
    /// The stroke as drawn, unrecognised.
    Freehand { points: Vec<Point> },
    Line { from: Point, to: Point },
    /// A line with a head at `to`.
    Arrow { from: Point, to: Point },
    Rect { rect: Rect },
    Ellipse { rect: Rect },
    /// A translucent wash, for picking something out rather than ringing it.
    Highlight { rect: Rect },
}

/// Alpha a highlight is drawn at, whatever alpha its colour carries.
///
/// Fixed rather than taken from the colour so a highlight cannot be committed
/// opaque, which would black out the very thing it was drawn to point at.
const HIGHLIGHT_ALPHA: f32 = 0.35;

/// How long an arrow's head is, as a multiple of its stroke width.
const ARROW_HEAD_LENGTHS: f32 = 4.0;

/// Half-angle of the arrow head, in radians (about 25°).
const ARROW_HEAD_ANGLE: f32 = 0.44;

/// Draw `marks` into a captured bitmap.
///
/// The bitmap must be the one [`RegionPixels`] describes: the transform from page
/// points to pixels comes from `region`, so a mismatched pair puts every mark in
/// the wrong place rather than failing.
///
/// A pure function of `(bitmap, marks, region)` — same inputs, byte-identical
/// output — which is what makes a capture reproducible.
pub fn composite(bitmap: &mut Bitmap, marks: &[Markup], region: &RegionPixels) -> Result<()> {
    if marks.is_empty() {
        return Ok(());
    }
    if bitmap.order != PixelOrder::Rgba {
        return Err(PdfError::InvalidBitmap(format!(
            "markup needs an RGBA bitmap, got {:?}",
            bitmap.order
        )));
    }
    if bitmap.stride != bitmap.width as usize * 4 {
        return Err(PdfError::InvalidBitmap(
            "markup needs a tightly packed bitmap".to_string(),
        ));
    }

    let mut pixmap = PixmapMut::from_bytes(&mut bitmap.data, bitmap.width, bitmap.height)
        .ok_or_else(|| PdfError::InvalidBitmap("could not borrow the capture".to_string()))?;

    for mark in marks {
        draw(&mut pixmap, mark, region);
    }

    Ok(())
}

/// Page points to capture pixels.
///
/// The capture is the crop, so a page point has the crop's own top-left
/// subtracted before it is scaled — otherwise every mark lands offset by however
/// far into the page the crop began.
fn to_pixels(p: Point, region: &RegionPixels) -> (f32, f32) {
    (
        p.x * region.scale - region.offset_x as f32,
        p.y * region.scale - region.offset_y as f32,
    )
}

fn draw(pixmap: &mut PixmapMut, mark: &Markup, region: &RegionPixels) {
    let width_px = (mark.width_pt * region.scale).max(1.0);

    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.set_color_rgba8(mark.color.r, mark.color.g, mark.color.b, mark.color.a);

    let stroke = SkStroke {
        width: width_px,
        // Round on both, so a stroke reads as ink rather than as a ruled line and
        // a corner does not spike out past the shape it belongs to.
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..SkStroke::default()
    };

    let path = match &mark.shape {
        Shape::Highlight { rect } => {
            // Filled, and at a fixed alpha: a highlight that covers what it marks
            // has failed at the one thing it is for.
            paint.set_color_rgba8(
                mark.color.r,
                mark.color.g,
                mark.color.b,
                (255.0 * HIGHLIGHT_ALPHA) as u8,
            );
            let (left, top) = to_pixels(
                Point {
                    x: rect.left,
                    y: rect.top,
                },
                region,
            );
            let (right, bottom) = to_pixels(
                Point {
                    x: rect.right,
                    y: rect.bottom,
                },
                region,
            );
            let mut builder = PathBuilder::new();
            builder.push_rect(
                tiny_skia::Rect::from_ltrb(left, top, right, bottom).unwrap_or(
                    tiny_skia::Rect::from_xywh(left, top, 1.0, 1.0).expect("unit rect"),
                ),
            );
            match builder.finish() {
                Some(path) => {
                    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                    return;
                }
                None => return,
            }
        }
        Shape::Freehand { points } => freehand_path(points, region),
        Shape::Line { from, to } => segment_path(*from, *to, region),
        Shape::Arrow { from, to } => arrow_path(*from, *to, width_px, region),
        Shape::Rect { rect } => rect_path(*rect, region),
        Shape::Ellipse { rect } => ellipse_path(*rect, region),
    };

    if let Some(path) = path {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn freehand_path(points: &[Point], region: &RegionPixels) -> Option<tiny_skia::Path> {
    if points.len() < 2 {
        return None;
    }
    let mut builder = PathBuilder::new();
    let (x, y) = to_pixels(points[0], region);
    builder.move_to(x, y);

    // Quadratics through the midpoints, so a fast drag reads as a smooth line
    // rather than as the visible polygon that joining raw touch samples gives.
    for pair in points.windows(2) {
        let (px, py) = to_pixels(pair[0], region);
        let (cx, cy) = to_pixels(pair[1], region);
        builder.quad_to(px, py, (px + cx) / 2.0, (py + cy) / 2.0);
    }
    let (x, y) = to_pixels(*points.last().expect("checked non-empty"), region);
    builder.line_to(x, y);

    builder.finish()
}

fn segment_path(from: Point, to: Point, region: &RegionPixels) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let (x0, y0) = to_pixels(from, region);
    let (x1, y1) = to_pixels(to, region);
    builder.move_to(x0, y0);
    builder.line_to(x1, y1);
    builder.finish()
}

fn arrow_path(
    from: Point,
    to: Point,
    width_px: f32,
    region: &RegionPixels,
) -> Option<tiny_skia::Path> {
    let (x0, y0) = to_pixels(from, region);
    let (x1, y1) = to_pixels(to, region);

    let angle = (y1 - y0).atan2(x1 - x0);
    let head = width_px * ARROW_HEAD_LENGTHS;

    let mut builder = PathBuilder::new();
    builder.move_to(x0, y0);
    builder.line_to(x1, y1);
    // Two barbs back from the tip, drawn as separate strokes from the same point
    // so the join stays at the tip where the eye expects it.
    for side in [-1.0f32, 1.0] {
        let barb = angle + std::f32::consts::PI + side * ARROW_HEAD_ANGLE;
        builder.move_to(x1, y1);
        builder.line_to(x1 + head * barb.cos(), y1 + head * barb.sin());
    }
    builder.finish()
}

fn rect_path(rect: Rect, region: &RegionPixels) -> Option<tiny_skia::Path> {
    let (left, top) = to_pixels(
        Point {
            x: rect.left,
            y: rect.top,
        },
        region,
    );
    let (right, bottom) = to_pixels(
        Point {
            x: rect.right,
            y: rect.bottom,
        },
        region,
    );
    let mut builder = PathBuilder::new();
    builder.move_to(left, top);
    builder.line_to(right, top);
    builder.line_to(right, bottom);
    builder.line_to(left, bottom);
    builder.close();
    builder.finish()
}

fn ellipse_path(rect: Rect, region: &RegionPixels) -> Option<tiny_skia::Path> {
    let (left, top) = to_pixels(
        Point {
            x: rect.left,
            y: rect.top,
        },
        region,
    );
    let (right, bottom) = to_pixels(
        Point {
            x: rect.right,
            y: rect.bottom,
        },
        region,
    );
    let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
    let (rx, ry) = ((right - left) / 2.0, (bottom - top) / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }

    // Four cubics, with the standard circle constant. An ellipse from arcs would
    // need a transform, and a transform on the path means the stroke is scaled
    // with it — which turns a circle drawn wide into one with varying thickness.
    const K: f32 = 0.552_284_75;
    let mut builder = PathBuilder::new();
    builder.move_to(cx, cy - ry);
    builder.cubic_to(cx + rx * K, cy - ry, cx + rx, cy - ry * K, cx + rx, cy);
    builder.cubic_to(cx + rx, cy + ry * K, cx + rx * K, cy + ry, cx, cy + ry);
    builder.cubic_to(cx - rx * K, cy + ry, cx - rx, cy + ry * K, cx - rx, cy);
    builder.cubic_to(cx - rx, cy - ry * K, cx - rx * K, cy - ry, cx, cy - ry);
    builder.close();
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::PageSize;

    /// A 100 x 100 pt crop of a 200 x 200 pt page, starting at (50, 50), at 2×.
    ///
    /// Offset on both axes deliberately: a transform that forgets to subtract the
    /// crop's origin still looks right when the crop starts at zero.
    fn region() -> RegionPixels {
        RegionPixels::resolve(
            PageSize {
                width_pt: 200.0,
                height_pt: 200.0,
            },
            Rect {
                left: 50.0,
                top: 50.0,
                right: 150.0,
                bottom: 150.0,
            },
            2.0,
        )
        .expect("region")
    }

    fn white_capture() -> Bitmap {
        let region = region();
        let mut bitmap = Bitmap::new(region.width, region.height, PixelOrder::Rgba).unwrap();
        bitmap.data.fill(0xFF);
        bitmap
    }

    fn pixel(bitmap: &Bitmap, x: u32, y: u32) -> (u8, u8, u8) {
        let at = y as usize * bitmap.stride + x as usize * 4;
        (bitmap.data[at], bitmap.data[at + 1], bitmap.data[at + 2])
    }

    fn is_white(p: (u8, u8, u8)) -> bool {
        p.0 > 250 && p.1 > 250 && p.2 > 250
    }

    fn red() -> Color {
        Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        }
    }

    fn mark(shape: Shape) -> Markup {
        Markup {
            shape,
            color: red(),
            width_pt: 4.0,
        }
    }

    #[test]
    fn a_line_is_drawn_where_the_page_points_say_and_nowhere_else() {
        let mut bitmap = white_capture();
        // Across the middle of the crop in page space: y = 100 pt, which is
        // (100 - 50) * 2 = 100 px down the capture.
        composite(
            &mut bitmap,
            &[mark(Shape::Line {
                from: Point { x: 60.0, y: 100.0 },
                to: Point { x: 140.0, y: 100.0 },
            })],
            &region(),
        )
        .unwrap();

        assert!(!is_white(pixel(&bitmap, 100, 100)), "the line is not on it");
        assert!(is_white(pixel(&bitmap, 100, 60)), "ink well above the line");
        assert!(is_white(pixel(&bitmap, 100, 140)), "ink well below the line");
    }

    #[test]
    fn the_crops_origin_is_subtracted_rather_than_ignored() {
        // The trap: a transform that scales but forgets the crop's own top-left
        // draws this at (120, 120) instead of (20, 20), and every test whose crop
        // starts at the page corner still passes.
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Line {
                from: Point { x: 55.0, y: 60.0 },
                to: Point { x: 65.0, y: 60.0 },
            })],
            &region(),
        )
        .unwrap();

        assert!(!is_white(pixel(&bitmap, 20, 20)), "not where the crop puts it");
        assert!(is_white(pixel(&bitmap, 120, 120)), "drawn as if uncropped");
    }

    #[test]
    fn a_rectangle_is_outlined_and_not_filled() {
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Rect {
                rect: Rect {
                    left: 70.0,
                    top: 70.0,
                    right: 130.0,
                    bottom: 130.0,
                },
            })],
            &region(),
        )
        .unwrap();

        // (70 - 50) * 2 = 40 px; the middle is (100, 100).
        assert!(!is_white(pixel(&bitmap, 40, 100)), "left edge missing");
        assert!(!is_white(pixel(&bitmap, 160, 100)), "right edge missing");
        assert!(is_white(pixel(&bitmap, 100, 100)), "the middle was filled in");
    }

    #[test]
    fn an_ellipse_touches_its_bounds_at_the_midpoints_and_misses_the_corners() {
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Ellipse {
                rect: Rect {
                    left: 70.0,
                    top: 70.0,
                    right: 130.0,
                    bottom: 130.0,
                },
            })],
            &region(),
        )
        .unwrap();

        assert!(!is_white(pixel(&bitmap, 100, 40)), "top of the ellipse");
        assert!(!is_white(pixel(&bitmap, 40, 100)), "left of the ellipse");
        // A rectangle drawn instead of an ellipse would put ink here.
        assert!(is_white(pixel(&bitmap, 42, 42)), "the corner should be clear");
        assert!(is_white(pixel(&bitmap, 100, 100)), "not filled");
    }

    #[test]
    fn a_highlight_lets_the_page_through() {
        let mut bitmap = white_capture();
        // Something to see through it: a black band under where the wash goes.
        for y in 90..110 {
            for x in 40..160 {
                let at = y * bitmap.stride + x * 4;
                bitmap.data[at..at + 3].copy_from_slice(&[0, 0, 0]);
            }
        }

        composite(
            &mut bitmap,
            &[Markup {
                shape: Shape::Highlight {
                    rect: Rect {
                        left: 70.0,
                        top: 95.0,
                        right: 130.0,
                        bottom: 105.0,
                    },
                },
                color: Color {
                    r: 255,
                    g: 224,
                    b: 102,
                    a: 255,
                },
                width_pt: 0.0,
            }],
            &region(),
        )
        .unwrap();

        let over_text = pixel(&bitmap, 100, 100);
        assert!(
            over_text.0 < 200,
            "the black band was covered rather than washed: {over_text:?}",
        );
        assert!(over_text.0 > 0, "the wash left no colour at all: {over_text:?}");
    }

    #[test]
    fn an_arrow_puts_ink_at_its_tip() {
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Arrow {
                from: Point { x: 60.0, y: 60.0 },
                to: Point { x: 140.0, y: 140.0 },
            })],
            &region(),
        )
        .unwrap();

        // The barbs sit back from the tip along the shaft, so a plain line and an
        // arrow differ exactly here: just off the shaft, near the end.
        assert!(!is_white(pixel(&bitmap, 180, 180)), "the tip is missing");
        let barb_side = (165..180).any(|x| !is_white(pixel(&bitmap, x, 150)));
        assert!(barb_side, "no head — this drew a plain line");
    }

    #[test]
    fn a_freehand_stroke_of_one_point_draws_nothing_rather_than_panicking() {
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Freehand {
                points: vec![Point { x: 100.0, y: 100.0 }],
            })],
            &region(),
        )
        .unwrap();
        assert!(bitmap.data.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn compositing_is_deterministic() {
        // A capture that composites differently run to run cannot be compared with
        // anything, including a later export of the same markup.
        let marks = vec![
            mark(Shape::Ellipse {
                rect: Rect {
                    left: 70.0,
                    top: 70.0,
                    right: 130.0,
                    bottom: 120.0,
                },
            }),
            mark(Shape::Freehand {
                points: vec![
                    Point { x: 60.0, y: 60.0 },
                    Point { x: 90.0, y: 110.0 },
                    Point { x: 130.0, y: 70.0 },
                ],
            }),
        ];

        let mut once = white_capture();
        let mut twice = white_capture();
        composite(&mut once, &marks, &region()).unwrap();
        composite(&mut twice, &marks, &region()).unwrap();
        assert_eq!(once.data, twice.data);
    }

    #[test]
    fn marks_are_drawn_in_order_so_the_last_one_is_on_top() {
        let blue = Color {
            r: 0,
            g: 0,
            b: 255,
            a: 255,
        };
        let across = |color: Color| Markup {
            shape: Shape::Line {
                from: Point { x: 60.0, y: 100.0 },
                to: Point { x: 140.0, y: 100.0 },
            },
            color,
            width_pt: 6.0,
        };

        let mut bitmap = white_capture();
        composite(&mut bitmap, &[across(red()), across(blue)], &region()).unwrap();

        let (r, _, b) = pixel(&bitmap, 100, 100);
        assert!(b > r, "the second stroke should be the visible one");
    }

    #[test]
    fn nothing_at_all_is_drawn_for_an_empty_list() {
        let mut bitmap = white_capture();
        composite(&mut bitmap, &[], &region()).unwrap();
        assert!(bitmap.data.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn a_stroke_width_is_in_page_points_so_it_scales_with_the_export() {
        let thin = {
            let mut bitmap = white_capture();
            composite(
                &mut bitmap,
                &[Markup {
                    shape: Shape::Line {
                        from: Point { x: 60.0, y: 100.0 },
                        to: Point { x: 140.0, y: 100.0 },
                    },
                    color: red(),
                    width_pt: 2.0,
                }],
                &region(),
            )
            .unwrap();
            (0..bitmap.height).filter(|&y| !is_white(pixel(&bitmap, 100, y))).count()
        };

        let thick = {
            let mut bitmap = white_capture();
            composite(&mut bitmap, &[mark(Shape::Line {
                from: Point { x: 60.0, y: 100.0 },
                to: Point { x: 140.0, y: 100.0 },
            })], &region())
            .unwrap();
            (0..bitmap.height).filter(|&y| !is_white(pixel(&bitmap, 100, y))).count()
        };

        // 4 pt at 2x is 8 px; 2 pt is 4 px. The exact counts depend on
        // anti-aliasing, the ordering does not.
        assert!(thick > thin, "thick={thick} thin={thin}");
    }

    #[test]
    fn the_wire_form_is_what_the_app_sends() {
        // Pinned as a literal, because both sides have to agree and a shared
        // helper would let them agree on the wrong thing.
        let json = r#"{"shape":{"kind":"arrow","from":{"x":1.0,"y":2.0},"to":{"x":3.0,"y":4.0}},"color":{"r":255,"g":0,"b":0,"a":255},"widthPt":2.5}"#;
        let decoded: Markup = serde_json::from_str(json).expect("decode");

        assert_eq!(
            decoded,
            Markup {
                shape: Shape::Arrow {
                    from: Point { x: 1.0, y: 2.0 },
                    to: Point { x: 3.0, y: 4.0 },
                },
                color: red(),
                width_pt: 2.5,
            },
        );
    }
}

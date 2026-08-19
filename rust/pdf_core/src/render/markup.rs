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
//! Shapes are in **capture-local units, top-left origin** — the picture's own
//! space, not any page's. A capture can span two pages and a mark drawn across
//! the join belongs to neither. Nor are they in pixels: the editor lets the export
//! scale change after a mark is drawn, and a pixel-space shape would land
//! somewhere else entirely the moment it did.

use serde::{Deserialize, Serialize};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Stroke as SkStroke, Transform,
};

use crate::document::{Color, Point, Rect};
use crate::error::{PdfError, Result};
use crate::render::bitmap::{Bitmap, PixelOrder};


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

/// The most opaque a highlight may be drawn, whatever alpha it carries.
///
/// The intensity is the user's to set and arrives in the colour's alpha — but not
/// all the way to opaque. A highlight exists to pick something out, so one that
/// covers what it marks has failed at its only job, and a slider that can reach
/// that state offers a setting nobody wants once they see it.
const HIGHLIGHT_ALPHA_CEILING: u8 = 216;

/// How long an arrow's head is, as a multiple of its stroke width.
const ARROW_HEAD_LENGTHS: f32 = 4.0;

/// Half-angle of the arrow head, in radians (about 25°).
const ARROW_HEAD_ANGLE: f32 = 0.44;

/// Draw `marks` into a captured bitmap.
///
/// `scale` is capture units to pixels. Marks are in **capture-local units** with
/// the origin at the picture's own top-left, not in the coordinates of any page
/// inside it: a capture can span two pages, and a mark drawn across the join
/// belongs to neither of them.
///
/// A pure function of `(bitmap, marks, scale)` — same inputs, byte-identical
/// output — which is what makes a capture reproducible.
pub fn composite(bitmap: &mut Bitmap, marks: &[Markup], scale: f32) -> Result<()> {
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
        draw(&mut pixmap, mark, scale);
    }

    Ok(())
}

/// Capture-local units to pixels.
fn to_pixels(p: Point, scale: f32) -> (f32, f32) {
    (p.x * scale, p.y * scale)
}

fn draw(pixmap: &mut PixmapMut, mark: &Markup, scale: f32) {
    let width_px = (mark.width_pt * scale).max(1.0);

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
            // Filled, at the intensity the colour carries — capped, so however
            // far the slider is pushed the page still reads through it.
            paint.set_color_rgba8(
                mark.color.r,
                mark.color.g,
                mark.color.b,
                mark.color.a.min(HIGHLIGHT_ALPHA_CEILING),
            );
            let (left, top) = to_pixels(
                Point {
                    x: rect.left,
                    y: rect.top,
                },
                scale,
            );
            let (right, bottom) = to_pixels(
                Point {
                    x: rect.right,
                    y: rect.bottom,
                },
                scale,
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
        Shape::Freehand { points } => freehand_path(points, scale),
        Shape::Line { from, to } => segment_path(*from, *to, scale),
        Shape::Arrow { from, to } => arrow_path(*from, *to, width_px, scale),
        Shape::Rect { rect } => rect_path(*rect, scale),
        Shape::Ellipse { rect } => ellipse_path(*rect, scale),
    };

    if let Some(path) = path {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn freehand_path(points: &[Point], scale: f32) -> Option<tiny_skia::Path> {
    if points.len() < 2 {
        return None;
    }
    let mut builder = PathBuilder::new();
    let (x, y) = to_pixels(points[0], scale);
    builder.move_to(x, y);

    // Quadratics through the midpoints, so a fast drag reads as a smooth line
    // rather than as the visible polygon that joining raw touch samples gives.
    for pair in points.windows(2) {
        let (px, py) = to_pixels(pair[0], scale);
        let (cx, cy) = to_pixels(pair[1], scale);
        builder.quad_to(px, py, (px + cx) / 2.0, (py + cy) / 2.0);
    }
    let (x, y) = to_pixels(*points.last().expect("checked non-empty"), scale);
    builder.line_to(x, y);

    builder.finish()
}

fn segment_path(from: Point, to: Point, scale: f32) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let (x0, y0) = to_pixels(from, scale);
    let (x1, y1) = to_pixels(to, scale);
    builder.move_to(x0, y0);
    builder.line_to(x1, y1);
    builder.finish()
}

fn arrow_path(
    from: Point,
    to: Point,
    width_px: f32,
    scale: f32,
) -> Option<tiny_skia::Path> {
    let (x0, y0) = to_pixels(from, scale);
    let (x1, y1) = to_pixels(to, scale);

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

fn rect_path(rect: Rect, scale: f32) -> Option<tiny_skia::Path> {
    let (left, top) = to_pixels(
        Point {
            x: rect.left,
            y: rect.top,
        },
        scale,
    );
    let (right, bottom) = to_pixels(
        Point {
            x: rect.right,
            y: rect.bottom,
        },
        scale,
    );
    let mut builder = PathBuilder::new();
    builder.move_to(left, top);
    builder.line_to(right, top);
    builder.line_to(right, bottom);
    builder.line_to(left, bottom);
    builder.close();
    builder.finish()
}

fn ellipse_path(rect: Rect, scale: f32) -> Option<tiny_skia::Path> {
    let (left, top) = to_pixels(
        Point {
            x: rect.left,
            y: rect.top,
        },
        scale,
    );
    let (right, bottom) = to_pixels(
        Point {
            x: rect.right,
            y: rect.bottom,
        },
        scale,
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

    /// The capture used throughout: 100 x 100 capture units at 2×, so 200 x 200 px.
    ///
    /// Units, not page points. A capture can span two pages, and a mark drawn
    /// across the join belongs to neither of them, so marks are positioned against
    /// the picture itself — which makes the pixel arithmetic a single multiply.
    const SCALE: f32 = 2.0;

    fn white_capture() -> Bitmap {
        let mut bitmap = Bitmap::new(200, 200, PixelOrder::Rgba).unwrap();
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
        Color { r: 255, g: 0, b: 0, a: 255 }
    }

    fn mark(shape: Shape) -> Markup {
        Markup { shape, color: red(), width_pt: 4.0 }
    }

    fn at(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect { left, top, right, bottom }
    }

    #[test]
    fn a_line_is_drawn_where_the_units_say_and_nowhere_else() {
        let mut bitmap = white_capture();
        // Across the middle: y = 50 units, which is 100 px down the capture.
        composite(
            &mut bitmap,
            &[mark(Shape::Line { from: at(10.0, 50.0), to: at(90.0, 50.0) })],
            SCALE,
        )
        .unwrap();

        assert!(!is_white(pixel(&bitmap, 100, 100)), "the line is not on it");
        assert!(is_white(pixel(&bitmap, 100, 20)), "ink well above the line");
        assert!(is_white(pixel(&bitmap, 100, 180)), "ink well below the line");
    }

    #[test]
    fn a_mark_is_placed_by_the_capture_rather_than_by_any_page_inside_it() {
        // The trap this guards: treating a mark's coordinates as page points and
        // subtracting a crop's origin. Marks arrive already relative to the
        // picture, so any further offset moves every one of them.
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Line { from: at(5.0, 10.0), to: at(15.0, 10.0) })],
            SCALE,
        )
        .unwrap();

        assert!(!is_white(pixel(&bitmap, 20, 20)), "not where the units put it");
        assert!(is_white(pixel(&bitmap, 120, 120)), "drawn somewhere else entirely");
    }

    #[test]
    fn a_rectangle_is_outlined_and_not_filled() {
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Rect { rect: rect(20.0, 20.0, 80.0, 80.0) })],
            SCALE,
        )
        .unwrap();

        assert!(!is_white(pixel(&bitmap, 40, 100)), "left edge missing");
        assert!(!is_white(pixel(&bitmap, 160, 100)), "right edge missing");
        assert!(is_white(pixel(&bitmap, 100, 100)), "the middle was filled in");
    }

    #[test]
    fn an_ellipse_touches_its_bounds_at_the_midpoints_and_misses_the_corners() {
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Ellipse { rect: rect(20.0, 20.0, 80.0, 80.0) })],
            SCALE,
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
                shape: Shape::Highlight { rect: rect(20.0, 45.0, 80.0, 55.0) },
                // Alpha is the intensity, which is the user's to set.
                color: Color { r: 255, g: 224, b: 102, a: 90 },
                width_pt: 0.0,
            }],
            SCALE,
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
    fn a_highlights_intensity_comes_from_its_colour() {
        // The intensity slider sets this. A fixed alpha — which is what this was —
        // would leave the slider looking connected to nothing.
        fn wash(alpha: u8) -> i32 {
            let mut bitmap = white_capture();
            composite(
                &mut bitmap,
                &[Markup {
                    shape: Shape::Highlight { rect: rect(20.0, 20.0, 80.0, 80.0) },
                    color: Color { r: 0, g: 0, b: 0, a: alpha },
                    width_pt: 0.0,
                }],
                SCALE,
            )
            .unwrap();
            pixel(&bitmap, 100, 100).0 as i32
        }

        assert!(wash(200) < wash(90), "a stronger alpha should darken further");
        assert!(wash(30) > wash(90), "a weaker alpha should darken less");
    }

    #[test]
    fn a_highlight_can_never_be_drawn_fully_opaque() {
        // However far the slider is pushed, the page has to read through it, or
        // the tool hides the very thing it was used to point at.
        let mut bitmap = white_capture();
        for y in 90..110 {
            for x in 40..160 {
                let at = y * bitmap.stride + x * 4;
                bitmap.data[at..at + 3].copy_from_slice(&[0, 0, 0]);
            }
        }

        composite(
            &mut bitmap,
            &[Markup {
                shape: Shape::Highlight { rect: rect(20.0, 45.0, 80.0, 55.0) },
                color: Color { r: 255, g: 255, b: 0, a: 255 },
                width_pt: 0.0,
            }],
            SCALE,
        )
        .unwrap();

        // The black band underneath still shows through the strongest wash.
        let over_text = pixel(&bitmap, 100, 100);
        assert!(over_text.1 < 250, "the wash went opaque: {over_text:?}");
    }

    #[test]
    fn an_arrow_puts_ink_at_its_tip() {
        let mut bitmap = white_capture();
        composite(
            &mut bitmap,
            &[mark(Shape::Arrow { from: at(10.0, 10.0), to: at(90.0, 90.0) })],
            SCALE,
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
            &[mark(Shape::Freehand { points: vec![at(50.0, 50.0)] })],
            SCALE,
        )
        .unwrap();
        assert!(bitmap.data.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn compositing_is_deterministic() {
        // A capture that composites differently run to run cannot be compared with
        // anything, including a later export of the same markup.
        let marks = vec![
            mark(Shape::Ellipse { rect: rect(20.0, 20.0, 80.0, 70.0) }),
            mark(Shape::Freehand {
                points: vec![at(10.0, 10.0), at(40.0, 60.0), at(80.0, 20.0)],
            }),
        ];

        let mut once = white_capture();
        let mut twice = white_capture();
        composite(&mut once, &marks, SCALE).unwrap();
        composite(&mut twice, &marks, SCALE).unwrap();
        assert_eq!(once.data, twice.data);
    }

    #[test]
    fn marks_are_drawn_in_order_so_the_last_one_is_on_top() {
        let blue = Color { r: 0, g: 0, b: 255, a: 255 };
        let across = |color: Color| Markup {
            shape: Shape::Line { from: at(10.0, 50.0), to: at(90.0, 50.0) },
            color,
            width_pt: 6.0,
        };

        let mut bitmap = white_capture();
        composite(&mut bitmap, &[across(red()), across(blue)], SCALE).unwrap();

        let (r, _, b) = pixel(&bitmap, 100, 100);
        assert!(b > r, "the second stroke should be the visible one");
    }

    #[test]
    fn nothing_at_all_is_drawn_for_an_empty_list() {
        let mut bitmap = white_capture();
        composite(&mut bitmap, &[], SCALE).unwrap();
        assert!(bitmap.data.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn a_stroke_width_scales_with_the_export_rather_than_being_fixed_in_pixels() {
        let thickness = |width_pt: f32| {
            let mut bitmap = white_capture();
            composite(
                &mut bitmap,
                &[Markup {
                    shape: Shape::Line { from: at(10.0, 50.0), to: at(90.0, 50.0) },
                    color: red(),
                    width_pt,
                }],
                SCALE,
            )
            .unwrap();
            (0..bitmap.height).filter(|&y| !is_white(pixel(&bitmap, 100, y))).count()
        };

        // 4 units at 2x is 8 px; 2 units is 4 px. The exact counts depend on
        // anti-aliasing, the ordering does not.
        assert!(thickness(4.0) > thickness(2.0));
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
                shape: Shape::Arrow { from: at(1.0, 2.0), to: at(3.0, 4.0) },
                color: red(),
                width_pt: 2.5,
            },
        );
    }
}

//! A page's marks drawn into a rendered page — what the Snapshot needs and a
//! render does not have.
//!
//! A page's raster is the *file's* page. The shapes drawn on top of it live in
//! this app's own [`Layer`] until a save commits them (see
//! [`crate::commit::commit_page`]); the screen shows them because the app
//! paints them itself (`overlay::draw_layer`), and a snapshot taken at that
//! moment came out as a page with nothing drawn on it — reported from use:
//! "if i have drawn 2d shapes on the page, they do not show up in the
//! snapshot".
//!
//! The painting mirrors `commit_page`'s: each object's outline through
//! [`crate::commit::flatten`], drawn with the same resolved pen and width, a
//! filled boundary's interior beneath its own crisp stroke, an arrowhead a
//! filled triangle. Two writers, one geometry — this one writes pixels where
//! the other writes PDF annotations, and what they share is the flattening.

use cad_kernel::{Color as KernelColor, Geom};

use pdf_core::document::{Color, Point};

use crate::markup::Layer;
use crate::session::PageRaster;

/// Draw `layer`'s marks into `raster` — `scale` pixels per point, the same
/// scale the page was rastered at.
///
/// `page_pen` and `page_width` are what an object with no colour or
/// lineweight of its own follows, exactly as `commit_page`'s own two
/// arguments are: the layer and the raster agree about defaults the way the
/// screen does.
pub fn paint_layer(
    raster: &mut PageRaster,
    layer: &Layer,
    scale: f32,
    page_pen: Color,
    page_width: f32,
) {
    PageMarks::of(layer).paint(raster, scale, page_pen, page_width);
}

/// **A page's marks, owned and `Send`** — everything [`paint_layer`] reads off
/// a live [`Layer`], and nothing else (no undo history, no selection, no
/// spatial index).
///
/// The Conversion tab's To Image exports its pages on a worker, and the marks
/// drawn on a page live in the app's own layer, which is not the worker's to
/// borrow. This is the copy it carries instead, so an exported PNG is the
/// page as it is being read — marks and all — exactly as the Snapshot is.
pub struct PageMarks {
    height_pt: f64,
    marks: Vec<PageMark>,
}

struct PageMark {
    geom: Geom,
    /// The object's own colour, resolved against the layer's own tables at
    /// copy time — `None` means "follow the page's pen", which is what
    /// `Color::ByLayer` means and what an unresolvable colour falls back to.
    colour: Option<(u8, u8, u8)>,
    /// The object's own lineweight in page points, likewise resolved at copy
    /// time; `None` means "follow the page's width".
    width_pt: Option<f32>,
    filled: bool,
    arrows: (bool, bool),
}

impl PageMarks {
    /// Take the copy from a live layer. Exact: every object the layer has is
    /// carried, including kinds a save cannot store — a paint copy is for
    /// drawing, not for writing back.
    pub fn of(layer: &Layer) -> Self {
        let marks = layer
            .objects()
            .iter()
            .enumerate()
            .map(|(index, object)| PageMark {
                geom: object.geom.clone(),
                colour: match object.style.color {
                    KernelColor::ByLayer => None,
                    _ => layer.resolved_color(index),
                },
                width_pt: layer.resolved_lineweight_mm(index).map(|mm| mm * 72.0 / 25.4),
                filled: layer.is_filled(index),
                arrows: layer.arrow_ends(index),
            })
            .collect();
        PageMarks { height_pt: layer.space().height_pt(), marks }
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Draw them — see [`paint_layer`] for what the arguments mean.
    pub fn paint(&self, raster: &mut PageRaster, scale: f32, page_pen: Color, page_width: f32) {
        let space = crate::page_space::PageSpace::new(self.height_pt);
        for mark in &self.marks {
            // A hatch has no shape of its own to draw — it says its boundary
            // is filled, consulted below, exactly as `overlay::draw_layer`
            // and `commit_page` both do.
            if matches!(mark.geom, Geom::Hatch(_)) {
                continue;
            }
            let outline = crate::commit::flatten(&mark.geom, space);
            if outline.is_empty() {
                continue;
            }

            // An object nobody gave its own colour follows this page's pen.
            let pen = mark
                .colour
                .map(|(r, g, b)| Color { r, g, b, a: 255 })
                .unwrap_or(page_pen);
            let width_pt = mark.width_pt.unwrap_or(page_width);

            // The interior first, so the crisp stroke ends up on top of the
            // solid fill rather than lost beneath it — `commit_page` writes
            // its `Fill` before its `Ink` for the same reason.
            if mark.filled {
                if let Some(ring) = outline.first() {
                    fill_ring(raster, ring, scale, pen);
                }
            }
            for path in &outline {
                stroke_path(raster, path, scale, width_pt, pen);
            }

            // A line's own arrowhead(s), each a filled triangle — the same
            // `arrowhead_triangle` `commit_page` writes as a `Fill`.
            if let Geom::Line(line) = &mark.geom {
                let mut heads = Vec::new();
                if mark.arrows.0 {
                    heads.extend(crate::commit::arrowhead_triangle(line.b, line.a));
                }
                if mark.arrows.1 {
                    heads.extend(crate::commit::arrowhead_triangle(line.a, line.b));
                }
                for head in heads {
                    let ring: Vec<Point> = head
                        .iter()
                        .map(|&v| {
                            let p = space.from_kernel(v);
                            Point { x: p.x as f32, y: p.y as f32 }
                        })
                        .collect();
                    fill_ring(raster, &ring, scale, pen);
                }
            }
        }
    }
}

/// One outline, stroked: discs stamped along every segment, the stamp step
/// small enough that the runs join into a line rather than a dotted one.
///
/// A stamped round disc rather than a scanline thick line because it is the
/// same three lines of code for caps, corners and joins — and the shapes
/// this draws are a draughtsman's lines, where the round join is the honest
/// one.
fn stroke_path(raster: &mut PageRaster, path: &[Point], scale: f32, width_pt: f32, pen: Color) {
    let radius = (width_pt * scale * 0.5).max(0.5);
    let step = (radius * 0.5).max(0.4);
    if path.len() == 1 {
        stamp(raster, path[0].x * scale, path[0].y * scale, radius, pen);
        return;
    }
    for pair in path.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (ax, ay) = (a.x * scale, a.y * scale);
        let (bx, by) = (b.x * scale, b.y * scale);
        let length = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        let steps = (length / step).ceil().max(1.0) as u32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            stamp(raster, ax + (bx - ax) * t, ay + (by - ay) * t, radius, pen);
        }
    }
}

/// One disc, anti-aliased by its own coverage at the rim — the half-point
/// feather that keeps a 1.5-point line from looking like a staircase.
fn stamp(raster: &mut PageRaster, cx: f32, cy: f32, radius: f32, pen: Color) {
    let reach = radius + 1.0;
    let x0 = (cx - reach).floor().max(0.0) as u32;
    let y0 = (cy - reach).floor().max(0.0) as u32;
    let x1 = ((cx + reach).ceil().max(0.0) as u32).min(raster.width.saturating_sub(1));
    let y1 = ((cy + reach).ceil().max(0.0) as u32).min(raster.height.saturating_sub(1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if coverage > 0.0 {
                blend(raster, x, y, pen, coverage);
            }
        }
    }
}

/// A ring, filled even-odd: one scanline walk, crossings paired left to
/// right. Enough for a boundary the draughtsman closed — a rectangle, a
/// circle flattened to points — and honest about holes, which even-odd gives
/// for free.
fn fill_ring(raster: &mut PageRaster, ring: &[Point], scale: f32, pen: Color) {
    if ring.len() < 3 {
        return;
    }
    let points: Vec<(f32, f32)> = ring.iter().map(|p| (p.x * scale, p.y * scale)).collect();
    let top = points.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    let bottom = points.iter().map(|p| p.1).fold(f32::MIN, f32::max);
    let y0 = top.floor().max(0.0) as u32;
    let y1 = bottom.ceil().max(0.0) as u32;
    for y in y0..=y1.min(raster.height.saturating_sub(1)) {
        let scan = y as f32 + 0.5;
        let mut crossings: Vec<f32> = Vec::new();
        for i in 0..points.len() {
            let (ax, ay) = points[i];
            let (bx, by) = points[(i + 1) % points.len()];
            if (ay <= scan) == (by <= scan) {
                continue;
            }
            let t = (scan - ay) / (by - ay);
            crossings.push(ax + (bx - ax) * t);
        }
        crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in crossings.chunks_exact(2) {
            let x0 = pair[0].floor().max(0.0) as u32;
            let x1 = (pair[1].ceil().max(0.0) as u32).min(raster.width.saturating_sub(1));
            for x in x0..=x1 {
                blend(raster, x, y, pen, 1.0);
            }
        }
    }
}

/// One pixel, `pen` mixed in over what is there by `coverage`.
fn blend(raster: &mut PageRaster, x: u32, y: u32, pen: Color, coverage: f32) {
    if x >= raster.width || y >= raster.height {
        return;
    }
    let alpha = coverage * (f32::from(pen.a) / 255.0);
    if alpha <= 0.0 {
        return;
    }
    let at = ((y * raster.width + x) * 4) as usize;
    for (channel, source) in [pen.r, pen.g, pen.b].into_iter().enumerate() {
        let under = raster.pixels[at + channel];
        let mixed = f32::from(source) * alpha + f32::from(under) * (1.0 - alpha);
        raster.pixels[at + channel] = mixed.round().clamp(0.0, 255.0) as u8;
    }
    // The destination is opaque paper; the copy stays opaque.
    raster.pixels[at + 3] = 255;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_kernel::{Line, Vec2};

    fn blank(width: u32, height: u32) -> PageRaster {
        PageRaster {
            width,
            height,
            pixels: vec![255; (width * height * 4) as usize],
            from_cache: false,
        }
    }

    fn ink(raster: &PageRaster) -> usize {
        raster.pixels.chunks_exact(4).filter(|px| px[..3] != [255, 255, 255]).count()
    }

    fn ink_in(raster: &PageRaster, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
        let mut count = 0;
        for y in y0..y1.min(raster.height) {
            for x in x0..x1.min(raster.width) {
                let at = ((y * raster.width + x) * 4) as usize;
                if raster.pixels[at..at + 3] != [255, 255, 255] {
                    count += 1;
                }
            }
        }
        count
    }

    const PAGE: f32 = 800.0;
    const PEN: Color = Color { r: 200, g: 30, b: 30, a: 255 };

    /// A line drawn on the layer lands on the raster, in its own place —
    /// and nowhere else.
    #[test]
    fn a_stroke_lands_where_the_layer_has_it() {
        let mut layer = Layer::new(PAGE as f64);
        // Layer coordinates are the kernel's: y up from the page's bottom,
        // so y = 700 is 100 points down from the top of an 800-point page.
        layer.add(Geom::Line(Line { a: Vec2::new(100.0, 700.0), b: Vec2::new(300.0, 700.0) }));

        let mut raster = blank(800, 800);
        paint_layer(&mut raster, &layer, 1.0, PEN, 1.5);

        assert!(ink_in(&raster, 100, 95, 300, 106) > 150, "the line is not on the raster");
        assert_eq!(ink_in(&raster, 0, 0, 800, 90), 0, "something was drawn above the line");
        assert_eq!(ink_in(&raster, 320, 0, 800, 800), 0, "something was drawn past the line's end");
    }

    /// A closed, filled shape is filled — a drawn rectangle is stored as a
    /// closed polyline, and the fill is the `Hatch` the layer keeps for it.
    #[test]
    fn a_filled_shape_is_filled() {
        let mut layer = Layer::new(PAGE as f64);
        layer.add(Geom::Polyline(cad_kernel::Polyline {
            vertices: vec![
                cad_kernel::PolyVertex { pos: Vec2::new(100.0, 600.0), bulge: 0.0 },
                cad_kernel::PolyVertex { pos: Vec2::new(300.0, 600.0), bulge: 0.0 },
                cad_kernel::PolyVertex { pos: Vec2::new(300.0, 700.0), bulge: 0.0 },
                cad_kernel::PolyVertex { pos: Vec2::new(100.0, 700.0), bulge: 0.0 },
            ],
            closed: true,
            widths: Vec::new(),
        }));
        assert!(layer.set_filled(0, true));

        let mut raster = blank(800, 800);
        paint_layer(&mut raster, &layer, 1.0, PEN, 1.5);

        // The middle of the rectangle is ink, not paper: kernel y 600..700
        // on an 800-point page is 100..200 points down from the top.
        let middle = ((150 * 800 + 200) * 4) as usize;
        assert_ne!(raster.pixels[middle..middle + 3], [255, 255, 255]);
    }

    /// **The paint copy is exact.** The Conversion tab's To Image paints
    /// from a [`PageMarks`] taken off the live layer, and both roads have to
    /// leave the same pixels — one painter, two callers.
    #[test]
    fn the_paint_copy_paints_what_the_layer_leaves() {
        let mut layer = Layer::new(PAGE as f64);
        layer.add(Geom::Line(Line { a: Vec2::new(100.0, 700.0), b: Vec2::new(300.0, 700.0) }));
        layer.add(Geom::Polyline(cad_kernel::Polyline {
            vertices: vec![
                cad_kernel::PolyVertex { pos: Vec2::new(400.0, 600.0), bulge: 0.0 },
                cad_kernel::PolyVertex { pos: Vec2::new(500.0, 600.0), bulge: 0.0 },
                cad_kernel::PolyVertex { pos: Vec2::new(500.0, 650.0), bulge: 0.0 },
                cad_kernel::PolyVertex { pos: Vec2::new(400.0, 650.0), bulge: 0.0 },
            ],
            closed: true,
            widths: Vec::new(),
        }));
        assert!(layer.set_filled(1, true));

        let mut live = blank(800, 800);
        paint_layer(&mut live, &layer, 1.0, PEN, 1.5);
        let mut copy = blank(800, 800);
        let marks = PageMarks::of(&layer);
        assert!(!marks.is_empty());
        marks.paint(&mut copy, 1.0, PEN, 1.5);

        assert_eq!(live.pixels, copy.pixels, "the copy painted different pixels");
    }

    /// Nothing on the layer is nothing on the raster: a page with no marks
    /// keeps its render exactly.
    #[test]
    fn an_empty_layer_changes_nothing() {
        let layer = Layer::new(PAGE as f64);
        let mut raster = blank(64, 64);
        paint_layer(&mut raster, &layer, 2.0, PEN, 1.5);
        assert_eq!(ink(&raster), 0);
    }
}

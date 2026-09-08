//! A drawing, into pixels.
//!
//! **Strokes, not triangles.** The STEP viewer's rasteriser fills triangles
//! against a depth buffer, which is the right shape for a solid and the wrong
//! one for a sheet of lines: a drawing has no depth, and its lines have width
//! measured in pixels rather than in the drawing's own units. So this is a
//! separate path — a short one, because `tiny-skia` is already in the build for
//! markup on a capture and already strokes anti-aliased paths.

use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

use super::model::{Drawing, Point, Shape};

/// How the sheet is drawn.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub background: [u8; 4],
    /// How wide a line is on screen, in pixels, whatever the zoom.
    ///
    /// A drawing's own line weights are in millimetres on a plotted sheet. At
    /// the zoom somebody reads a plan on a phone they would come out either
    /// invisible or a centimetre thick, so what is drawn is a constant on
    /// screen — which is what every CAD viewer does at low zoom for the same
    /// reason.
    pub line_width: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self { background: [24, 26, 30, 255], line_width: 1.2 }
    }
}

/// Where the sheet sits in the view.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Which point of the drawing is at the middle of the screen.
    pub centre: Point,
    /// Screen pixels per drawing unit.
    pub scale: f64,
}

impl View {
    /// The view that shows the whole drawing.
    pub fn fitted(drawing: &Drawing, width: u32, height: u32) -> Self {
        let Some((low, high)) = drawing.bounds() else {
            return Self { centre: Point::new(0.0, 0.0), scale: 1.0 };
        };
        let (across, down) = ((high.x - low.x).max(1e-9), (high.y - low.y).max(1e-9));
        // A tenth of a margin, so the outermost line is not against the edge.
        let scale = ((width as f64 / across).min(height as f64 / down)) * 0.9;
        Self {
            centre: Point::new((low.x + high.x) / 2.0, (low.y + high.y) / 2.0),
            scale: if scale.is_finite() && scale > 0.0 { scale } else { 1.0 },
        }
    }

    /// A drawing point to a screen pixel.
    ///
    /// **The sheet's y runs up and the screen's runs down**, so the sign flips
    /// here. Forgetting it does not fail — it draws the plan upside down, which
    /// on a symmetrical building is not obvious at all.
    fn place(&self, p: Point, width: u32, height: u32) -> (f32, f32) {
        (
            ((p.x - self.centre.x) * self.scale + width as f64 / 2.0) as f32,
            (height as f64 / 2.0 - (p.y - self.centre.y) * self.scale) as f32,
        )
    }
}

/// Draw the whole sheet.
pub fn draw(drawing: &Drawing, view: &View, style: &Style, into: &mut Pixmap) {
    let (width, height) = (into.width(), into.height());
    into.fill(tiny_skia::Color::from_rgba8(
        style.background[0],
        style.background[1],
        style.background[2],
        style.background[3],
    ));

    let stroke = Stroke { width: style.line_width, ..Stroke::default() };
    // Grouped by layer so a paint is built once per colour rather than once per
    // line. On a drawing of twenty-seven thousand shapes that is the difference
    // between a frame and a wait.
    for (id, layer) in drawing.layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        let mut paint = Paint::default();
        paint.set_color_rgba8(layer.colour[0], layer.colour[1], layer.colour[2], 255);
        paint.anti_alias = true;

        let mut builder = PathBuilder::new();
        let mut any = false;
        for entity in drawing.entities.iter().filter(|e| e.layer as usize == id) {
            if add(&mut builder, &entity.shape, view, width, height) {
                any = true;
            }
        }
        if !any {
            continue;
        }
        if let Some(path) = builder.finish() {
            into.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// Add one shape to the path being built. `false` when it contributed nothing.
fn add(
    builder: &mut PathBuilder,
    shape: &Shape,
    view: &View,
    width: u32,
    height: u32,
) -> bool {
    let to_screen = |p: Point| view.place(p, width, height);

    match shape {
        Shape::Line { a, b } => {
            let (ax, ay) = to_screen(*a);
            let (bx, by) = to_screen(*b);
            if !ax.is_finite() || !ay.is_finite() || !bx.is_finite() || !by.is_finite() {
                return false;
            }
            builder.move_to(ax, ay);
            builder.line_to(bx, by);
            true
        }

        Shape::Arc { centre, radius, start, sweep } => {
            let steps = steps_for(*radius * view.scale, *sweep);
            for step in 0..=steps {
                let angle = start + sweep * step as f64 / steps as f64;
                let at = Point::new(
                    centre.x + radius * angle.cos(),
                    centre.y + radius * angle.sin(),
                );
                let (x, y) = to_screen(at);
                if !x.is_finite() || !y.is_finite() {
                    return false;
                }
                if step == 0 {
                    builder.move_to(x, y);
                } else {
                    builder.line_to(x, y);
                }
            }
            true
        }

        Shape::Ellipse { centre, major, ratio, start, sweep } => {
            let long = major.x.hypot(major.y);
            let short = long * ratio;
            let tilt = major.y.atan2(major.x);
            let steps = steps_for(long.max(short) * view.scale, *sweep);
            for step in 0..=steps {
                let angle = start + sweep * step as f64 / steps as f64;
                // The parameter runs round the ellipse's own axes, then the
                // whole thing is turned by however the major axis lies.
                let (x, y) = (long * angle.cos(), short * angle.sin());
                let (sin, cos) = tilt.sin_cos();
                let at = Point::new(centre.x + x * cos - y * sin, centre.y + x * sin + y * cos);
                let (sx, sy) = to_screen(at);
                if !sx.is_finite() || !sy.is_finite() {
                    return false;
                }
                if step == 0 {
                    builder.move_to(sx, sy);
                } else {
                    builder.line_to(sx, sy);
                }
            }
            true
        }

        Shape::Polyline { vertices, closed } => {
            if vertices.len() < 2 {
                return false;
            }
            let (x, y) = to_screen(vertices[0].at);
            if !x.is_finite() || !y.is_finite() {
                return false;
            }
            builder.move_to(x, y);

            let last = if *closed { vertices.len() } else { vertices.len() - 1 };
            for index in 0..last {
                let from = vertices[index];
                let to = vertices[(index + 1) % vertices.len()];
                if from.bulge.abs() < 1e-12 {
                    let (x, y) = to_screen(to.at);
                    builder.line_to(x, y);
                } else {
                    bulged(builder, from.at, to.at, from.bulge, view, width, height);
                }
            }
            if *closed {
                builder.close();
            }
            true
        }
    }
}

/// A bowed polyline segment, as the arc it is.
///
/// **Bulge is the tangent of a quarter of the included angle**, signed
/// anticlockwise. It is a compact way to store an arc through two known points
/// and it is easy to read as something else — treating it as a sag, or as a
/// radius, gives a curve of about the right shape in about the right place,
/// which is the hardest kind of wrong to notice on a plan full of arcs.
fn bulged(
    builder: &mut PathBuilder,
    from: Point,
    to: Point,
    bulge: f64,
    view: &View,
    width: u32,
    height: u32,
) {
    let included = 4.0 * bulge.atan();
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let chord = dx.hypot(dy);
    if chord < 1e-12 {
        return;
    }
    let radius = chord / (2.0 * (included / 2.0).sin());
    // The centre is out along the chord's perpendicular, on the side the sign
    // of the bulge chooses.
    let height_of = radius * (included / 2.0).cos();
    let centre = Point::new(
        (from.x + to.x) / 2.0 - height_of * dy / chord,
        (from.y + to.y) / 2.0 + height_of * dx / chord,
    );
    let start = (from.y - centre.y).atan2(from.x - centre.x);

    let steps = steps_for(radius.abs() * view.scale, included.abs());
    for step in 1..=steps {
        let angle = start + included * step as f64 / steps as f64;
        let at = Point::new(
            centre.x + radius.abs() * angle.cos(),
            centre.y + radius.abs() * angle.sin(),
        );
        let (x, y) = view.place(at, width, height);
        if x.is_finite() && y.is_finite() {
            builder.line_to(x, y);
        }
    }
}

/// How many straight pieces a curve needs to look curved.
///
/// From its size **on screen**, not in the drawing: a circle of ten metres and
/// one of ten millimetres need the same number of segments when they are drawn
/// the same size, and a fixed count wastes work on one and shows corners on the
/// other.
fn steps_for(radius_in_pixels: f64, sweep: f64) -> usize {
    let sweep = sweep.abs().max(1e-6);
    if !radius_in_pixels.is_finite() || radius_in_pixels <= 0.5 {
        return 2;
    }
    // Half a pixel of sag: the angle whose chord departs from the arc by that
    // much is 2·acos(1 − 0.5/r).
    let step = 2.0 * (1.0 - 0.5 / radius_in_pixels).clamp(-1.0, 1.0).acos();
    if step <= f64::EPSILON {
        return 256;
    }
    ((sweep / step).ceil() as usize).clamp(2, 256)
}

/// **Ignored, because real drawings cannot be committed.** Point it at one and
/// look at what comes out — the only check that catches a plan drawn upside
/// down, mirrored, or with its curves bowing the wrong way, none of which any
/// count would show:
///
/// ```text
/// PAGIFY_DXF_FILE="/path/plan.dxf" PAGIFY_DXF_PNG="/path/out.png" \
///   cargo test --release --lib drawing::raster::picture -- --ignored --nocapture
/// ```
#[cfg(test)]
mod picture {
    use super::*;

    #[test]
    #[ignore = "needs a real drawing; set PAGIFY_DXF_FILE and PAGIFY_DXF_PNG"]
    fn a_real_drawing_can_be_looked_at() {
        let path = std::env::var("PAGIFY_DXF_FILE").expect("set PAGIFY_DXF_FILE");
        let out = std::env::var("PAGIFY_DXF_PNG").expect("set PAGIFY_DXF_PNG");
        let bytes = std::fs::read(&path).expect("readable");
        let drawing = crate::drawing::dxf::read(&String::from_utf8_lossy(&bytes)).expect("parses");

        const WIDE: u32 = 1400;
        const HIGH: u32 = 900;
        let mut sheet = Pixmap::new(WIDE, HIGH).expect("a canvas");
        let view = View::fitted(&drawing, WIDE, HIGH);
        let style = Style::default();

        let started = std::time::Instant::now();
        draw(&drawing, &view, &style, &mut sheet);
        println!(
            "{} shapes drawn in {:?}, {} pixels per unit",
            drawing.kept(),
            started.elapsed(),
            view.scale,
        );

        let ink = sheet
            .pixels()
            .iter()
            .filter(|p| [p.red(), p.green(), p.blue()] != [style.background[0], style.background[1], style.background[2]])
            .count();
        println!("  {ink} pixels have something on them");
        assert!(ink > 1000, "the sheet came out all but blank");

        // Encoded with `image` rather than tiny-skia, whose PNG feature is off
        // in this build: nothing in the app writes one, only this diagnostic.
        image::save_buffer(&out, sheet.data(), WIDE, HIGH, image::ColorType::Rgba8)
            .expect("the picture is written");
        println!("  wrote {out}");
    }
}

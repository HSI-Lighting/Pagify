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
#[derive(Debug, Clone)]
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
    /// The font the sheet's text is drawn with, if one is registered.
    ///
    /// **A drawing carries no font of its own that this can use.** CAD text
    /// names an SHX stroke font or a Windows typeface, neither of which travels
    /// with the file — so every viewer substitutes, and this substitutes the
    /// app's own. Held as bytes rather than a parsed face because a parsed one
    /// borrows them, and this has to be `Clone` and outlive any single frame.
    pub font: Option<std::sync::Arc<Vec<u8>>>,
}

impl Style {
    /// The font, parsed, or `None` when there is none to draw with.
    ///
    /// Parsed per frame rather than held: `ttf_parser::Face` borrows its bytes,
    /// and the parse is a header read rather than anything expensive.
    pub(crate) fn face(&self) -> Option<ttf_parser::Face<'_>> {
        ttf_parser::Face::parse(self.font.as_ref()?, 0).ok()
    }
}

impl Default for Style {
    fn default() -> Self {
        Self { background: [24, 26, 30, 255], line_width: 1.2, font: None }
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

/// A font's outlines, turned into a path the stroker can use.
///
/// `ttf-parser` hands a glyph over one segment at a time through this, in the
/// font's own units with y running up. The flip, the turn and the scale happen
/// here, once per point, rather than being applied to a finished path.
struct Glyph<'a> {
    into: &'a mut PathBuilder,
    /// Font units to drawing units.
    scale: f64,
    /// Where the glyph's origin sits on the sheet.
    at: Point,
    sin: f64,
    cos: f64,
    view: &'a View,
    width: u32,
    height: u32,
}

impl Glyph<'_> {
    /// A point in font units to a pixel on screen.
    fn place(&self, x: f32, y: f32) -> (f32, f32) {
        let (dx, dy) = (x as f64 * self.scale, y as f64 * self.scale);
        let on_sheet = Point::new(
            self.at.x + dx * self.cos - dy * self.sin,
            self.at.y + dx * self.sin + dy * self.cos,
        );
        self.view.place(on_sheet, self.width, self.height)
    }
}

impl ttf_parser::OutlineBuilder for Glyph<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (px, py) = self.place(x, y);
        self.into.move_to(px, py);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (px, py) = self.place(x, y);
        self.into.line_to(px, py);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (cx, cy) = self.place(x1, y1);
        let (px, py) = self.place(x, y);
        self.into.quad_to(cx, cy, px, py);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1x, c1y) = self.place(x1, y1);
        let (c2x, c2y) = self.place(x2, y2);
        let (px, py) = self.place(x, y);
        self.into.cubic_to(c1x, c1y, c2x, c2y, px, py);
    }

    fn close(&mut self) {
        self.into.close();
    }
}

/// Lay one line of text out along the sheet, as outlines.
///
/// **Filled, not stroked.** Everything else here is a line with a width; a
/// letter is a shape with an inside, and stroking its outline at the same
/// weight as a wall turns small text into an unreadable smudge.
///
/// Returns `false` when there is no font to draw with, so the caller can count
/// the text as not shown rather than leave a gap nobody is told about.
fn text_path(
    face: &ttf_parser::Face,
    content: &str,
    at: Point,
    height: f64,
    rotation: f64,
    view: &View,
    width: u32,
    height_px: u32,
    into: &mut PathBuilder,
) -> bool {
    let per_em = face.units_per_em() as f64;
    if per_em <= 0.0 {
        return false;
    }
    // **CAD's text height is the height of a capital**, not the em size. Using
    // the em would draw every label about a third too small, which on a plan
    // full of 2.5 mm text is the difference between readable and not.
    let cap = face
        .capital_height()
        .filter(|c| *c > 0)
        .map(|c| c as f64)
        .unwrap_or(per_em * 0.7);
    let scale = height / cap;
    let (sin, cos) = rotation.sin_cos();

    let mut pen = 0.0_f64;
    for character in content.chars() {
        let Some(glyph) = face.glyph_index(character) else {
            // A character this font has no glyph for. Advancing by a space
            // keeps the rest of the line where it belongs instead of closing
            // up around the hole.
            pen += per_em * 0.5;
            continue;
        };

        let origin = Point::new(
            at.x + pen * scale * cos,
            at.y + pen * scale * sin,
        );
        let mut builder = Glyph {
            into,
            scale,
            at: origin,
            sin,
            cos,
            view,
            width,
            height: height_px,
        };
        face.outline_glyph(glyph, &mut builder);

        pen += face.glyph_hor_advance(glyph).unwrap_or(0) as f64;
    }

    true
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

        // Two paths per layer, because letters are filled and lines are
        // stroked. Kept apart rather than drawn per entity so the whole layer
        // is still two calls into the rasteriser rather than thousands.
        let mut lines = PathBuilder::new();
        let mut letters = PathBuilder::new();
        let mut any_line = false;
        let mut any_letter = false;

        for entity in drawing.entities.iter().filter(|e| e.layer as usize == id) {
            match &entity.shape {
                Shape::Text { at, height: size, rotation, content } => {
                    let Some(face) = style.face() else { continue };
                    // Below about four pixels a letter is a smudge, and a plan
                    // holds thousands of them. Left out until it would say
                    // something — which is what every CAD viewer does, and is
                    // why zooming in makes the labels appear.
                    if size * view.scale < 4.0 {
                        continue;
                    }
                    if text_path(
                        &face, content, *at, *size, *rotation, view, width, height, &mut letters,
                    ) {
                        any_letter = true;
                    }
                }
                other => {
                    if add(&mut lines, other, view, width, height) {
                        any_line = true;
                    }
                }
            }
        }

        if any_line {
            if let Some(path) = lines.finish() {
                into.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
            }
        }
        if any_letter {
            if let Some(path) = letters.finish() {
                into.fill_path(
                    &path,
                    &paint,
                    // Non-zero, not even-odd: a glyph's counters — the hole in
                    // an "o" — are wound against its outline, and even-odd
                    // would fill some letters solid and leave others hollow.
                    tiny_skia::FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
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
        // Handled by `draw`, which fills letters rather than stroking them.
        // Reaching here would draw a label in outline at wall weight.
        Shape::Text { .. } => false,

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
        // Either format, chosen by the name: the whole point is that what comes
        // out is the same drawing whichever door it came through.
        let drawing = if path.to_ascii_lowercase().ends_with(".dwg") {
            crate::drawing::dwg::read(std::path::Path::new(&path)).expect("parses")
        } else {
            let bytes = std::fs::read(&path).expect("readable");
            crate::drawing::dxf::read(&String::from_utf8_lossy(&bytes)).expect("parses")
        };

        const WIDE: u32 = 1400;
        const HIGH: u32 = 900;
        let mut sheet = Pixmap::new(WIDE, HIGH).expect("a canvas");
        let view = View::fitted(&drawing, WIDE, HIGH);
        let mut style = Style::default();
        // The app hands over its own font over the bridge; here it is read
        // straight off disk, so the picture is the one the phone would draw.
        if let Ok(name) = std::env::var("PAGIFY_DXF_FONT") {
            if let Ok(bytes) = std::fs::read(&name) {
                style.font = Some(std::sync::Arc::new(bytes));
            }
        }

        // Zoomed, so text large enough to be worth drawing actually is. At a
        // whole-sheet fit a 2.5 mm label is two pixels tall and is left out on
        // purpose, which is exactly the behaviour that needs looking past here.
        let mut view = view;
        if let Ok(by) = std::env::var("PAGIFY_DXF_ZOOM") {
            if let Ok(by) = by.parse::<f64>() {
                view.scale *= by;
            }
        }
        if let (Ok(x), Ok(y)) = (std::env::var("PAGIFY_DXF_AT_X"), std::env::var("PAGIFY_DXF_AT_Y")) {
            if let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>()) {
                view.centre = crate::drawing::model::Point::new(x, y);
            }
        }

        let started = std::time::Instant::now();
        draw(&drawing, &view, &style, &mut sheet);
        if std::env::var("PAGIFY_DXF_DUPES").is_ok() {
            let mut seen: std::collections::HashSet<(i64, i64, i64, i64)> = Default::default();
            let mut lines = 0usize;
            let mut repeats = 0usize;
            for entity in &drawing.entities {
                if let crate::drawing::model::Shape::Line { a, b } = entity.shape {
                    lines += 1;
                    let key = |p: crate::drawing::model::Point| {
                        ((p.x * 1000.0) as i64, (p.y * 1000.0) as i64)
                    };
                    let (ka, kb) = (key(a), key(b));
                    let ordered = if ka <= kb { (ka.0, ka.1, kb.0, kb.1) } else { (kb.0, kb.1, ka.0, ka.1) };
                    if !seen.insert(ordered) {
                        repeats += 1;
                    }
                }
            }
            eprintln!("{lines} lines, {repeats} of them drawn somewhere already");
        }

        let words = drawing
            .entities
            .iter()
            .filter(|e| matches!(e.shape, crate::drawing::model::Shape::Text { .. }))
            .count();
        println!(
            "{} shapes drawn in {:?}, {} pixels per unit, {words} of them text",
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

//! Drawing the markup layer over the page — build plan phase 4.
//!
//! The overlay is drawn from the *live* geometry every frame, not from the ink
//! that gets written on save. That is what makes a fillet show up the instant it
//! is applied, and it is why a circle is drawn as a circle here rather than as
//! the sixty-odd segments the PDF will eventually carry.

use cad_kernel::{Geom, Vec2};
use egui::{Color32, Painter, Pos2, Stroke};
use pagify_shell::markup::Layer;
use pagify_shell::page_space::AppPoint;
use pdf_core::Rotation;

/// Maps page points to screen position for one page — **turned views
/// included.**
///
/// A page laid out turned on screen (`rotate_view`, the strip's own retarget)
/// still reports everything about itself in the page's own upright points:
/// character boxes, runs, marks, signatures. The turn is the last step, and
/// it belongs here, in the one conversion every marking and every click goes
/// through — it was missing, and the report is exactly what that looks like:
/// "when i rotate, extract text and the similar tools behave as if the page
/// is still vertical, the highlights are wrong". Clicking a word selected
/// whatever sat at its *upright* position, and the boxes drawn over the words
/// were the upright ones.
#[derive(Debug, Clone, Copy)]
pub struct PageView {
    /// Where the **turned page's** top-left corner sits on screen.
    pub origin: Pos2,
    /// Screen points per page point.
    pub scale: f32,
    /// How the page is turned on screen.
    pub rotation: Rotation,
    /// The page's frame **as drawn**: `(w, h)` in points, already turned —
    /// the strip lays a rotated page out by its turned size, and the turn
    /// below needs to know which way that size runs.
    pub frame: (f32, f32),
}

impl PageView {
    /// A view of an upright page — what every caller with no turn to make
    /// wants, which is all of them but the canvas itself. The canvas builds
    /// its own literal (it has the turn and the frame to put in); this is
    /// what everything else, tests included, builds a view with.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn upright(origin: Pos2, scale: f32) -> Self {
        PageView { origin, scale, rotation: Rotation::None, frame: (0.0, 0.0) }
    }

    /// The same point in the **turned page's** own top-left coordinates:
    /// page points in, page points out, only turned.
    ///
    /// Each quarter turn is its own mapping, corners pinned: a clockwise
    /// quarter takes the page's top edge to the drawn image's right edge, so
    /// its top-left corner lands at the drawn top-right.
    fn turned(&self, p: AppPoint) -> AppPoint {
        let (w, h) = (f64::from(self.frame.0), f64::from(self.frame.1));
        match self.rotation {
            Rotation::None => p,
            Rotation::Clockwise90 => AppPoint::new(w - p.y, p.x),
            Rotation::Clockwise180 => AppPoint::new(w - p.x, h - p.y),
            Rotation::Clockwise270 => AppPoint::new(p.y, h - p.x),
        }
    }

    /// [`Self::turned`]'s own inverse.
    fn unturned(&self, p: AppPoint) -> AppPoint {
        let (w, h) = (f64::from(self.frame.0), f64::from(self.frame.1));
        match self.rotation {
            Rotation::None => p,
            Rotation::Clockwise90 => AppPoint::new(p.y, w - p.x),
            Rotation::Clockwise180 => AppPoint::new(w - p.x, h - p.y),
            Rotation::Clockwise270 => AppPoint::new(h - p.y, p.x),
        }
    }

    pub fn to_screen(&self, p: AppPoint) -> Pos2 {
        let p = self.turned(p);
        Pos2::new(
            self.origin.x + p.x as f32 * self.scale,
            self.origin.y + p.y as f32 * self.scale,
        )
    }

    pub fn to_page(&self, p: Pos2) -> AppPoint {
        let at = AppPoint::new(
            ((p.x - self.origin.x) / self.scale) as f64,
            ((p.y - self.origin.y) / self.scale) as f64,
        );
        self.unturned(at)
    }

    /// A page-space rectangle's own box on screen — **normalised**, because a
    /// quarter turn swaps which of its two corners is the screen's top-left.
    ///
    /// The two mapped corners are opposite corners of an axis-aligned rect in
    /// both spaces, but at 90°/270° the page's top-left corner lands at the
    /// screen's bottom-left or top-right, so feeding both to
    /// `Rect::from_min_max` gives an inverted (negative-size) box. Every
    /// overlay drawn from a page rect — selection outlines, highlight washes,
    /// editors, previews — goes through this, not straight to `to_screen`.
    pub fn screen_rect(&self, rect: &pdf_core::document::Rect) -> egui::Rect {
        self.screen_rect_of(rect.left, rect.top, rect.right, rect.bottom)
    }

    /// [`Self::screen_rect`] for a box that is not a `pdf_core` rect — the
    /// reader's own character-line boxes ([`pagify_shell::reader::Rect`]) are
    /// the same four f32 fields in a different type, and the highlight washes
    /// are built from them.
    pub fn screen_rect_of(&self, left: f32, top: f32, right: f32, bottom: f32) -> egui::Rect {
        egui::Rect::from_two_pos(
            self.to_screen(AppPoint::new(f64::from(left), f64::from(top))),
            self.to_screen(AppPoint::new(f64::from(right), f64::from(bottom))),
        )
    }
}

/// How finely curves are drawn. Screen-space, so a circle stays smooth when
/// zoomed in rather than turning into a visible polygon.
fn steps_for(radius_screen: f32) -> usize {
    ((radius_screen * 0.7) as usize).clamp(12, 480)
}

pub fn draw_layer(painter: &Painter, layer: &Layer, view: PageView, plain: Color32, chosen: Color32) {
    for (index, object) in layer.objects().iter().enumerate() {
        // A hatch has no shape of its own to draw — see `Layer::is_filled`,
        // consulted below to fill its boundary before that boundary's own
        // stroke.
        if matches!(object.geom, Geom::Hatch(_)) {
            continue;
        }
        let selected = layer.selection().contains(&index);
        // An object nobody has given its own colour still follows this
        // page's pen — `Style::default()` is `Color::ByLayer`, and this app
        // has no layer of its own for that to resolve against.
        let own_colour = (!matches!(object.style.color, cad_kernel::Color::ByLayer))
            .then(|| layer.resolved_color(index))
            .flatten()
            .map(|(r, g, b)| Color32::from_rgb(r, g, b));
        let colour = if selected { chosen } else { own_colour.unwrap_or(plain) };
        // A shape with no lineweight of its own still follows the same
        // selection-highlight width as before; one that has been given a
        // real thickness is drawn at it (mm converted through points to
        // screen pixels), only ever thickened, never thinned, by selection.
        let default_width = if selected { 2.5 } else { 1.6 };
        let width = match layer.resolved_lineweight_mm(index) {
            Some(mm) => {
                let px = mm * 72.0 / 25.4 * view.scale;
                if selected { px.max(2.5) } else { px }
            }
            None => default_width,
        };
        let stroke = Stroke::new(width, colour);
        if layer.is_filled(index) {
            fill_geom(painter, &object.geom, layer, view, colour);
        }
        draw_geom(painter, &object.geom, layer, view, stroke);

        // An arrowhead is not a shape of its own — see `Layer::arrow_ends` —
        // so it is painted here, alongside the line it belongs to, rather
        // than inside `draw_geom`'s per-kind match. Filled, not stroked, so
        // its tip stays sharp no matter how thick the line itself is drawn
        // — see `arrowhead_triangle`'s own doc.
        if let Geom::Line(l) = &object.geom {
            let (start, end) = layer.arrow_ends(index);
            if start || end {
                let space = layer.space();
                let at = |v: Vec2| view.to_screen(space.from_kernel(v));
                let mut ends = Vec::new();
                if start { ends.push((l.b, l.a)); }
                if end { ends.push((l.a, l.b)); }
                for (from, to) in ends {
                    if let Some(tri) = pagify_shell::commit::arrowhead_triangle(from, to) {
                        let points: Vec<Pos2> = tri.iter().map(|&v| at(v)).collect();
                        painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
                    }
                }
            }
        }
    }
}

/// Copies of `objects` moved by `by` (kernel space), outlined in `colour` — the
/// shapes of a paste that has been picked up and not yet put down. `layer` only
/// gives the page space the shapes are drawn through.
pub fn draw_ghost(
    painter: &Painter,
    objects: &[(cad_kernel::DObject, bool)],
    layer: &Layer,
    view: PageView,
    by: Vec2,
    colour: Color32,
) {
    for (object, filled) in objects {
        let moved = object.translated(by);
        if *filled {
            fill_geom(painter, &moved.geom, layer, view, colour.gamma_multiply(0.5));
        }
        draw_geom(painter, &moved.geom, layer, view, Stroke::new(1.6, colour));
    }
}

/// The solid interior a filled Rectangle or Circle carries, painted before
/// its own outline stroke — the live match for the fill `commit_page` writes
/// on save, so drawing one does not look identical to leaving it hollow until
/// the document is saved and reopened.
fn fill_geom(painter: &Painter, geom: &Geom, layer: &Layer, view: PageView, colour: Color32) {
    let space = layer.space();
    let at = |v: Vec2| view.to_screen(space.from_kernel(v));

    match geom {
        Geom::Circle(c) => {
            let radius = (c.radius as f32) * view.scale;
            let steps = steps_for(radius);
            let points: Vec<Pos2> = (0..steps)
                .map(|i| {
                    let t = std::f64::consts::TAU * (i as f64 / steps as f64);
                    at(Vec2::new(c.center.x + c.radius * t.cos(), c.center.y + c.radius * t.sin()))
                })
                .collect();
            painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
        }
        // Only ever a rectangle in practice — the one closed polyline the
        // Draw tab's fill toggle applies to — and a rectangle is convex, so
        // this needs no general polygon tessellation to fill correctly.
        Geom::Polyline(p) if p.closed => {
            let points: Vec<Pos2> = p.vertices.iter().map(|v| at(v.pos)).collect();
            painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
        }
        _ => {}
    }
}

fn draw_geom(painter: &Painter, geom: &Geom, layer: &Layer, view: PageView, stroke: Stroke) {
    let space = layer.space();
    let at = |v: Vec2| view.to_screen(space.from_kernel(v));

    match geom {
        Geom::Line(l) => {
            painter.line_segment([at(l.a), at(l.b)], stroke);
        }

        Geom::Circle(c) => {
            let centre = at(c.center);
            let radius = (c.radius as f32) * view.scale;
            // Drawn as a path rather than `circle_stroke` so it matches exactly
            // what the arc code produces and cannot drift from it.
            let steps = steps_for(radius);
            let points: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let t = std::f64::consts::TAU * (i as f64 / steps as f64);
                    at(Vec2::new(c.center.x + c.radius * t.cos(), c.center.y + c.radius * t.sin()))
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            let _ = centre;
        }

        Geom::Arc(a) => {
            let radius = (a.radius as f32) * view.scale;
            let steps = steps_for(radius);
            let points: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let t = a.start_angle + a.sweep_angle * (i as f64 / steps as f64);
                    at(Vec2::new(a.center.x + a.radius * t.cos(), a.center.y + a.radius * t.sin()))
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
        }

        Geom::Polyline(p) => {
            let mut points: Vec<Pos2> = p.vertices.iter().map(|v| at(v.pos)).collect();
            if p.closed {
                if let Some(first) = points.first().copied() {
                    points.push(first);
                }
            }
            painter.add(egui::Shape::line(points, stroke));
        }

        Geom::Spline(s) => {
            let points: Vec<Pos2> = s.tessellate(64).into_iter().map(at).collect();
            painter.add(egui::Shape::line(points, stroke));
        }

        Geom::Point(p) => {
            let c = at(p.location);
            let arm = 3.0;
            painter.line_segment([Pos2::new(c.x - arm, c.y), Pos2::new(c.x + arm, c.y)], stroke);
            painter.line_segment([Pos2::new(c.x, c.y - arm), Pos2::new(c.x, c.y + arm)], stroke);
        }

        // Everything the overlay cannot draw yet is left undrawn rather than
        // approximated. A mark drawn wrongly is worse than one not drawn: it
        // gets trusted.
        _ => {}
    }
}

/// A snap badge — a small marker whose shape says which kind of snap it is.
pub fn draw_snap(painter: &Painter, at: Pos2, kind: cad_kernel::SnapKind, colour: Color32) {
    use cad_kernel::SnapKind::*;
    let stroke = Stroke::new(1.5, colour);
    let r = 5.0;

    match kind {
        End => {
            painter.rect_stroke(
                egui::Rect::from_center_size(at, egui::vec2(r * 2.0, r * 2.0)),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
        }
        Mid => {
            painter.add(egui::Shape::closed_line(
                vec![
                    Pos2::new(at.x, at.y - r),
                    Pos2::new(at.x + r, at.y + r),
                    Pos2::new(at.x - r, at.y + r),
                ],
                stroke,
            ));
        }
        Cen => {
            painter.circle_stroke(at, r, stroke);
        }
        Qua => {
            painter.add(egui::Shape::closed_line(
                vec![
                    Pos2::new(at.x, at.y - r),
                    Pos2::new(at.x + r, at.y),
                    Pos2::new(at.x, at.y + r),
                    Pos2::new(at.x - r, at.y),
                ],
                stroke,
            ));
        }
        Int => {
            painter.line_segment([Pos2::new(at.x - r, at.y - r), Pos2::new(at.x + r, at.y + r)], stroke);
            painter.line_segment([Pos2::new(at.x - r, at.y + r), Pos2::new(at.x + r, at.y - r)], stroke);
        }
        _ => {
            painter.circle_stroke(at, r * 0.7, stroke);
        }
    }
}

#[cfg(test)]
mod tests {
    use pdf_core::Rotation;

    /// **The turn is the map's own last step**, corners pinned: a clockwise
    /// quarter takes the page's top edge to the drawn image's right edge, so
    /// its top-left corner lands at the drawn top-right. This is the mapping
    /// every click and every highlight goes through — see [`PageView`]'s own
    /// doc for the report that came of missing it.
    #[test]
    fn a_turned_view_maps_page_points_through_the_turn() {
        // A portrait page drawn turned: the frame is the page's size with
        // its sides swapped.
        let turned_view = |rotation| PageView {
            origin: Pos2::new(10.0, 20.0),
            scale: 2.0,
            rotation,
            frame: (792.0, 612.0),
        };
        let upright = PageView::upright(Pos2::new(10.0, 20.0), 2.0);

        let corner = AppPoint::new(0.0, 0.0);
        assert_eq!(upright.to_screen(corner), Pos2::new(10.0, 20.0));
        assert_eq!(
            turned_view(Rotation::Clockwise90).to_screen(corner),
            Pos2::new(10.0 + 792.0 * 2.0, 20.0),
            "the page's top-left lands at the drawn top-right"
        );
        assert_eq!(
            turned_view(Rotation::Clockwise180).to_screen(corner),
            Pos2::new(10.0 + 792.0 * 2.0, 20.0 + 612.0 * 2.0)
        );
        assert_eq!(
            turned_view(Rotation::Clockwise270).to_screen(corner),
            Pos2::new(10.0, 20.0 + 612.0 * 2.0),
            "the page's top-left lands at the drawn bottom-left"
        );

        // And every turn is its own inverse, exactly — a click has to come
        // back as the point the drawing put under it.
        for rotation in [
            Rotation::None,
            Rotation::Clockwise90,
            Rotation::Clockwise180,
            Rotation::Clockwise270,
        ] {
            let view = turned_view(rotation);
            for at in [
                AppPoint::new(0.0, 0.0),
                AppPoint::new(100.0, 300.0),
                AppPoint::new(612.0, 792.0),
            ] {
                let back = view.to_page(view.to_screen(at));
                assert!(
                    (back.x - at.x).abs() < 0.01 && (back.y - at.y).abs() < 0.01,
                    "{rotation:?}: {at:?} came back as {back:?}"
                );
            }
        }
    }

    use super::*;

    /// **A page rect's screen box stays right-side-up through a turn.** At
    /// 90°/270° the page's top-left corner lands at the screen's bottom-left or
    /// top-right, so building the box from the two mapped corners with
    /// `from_min_max` gave an inverted (negative-size) rect — the reported
    /// "highlights are wrong" after rotating the view.
    #[test]
    fn a_turned_view_normalises_a_page_rect_instead_of_inverting_it() {
        let view = |rotation| PageView {
            origin: Pos2::new(10.0, 20.0),
            scale: 1.0,
            rotation,
            frame: (792.0, 612.0),
        };
        let rect = pdf_core::document::Rect { left: 0.0, top: 0.0, right: 100.0, bottom: 50.0 };
        for rotation in [
            Rotation::None,
            Rotation::Clockwise90,
            Rotation::Clockwise180,
            Rotation::Clockwise270,
        ] {
            let screen = view(rotation).screen_rect(&rect);
            assert!(
                screen.width() > 0.0 && screen.height() > 0.0,
                "{rotation:?}: an inverted box: {screen:?}"
            );
            // A quarter turn swaps the box's sides on screen; a half turn
            // keeps them. Either way the box is right-side-up, which is the
            // point.
            let (want_w, want_h) = match rotation {
                Rotation::Clockwise90 | Rotation::Clockwise270 => (50.0, 100.0),
                _ => (100.0, 50.0),
            };
            assert!(
                (screen.width() - want_w).abs() < 0.01
                    && (screen.height() - want_h).abs() < 0.01,
                "{rotation:?}: a turned box is not its turned size, got {screen:?}"
            );
        }
    }

    #[test]
    fn the_page_view_round_trips_a_point() {
        let view = PageView::upright(Pos2::new(37.0, 91.0), 1.75);
        let start = AppPoint::new(123.0, 456.0);
        let back = view.to_page(view.to_screen(start));

        assert!((back.x - start.x).abs() < 1e-3, "{back:?}");
        assert!((back.y - start.y).abs() < 1e-3, "{back:?}");
    }

    #[test]
    fn curves_get_more_segments_the_larger_they_are_drawn() {
        assert!(steps_for(500.0) > steps_for(20.0));
        assert!(steps_for(0.0) >= 12, "a tiny circle still needs to look round");
        assert!(steps_for(100_000.0) <= 480, "and an enormous one must stay bounded");
    }

    /// **Reported from use: turning Fill on and drawing a circle looked
    /// exactly like drawing a hollow one.** The live overlay is what a
    /// person actually sees while drawing; the fill this app writes on save
    /// is invisible until then unless the overlay paints one too.
    #[test]
    fn a_filled_circle_paints_a_solid_interior_not_only_an_outline() {
        use cad_kernel::{Circle, Hatch, HatchPattern};
        use pagify_shell::markup::Layer;

        let mut layer = Layer::new(800.0);
        let index = layer.add(Geom::Circle(Circle { center: Vec2::new(100.0, 100.0), radius: 40.0 }));
        let handle = layer.objects()[index].handle;
        layer.add(Geom::Hatch(Hatch { boundary_handles: vec![handle], pattern: HatchPattern::Solid }));

        let ctx = egui::Context::default();
        let layer_id = egui::LayerId::new(egui::Order::Middle, egui::Id::new("fill-test"));
        let painter = Painter::new(ctx.clone(), layer_id, egui::Rect::EVERYTHING);
        let view = PageView::upright(Pos2::ZERO, 1.0);
        let colour = Color32::from_rgb(200, 40, 90);

        draw_layer(&painter, &layer, view, colour, colour);

        let filled = ctx.graphics_mut(|g| {
            g.entry(layer_id)
                .all_entries()
                .any(|cs| matches!(&cs.shape, egui::Shape::Path(p) if p.fill == colour))
        });
        assert!(filled, "the circle was drawn hollow — no filled path reached the painter");
    }

    /// The same, for a filled rectangle — the other shape Fill applies to.
    #[test]
    fn a_filled_rectangle_paints_a_solid_interior_not_only_an_outline() {
        use cad_kernel::{Hatch, HatchPattern, PolyVertex, Polyline};
        use pagify_shell::markup::Layer;

        let mut layer = Layer::new(800.0);
        let corners = [
            Vec2::new(10.0, 10.0),
            Vec2::new(60.0, 10.0),
            Vec2::new(60.0, 40.0),
            Vec2::new(10.0, 40.0),
        ];
        let index = layer.add(Geom::Polyline(Polyline {
            vertices: corners.iter().map(|v| PolyVertex { pos: *v, bulge: 0.0 }).collect(),
            closed: true,
            widths: Vec::new(),
        }));
        let handle = layer.objects()[index].handle;
        layer.add(Geom::Hatch(Hatch { boundary_handles: vec![handle], pattern: HatchPattern::Solid }));

        let ctx = egui::Context::default();
        let layer_id = egui::LayerId::new(egui::Order::Middle, egui::Id::new("fill-test-rect"));
        let painter = Painter::new(ctx.clone(), layer_id, egui::Rect::EVERYTHING);
        let view = PageView::upright(Pos2::ZERO, 1.0);
        let colour = Color32::from_rgb(40, 160, 90);

        draw_layer(&painter, &layer, view, colour, colour);

        let filled = ctx.graphics_mut(|g| {
            g.entry(layer_id)
                .all_entries()
                .any(|cs| matches!(&cs.shape, egui::Shape::Path(p) if p.fill == colour))
        });
        assert!(filled, "the rectangle was drawn hollow — no filled path reached the painter");
    }

    /// **Part of the properties panel's promise: a shape given its own
    /// colour is drawn in it, not the page's pen.** An untouched shape
    /// keeps following the page's pen exactly as before — see
    /// `a_hollow_circle_paints_no_fill`'s sibling assumption that nothing
    /// here silently repaints an object nobody touched.
    #[test]
    fn a_shape_given_its_own_colour_is_drawn_in_it_not_the_pages_pen() {
        use pagify_shell::markup::Layer;

        let mut layer = Layer::new(800.0);
        layer.add(Geom::Circle(cad_kernel::Circle { center: Vec2::new(100.0, 100.0), radius: 40.0 }));
        assert!(layer.set_color(0, (10, 200, 90)));

        let ctx = egui::Context::default();
        let layer_id = egui::LayerId::new(egui::Order::Middle, egui::Id::new("own-colour-test"));
        let painter = Painter::new(ctx.clone(), layer_id, egui::Rect::EVERYTHING);
        let view = PageView::upright(Pos2::ZERO, 1.0);
        // Deliberately different from the shape's own colour, so drawing in
        // the page's pen instead would be caught rather than coincide.
        let pen = Color32::from_rgb(200, 40, 90);
        let own = Color32::from_rgb(10, 200, 90);

        draw_layer(&painter, &layer, view, pen, pen);

        let drawn_in_own_colour = ctx.graphics_mut(|g| {
            g.entry(layer_id).all_entries().any(|cs| match &cs.shape {
                egui::Shape::Path(p) => {
                    matches!(p.stroke.color, egui::epaint::ColorMode::Solid(c) if c == own)
                }
                _ => false,
            })
        });
        assert!(drawn_in_own_colour, "the shape was drawn in the page's pen, not its own colour");
    }

    /// A hollow shape — Fill left off — must not gain an interior on its own.
    #[test]
    fn a_hollow_circle_paints_no_fill() {
        use pagify_shell::markup::Layer;

        let mut layer = Layer::new(800.0);
        layer.add(Geom::Circle(cad_kernel::Circle { center: Vec2::new(100.0, 100.0), radius: 40.0 }));

        let ctx = egui::Context::default();
        let layer_id = egui::LayerId::new(egui::Order::Middle, egui::Id::new("hollow-test"));
        let painter = Painter::new(ctx.clone(), layer_id, egui::Rect::EVERYTHING);
        let view = PageView::upright(Pos2::ZERO, 1.0);
        let colour = Color32::from_rgb(200, 40, 90);

        draw_layer(&painter, &layer, view, colour, colour);

        let filled = ctx.graphics_mut(|g| {
            g.entry(layer_id)
                .all_entries()
                .any(|cs| matches!(&cs.shape, egui::Shape::Path(p) if p.fill == colour))
        });
        assert!(!filled, "a hollow circle was filled anyway");
    }
}

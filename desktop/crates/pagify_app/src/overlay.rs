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

/// Maps page points to screen position for one page.
#[derive(Debug, Clone, Copy)]
pub struct PageView {
    /// Where the page's top-left corner sits on screen.
    pub origin: Pos2,
    /// Screen points per page point.
    pub scale: f32,
}

impl PageView {
    pub fn to_screen(&self, p: AppPoint) -> Pos2 {
        Pos2::new(
            self.origin.x + p.x as f32 * self.scale,
            self.origin.y + p.y as f32 * self.scale,
        )
    }

    pub fn to_page(&self, p: Pos2) -> AppPoint {
        AppPoint::new(
            ((p.x - self.origin.x) / self.scale) as f64,
            ((p.y - self.origin.y) / self.scale) as f64,
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
        let selected = layer.selection().contains(&index);
        let stroke = Stroke::new(if selected { 2.5 } else { 1.6 }, if selected { chosen } else { plain });
        draw_geom(painter, &object.geom, layer, view, stroke);
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
    use super::*;

    #[test]
    fn the_page_view_round_trips_a_point() {
        let view = PageView { origin: Pos2::new(37.0, 91.0), scale: 1.75 };
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
}

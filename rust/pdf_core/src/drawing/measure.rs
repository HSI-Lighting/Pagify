//! Measuring between two points on a drawing, and snapping to the ones that
//! mean something.
//!
//! **A measurement is only worth anything if it lands on the geometry.** A
//! finger on a phone covers several millimetres of screen and rather more of
//! the drawing; asked to measure a wall, somebody taps *near* its end and the
//! answer comes back a few hundred millimetres short, which is worse than no
//! answer because it looks like one. So a tap does not measure where it landed
//! — it measures the nearest thing the drawing actually says is there.
//!
//! The order matters and is CAD's own: an endpoint beats an intersection, an
//! intersection beats a midpoint, and a centre comes last. Two of them are
//! often within a few pixels of each other near a corner, and picking the
//! nearest rather than the most specific gives a different answer each time a
//! finger lands a pixel to the left.

use super::model::{Drawing, Point, Shape};

/// What a tap found to hold on to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snap {
    /// The end of a line, an arc or a run of polyline — and a placed point.
    Endpoint,
    /// Where two lines cross, whether or not anything is drawn at the crossing.
    Intersection,
    /// Halfway along a straight run or an arc.
    Midpoint,
    /// The middle of a circle, an arc or an ellipse.
    Centre,
    /// Nothing near enough. The measurement is where the finger went.
    Free,
}

impl Snap {
    /// What the screen calls it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Endpoint => "endpoint",
            Self::Intersection => "intersection",
            Self::Midpoint => "midpoint",
            Self::Centre => "centre",
            Self::Free => "point",
        }
    }

    /// Lower is better. **Not distance** — see the module note.
    fn rank(&self) -> u8 {
        match self {
            Self::Endpoint => 0,
            Self::Intersection => 1,
            Self::Midpoint => 2,
            Self::Centre => 3,
            Self::Free => 4,
        }
    }
}

/// Where a tap ended up, and why.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapped {
    pub at: Point,
    pub kind: Snap,
}

/// The nearest thing worth holding on to, within `reach` of `near`.
///
/// `reach` is in drawing units and comes from how large the finger is on
/// screen, so the same tap snaps over a smaller piece of drawing as somebody
/// zooms in — which is what makes it possible to pick one end of a 40 mm
/// fitting apart from the other.
pub fn snap(drawing: &Drawing, near: Point, reach: f64) -> Snapped {
    let mut best: Option<(Snap, f64, Point)> = None;
    let mut consider = |kind: Snap, at: Point| {
        let away = (at.x - near.x).hypot(at.y - near.y);
        if away > reach {
            return;
        }
        let better = match &best {
            None => true,
            // A more specific kind wins outright; within one kind, the nearest.
            Some((was, distance, _)) => {
                kind.rank() < was.rank() || (kind.rank() == was.rank() && away < *distance)
            }
        };
        if better {
            best = Some((kind, away, at));
        }
    };

    // The straight runs near the tap, kept for the intersection pass below.
    let mut nearby: Vec<(Point, Point)> = Vec::new();

    for entity in &drawing.entities {
        // Layers turned off are not there to be measured against. Somebody who
        // hid the grid did it so they could work on what is left.
        if drawing.layers.get(entity.layer as usize).is_some_and(|l| !l.visible) {
            continue;
        }

        match &entity.shape {
            Shape::Line { a, b } => {
                consider(Snap::Endpoint, *a);
                consider(Snap::Endpoint, *b);
                consider(Snap::Midpoint, middle(*a, *b));
                keep_near(&mut nearby, *a, *b, near, reach);
            }

            Shape::Marker { at } => consider(Snap::Endpoint, *at),

            Shape::Arc { centre, radius, start, sweep } => {
                consider(Snap::Centre, *centre);
                let on = |angle: f64| {
                    Point::new(centre.x + radius * angle.cos(), centre.y + radius * angle.sin())
                };
                // A whole circle has no ends to speak of; an arc does.
                if (sweep.abs() - std::f64::consts::TAU).abs() > 1e-9 {
                    consider(Snap::Endpoint, on(*start));
                    consider(Snap::Endpoint, on(start + sweep));
                }
                consider(Snap::Midpoint, on(start + sweep / 2.0));
            }

            Shape::Ellipse { centre, .. } => consider(Snap::Centre, *centre),

            Shape::Polyline { vertices, closed } => {
                let last = if *closed { vertices.len() } else { vertices.len().saturating_sub(1) };
                for (at, vertex) in vertices.iter().enumerate() {
                    consider(Snap::Endpoint, vertex.at);
                    if at < last {
                        let next = vertices[(at + 1) % vertices.len()].at;
                        // Straight runs only. A bowed one's midpoint is not
                        // halfway between its ends, and offering the chord's
                        // middle as "midpoint" would be an answer about a line
                        // that is not on the drawing.
                        if vertex.bulge.abs() < 1e-12 {
                            consider(Snap::Midpoint, middle(vertex.at, next));
                            keep_near(&mut nearby, vertex.at, next, near, reach);
                        }
                    }
                }
            }

            Shape::Text { .. } => {}
        }
    }

    // **Crossings, which are usually not drawn.** Two walls meeting at a corner
    // have no vertex where they cross — each runs past the other — so without
    // this the one point somebody most wants to measure from is the one point
    // there is nothing to snap to.
    for first in 0..nearby.len() {
        for second in (first + 1)..nearby.len() {
            if let Some(at) = crossing(nearby[first], nearby[second]) {
                consider(Snap::Intersection, at);
            }
        }
    }

    match best {
        Some((kind, _, at)) => Snapped { at, kind },
        None => Snapped { at: near, kind: Snap::Free },
    }
}

fn middle(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

/// Keep a segment only if it passes close enough to matter.
///
/// The bound on the intersection pass: without it a plan of twenty-seven
/// thousand shapes would be a hundred million pairs to cross, per tap.
fn keep_near(into: &mut Vec<(Point, Point)>, a: Point, b: Point, near: Point, reach: f64) {
    if into.len() >= MOST_NEARBY {
        return;
    }
    // Distance from the tap to the segment, which is what "near" means for
    // something long — a wall ten metres away at one end can still pass under
    // the finger.
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx.hypot(dy);
    let away = if length < 1e-12 {
        (a.x - near.x).hypot(a.y - near.y)
    } else {
        let along = (((near.x - a.x) * dx + (near.y - a.y) * dy) / (length * length)).clamp(0.0, 1.0);
        let on = Point::new(a.x + dx * along, a.y + dy * along);
        (on.x - near.x).hypot(on.y - near.y)
    };
    if away <= reach {
        into.push((a, b));
    }
}

/// How many segments the intersection pass will look at.
///
/// Reached only where a tap lands on something like a hatched region, where
/// the answer is a thicket of crossings none of which anybody wanted.
const MOST_NEARBY: usize = 64;

/// Where two segments cross, if they do within their own lengths.
fn crossing(one: (Point, Point), other: (Point, Point)) -> Option<Point> {
    let (p, q) = one;
    let (r, s) = other;
    let (px, py) = (q.x - p.x, q.y - p.y);
    let (rx, ry) = (s.x - r.x, s.y - r.y);

    let denominator = px * ry - py * rx;
    // Parallel, or one of them has no length. Overlapping collinear segments
    // have no single crossing to offer, so they are left alone rather than
    // answered arbitrarily.
    if denominator.abs() < 1e-12 {
        return None;
    }

    let along = ((r.x - p.x) * ry - (r.y - p.y) * rx) / denominator;
    let across = ((r.x - p.x) * py - (r.y - p.y) * px) / denominator;
    if !(0.0..=1.0).contains(&along) || !(0.0..=1.0).contains(&across) {
        return None;
    }

    Some(Point::new(p.x + px * along, p.y + py * along))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::model::{Entity, Layer, Vertex};

    fn drawing_of(shapes: Vec<Shape>) -> Drawing {
        Drawing {
            entities: shapes.into_iter().map(|shape| Entity { layer: 0, shape }).collect(),
            layers: vec![Layer { name: "0".into(), colour: [255, 255, 255], visible: true }],
            ..Drawing::default()
        }
    }

    fn line(ax: f64, ay: f64, bx: f64, by: f64) -> Shape {
        Shape::Line { a: Point::new(ax, ay), b: Point::new(bx, by) }
    }

    /// A tap near the end of a wall measures from the end of the wall.
    #[test]
    fn a_tap_near_an_end_takes_the_end() {
        let drawing = drawing_of(vec![line(0.0, 0.0, 100.0, 0.0)]);

        let found = snap(&drawing, Point::new(97.0, 3.0), 10.0);

        assert_eq!(Snap::Endpoint, found.kind);
        assert_eq!(Point::new(100.0, 0.0), found.at);
    }

    /// **The most specific answer wins, not the nearest one.**
    ///
    /// Near a corner an endpoint and a midpoint are often a few pixels apart.
    /// Taking whichever is closer means the answer changes when the finger
    /// lands a pixel to the left, and neither answer is wrong enough to look
    /// wrong — which is exactly how a wrong measurement gets written down.
    #[test]
    fn an_endpoint_beats_a_nearer_midpoint() {
        // The midpoint of the short line is nearer the tap than the long
        // line's end, and the end is what should win.
        let drawing = drawing_of(vec![line(0.0, 0.0, 100.0, 0.0), line(90.0, 4.0, 100.0, 4.0)]);

        let found = snap(&drawing, Point::new(96.0, 3.0), 10.0);

        assert_eq!(Snap::Endpoint, found.kind, "{found:?}");
    }

    /// A crossing is offered even though nothing is drawn there.
    ///
    /// Two walls that run past each other have no vertex at the corner, and
    /// that corner is the point somebody most often wants to measure from.
    #[test]
    fn two_lines_that_cross_offer_the_crossing() {
        let drawing = drawing_of(vec![line(0.0, 0.0, 100.0, 0.0), line(50.0, -50.0, 50.0, 50.0)]);

        let found = snap(&drawing, Point::new(52.0, 3.0), 10.0);

        assert_eq!(Snap::Intersection, found.kind, "{found:?}");
        assert!((found.at.x - 50.0).abs() < 1e-9 && found.at.y.abs() < 1e-9, "{found:?}");
    }

    /// Lines that only cross if extended are not a crossing.
    #[test]
    fn lines_that_miss_each_other_do_not_cross() {
        let drawing = drawing_of(vec![line(0.0, 0.0, 10.0, 0.0), line(50.0, -50.0, 50.0, 50.0)]);

        let found = snap(&drawing, Point::new(30.0, 0.0), 25.0);

        assert_ne!(Snap::Intersection, found.kind, "{found:?}");
    }

    #[test]
    fn halfway_along_a_wall_is_the_midpoint() {
        let drawing = drawing_of(vec![line(0.0, 0.0, 100.0, 0.0)]);

        let found = snap(&drawing, Point::new(51.0, 2.0), 10.0);

        assert_eq!(Snap::Midpoint, found.kind);
        assert_eq!(Point::new(50.0, 0.0), found.at);
    }

    /// A circle offers its centre, which is not on the circle at all.
    #[test]
    fn a_circle_offers_its_centre() {
        let drawing = drawing_of(vec![Shape::Arc {
            centre: Point::new(20.0, 20.0),
            radius: 15.0,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        }]);

        let found = snap(&drawing, Point::new(22.0, 21.0), 10.0);

        assert_eq!(Snap::Centre, found.kind);
        assert_eq!(Point::new(20.0, 20.0), found.at);
    }

    /// Every corner of a polyline is an end to hold on to.
    #[test]
    fn a_polyline_corner_is_an_endpoint() {
        let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 80.0)];
        let drawing = drawing_of(vec![Shape::Polyline {
            vertices: corners
                .iter()
                .map(|(x, y)| Vertex { at: Point::new(*x, *y), bulge: 0.0 })
                .collect(),
            closed: false,
        }]);

        let found = snap(&drawing, Point::new(98.0, 3.0), 10.0);

        assert_eq!(Snap::Endpoint, found.kind);
        assert_eq!(Point::new(100.0, 0.0), found.at);
    }

    /// **A bowed segment offers no midpoint.**
    ///
    /// Halfway between the ends of a curve is not on the curve. Offering it
    /// would be an answer about a straight line that is not on the drawing —
    /// and it would look right, because it is in about the right place.
    #[test]
    fn a_curved_segment_does_not_offer_a_chord_midpoint() {
        let drawing = drawing_of(vec![Shape::Polyline {
            vertices: vec![
                Vertex { at: Point::new(0.0, 0.0), bulge: 0.5 },
                Vertex { at: Point::new(100.0, 0.0), bulge: 0.0 },
            ],
            closed: false,
        }]);

        let found = snap(&drawing, Point::new(50.0, 0.0), 5.0);

        assert_eq!(Snap::Free, found.kind, "{found:?}");
    }

    /// Nothing near enough means the measurement stands where it was put.
    #[test]
    fn a_tap_in_open_space_measures_where_it_landed() {
        let drawing = drawing_of(vec![line(0.0, 0.0, 100.0, 0.0)]);

        let found = snap(&drawing, Point::new(50.0, 500.0), 10.0);

        assert_eq!(Snap::Free, found.kind);
        assert_eq!(Point::new(50.0, 500.0), found.at);
    }

    /// A layer that has been turned off is not there to be measured against.
    #[test]
    fn a_hidden_layer_offers_nothing() {
        let mut drawing = drawing_of(vec![line(0.0, 0.0, 100.0, 0.0)]);
        drawing.layers[0].visible = false;

        assert_eq!(Snap::Free, snap(&drawing, Point::new(99.0, 1.0), 10.0).kind);
    }

    /// The reach is what makes one end of a small fitting pickable from the
    /// other: outside it, nothing is offered.
    #[test]
    fn reach_bounds_what_can_be_caught() {
        let drawing = drawing_of(vec![line(0.0, 0.0, 100.0, 0.0)]);

        assert_eq!(Snap::Endpoint, snap(&drawing, Point::new(95.0, 0.0), 10.0).kind);
        assert_eq!(Snap::Free, snap(&drawing, Point::new(95.0, 0.0), 2.0).kind);
    }
}

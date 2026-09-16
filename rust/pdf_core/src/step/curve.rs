//! Turning an edge's curve into a chain of points, once.
//!
//! # The first trap: every edge belongs to two faces
//!
//! In a closed solid each edge is shared by exactly two faces. If both faces
//! discretise it themselves, the two chains differ — not by much, but by
//! enough: a different number of segments, or the same number rounded
//! differently, and the two faces no longer meet. The result is a hairline gap
//! along every seam of the model, through which the inside of the part is
//! visible. It reads as a rendering fault and is a geometry one.
//!
//! So an edge is discretised **once**, cached by [`EdgeId`], and both faces
//! consume the same points — the second face taking them in reverse. Reversing
//! a stored chain is exact; recomputing it backwards is not.
//!
//! # How fine is fine enough
//!
//! By sag: the greatest distance between the true curve and the chord that
//! replaces it. A fixed segment count is wrong at both ends — wasteful on a
//! 2 mm fillet and visibly faceted on a 200 mm barrel — whereas a sag limit
//! asks the only question that matters, which is whether the error is visible.

use std::collections::HashMap;

use super::model::{Curve, Edge, EdgeId, Frame, Point3};

/// The greatest allowed distance between a curve and the chords replacing it.
///
/// In model units, which for every audited file is millimetres. A fiftieth of a
/// millimetre is far below what a phone screen can show for a part that fits on
/// it, and coarse enough that a cylinder does not become a thousand triangles.
pub const DEFAULT_SAG: f64 = 0.02;

/// The fewest segments any curved edge is given.
///
/// A sag limit alone will happily approve a single chord across a half-circle
/// of small radius, which is correct by the arithmetic and looks like a cut
/// corner. Four is the point at which a circle stops reading as a polygon.
const MIN_SEGMENTS: usize = 4;

/// And the most, so one absurd radius cannot produce a million triangles.
const MAX_SEGMENTS: usize = 512;

/// Every edge's points, computed once and shared.
///
/// Built for a whole solid before any face is tessellated, so no face can
/// possibly disagree with its neighbour about where their shared boundary is.
#[derive(Debug, Default)]
pub struct EdgeCache {
    chains: HashMap<EdgeId, Vec<Point3>>,
}

impl EdgeCache {
    /// Discretise every edge of a solid at one tolerance.
    pub fn build(edges: &[Edge], sag: f64) -> Self {
        let mut chains = HashMap::new();
        for edge in edges {
            chains.insert(edge.id, discretise(edge, sag));
        }
        Self { chains }
    }

    /// The points of an edge, in the direction it is being walked.
    ///
    /// **The same points either way.** The reversed form is the stored chain
    /// turned around, not a fresh discretisation from the other end, which is
    /// what keeps the two faces sharing this edge exactly coincident.
    pub fn points(&self, id: EdgeId, forwards: bool) -> Option<Vec<Point3>> {
        let chain = self.chains.get(&id)?;
        if forwards {
            Some(chain.clone())
        } else {
            Some(chain.iter().rev().copied().collect())
        }
    }

    pub fn len(&self) -> usize {
        self.chains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chains.is_empty()
    }
}

/// One edge as a chain of points, always from its start vertex to its end.
pub fn discretise(edge: &Edge, sag: f64) -> Vec<Point3> {
    let points = match &edge.curve {
        // A straight edge is its two ends. Subdividing it adds vertices that
        // carry no shape, and every one of them is a chance to disagree with
        // the neighbouring face.
        Curve::Line { .. } => vec![edge.start, edge.end],

        Curve::Circle { frame, radius } => {
            arc(frame, *radius, *radius, edge.start, edge.end, edge.same_sense, sag)
        }

        Curve::Ellipse { frame, major, minor } => {
            // Sagitta on an ellipse varies along it; using the larger radius
            // makes the step conservative everywhere rather than correct in one
            // place and coarse at the ends.
            arc(frame, *major, *minor, edge.start, edge.end, edge.same_sense, sag)
        }

        Curve::Polyline { points } => points.clone(),

        Curve::Spline { degree, control, knots, weights } => {
            spline(*degree, control, knots, weights.as_deref(), sag)
        }
    };

    // The vertices win over the curve's own arithmetic. A circle evaluated at
    // its start angle lands a rounding error away from the vertex the file
    // gives, and the neighbouring edge starts from that vertex exactly — so
    // trusting the evaluation opens a gap of exactly the size nobody thinks to
    // look for.
    let mut points = points;
    if let Some(first) = points.first_mut() {
        *first = edge.start;
    }
    if let Some(last) = points.last_mut() {
        *last = edge.end;
    }
    points
}

/// How many segments a curve of a given radius needs to stay within the sag.
///
/// For a chord subtending angle θ on radius r the sag is r(1 − cos(θ/2)), so
/// the largest angle allowed is 2·acos(1 − sag/r).
pub fn segments_for(radius: f64, sweep: f64, sag: f64) -> usize {
    let sweep = sweep.abs();
    if radius <= 0.0 || sweep <= 0.0 {
        return MIN_SEGMENTS;
    }
    // A sag larger than the radius means one chord would do; clamp so the
    // arccos stays defined rather than producing a NaN segment count.
    let ratio = (1.0 - sag / radius).clamp(-1.0, 1.0);
    let step = 2.0 * ratio.acos();
    if step <= f64::EPSILON {
        return MAX_SEGMENTS;
    }
    ((sweep / step).ceil() as usize).clamp(MIN_SEGMENTS, MAX_SEGMENTS)
}

/// An arc of a circle or ellipse, from one point round to another.
fn arc(
    frame: &Frame,
    major: f64,
    minor: f64,
    start: Point3,
    end: Point3,
    same_sense: bool,
    sag: f64,
) -> Vec<Point3> {
    let from = angle_of(frame, start);
    let to = angle_of(frame, end);

    // STEP parameterises a circle anticlockwise about its axis, and
    // `EDGE_CURVE.same_sense` says whether this edge runs with that or against
    // it. **Which way round matters far more than it looks.** Taking every arc
    // anticlockwise turns a small clockwise one — a five hundredth of a turn —
    // into 6.23 radians, very nearly the whole circle. It does not fail: the
    // chain still starts and ends on the right vertices, so the loop closes,
    // nothing is reported missing, and the boundary quietly encircles the part
    // an extra time. On a gear with ninety-four teeth whose tip arcs run
    // clockwise, that was ninety-four extra turns around the rim, an outline
    // enclosing ninety-four times the area it should, and the open sectors
    // between the spokes filled with metal that is not there.
    //
    // Coincident ends still mean a whole circle rather than nothing: a
    // cylinder's rim is one edge whose two vertices are the same point, and
    // reading that as a zero sweep loses the entire face.
    let mut sweep = to - from;
    if same_sense {
        while sweep <= 1e-9 {
            sweep += std::f64::consts::TAU;
        }
    } else {
        while sweep >= -1e-9 {
            sweep -= std::f64::consts::TAU;
        }
    }

    let steps = segments_for(major.max(minor), sweep, sag);
    (0..=steps)
        .map(|index| {
            let angle = from + sweep * (index as f64 / steps as f64);
            frame.to_world(major * angle.cos(), minor * angle.sin(), 0.0)
        })
        .collect()
}

/// Where a point sits round a frame's own x/y plane.
fn angle_of(frame: &Frame, point: Point3) -> f64 {
    let local = point.minus(frame.origin);
    local.dot(frame.second()).atan2(local.dot(frame.reference))
}

/// A B-spline, sampled by de Boor.
///
/// **Here because the audit found it, not because the plan asked for it.**
/// Freeform *edges* appear in 68% of real supplier files, including files whose
/// surfaces are entirely analytic — so without this, faces that are otherwise
/// perfectly supported lose a boundary and cannot be closed.
fn spline(
    degree: usize,
    control: &[Point3],
    knots: &[f64],
    weights: Option<&[f64]>,
    sag: f64,
) -> Vec<Point3> {
    if control.len() < 2 || degree == 0 || knots.len() < control.len() + degree + 1 {
        // Not a spline this can evaluate. The control polygon is a poor curve
        // but it is the right shape and the right endpoints, which keeps the
        // face closed instead of dropping it.
        return control.to_vec();
    }

    let first = knots[degree];
    let last = knots[control.len()];
    if !(last > first) {
        return control.to_vec();
    }

    // **Measured, not estimated.** A B-spline has no radius to put into the
    // sag formula, and the earlier version fed it the control polygon's
    // length as a stand-in — which is not a curvature and produced up to
    // five hundred points on an edge that needed thirty. The cost of that
    // is not the edge: every one of those points becomes a boundary vertex,
    // and a face bounded by four of them triangulates into thousands.
    //
    // So the deviation is measured. Sample, check how far the true curve
    // bows away from the chords replacing it, and double until that is
    // within the sag. Two or three passes settle it, and the answer is the
    // count the curve actually needs rather than a guess about it.
    let mut steps = (control.len() * 2).max(MIN_SEGMENTS).min(MAX_SEGMENTS);
    let mut points = sample(degree, control, knots, weights, first, last, steps);

    while steps < MAX_SEGMENTS {
        let mut worst = 0.0_f64;
        for index in 0..steps {
            let low = first + (last - first) * (index as f64 / steps as f64);
            let high = first + (last - first) * ((index + 1) as f64 / steps as f64);
            let middle = de_boor(degree, control, knots, weights, (low + high) / 2.0);
            let chord = points[index].plus(points[index + 1]).scaled(0.5);
            worst = worst.max(middle.minus(chord).length());
        }
        if worst <= sag {
            break;
        }
        steps = (steps * 2).min(MAX_SEGMENTS);
        points = sample(degree, control, knots, weights, first, last, steps);
    }

    points
}

/// A spline sampled evenly over its usable range.
fn sample(
    degree: usize,
    control: &[Point3],
    knots: &[f64],
    weights: Option<&[f64]>,
    first: f64,
    last: f64,
    steps: usize,
) -> Vec<Point3> {
    (0..=steps)
        .map(|index| {
            let t = first + (last - first) * (index as f64 / steps as f64);
            de_boor(degree, control, knots, weights, t)
        })
        .collect()
}

/// One point on a B-spline at parameter `t`.
fn de_boor(
    degree: usize,
    control: &[Point3],
    knots: &[f64],
    weights: Option<&[f64]>,
    t: f64,
) -> Point3 {
    // The span containing t, clamped so the last parameter value evaluates on
    // the final span rather than falling off the end.
    let mut span = degree;
    while span + 1 < control.len() && knots[span + 1] <= t {
        span += 1;
    }

    // Homogeneous coordinates, so the rational form falls out of the same
    // recurrence rather than needing a second implementation.
    let mut working: Vec<(Point3, f64)> = (0..=degree)
        .map(|index| {
            let at = span + index - degree;
            let weight = weights.and_then(|w| w.get(at)).copied().unwrap_or(1.0);
            (control[at].scaled(weight), weight)
        })
        .collect();

    for round in 1..=degree {
        for index in (round..=degree).rev() {
            let at = span + index - degree;
            let low = knots[at];
            let high = knots[at + degree + 1 - round];
            let alpha = if (high - low).abs() < 1e-12 {
                0.0
            } else {
                (t - low) / (high - low)
            };
            let (previous_point, previous_weight) = working[index - 1];
            let (this_point, this_weight) = working[index];
            working[index] = (
                previous_point.scaled(1.0 - alpha).plus(this_point.scaled(alpha)),
                previous_weight * (1.0 - alpha) + this_weight * alpha,
            );
        }
    }

    let (point, weight) = working[degree];
    if weight.abs() < 1e-12 {
        point
    } else {
        point.scaled(1.0 / weight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame {
            origin: Point3::new(0.0, 0.0, 0.0),
            axis: Point3::new(0.0, 0.0, 1.0),
            reference: Point3::new(1.0, 0.0, 0.0),
        }
    }

    fn circle_edge(id: u64, radius: f64, from: Point3, to: Point3) -> Edge {
        Edge {
            id: EdgeId(id),
            curve: Curve::Circle { frame: frame(), radius },
            start: from,
            end: to,
            same_sense: true,
        }
    }

    // ---- which way round an arc goes ------------------------------------------

    /// A point on the unit-frame circle at a given angle.
    fn at(radius: f64, angle: f64) -> Point3 {
        Point3::new(radius * angle.cos(), radius * angle.sin(), 0.0)
    }

    /// How far the chain travels, end to end along itself.
    fn walked(points: &[Point3]) -> f64 {
        points.windows(2).map(|pair| pair[1].minus(pair[0]).length()).sum()
    }

    /// **A short arc the other way round stays short.**
    ///
    /// `EDGE_CURVE.same_sense` says whether the edge runs with the circle's own
    /// anticlockwise parameterisation or against it. Taking every arc
    /// anticlockwise regardless turns a hundredth of a turn into very nearly a
    /// whole one — and it does not fail anywhere: the chain still begins and
    /// ends on the right vertices, so the loop closes and nothing is reported.
    /// The boundary simply encircles the part one more time than it should.
    #[test]
    fn a_short_clockwise_arc_does_not_become_a_whole_circle() {
        let radius = 10.0;
        let span = 0.2;
        // Running against the circle's own anticlockwise direction: the angle
        // decreases from start to end, which is a fifth of a radian of travel.
        // This is the shape a gear's tooth tips are written in.
        let mut edge = circle_edge(1, radius, at(radius, span), at(radius, 0.0));
        edge.same_sense = false;

        let length = walked(&discretise(&edge, DEFAULT_SAG));

        assert!(
            (length - radius * span).abs() < radius * span * 0.05,
            "travelled {length:.3} where {:.3} was expected; a whole circle is {:.3}",
            radius * span,
            std::f64::consts::TAU * radius,
        );
    }

    /// And it goes the other way round from the one that says so.
    #[test]
    fn same_sense_decides_the_direction_travelled() {
        let radius = 10.0;
        let span = 0.6;
        let forwards = circle_edge(1, radius, at(radius, 0.0), at(radius, span));
        let mut backwards = forwards.clone();
        backwards.same_sense = false;

        let one = discretise(&forwards, DEFAULT_SAG);
        let other = discretise(&backwards, DEFAULT_SAG);

        // Both start and finish on the same two vertices...
        assert!(one[0].minus(other[0]).length() < 1e-9);
        assert!(
            one[one.len() - 1].minus(other[other.len() - 1]).length() < 1e-9,
            "the ends moved",
        );
        // ...but one goes the short way and the other all the way round.
        let short = walked(&one);
        let long = walked(&other);
        assert!(short < long, "{short:.2} should be shorter than {long:.2}");
        assert!(
            (short + long - std::f64::consts::TAU * radius).abs() < radius * 0.05,
            "the two ways round should add up to a circle: {short:.2} + {long:.2}",
        );
    }

    /// A rim is still a whole circle, whichever way it runs.
    ///
    /// A cylinder's rim is one edge whose two vertices are the same point.
    /// Reading that as a zero sweep loses the entire face, so it has to stay a
    /// full turn for both senses.
    #[test]
    fn an_edge_that_begins_where_it_ends_is_a_whole_circle() {
        let radius = 4.0;
        for same_sense in [true, false] {
            let mut edge = circle_edge(1, radius, at(radius, 0.0), at(radius, 0.0));
            edge.same_sense = same_sense;

            let length = walked(&discretise(&edge, DEFAULT_SAG));
            assert!(
                (length - std::f64::consts::TAU * radius).abs() < radius * 0.05,
                "same_sense={same_sense}: travelled {length:.3}",
            );
        }
    }

    fn line_edge(id: u64, from: Point3, to: Point3) -> Edge {
        Edge {
            id: EdgeId(id),
            curve: Curve::Line { from, direction: to.minus(from) },
            start: from,
            end: to,
            same_sense: true,
        }
    }

    // ---- the crack, which is the whole reason for the cache -----------------

    /// The two faces meeting at an edge get the *same* points, one reversed.
    ///
    /// Exactly the same: reversing a stored chain is lossless, whereas
    /// discretising from the other end lands on different floating point and
    /// leaves a hairline gap down every seam of the model.
    #[test]
    fn both_faces_of_an_edge_get_identical_points() {
        let edge = circle_edge(1, 10.0, Point3::new(10.0, 0.0, 0.0), Point3::new(-10.0, 0.0, 0.0));
        let cache = EdgeCache::build(&[edge], DEFAULT_SAG);

        let one_way = cache.points(EdgeId(1), true).expect("cached");
        let other_way = cache.points(EdgeId(1), false).expect("cached");

        assert_eq!(one_way.len(), other_way.len());
        for (forward, backward) in one_way.iter().zip(other_way.iter().rev()) {
            // Bit-for-bit, not nearly.
            assert_eq!(forward.x.to_bits(), backward.x.to_bits());
            assert_eq!(forward.y.to_bits(), backward.y.to_bits());
            assert_eq!(forward.z.to_bits(), backward.z.to_bits());
        }
    }

    /// Asking twice gives the same answer, however many faces ask.
    #[test]
    fn the_cache_does_not_recompute() {
        let edge = circle_edge(7, 3.0, Point3::new(3.0, 0.0, 0.0), Point3::new(0.0, 3.0, 0.0));
        let cache = EdgeCache::build(&[edge], DEFAULT_SAG);

        assert_eq!(cache.points(EdgeId(7), true), cache.points(EdgeId(7), true));
    }

    #[test]
    fn an_edge_that_was_never_cached_is_reported_rather_than_invented() {
        let cache = EdgeCache::default();
        assert!(cache.points(EdgeId(99), true).is_none());
    }

    // ---- the ends ------------------------------------------------------------

    /// A chain starts and ends exactly on the vertices the file gave.
    ///
    /// The neighbouring edge starts from the same vertex, so a curve evaluated
    /// to a rounding error away from it opens a gap of precisely the size
    /// nobody thinks to look for.
    #[test]
    fn a_chain_lands_on_its_vertices_exactly() {
        let start = Point3::new(5.0, 0.0, 0.0);
        let end = Point3::new(0.0, 5.0, 0.0);
        let points = discretise(&circle_edge(1, 5.0, start, end), DEFAULT_SAG);

        assert_eq!(start, points[0]);
        assert_eq!(end, *points.last().expect("a chain has an end"));
    }

    #[test]
    fn a_straight_edge_is_two_points() {
        let points = discretise(
            &line_edge(1, Point3::new(0.0, 0.0, 0.0), Point3::new(10.0, 0.0, 0.0)),
            DEFAULT_SAG,
        );
        assert_eq!(2, points.len());
    }

    // ---- how fine ------------------------------------------------------------

    /// A big radius needs more segments than a small one for the same sag.
    #[test]
    fn a_larger_radius_is_divided_more_finely() {
        let small = segments_for(1.0, std::f64::consts::PI, DEFAULT_SAG);
        let large = segments_for(100.0, std::f64::consts::PI, DEFAULT_SAG);
        assert!(large > small, "small {small}, large {large}");
    }

    /// And the sag is actually met, which is the claim being made.
    #[test]
    fn the_chords_stay_within_the_sag() {
        let radius = 20.0;
        let points = discretise(
            &circle_edge(1, radius, Point3::new(20.0, 0.0, 0.0), Point3::new(-20.0, 0.0, 0.0)),
            DEFAULT_SAG,
        );

        for pair in points.windows(2) {
            let midpoint = pair[0].plus(pair[1]).scaled(0.5);
            // How far the chord's middle falls inside the true circle.
            let sag = radius - midpoint.length();
            assert!(sag <= DEFAULT_SAG * 1.05, "sag {sag} exceeded {DEFAULT_SAG}");
        }
    }

    /// A tight arc still gets enough segments to stop looking like a corner.
    #[test]
    fn a_small_arc_is_not_reduced_to_one_chord() {
        let points = discretise(
            &circle_edge(1, 0.5, Point3::new(0.5, 0.0, 0.0), Point3::new(0.0, 0.5, 0.0)),
            DEFAULT_SAG,
        );
        assert!(points.len() >= MIN_SEGMENTS, "only {} points", points.len());
    }

    /// An absurd radius cannot produce unbounded geometry.
    #[test]
    fn the_segment_count_is_capped() {
        assert!(segments_for(1e9, std::f64::consts::TAU, 1e-9) <= MAX_SEGMENTS);
    }

    /// A closed edge is a whole circle, not an empty sweep.
    ///
    /// A cylinder's rim is one edge whose two vertices are the same point.
    /// Reading that as a zero sweep loses the face it bounds — silently, since
    /// an empty loop triangulates to nothing rather than failing.
    #[test]
    fn an_edge_that_closes_on_itself_sweeps_the_whole_circle() {
        let point = Point3::new(4.0, 0.0, 0.0);
        let points = discretise(&circle_edge(1, 4.0, point, point), DEFAULT_SAG);

        assert!(points.len() > MIN_SEGMENTS, "a full circle needs more than a few points");
        // It comes back to where it started, having been all the way round.
        let quarter = points[points.len() / 4];
        assert!(quarter.y.abs() > 1.0, "never left the start: {quarter:?}");
    }

    // ---- splines -------------------------------------------------------------

    /// A spline passes through its clamped ends.
    #[test]
    fn a_clamped_spline_starts_and_ends_on_its_control_points() {
        let control = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 2.0, 0.0),
            Point3::new(3.0, 2.0, 0.0),
            Point3::new(4.0, 0.0, 0.0),
        ];
        let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
        let points = spline(3, &control, &knots, None, DEFAULT_SAG);

        let first = points.first().expect("a spline has a start");
        let last = points.last().expect("a spline has an end");
        assert!(first.minus(control[0]).length() < 1e-9, "{first:?}");
        assert!(last.minus(control[3]).length() < 1e-9, "{last:?}");
    }

    /// A rational spline with equal weights is the same curve as a plain one.
    ///
    /// The homogeneous form has to reduce to the ordinary one, or every
    /// non-rational spline in a real file is quietly evaluated wrongly.
    #[test]
    fn equal_weights_change_nothing() {
        let control = vec![
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 3.0, 0.0),
            Point3::new(4.0, 3.0, 0.0),
            Point3::new(5.0, 0.0, 0.0),
        ];
        let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];

        let plain = spline(3, &control, &knots, None, DEFAULT_SAG);
        let weighted = spline(3, &control, &knots, Some(&[1.0, 1.0, 1.0, 1.0]), DEFAULT_SAG);

        assert_eq!(plain.len(), weighted.len());
        for (a, b) in plain.iter().zip(weighted.iter()) {
            assert!(a.minus(*b).length() < 1e-9, "{a:?} against {b:?}");
        }
    }

    /// A quadratic rational spline with weight √2/2 in the middle is an exact
    /// quarter circle — the standard way a circle is written as a NURBS, and
    /// the case that catches a de Boor that ignores weights.
    #[test]
    fn a_rational_quarter_circle_is_actually_circular() {
        let control = vec![
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        let knots = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let weights = [1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0];

        for point in spline(2, &control, &knots, Some(&weights), DEFAULT_SAG) {
            let radius = point.length();
            assert!((radius - 1.0).abs() < 1e-6, "off the circle at {point:?}: r={radius}");
        }
    }

    /// Nonsense in, control polygon out — never an empty chain.
    ///
    /// An empty chain leaves the loop open, and an open loop triangulates to
    /// nothing at all: the face disappears with no error anywhere. The control
    /// polygon is the wrong curve but the right shape and the right ends.
    #[test]
    fn a_spline_that_cannot_be_evaluated_still_closes_its_loop() {
        let control = vec![Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)];
        // Far too few knots for the degree claimed.
        let points = spline(5, &control, &[0.0, 1.0], None, DEFAULT_SAG);

        assert_eq!(control, points);
    }
}

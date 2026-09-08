//! Freeform surfaces: evaluating them, and finding your way back.
//!
//! # Why this exists
//!
//! The [audit](super::audit) of 41 files found freeform surfaces in half of
//! them, always as a minority — fillets and blends on parts that are otherwise
//! prismatic — which is why v1 could skip them and still draw 95% of that
//! catalogue usefully. A turbine impeller is the other case entirely: 76% of
//! its faces are freeform, because the *blades* are freeform, and skipping them
//! leaves a hub and some stray edges. There is no threshold that makes such a
//! part viewable. The surfaces have to be tessellated.
//!
//! # The hard part is not evaluation
//!
//! Getting a point from `(u, v)` is de Boor twice — the same recurrence the
//! curves already use, applied along one direction and then the other. What has
//! no closed form is the **inverse**: the tessellator works by projecting a
//! face's boundary into `(u, v)`, and for a B-spline there is no formula that
//! turns a point in space back into parameters.
//!
//! So: a grid is sampled once per surface, the nearest node gives a starting
//! guess, and Gauss–Newton walks from there to the actual foot of the point.
//! Sampling alone is not enough — a coarse grid puts boundary points on the
//! wrong side of an edge and the polygon self-crosses — and Newton alone is not
//! enough either, because from a bad start it converges to the wrong sheet of a
//! folded surface. Each covers the other's failure.

use std::sync::Arc;

use super::model::{Point2, Point3};

/// A B-spline surface, and a sampling of it to navigate by.
#[derive(Debug, Clone)]
pub struct Spline {
    pub degree_u: usize,
    pub degree_v: usize,
    /// Control points, indexed `[u][v]` as STEP lists them.
    pub control: Vec<Vec<Point3>>,
    pub knots_u: Vec<f64>,
    pub knots_v: Vec<f64>,
    /// Present only for the rational form.
    pub weights: Option<Vec<Vec<f64>>>,
    /// The usable parameter range, which is not the whole knot vector.
    pub range_u: (f64, f64),
    pub range_v: (f64, f64),
    /// `GRID × GRID` samples, row-major in u, for [`Spline::nearest`].
    samples: Arc<Vec<Point3>>,
}

/// How finely a surface is sampled to navigate by.
///
/// Not for drawing — the tessellator decides that from the sag. This grid only
/// has to be fine enough that the nearest node is in the right neighbourhood,
/// and 24 × 24 is 576 evaluations once per surface against thousands of
/// projections that then start close enough to converge in three or four steps.
const GRID: usize = 24;

impl Spline {
    pub fn new(
        degree_u: usize,
        degree_v: usize,
        control: Vec<Vec<Point3>>,
        knots_u: Vec<f64>,
        knots_v: Vec<f64>,
        weights: Option<Vec<Vec<f64>>>,
    ) -> Option<Self> {
        let rows = control.len();
        let columns = control.first()?.len();
        if rows < 2 || columns < 2 || degree_u == 0 || degree_v == 0 {
            return None;
        }
        // A clamped knot vector has `points + degree + 1` entries. Fewer means
        // the file is describing something this cannot evaluate, and guessing
        // at the missing ones produces a surface in the wrong place rather than
        // an error.
        if knots_u.len() < rows + degree_u + 1 || knots_v.len() < columns + degree_v + 1 {
            return None;
        }

        // The usable range excludes the repeated ends: outside it the basis
        // functions do not sum to one and the surface flies off.
        let range_u = (knots_u[degree_u], knots_u[rows]);
        let range_v = (knots_v[degree_v], knots_v[columns]);
        if !(range_u.1 > range_u.0) || !(range_v.1 > range_v.0) {
            return None;
        }

        let mut spline = Self {
            degree_u,
            degree_v,
            control,
            knots_u,
            knots_v,
            weights,
            range_u,
            range_v,
            samples: Arc::new(Vec::new()),
        };

        let mut samples = Vec::with_capacity(GRID * GRID);
        for row in 0..GRID {
            for column in 0..GRID {
                let (u, v) = spline.grid_parameters(row, column);
                samples.push(spline.at(Point2::new(u, v)));
            }
        }
        spline.samples = Arc::new(samples);
        Some(spline)
    }

    fn grid_parameters(&self, row: usize, column: usize) -> (f64, f64) {
        let across = row as f64 / (GRID - 1) as f64;
        let down = column as f64 / (GRID - 1) as f64;
        (
            self.range_u.0 + (self.range_u.1 - self.range_u.0) * across,
            self.range_v.0 + (self.range_v.1 - self.range_v.0) * down,
        )
    }

    /// A point on the surface.
    pub fn at(&self, at: Point2) -> Point3 {
        let u = at.u.clamp(self.range_u.0, self.range_u.1);
        let v = at.v.clamp(self.range_v.0, self.range_v.1);

        // Along v first, giving one point per row of control points, then along
        // u through those. Either order gives the same surface; this one keeps
        // the inner loop over the shorter axis in the common case.
        let span_u = span_of(&self.knots_u, self.degree_u, self.control.len(), u);

        let mut along_u: Vec<(Point3, f64)> = Vec::with_capacity(self.degree_u + 1);
        for offset in 0..=self.degree_u {
            let row = span_u + offset - self.degree_u;
            let points = &self.control[row];
            let row_weights = self.weights.as_ref().map(|w| w[row].as_slice());
            along_u.push(de_boor(self.degree_v, points, &self.knots_v, row_weights, v));
        }

        let (point, weight) = de_boor_homogeneous(
            self.degree_u,
            &along_u,
            &self.knots_u,
            span_u,
            u,
        );
        if weight.abs() < 1e-12 {
            point
        } else {
            point.scaled(1.0 / weight)
        }
    }

    /// The parameters of the point on the surface nearest to `target`.
    ///
    /// Grid first, then Gauss–Newton. See the module note: neither alone is
    /// enough, and the failure of each is silent — a coarse grid puts a
    /// boundary point on the wrong side of an edge, and Newton from a bad start
    /// walks onto a different fold of the same surface. Both produce a polygon
    /// that self-crosses in parameter space and triangulates into knots.
    pub fn nearest(&self, target: Point3) -> Point2 {
        let mut best = (0usize, 0usize);
        let mut closest = f64::MAX;
        for row in 0..GRID {
            for column in 0..GRID {
                let away = self.samples[row * GRID + column].minus(target).dot(
                    self.samples[row * GRID + column].minus(target),
                );
                if away < closest {
                    closest = away;
                    best = (row, column);
                }
            }
        }

        let (mut u, mut v) = self.grid_parameters(best.0, best.1);
        let step_u = (self.range_u.1 - self.range_u.0) / (GRID - 1) as f64;
        let step_v = (self.range_v.1 - self.range_v.0) / (GRID - 1) as f64;

        // Six is generous: from a grid node the foot is within one cell, and
        // Gauss-Newton halves the error each time. The cap is what stops a
        // pathological surface spinning here for ever.
        for _ in 0..6 {
            let here = self.at(Point2::new(u, v));
            let error = here.minus(target);
            if error.dot(error) < 1e-14 {
                break;
            }

            let (d_u, d_v) = self.derivatives(u, v, step_u, step_v);

            // Solve the 2x2 normal equations for the step that best cancels the
            // error along the surface. A singular system means the surface has
            // no local plane here — a pole or a crease — and the grid guess is
            // the best answer available.
            let a = d_u.dot(d_u);
            let b = d_u.dot(d_v);
            let c = d_v.dot(d_v);
            let determinant = a * c - b * b;
            if determinant.abs() < 1e-18 {
                break;
            }

            let rhs_u = -error.dot(d_u);
            let rhs_v = -error.dot(d_v);
            let move_u = (c * rhs_u - b * rhs_v) / determinant;
            let move_v = (a * rhs_v - b * rhs_u) / determinant;

            u = (u + move_u).clamp(self.range_u.0, self.range_u.1);
            v = (v + move_v).clamp(self.range_v.0, self.range_v.1);
        }

        Point2::new(u, v)
    }

    /// The surface's slopes along u and v, by central difference.
    ///
    /// Differences rather than the analytic derivative: the analytic form of a
    /// rational B-spline's partial is a quotient rule over two de Boor
    /// evaluations, which is more code to get subtly wrong than it is worth for
    /// a quantity used only to point Newton downhill and to orient a normal.
    fn derivatives(&self, u: f64, v: f64, step_u: f64, step_v: f64) -> (Point3, Point3) {
        let nudge_u = (step_u * 1e-3).max(1e-9);
        let nudge_v = (step_v * 1e-3).max(1e-9);

        let u_low = (u - nudge_u).max(self.range_u.0);
        let u_high = (u + nudge_u).min(self.range_u.1);
        let v_low = (v - nudge_v).max(self.range_v.0);
        let v_high = (v + nudge_v).min(self.range_v.1);

        let along_u = self
            .at(Point2::new(u_high, v))
            .minus(self.at(Point2::new(u_low, v)))
            .scaled(1.0 / (u_high - u_low).max(1e-12));
        let along_v = self
            .at(Point2::new(u, v_high))
            .minus(self.at(Point2::new(u, v_low)))
            .scaled(1.0 / (v_high - v_low).max(1e-12));

        (along_u, along_v)
    }

    /// The surface normal, from the two slopes.
    pub fn normal(&self, at: Point2) -> Option<Point3> {
        let step_u = (self.range_u.1 - self.range_u.0) / (GRID - 1) as f64;
        let step_v = (self.range_v.1 - self.range_v.0) / (GRID - 1) as f64;
        let (d_u, d_v) = self.derivatives(at.u, at.v, step_u, step_v);
        d_u.cross(d_v).normalised()
    }

    /// How far apart triangles may be in parameter space for a given sag.
    ///
    /// Estimated from the sampled grid rather than from curvature arithmetic:
    /// the worst bow of the surface away from a chord over one grid cell says
    /// directly how much finer than a cell the triangles have to be.
    pub fn parameter_limits(&self, sag: f64) -> (f64, f64) {
        let step_u = (self.range_u.1 - self.range_u.0) / (GRID - 1) as f64;
        let step_v = (self.range_v.1 - self.range_v.0) / (GRID - 1) as f64;

        let mut worst_u: f64 = 0.0;
        let mut worst_v: f64 = 0.0;
        for row in 0..GRID {
            for column in 0..GRID {
                let here = self.samples[row * GRID + column];
                if row + 1 < GRID {
                    let next = self.samples[(row + 1) * GRID + column];
                    let (u, v) = self.grid_parameters(row, column);
                    let middle = self.at(Point2::new(u + step_u / 2.0, v));
                    worst_u = worst_u.max(middle.minus(here.plus(next).scaled(0.5)).length());
                }
                if column + 1 < GRID {
                    let next = self.samples[row * GRID + column + 1];
                    let (u, v) = self.grid_parameters(row, column);
                    let middle = self.at(Point2::new(u, v + step_v / 2.0));
                    worst_v = worst_v.max(middle.minus(here.plus(next).scaled(0.5)).length());
                }
            }
        }

        // Sag falls as the square of the step, so halving the step quarters it.
        let shrink = |bow: f64, step: f64| -> f64 {
            if bow <= sag {
                step
            } else {
                (step * (sag / bow).sqrt()).max(step / 64.0)
            }
        };

        (shrink(worst_u, step_u), shrink(worst_v, step_v))
    }
}

/// Which knot span a parameter falls in, clamped to the last usable one.
fn span_of(knots: &[f64], degree: usize, points: usize, t: f64) -> usize {
    let mut span = degree;
    while span + 1 < points && knots[span + 1] <= t {
        span += 1;
    }
    span
}

/// One point of a B-spline curve, in homogeneous form.
fn de_boor(
    degree: usize,
    control: &[Point3],
    knots: &[f64],
    weights: Option<&[f64]>,
    t: f64,
) -> (Point3, f64) {
    let span = span_of(knots, degree, control.len(), t);
    let working: Vec<(Point3, f64)> = (0..=degree)
        .map(|offset| {
            let at = span + offset - degree;
            let weight = weights.and_then(|w| w.get(at)).copied().unwrap_or(1.0);
            (control[at].scaled(weight), weight)
        })
        .collect();

    de_boor_homogeneous(degree, &working, knots, span, t)
}

/// The de Boor recurrence over already-weighted points.
///
/// Shared by the curve and both directions of the surface. Homogeneous
/// throughout, so the rational form needs no second implementation — dividing
/// by the accumulated weight at the end is the only difference, and that is the
/// caller's business.
fn de_boor_homogeneous(
    degree: usize,
    start: &[(Point3, f64)],
    knots: &[f64],
    span: usize,
    t: f64,
) -> (Point3, f64) {
    let mut working = start.to_vec();

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

    working[degree]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat 10 x 10 patch, as a bilinear spline.
    ///
    /// Flat on purpose: everything about it can be checked against arithmetic
    /// anybody can do in their head, which is what a test of a surface
    /// evaluator needs before it is pointed at a turbine blade.
    fn flat_patch() -> Spline {
        Spline::new(
            1,
            1,
            vec![
                vec![Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 10.0, 0.0)],
                vec![Point3::new(10.0, 0.0, 0.0), Point3::new(10.0, 10.0, 0.0)],
            ],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            None,
        )
        .expect("a bilinear patch is valid")
    }

    /// A patch with a bulge in the middle, so curvature is exercised.
    fn domed_patch() -> Spline {
        let mut control = Vec::new();
        for row in 0..3 {
            let mut line = Vec::new();
            for column in 0..3 {
                let x = row as f64 * 5.0;
                let y = column as f64 * 5.0;
                // The middle control point is lifted, which lifts the surface
                // by less — a B-spline does not pass through its middle points.
                let z = if row == 1 && column == 1 { 6.0 } else { 0.0 };
                line.push(Point3::new(x, y, z));
            }
            control.push(line);
        }
        Spline::new(
            2,
            2,
            control,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            None,
        )
        .expect("a biquadratic patch is valid")
    }

    // ---- evaluation ----------------------------------------------------------

    /// A clamped patch touches its corner control points exactly.
    #[test]
    fn the_corners_are_the_corner_control_points() {
        let patch = flat_patch();

        for (u, v, expected) in [
            (0.0, 0.0, Point3::new(0.0, 0.0, 0.0)),
            (1.0, 0.0, Point3::new(10.0, 0.0, 0.0)),
            (0.0, 1.0, Point3::new(0.0, 10.0, 0.0)),
            (1.0, 1.0, Point3::new(10.0, 10.0, 0.0)),
        ] {
            let point = patch.at(Point2::new(u, v));
            assert!(point.minus(expected).length() < 1e-9, "({u},{v}) gave {point:?}");
        }
    }

    /// A flat patch is flat everywhere, not only at its corners.
    #[test]
    fn a_flat_patch_stays_flat() {
        let patch = flat_patch();
        for step in 0..=10 {
            let t = step as f64 / 10.0;
            let point = patch.at(Point2::new(t, 1.0 - t));
            assert!(point.z.abs() < 1e-9, "bulged to {} at {t}", point.z);
        }
    }

    /// A lifted middle control point lifts the surface, by less than itself.
    ///
    /// The "by less" is the check that matters: a surface that passes through
    /// its middle control point is being evaluated as an interpolation rather
    /// than as a B-spline, which is wrong everywhere except the corners.
    #[test]
    fn a_domed_patch_rises_but_not_to_its_control_point() {
        let middle = domed_patch().at(Point2::new(0.5, 0.5));

        assert!(middle.z > 0.5, "did not rise at all: {}", middle.z);
        assert!(middle.z < 6.0, "passed through the control point: {}", middle.z);
    }

    /// The rational form with equal weights is the ordinary one.
    #[test]
    fn equal_weights_change_nothing() {
        let plain = domed_patch();
        let weighted = Spline::new(
            plain.degree_u,
            plain.degree_v,
            plain.control.clone(),
            plain.knots_u.clone(),
            plain.knots_v.clone(),
            Some(vec![vec![1.0; 3]; 3]),
        )
        .expect("valid");

        for step in 0..=8 {
            let t = step as f64 / 8.0;
            let at = Point2::new(t, 1.0 - t);
            assert!(
                plain.at(at).minus(weighted.at(at)).length() < 1e-9,
                "differed at {t}",
            );
        }
    }

    // ---- the inverse ---------------------------------------------------------

    /// A point taken off the surface is found again, to close tolerance.
    ///
    /// This is the operation with no closed form and the one the tessellator
    /// depends on: every boundary point of every freeform face goes through it.
    #[test]
    fn a_point_on_the_surface_is_found_again() {
        let patch = domed_patch();

        for (u, v) in [(0.2, 0.3), (0.5, 0.5), (0.9, 0.1), (0.05, 0.95)] {
            let target = patch.at(Point2::new(u, v));
            let found = patch.nearest(target);
            let landed = patch.at(found);

            assert!(
                landed.minus(target).length() < 1e-6,
                "({u},{v}) came back as ({},{}) which is {landed:?} not {target:?}",
                found.u,
                found.v,
            );
        }
    }

    /// Including the corners, where the search cannot step outside the range.
    #[test]
    fn the_corners_are_found_too() {
        let patch = domed_patch();
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0)] {
            let target = patch.at(Point2::new(u, v));
            let landed = patch.at(patch.nearest(target));
            assert!(landed.minus(target).length() < 1e-6, "({u},{v}) was lost");
        }
    }

    /// A point off the surface lands on the nearest part of it, not at random.
    #[test]
    fn a_point_above_the_surface_lands_beneath_itself() {
        let patch = flat_patch();
        let above = Point3::new(3.0, 7.0, 5.0);

        let found = patch.at(patch.nearest(above));

        assert!((found.x - 3.0).abs() < 1e-4, "x went to {}", found.x);
        assert!((found.y - 7.0).abs() < 1e-4, "y went to {}", found.y);
    }

    /// Every parameter it returns is inside the surface's own range.
    #[test]
    fn the_answer_is_always_in_range() {
        let patch = domed_patch();
        for far in [
            Point3::new(-100.0, -100.0, 50.0),
            Point3::new(1000.0, 1000.0, -50.0),
        ] {
            let found = patch.nearest(far);
            assert!(found.u >= patch.range_u.0 && found.u <= patch.range_u.1, "{found:?}");
            assert!(found.v >= patch.range_v.0 && found.v <= patch.range_v.1, "{found:?}");
        }
    }

    // ---- normals -------------------------------------------------------------

    #[test]
    fn a_flat_patch_has_a_constant_normal() {
        let patch = flat_patch();
        let normal = patch.normal(Point2::new(0.5, 0.5)).expect("a normal");
        assert!(normal.z.abs() > 0.999, "{normal:?}");
    }

    /// A domed patch leans away from the top, which is what shades it.
    #[test]
    fn a_domed_patch_leans_off_its_summit() {
        let patch = domed_patch();
        let summit = patch.normal(Point2::new(0.5, 0.5)).expect("a normal");
        let flank = patch.normal(Point2::new(0.15, 0.5)).expect("a normal");

        assert!(summit.z.abs() > 0.99, "the top should face up: {summit:?}");
        assert!(flank.z.abs() < 0.98, "the flank should lean: {flank:?}");
    }

    // ---- how finely to cut it ------------------------------------------------

    /// A flat patch needs no subdivision; a curved one does.
    ///
    /// At a *tight* tolerance, because at an ordinary one the answer is
    /// that neither needs subdividing — a 24 x 24 grid cell on this dome
    /// already bows less than two hundredths of a millimetre, and the first
    /// version of this test failed for that reason rather than for a fault.
    #[test]
    fn a_curved_patch_is_cut_more_finely_than_a_flat_one() {
        let flat = flat_patch().parameter_limits(0.0001);
        let domed = domed_patch().parameter_limits(0.0001);

        assert!(domed.0 < flat.0, "flat {} against domed {}", flat.0, domed.0);
    }

    /// And a tighter tolerance cuts it finer still.
    #[test]
    fn a_finer_tolerance_cuts_more_finely() {
        let coarse = domed_patch().parameter_limits(1.0);
        let fine = domed_patch().parameter_limits(0.001);
        assert!(fine.0 < coarse.0, "coarse {} fine {}", coarse.0, fine.0);
    }

    // ---- refusing the impossible ---------------------------------------------

    /// Too few knots for the degree claimed is refused, not guessed at.
    #[test]
    fn a_surface_with_a_short_knot_vector_is_refused() {
        assert!(Spline::new(
            3,
            3,
            vec![vec![Point3::new(0.0, 0.0, 0.0); 2]; 2],
            vec![0.0, 1.0],
            vec![0.0, 1.0],
            None,
        )
        .is_none());
    }

    #[test]
    fn a_surface_with_one_row_of_control_points_is_refused() {
        assert!(Spline::new(
            1,
            1,
            vec![vec![Point3::new(0.0, 0.0, 0.0), Point3::new(1.0, 0.0, 0.0)]],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            None,
        )
        .is_none());
    }
}

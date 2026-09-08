//! Moving between a surface's own two coordinates and the world's three.
//!
//! Triangulating happens in two dimensions, so every boundary point has to be
//! expressed in the surface's `(u, v)` before it can be cut into triangles, and
//! every triangle mapped back afterwards.
//!
//! **Closed form, because there is no alternative.** A STEP file may carry
//! `PCURVE` entities — the boundary already written in parameter space, which
//! would make this module unnecessary. Not one of the 41 real files audited
//! contained a single one, so the analytic route is not an optimisation to fall
//! back from; it is the only route.
//!
//! # The third trap: seams and poles
//!
//! A cylinder's parameter space wraps: u = 0 and u = 2π are the same line in
//! space. A loop crossing that line reads, in two dimensions, as a polygon
//! that leaps the entire width of the domain and back — self-crossing, zero
//! area, and it triangulates to nothing. [`unwrap_seam`] is the answer.
//!
//! A cone's apex and a sphere's poles are worse: there u is not merely
//! ambiguous but undefined, since every u names the same point. [`project`]
//! reports those rather than inventing a value, and [`unwrap_seam`] fills them
//! from their neighbours.

use std::f64::consts::{PI, TAU};

use super::model::{Point2, Point3, Surface};

/// Where a point sits in a surface's own coordinates.
///
/// `None` at a singular point — a cone's apex, a sphere's pole — where the
/// answer genuinely does not exist. Returning a made-up zero there produces a
/// triangle fan twisted round the pole, which looks like a shading fault.
pub fn project(surface: &Surface, point: Point3) -> Option<Point2> {
    match surface {
        Surface::Plane { frame } => {
            let local = point.minus(frame.origin);
            Some(Point2::new(
                local.dot(frame.reference),
                local.dot(frame.second()),
            ))
        }

        Surface::Cylinder { frame, .. } => {
            let local = point.minus(frame.origin);
            let x = local.dot(frame.reference);
            let y = local.dot(frame.second());
            if x.hypot(y) < 1e-9 {
                return None; // on the axis: no angle exists
            }
            Some(Point2::new(y.atan2(x), local.dot(frame.axis)))
        }

        Surface::Cone { frame, .. } => {
            let local = point.minus(frame.origin);
            let x = local.dot(frame.reference);
            let y = local.dot(frame.second());
            if x.hypot(y) < 1e-9 {
                return None; // the apex
            }
            Some(Point2::new(y.atan2(x), local.dot(frame.axis)))
        }

        Surface::Sphere { frame, radius } => {
            let local = point.minus(frame.origin);
            let x = local.dot(frame.reference);
            let y = local.dot(frame.second());
            let z = local.dot(frame.axis);
            if x.hypot(y) < 1e-9 {
                return None; // a pole
            }
            let latitude = (z / radius.max(1e-12)).clamp(-1.0, 1.0).asin();
            Some(Point2::new(y.atan2(x), latitude))
        }

        Surface::Torus { frame, major, .. } => {
            let local = point.minus(frame.origin);
            let x = local.dot(frame.reference);
            let y = local.dot(frame.second());
            let z = local.dot(frame.axis);
            let from_axis = x.hypot(y);
            if from_axis < 1e-9 {
                return None;
            }
            Some(Point2::new(y.atan2(x), z.atan2(from_axis - major)))
        }

        Surface::Spline(spline) => Some(spline.nearest(point)),

        Surface::Unsupported { .. } => None,
    }
}

/// A parameter pair turned back into a point in space.
pub fn evaluate(surface: &Surface, at: Point2) -> Option<Point3> {
    match surface {
        Surface::Plane { frame } => Some(frame.to_world(at.u, at.v, 0.0)),

        Surface::Cylinder { frame, radius } => {
            Some(frame.to_world(radius * at.u.cos(), radius * at.u.sin(), at.v))
        }

        Surface::Cone { frame, radius, half_angle } => {
            // The radius grows with height at the half angle; past the apex it
            // would go negative, which is the other nappe of the cone and not
            // part of any real solid.
            let at_height = (radius + at.v * half_angle.tan()).max(0.0);
            Some(frame.to_world(at_height * at.u.cos(), at_height * at.u.sin(), at.v))
        }

        Surface::Sphere { frame, radius } => {
            let ring = radius * at.v.cos();
            Some(frame.to_world(ring * at.u.cos(), ring * at.u.sin(), radius * at.v.sin()))
        }

        Surface::Torus { frame, major, minor } => {
            let ring = major + minor * at.v.cos();
            Some(frame.to_world(ring * at.u.cos(), ring * at.u.sin(), minor * at.v.sin()))
        }

        Surface::Spline(spline) => Some(spline.at(at)),

        Surface::Unsupported { .. } => None,
    }
}

/// The surface's own normal at a parameter pair, before `same_sense`.
///
/// Analytic rather than taken from the triangle's winding: a triangle covering
/// a curved patch has a normal that is only the average of the real ones, which
/// is what makes a coarsely tessellated cylinder look faceted even where the
/// silhouette is smooth.
pub fn normal_at(surface: &Surface, at: Point2) -> Option<Point3> {
    match surface {
        Surface::Plane { frame } => Some(frame.axis),

        Surface::Cylinder { frame, .. } => frame
            .reference
            .scaled(at.u.cos())
            .plus(frame.second().scaled(at.u.sin()))
            .normalised(),

        Surface::Cone { frame, half_angle, .. } => {
            // Tilted out of the radial direction by the half angle, which is
            // what distinguishes a cone from a cylinder at every point.
            let radial = frame
                .reference
                .scaled(at.u.cos())
                .plus(frame.second().scaled(at.u.sin()));
            radial
                .scaled(half_angle.cos())
                .plus(frame.axis.scaled(-half_angle.sin()))
                .normalised()
        }

        Surface::Sphere { frame, .. } => {
            let ring = at.v.cos();
            frame
                .reference
                .scaled(ring * at.u.cos())
                .plus(frame.second().scaled(ring * at.u.sin()))
                .plus(frame.axis.scaled(at.v.sin()))
                .normalised()
        }

        Surface::Torus { frame, .. } => {
            let radial = frame
                .reference
                .scaled(at.u.cos())
                .plus(frame.second().scaled(at.u.sin()));
            radial
                .scaled(at.v.cos())
                .plus(frame.axis.scaled(at.v.sin()))
                .normalised()
        }

        Surface::Spline(spline) => spline.normal(at),

        Surface::Unsupported { .. } => None,
    }
}

/// Whether a surface's `u` wraps round.
pub fn u_is_periodic(surface: &Surface) -> bool {
    // A freeform patch has an ordinary rectangular domain -- it does not wrap,
    // and unwrapping one would add whole turns to parameters that never meant
    // angles, dragging its boundary off across an imaginary seam.
    !matches!(
        surface,
        Surface::Plane { .. } | Surface::Spline(_) | Surface::Unsupported { .. }
    )
}

/// Straighten a boundary that crosses the seam, and fill in the poles.
///
/// Walking a loop, `u` should change gently. A jump of more than half a turn
/// between neighbours is not the boundary racing round the surface — it is the
/// seam, where 2π became 0. Adding whole turns as they accumulate keeps the
/// polygon continuous, at the cost of a `u` outside `[0, 2π)`, which the
/// triangulator neither notices nor minds.
///
/// Points with no `u` at all — a cone's apex, a sphere's pole — take their
/// neighbour's, which is the one value that keeps the polygon closed and the
/// triangles at the pole thin rather than twisted.
pub fn unwrap_seam(points: &mut [Option<Point2>], periodic: bool) {
    // Poles first: a missing u cannot be compared against, so filling has to
    // happen before any jump is measured.
    let known = points.iter().position(|point| point.is_some());
    let Some(known) = known else { return };

    for index in (0..known).rev() {
        points[index] = points[index].or_else(|| {
            points[index + 1].map(|next| Point2::new(next.u, points[index].map_or(next.v, |p| p.v)))
        });
    }
    for index in (known + 1)..points.len() {
        if points[index].is_none() {
            if let Some(previous) = points[index - 1] {
                points[index] = Some(Point2::new(previous.u, previous.v));
            }
        }
    }

    if !periodic {
        return;
    }

    let mut turns = 0.0;
    for index in 1..points.len() {
        let (Some(previous), Some(current)) = (points[index - 1], points[index]) else {
            continue;
        };
        let raw = current.u - (previous.u - turns);
        if raw > PI {
            turns -= TAU;
        } else if raw < -PI {
            turns += TAU;
        }
        points[index] = Some(Point2::new(current.u + turns, current.v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::model::Frame;

    fn frame() -> Frame {
        Frame {
            origin: Point3::new(0.0, 0.0, 0.0),
            axis: Point3::new(0.0, 0.0, 1.0),
            reference: Point3::new(1.0, 0.0, 0.0),
        }
    }

    fn cylinder() -> Surface {
        Surface::Cylinder { frame: frame(), radius: 5.0 }
    }

    // ---- round trips ---------------------------------------------------------

    /// Every surface returns the point it was given.
    ///
    /// A projection that does not invert is the kind of error that puts one
    /// face slightly out of place — visible as a step where two faces meet, and
    /// very hard to attribute to the projection rather than the triangulation.
    #[test]
    fn every_surface_projects_and_comes_back() {
        let cases: Vec<(Surface, Point3)> = vec![
            (Surface::Plane { frame: frame() }, Point3::new(3.0, -4.0, 0.0)),
            (cylinder(), Point3::new(5.0, 0.0, 7.0)),
            (
                Surface::Cone { frame: frame(), radius: 2.0, half_angle: 0.4 },
                // On the cone: radius grows by tan(0.4) per unit height.
                Point3::new(2.0 + 3.0 * 0.4_f64.tan(), 0.0, 3.0),
            ),
            (
                Surface::Sphere { frame: frame(), radius: 6.0 },
                Point3::new(0.0, 6.0, 0.0),
            ),
            (
                Surface::Torus { frame: frame(), major: 10.0, minor: 2.0 },
                Point3::new(12.0, 0.0, 0.0),
            ),
        ];

        for (surface, point) in cases {
            let uv = project(&surface, point).expect("a regular point projects");
            let back = evaluate(&surface, uv).expect("and comes back");
            assert!(
                back.minus(point).length() < 1e-6,
                "{surface:?}: {point:?} became {back:?}",
            );
        }
    }

    /// A cylinder's normal points straight out from its axis.
    #[test]
    fn a_cylinders_normal_is_radial() {
        let normal = normal_at(&cylinder(), Point2::new(0.0, 3.0)).expect("a normal");
        assert!((normal.x - 1.0).abs() < 1e-9, "{normal:?}");
        assert!(normal.z.abs() < 1e-9, "a cylinder's normal has no axial part");
    }

    /// A cone's normal is tilted; a cone whose normal is radial is a cylinder.
    #[test]
    fn a_cones_normal_leans() {
        let cone = Surface::Cone { frame: frame(), radius: 2.0, half_angle: 0.4 };
        let normal = normal_at(&cone, Point2::new(0.0, 1.0)).expect("a normal");

        assert!(normal.z.abs() > 0.1, "not leaning at all: {normal:?}");
        assert!((normal.length() - 1.0).abs() < 1e-9, "not a unit vector");
    }

    // ---- the seam ------------------------------------------------------------

    /// A boundary crossing u = 0 is straightened, not left to leap the domain.
    ///
    /// Untreated, the two points either side of the seam are nearly 2π apart in
    /// parameter space, so the polygon doubles back across the whole surface.
    /// Its area cancels out and it triangulates to nothing — the face vanishes
    /// with no error raised anywhere.
    #[test]
    fn a_loop_crossing_the_seam_is_made_continuous() {
        let mut walk: Vec<Option<Point2>> = [
            (TAU - 0.2, 0.0),
            (TAU - 0.1, 0.0),
            (0.05, 0.0), // over the seam
            (0.15, 0.0),
        ]
        .into_iter()
        .map(|(u, v)| Some(Point2::new(u, v)))
        .collect();

        unwrap_seam(&mut walk, true);

        let us: Vec<f64> = walk.iter().map(|p| p.expect("kept").u).collect();
        for pair in us.windows(2) {
            let step = pair[1] - pair[0];
            assert!(step > 0.0 && step < 0.5, "a leap of {step} across the seam");
        }
    }

    /// Going round twice keeps climbing rather than resetting.
    #[test]
    fn a_boundary_that_wraps_twice_keeps_going() {
        let mut walk: Vec<Option<Point2>> = (0..24)
            .map(|step| Some(Point2::new((step as f64 * 0.6) % TAU, 0.0)))
            .collect();

        unwrap_seam(&mut walk, true);

        let first = walk.first().expect("kept").expect("kept").u;
        let last = walk.last().expect("kept").expect("kept").u;
        assert!(last - first > TAU, "did not get round twice: {first} to {last}");
    }

    /// A plane has no seam, so nothing is touched.
    #[test]
    fn a_plane_is_left_alone() {
        let original = vec![
            Some(Point2::new(0.0, 0.0)),
            Some(Point2::new(100.0, 0.0)),
            Some(Point2::new(100.0, 50.0)),
        ];
        let mut copy = original.clone();
        unwrap_seam(&mut copy, false);

        for (before, after) in original.iter().zip(copy.iter()) {
            assert_eq!(before.expect("kept").u, after.expect("kept").u);
        }
    }

    // ---- the poles -----------------------------------------------------------

    /// A cone's apex has no angle, and is reported rather than guessed.
    #[test]
    fn the_apex_of_a_cone_has_no_parameters() {
        let cone = Surface::Cone { frame: frame(), radius: 0.0, half_angle: 0.4 };
        assert!(project(&cone, Point3::new(0.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn a_spheres_pole_has_no_parameters() {
        let sphere = Surface::Sphere { frame: frame(), radius: 4.0 };
        assert!(project(&sphere, Point3::new(0.0, 0.0, 4.0)).is_none());
    }

    /// A pole in the middle of a boundary borrows its neighbour's angle.
    ///
    /// Any angle is geometrically correct there — every one names the same
    /// point — but only the neighbour's keeps the polygon from jumping across
    /// the domain and back on its way through the pole.
    #[test]
    fn a_pole_takes_the_angle_of_the_point_before_it() {
        let mut walk = vec![
            Some(Point2::new(1.0, 0.0)),
            None, // the pole
            Some(Point2::new(1.1, 0.5)),
        ];

        unwrap_seam(&mut walk, true);

        let filled = walk[1].expect("the pole was filled");
        assert!((filled.u - 1.0).abs() < 1e-9, "took {} instead", filled.u);
    }

    /// A pole at the very start borrows forwards instead.
    #[test]
    fn a_pole_at_the_start_takes_from_after_it() {
        let mut walk = vec![None, Some(Point2::new(2.0, 0.3))];
        unwrap_seam(&mut walk, true);
        assert!((walk[0].expect("filled").u - 2.0).abs() < 1e-9);
    }

    /// A boundary of nothing but poles is left as it is rather than invented.
    #[test]
    fn a_boundary_with_no_usable_point_is_not_invented() {
        let mut walk = vec![None, None, None];
        unwrap_seam(&mut walk, true);
        assert!(walk.iter().all(|point| point.is_none()));
    }
}

//! Where the part is looked at from.
//!
//! An orbit camera: the model stays where it is and the eye moves around it on
//! a sphere. That is the convention every CAD viewer uses, and the reason is
//! that a part has no natural "forward" — turning the object is the thing
//! somebody means when they drag, and orbiting the eye is how that is done
//! without ever accumulating a rotation that cannot be undone.

use super::model::Point3;

/// A right-handed look at a point from a distance.
///
/// **A trackball, not a turntable.** The orientation is kept as three axes
/// rather than a yaw and a pitch, because a pitch has poles: it must be clamped
/// short of straight up, and a drag that reaches the clamp stops doing anything
/// while sideways drags keep working. That feels exactly like a broken control,
/// and it was one — the part would spin but would not tip past a point.
///
/// Three axes have no poles. Any orientation is reachable, dragging always
/// moves, and there is nothing to clamp.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// What is being looked at, which is normally the middle of the part.
    pub target: Point3,
    /// How far the eye is from it.
    pub distance: f64,
    /// Screen right, in the world.
    pub right: Point3,
    /// Screen up, in the world.
    pub up: Point3,
    /// Out of the screen towards the eye.
    pub back: Point3,
    /// Vertical field of view, in radians.
    pub fov: f64,
}

impl Default for Camera {
    fn default() -> Self {
        // Not straight on. A part viewed square to a face shows one rectangle
        // and reads as flat; the standard three-quarter view shows three faces
        // and reads as a solid immediately.
        let back = Point3::new(0.62, 0.47, 0.63)
            .normalised()
            .expect("a fixed non-zero direction");
        let right = Point3::new(0.0, 0.0, 1.0)
            .cross(back)
            .normalised()
            .expect("not parallel to z");

        Self {
            target: Point3::new(0.0, 0.0, 0.0),
            distance: 10.0,
            right,
            up: back.cross(right),
            back,
            fov: 45.0_f64.to_radians(),
        }
    }
}

/// Turn a vector about an axis, by Rodrigues' formula.
fn rotated(vector: Point3, axis: Point3, angle: f64) -> Point3 {
    let (sin, cos) = angle.sin_cos();
    vector
        .scaled(cos)
        .plus(axis.cross(vector).scaled(sin))
        .plus(axis.scaled(axis.dot(vector) * (1.0 - cos)))
}

impl Camera {
    /// Where the eye is.
    pub fn eye(&self) -> Point3 {
        self.target.plus(self.back.scaled(self.distance))
    }

    /// Frame a box so all of it is visible, whatever shape it is.
    ///
    /// **Fitted to the bounding sphere, not to the box.** A box fitted exactly
    /// is only fitted from the angle it was measured at: turn a long part
    /// forty-five degrees and its corners swing outside the view. The sphere
    /// through those corners is the same from every direction, so a part framed
    /// once stays framed however it is turned — which matters here, because
    /// turning it is the entire feature.
    pub fn fit(low: Point3, high: Point3, fov: f64) -> Self {
        let target = low.plus(high).scaled(0.5);
        let radius = high.minus(low).length() / 2.0;

        // A part with no size still has to be looked at from somewhere.
        let radius = if radius < 1e-9 { 1.0 } else { radius };
        let half = (fov / 2.0).max(1e-3);

        Self {
            target,
            // The tenth is breathing room, so the silhouette does not touch the
            // edge of the screen where it is hardest to read.
            distance: radius / half.sin() * 1.1,
            fov,
            ..Self::default()
        }
    }

    /// Tumble the view: `across` about the screen's vertical, `down` about its
    /// horizontal.
    ///
    /// Both rotations act on the axes themselves, so there is no orientation
    /// that cannot be reached and no direction in which dragging stops working.
    pub fn turned(&self, across: f64, down: f64) -> Self {
        let mut right = self.right;
        let mut up = self.up;
        let mut back = self.back;

        // About the current up first.
        right = rotated(right, up, across);
        back = rotated(back, up, across);

        // Then about the new right, which is what makes a second drag continue
        // from where the first left off rather than from the world's idea of up.
        up = rotated(up, right, down);
        back = rotated(back, right, down);

        // Re-squared each time. Rotations drift after enough of them, and a
        // basis that is no longer square stretches the picture in one direction
        // — which reads as the part being the wrong shape.
        let back = back.normalised().unwrap_or(self.back);
        let right = up
            .cross(back)
            .normalised()
            .unwrap_or_else(|| self.right);

        Self {
            right,
            up: back.cross(right),
            back,
            ..*self
        }
    }

    /// Closer or further away, never through the middle and out the other side.
    pub fn zoomed(&self, by: f64) -> Self {
        Self {
            distance: (self.distance * by).max(1e-6),
            ..*self
        }
    }

    /// A point in the world, as the eye sees it: x right, y up, z towards it.
    /// The three axes of the view: right, up, and back towards the eye.
    pub fn axes(&self) -> (Point3, Point3, Point3) {
        (self.right, self.up, self.back)
    }

    /// A point in the world, as the eye sees it: x right, y up, z towards it.
    pub fn to_view(&self, point: Point3) -> Point3 {
        let relative = point.minus(self.eye());
        Point3::new(
            relative.dot(self.right),
            relative.dot(self.up),
            relative.dot(self.back),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_of(size: f64) -> (Point3, Point3) {
        (
            Point3::new(-size, -size, -size),
            Point3::new(size, size, size),
        )
    }

    #[test]
    fn the_eye_is_the_stated_distance_from_the_target() {
        let camera = Camera { distance: 25.0, ..Camera::default() };
        let away = camera.eye().minus(camera.target).length();
        assert!((away - 25.0).abs() < 1e-9, "{away}");
    }

    #[test]
    fn fitting_looks_at_the_middle_of_the_part() {
        let camera = Camera::fit(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0, 20.0, 30.0),
            45.0_f64.to_radians(),
        );
        assert_eq!(Point3::new(5.0, 10.0, 15.0), camera.target);
    }

    /// A fitted part stays in view from any angle.
    ///
    /// This is the reason for fitting to the sphere. Fitting the box exactly
    /// works from the angle it was measured at and fails as soon as the part is
    /// turned — corners swing out of frame, which looks like the model growing.
    #[test]
    fn a_fitted_part_stays_in_frame_however_it_is_turned() {
        let (low, high) = box_of(5.0);
        let fov = 45.0_f64.to_radians();
        let base = Camera::fit(low, high, fov);

        let corners = [
            Point3::new(low.x, low.y, low.z),
            Point3::new(high.x, low.y, low.z),
            Point3::new(low.x, high.y, low.z),
            Point3::new(high.x, high.y, low.z),
            Point3::new(low.x, low.y, high.z),
            Point3::new(high.x, low.y, high.z),
            Point3::new(low.x, high.y, high.z),
            Point3::new(high.x, high.y, high.z),
        ];

        for step in 0..16 {
            let camera = base.turned(step as f64 * 0.4, (step as f64 * 0.3).sin());
            for corner in corners {
                let view = camera.to_view(corner);
                let depth = -view.z;
                assert!(depth > 0.0, "a corner ended up behind the eye");
                // How far off the middle it may be at that depth.
                let half_height = depth * (fov / 2.0).tan();
                assert!(
                    view.y.abs() <= half_height,
                    "corner {corner:?} left the frame at step {step}",
                );
            }
        }
    }

    /// A part with no thickness is still looked at from somewhere sensible.
    #[test]
    fn a_flat_part_does_not_put_the_eye_inside_it() {
        let camera = Camera::fit(
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(10.0, 10.0, 0.0),
            45.0_f64.to_radians(),
        );
        assert!(camera.distance > 1.0, "{}", camera.distance);
    }

    /// And a part of no size at all does not divide by zero.
    #[test]
    fn a_part_of_no_size_still_gets_a_camera() {
        let point = Point3::new(3.0, 3.0, 3.0);
        let camera = Camera::fit(point, point, 45.0_f64.to_radians());
        assert!(camera.distance.is_finite() && camera.distance > 0.0);
    }

    /// **There is no orientation that cannot be reached.**
    ///
    /// The turntable this replaced had to clamp its pitch short of straight up,
    /// and a drag that reached the clamp did nothing at all while sideways
    /// drags kept working — which is what "it spins but will not tip" was.
    #[test]
    fn tilting_never_runs_out() {
        let mut camera = Camera::default();
        let mut seen: Vec<Point3> = Vec::new();

        // Twenty drags in the same direction, each a sixth of a turn.
        for _ in 0..20 {
            let before = camera.back;
            camera = camera.turned(0.0, 1.0);
            assert!(
                camera.back.minus(before).length() > 1e-6,
                "a drag stopped having any effect: {:?}",
                camera.back,
            );
            seen.push(camera.back);
        }

        // And it goes right over the top rather than stopping there: some pose
        // ends up looking from below.
        assert!(
            seen.iter().any(|back| back.z < -0.5),
            "never got past the top",
        );
    }

    /// Turning all the way round comes back to where it started.
    #[test]
    fn a_full_turn_returns_to_the_start() {
        let start = Camera::default();
        let mut camera = start;
        for _ in 0..8 {
            camera = camera.turned(std::f64::consts::FRAC_PI_4, 0.0);
        }

        assert!(
            camera.back.minus(start.back).length() < 1e-6,
            "{:?} against {:?}",
            camera.back,
            start.back,
        );
    }

    /// The axes stay square however long it is turned for.
    ///
    /// Rotations accumulate error, and a basis that is no longer square
    /// stretches the picture in one direction — which reads as the part being
    /// the wrong shape rather than as the camera drifting.
    #[test]
    fn the_axes_stay_square_after_a_long_tumble() {
        let mut camera = Camera::default();
        for step in 0..500 {
            camera = camera.turned(0.31 + step as f64 * 1e-4, -0.17);
        }

        let (right, up, back) = camera.axes();
        for axis in [right, up, back] {
            assert!((axis.length() - 1.0).abs() < 1e-9, "not unit: {axis:?}");
        }
        assert!(right.dot(up).abs() < 1e-9, "right and up drifted apart");
        assert!(right.dot(back).abs() < 1e-9, "right and back drifted apart");
        assert!(up.dot(back).abs() < 1e-9, "up and back drifted apart");
    }

    /// Zooming out and back in returns to where it started.
    #[test]
    fn zooming_is_reversible() {
        let camera = Camera { distance: 10.0, ..Camera::default() };
        let there_and_back = camera.zoomed(3.0).zoomed(1.0 / 3.0);
        assert!((there_and_back.distance - 10.0).abs() < 1e-9);
    }

    /// Zooming right in never puts the eye through the far side.
    #[test]
    fn zooming_in_stops_before_the_target() {
        let mut camera = Camera { distance: 10.0, ..Camera::default() };
        for _ in 0..200 {
            camera = camera.zoomed(0.5);
        }
        assert!(camera.distance > 0.0, "{}", camera.distance);
    }

    /// The view's axes are perpendicular and unit length.
    ///
    /// A skewed frame stretches the picture in one direction, which reads as
    /// the part being the wrong shape rather than the camera being wrong.
    #[test]
    fn the_view_axes_are_square_to_each_other() {
        for step in 0..12 {
            let camera = Camera::default().turned(step as f64 * 0.5, step as f64 * 0.2);
            let (right, up, back) = camera.axes();

            for axis in [right, up, back] {
                assert!((axis.length() - 1.0).abs() < 1e-9, "not unit: {axis:?}");
            }
            assert!(right.dot(up).abs() < 1e-9);
            assert!(right.dot(back).abs() < 1e-9);
            assert!(up.dot(back).abs() < 1e-9);
        }
    }

/// Looking from directly above is an ordinary pose, not a special case.    ///    /// It is the pole a turntable camera has to guard against; a trackball has    /// none.    #[test]    fn looking_straight_down_does_not_collapse_the_view() {        let camera = Camera::default().turned(0.0, std::f64::consts::FRAC_PI_2);        let (right, up, back) = camera.axes();        for axis in [right, up, back] {            assert!((axis.length() - 1.0).abs() < 1e-9, "collapsed: {axis:?}");        }    }

    /// The target sits straight ahead: no sideways or vertical offset.
    #[test]
    fn the_target_is_in_the_middle_of_the_view() {
        let camera = Camera::default();
        let view = camera.to_view(camera.target);

        assert!(view.x.abs() < 1e-9, "off to one side: {}", view.x);
        assert!(view.y.abs() < 1e-9, "off vertically: {}", view.y);
        assert!(view.z < 0.0, "the target is behind the eye");
    }
}

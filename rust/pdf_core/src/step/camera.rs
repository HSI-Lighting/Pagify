//! Where the part is looked at from.
//!
//! An orbit camera: the model stays where it is and the eye moves around it on
//! a sphere. That is the convention every CAD viewer uses, and the reason is
//! that a part has no natural "forward" — turning the object is the thing
//! somebody means when they drag, and orbiting the eye is how that is done
//! without ever accumulating a rotation that cannot be undone.

use super::model::Point3;

/// A right-handed look at a point from a distance.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// What is being looked at, which is normally the middle of the part.
    pub target: Point3,
    /// How far the eye is from it.
    pub distance: f64,
    /// Rotation about the vertical, in radians.
    pub yaw: f64,
    /// Rotation above and below the horizon, in radians.
    pub pitch: f64,
    /// Vertical field of view, in radians.
    pub fov: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: Point3::new(0.0, 0.0, 0.0),
            distance: 10.0,
            // Not straight on. A part viewed square to a face shows one
            // rectangle and reads as flat; the standard three-quarter view
            // shows three faces and reads as a solid immediately.
            yaw: 0.6,
            pitch: 0.5,
            fov: 45.0_f64.to_radians(),
        }
    }
}

impl Camera {
    /// Where the eye is.
    pub fn eye(&self) -> Point3 {
        let horizontal = self.distance * self.pitch.cos();
        self.target.plus(Point3::new(
            horizontal * self.yaw.cos(),
            horizontal * self.yaw.sin(),
            self.distance * self.pitch.sin(),
        ))
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

    /// The pitch, kept off the poles.
    ///
    /// Straight up is where the view's own idea of "up" stops being defined and
    /// the model spins about the eye instead of turning. Clamping just short of
    /// it costs nothing anybody wants and removes a way for the view to become
    /// unusable with no obvious way back.
    pub fn turned(&self, by_yaw: f64, by_pitch: f64) -> Self {
        const NEARLY_UP: f64 = std::f64::consts::FRAC_PI_2 - 0.01;
        Self {
            yaw: self.yaw + by_yaw,
            pitch: (self.pitch + by_pitch).clamp(-NEARLY_UP, NEARLY_UP),
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

    /// The three axes of the view: right, up, and back towards the eye.
    pub fn axes(&self) -> (Point3, Point3, Point3) {
        let back = self
            .eye()
            .minus(self.target)
            .normalised()
            .unwrap_or(Point3::new(0.0, 0.0, 1.0));

        // World up, unless the eye is nearly over the pole, where it says
        // nothing about which way round the view should be.
        let world_up = if back.z.abs() > 0.999 {
            Point3::new(0.0, 1.0, 0.0)
        } else {
            Point3::new(0.0, 0.0, 1.0)
        };

        let right = world_up
            .cross(back)
            .normalised()
            .unwrap_or(Point3::new(1.0, 0.0, 0.0));
        let up = back.cross(right);

        (right, up, back)
    }

    /// A point in the world, as the eye sees it: x right, y up, z towards it.
    pub fn to_view(&self, point: Point3) -> Point3 {
        let (right, up, back) = self.axes();
        let relative = point.minus(self.eye());
        Point3::new(relative.dot(right), relative.dot(up), relative.dot(back))
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

    /// Turning past straight up is where a view stops being recoverable.
    #[test]
    fn the_pitch_never_reaches_the_pole() {
        let mut camera = Camera::default();
        for _ in 0..50 {
            camera = camera.turned(0.0, 1.0);
        }
        assert!(camera.pitch < std::f64::consts::FRAC_PI_2, "{}", camera.pitch);

        for _ in 0..100 {
            camera = camera.turned(0.0, -1.0);
        }
        assert!(camera.pitch > -std::f64::consts::FRAC_PI_2, "{}", camera.pitch);
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

    /// Looking from directly above still produces a usable frame.
    #[test]
    fn looking_straight_down_does_not_collapse_the_view() {
        let camera = Camera {
            pitch: std::f64::consts::FRAC_PI_2 - 1e-6,
            ..Camera::default()
        };
        let (right, up, back) = camera.axes();
        assert!(right.length() > 0.9 && up.length() > 0.9 && back.length() > 0.9);
    }

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

//! Page-scale calibration and measurement — build plan phase 9.
//!
//! §6: a PDF of an architectural drawing is at *some* scale — 1:50, 1:100 — and
//! nothing in the file reliably says which. So before any measurement means
//! anything, the user picks two points and states the real distance between
//! them. That fixes the mapping from page points to real units for that
//! document.
//!
//! This is what `units` becomes on this side of the line. It is not a document
//! unit system: it is a per-document calibration, stored with the file,
//! revisable, and **clearly displayed** — because a wrong calibration silently
//! produces confident wrong numbers, which is worse than no number at all.
//! [`Calibration::describe`] exists so there is never an excuse not to show it,
//! and [`Measurement::render`] refuses to print a bare number without a unit.

use serde::{Deserialize, Serialize};

use crate::page_space::AppPoint;

/// The reference the calibration was derived from, kept so it can be shown and
/// revised rather than becoming an unexplained constant.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub stated: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Real-world units per page point.
    units_per_point: f64,
    /// What those units are called. Free text on purpose — a drawing may be in
    /// metres, feet, or chains, and Pagify has no business having opinions.
    pub unit: String,
    pub reference: Option<Reference>,
}

impl Default for Calibration {
    /// Page points, one for one. Not a guess at a real-world scale: an
    /// uncalibrated document measures in the only unit that is actually known
    /// about it, and says so.
    fn default() -> Self {
        Calibration { units_per_point: 1.0, unit: "pt".into(), reference: None }
    }
}

impl Calibration {
    /// Calibrate from two points on the page and the real distance between them.
    pub fn from_two_points(
        from: AppPoint,
        to: AppPoint,
        stated: f64,
        unit: &str,
    ) -> Result<Self, String> {
        let span = distance(from, to);
        if span < 1e-6 {
            return Err("calibrate: those two points are the same place".into());
        }
        if !stated.is_finite() || stated <= 0.0 {
            return Err("calibrate: the real distance must be a positive number".into());
        }
        if unit.trim().is_empty() {
            return Err("calibrate: say what unit that distance is in".into());
        }

        Ok(Calibration {
            units_per_point: stated / span,
            unit: unit.trim().to_string(),
            reference: Some(Reference {
                from: [from.x, from.y],
                to: [to.x, to.y],
                stated,
            }),
        })
    }

    pub fn is_calibrated(&self) -> bool {
        self.reference.is_some()
    }

    pub fn units_per_point(&self) -> f64 {
        self.units_per_point
    }

    pub fn length(&self, points: f64) -> f64 {
        points * self.units_per_point
    }

    /// Area scales by the *square* of the linear factor. Getting this wrong is
    /// the classic surveying error, and it is wrong by a factor of the scale —
    /// on a 1:100 drawing, by a hundred.
    pub fn area(&self, square_points: f64) -> f64 {
        square_points * self.units_per_point * self.units_per_point
    }

    /// A line that must be shown wherever a measurement is.
    pub fn describe(&self) -> String {
        match &self.reference {
            Some(r) => format!(
                "1 pt = {:.6} {} (from a stated {} {} between two points)",
                self.units_per_point, self.unit, r.stated, self.unit
            ),
            None => "not calibrated — measurements are in page points".to_string(),
        }
    }
}

/// A measured quantity, which knows whether it can be trusted.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    pub value: f64,
    pub unit: String,
    pub squared: bool,
    /// False when the document has never been calibrated.
    pub calibrated: bool,
}

impl Measurement {
    /// Formatted for display, never as a bare number.
    ///
    /// An uncalibrated measurement says so in the same breath as the figure.
    /// The failure mode this guards against is someone reading "12.4" off a
    /// screen and believing it is metres.
    pub fn render(&self) -> String {
        let unit = if self.squared { format!("{}²", self.unit) } else { self.unit.clone() };
        let figure = format!("{:.3} {unit}", self.value);
        if self.calibrated {
            figure
        } else {
            format!("{figure} — not calibrated, so this is page geometry, not real distance")
        }
    }
}

pub fn distance(a: AppPoint, b: AppPoint) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

/// Total length along a run of points, in page points.
pub fn path_length(points: &[AppPoint]) -> f64 {
    points.windows(2).map(|w| distance(w[0], w[1])).sum()
}

/// Area of a closed polygon in square page points, by the shoelace formula.
///
/// Absolute, so winding direction does not decide the sign — nobody measuring a
/// room wants a negative area because they clicked anticlockwise.
pub fn polygon_area(points: &[AppPoint]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut twice = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        twice += a.x * b.y - b.x * a.y;
    }
    (twice / 2.0).abs()
}

pub fn measure_distance(calibration: &Calibration, a: AppPoint, b: AppPoint) -> Measurement {
    Measurement {
        value: calibration.length(distance(a, b)),
        unit: calibration.unit.clone(),
        squared: false,
        calibrated: calibration.is_calibrated(),
    }
}

pub fn measure_path(calibration: &Calibration, points: &[AppPoint]) -> Measurement {
    Measurement {
        value: calibration.length(path_length(points)),
        unit: calibration.unit.clone(),
        squared: false,
        calibrated: calibration.is_calibrated(),
    }
}

pub fn measure_area(calibration: &Calibration, points: &[AppPoint]) -> Measurement {
    Measurement {
        value: calibration.area(polygon_area(points)),
        unit: calibration.unit.clone(),
        squared: true,
        calibrated: calibration.is_calibrated(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> AppPoint {
        AppPoint::new(x, y)
    }

    #[test]
    fn calibrating_from_two_points_fixes_the_scale() {
        // 200 points across, stated as 10 metres: 1 pt = 0.05 m.
        let c = Calibration::from_two_points(p(100.0, 100.0), p(300.0, 100.0), 10.0, "m").unwrap();
        assert!((c.units_per_point() - 0.05).abs() < 1e-12);
        assert!((c.length(400.0) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn area_scales_by_the_square_of_the_linear_factor() {
        // The classic surveying error. On a 1:100 drawing, getting this wrong
        // is wrong by a hundred.
        let c = Calibration::from_two_points(p(0.0, 0.0), p(100.0, 0.0), 10.0, "m").unwrap();
        assert!((c.units_per_point() - 0.1).abs() < 1e-12);

        // A 100x100 pt square is 10m x 10m = 100 m², not 10 m².
        assert!((c.area(100.0 * 100.0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_degenerate_or_nonsense_calibration_is_refused() {
        assert!(Calibration::from_two_points(p(5.0, 5.0), p(5.0, 5.0), 10.0, "m").is_err());
        assert!(Calibration::from_two_points(p(0.0, 0.0), p(10.0, 0.0), 0.0, "m").is_err());
        assert!(Calibration::from_two_points(p(0.0, 0.0), p(10.0, 0.0), -3.0, "m").is_err());
        assert!(Calibration::from_two_points(p(0.0, 0.0), p(10.0, 0.0), f64::NAN, "m").is_err());
        assert!(Calibration::from_two_points(p(0.0, 0.0), p(10.0, 0.0), 5.0, "  ").is_err());
    }

    #[test]
    fn an_uncalibrated_document_measures_in_points_and_says_so() {
        let c = Calibration::default();
        assert!(!c.is_calibrated());
        assert!(c.describe().contains("not calibrated"));

        let m = measure_distance(&c, p(0.0, 0.0), p(3.0, 4.0));
        assert_eq!(m.value, 5.0);
        assert!(m.render().contains("not calibrated"), "a bare number invites being read as metres");
    }

    #[test]
    fn a_calibrated_measurement_still_always_carries_its_unit() {
        let c = Calibration::from_two_points(p(0.0, 0.0), p(100.0, 0.0), 5.0, "m").unwrap();
        let m = measure_distance(&c, p(0.0, 0.0), p(100.0, 0.0));
        assert_eq!(m.render(), "5.000 m");

        let a = measure_area(&c, &[p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0), p(0.0, 100.0)]);
        assert_eq!(a.render(), "25.000 m²");
    }

    #[test]
    fn the_calibration_shows_where_it_came_from() {
        let c = Calibration::from_two_points(p(0.0, 0.0), p(200.0, 0.0), 10.0, "m").unwrap();
        let shown = c.describe();
        assert!(shown.contains("10"), "does not say what was stated: {shown}");
        assert!(shown.contains('m'), "does not say the unit: {shown}");
    }

    #[test]
    fn area_does_not_care_which_way_round_the_points_were_clicked() {
        let clockwise = [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
        let widdershins = [p(0.0, 10.0), p(10.0, 10.0), p(10.0, 0.0), p(0.0, 0.0)];

        assert_eq!(polygon_area(&clockwise), 100.0);
        assert_eq!(polygon_area(&widdershins), 100.0);
    }

    #[test]
    fn an_unclosed_shape_has_no_area_rather_than_a_wrong_one() {
        assert_eq!(polygon_area(&[]), 0.0);
        assert_eq!(polygon_area(&[p(0.0, 0.0)]), 0.0);
        assert_eq!(polygon_area(&[p(0.0, 0.0), p(10.0, 0.0)]), 0.0);
    }

    #[test]
    fn a_path_measures_along_every_leg() {
        assert_eq!(path_length(&[p(0.0, 0.0), p(3.0, 4.0), p(3.0, 14.0)]), 15.0);
    }

    #[test]
    fn a_calibration_survives_being_written_down() {
        let c = Calibration::from_two_points(p(0.0, 0.0), p(200.0, 0.0), 10.0, "m").unwrap();
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Calibration>(&json).unwrap(), c);
    }
}

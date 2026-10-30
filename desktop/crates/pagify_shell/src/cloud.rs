//! Revision-cloud outlines — the scalloped border Foxit's Cloud markup draws.
//!
//! A cloud is the polygon somebody clicks, with every edge replaced by a run
//! of outward circular arcs ("scallops"). [`outline`] returns that drawing as
//! a closed ring of points in the same space as its input, so the Draw tab can
//! keep it as ordinary `Geom::Polyline` geometry: the live overlay, selecting,
//! moving, saving and reopening all treat it as a shape, and nothing else has
//! to know what a cloud is.
//!
//! Points rather than bulges on purpose: `commit::flatten` does not expand a
//! `PolyVertex::bulge`, so a bulge-drawn cloud would print as its plain
//! polygon. Tessellating once, here, is what makes the overlay and the page
//! agree.
//!
//! Pure: no window, no engine, no page space. The caller passes points in
//! whatever space it draws in; orientation is read from the polygon itself, so
//! app space (y down) and kernel space (y up) both come out the same way round.

use cad_kernel::Vec2;

/// The chord length one scallop aims for, in page points.
///
/// Foxit's own clouds put about one bump per 15–25 pt of border; this sits in
/// the middle and gives an A4 page's edge about fifteen bumps.
pub const SCALLOP_PITCH_PT: f64 = 20.0;

/// How high a scallop stands above its chord, as a share of the chord.
///
/// A little under a third is the shape a revision cloud is drawn with: deep
/// enough to read as a cloud, shallow enough that neighbouring bumps still
/// meet at a crisp inward cusp.
pub const SCALLOP_HEIGHT: f64 = 0.32;

/// Points closer together than this are the same click.
const SAME_POINT_PT: f64 = 0.05;

/// Arc segments per full circle. A scallop sweeps about 130°, so this lands
/// on roughly nine points a bump — smooth at any zoom the page is read at,
/// and a few hundred points for a whole cloud rather than thousands.
const ARC_SEGMENTS_PER_TURN: f64 = 24.0;

/// How much area a ring must enclose before it is a cloud rather than a
/// line of clicks, in square points.
const LEAST_AREA_PT2: f64 = 1.0;

/// The scalloped ring of a closed polygon through `points`, in the same space.
///
/// Empty when there are fewer than three distinct points or the points enclose
/// no area — the caller says so rather than adding a shape with nothing in it.
/// Consecutive duplicate points (including a last point equal to the first)
/// are ignored, so a person who clicks "back home" does not get a zero edge.
pub fn outline(points: &[Vec2]) -> Vec<Vec2> {
    let ring = without_repeats(points);
    if ring.len() < 3 {
        return Vec::new();
    }
    let area = signed_area(&ring);
    if area.abs() < LEAST_AREA_PT2 {
        return Vec::new();
    }
    // Outward is the right of each directed edge on a ring with positive
    // signed area, and the left on one with negative area. That is the same
    // rule in either handedness, which is what lets app space (y down) use it
    // without a conversion.
    let outward = if area > 0.0 { 1.0 } else { -1.0 };

    let mut out = Vec::new();
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        let edge = b - a;
        let length = edge.len();
        if length < SAME_POINT_PT {
            continue;
        }
        let normal = Vec2::new(edge.y, -edge.x) / length * outward;
        let count = ((length / SCALLOP_PITCH_PT).round() as usize).max(1);
        let chord = length / count as f64;
        let sagitta = chord * SCALLOP_HEIGHT;
        for k in 0..count {
            let t0 = k as f64 / count as f64;
            let t1 = (k + 1) as f64 / count as f64;
            let from = a + edge * t0;
            let to = a + edge * t1;
            if out.is_empty() {
                out.push(from);
            }
            scallop(&mut out, from, to, normal, sagitta);
        }
    }

    // The last scallop ends back on the first point; the polyline's own
    // `closed` flag closes the ring, so that repeated point would only add a
    // zero-length edge.
    if out.len() > 1 && (out[out.len() - 1] - out[0]).len() < SAME_POINT_PT {
        out.pop();
    }
    out
}

/// Push the points of one outward arc from `from` to `to`, ending on `to`.
fn scallop(out: &mut Vec<Vec2>, from: Vec2, to: Vec2, normal: Vec2, sagitta: f64) {
    let chord = to - from;
    let length = chord.len();
    if length < 1e-9 || sagitta <= 0.0 {
        out.push(to);
        return;
    }
    // The circle through both ends whose bulge away from the chord is
    // `sagitta`. Its centre sits behind the chord, so `normal` points from
    // the centre to the bump's highest point.
    let radius = (length * length / 4.0 + sagitta * sagitta) / (2.0 * sagitta);
    let centre = (from + to) * 0.5 - normal * (radius - sagitta);
    let start = (from - centre).y.atan2((from - centre).x);
    let end = (to - centre).y.atan2((to - centre).x);
    // The bump is the minor arc (a sagitta under the radius guarantees that),
    // so the shortest signed sweep is the one through the outward apex.
    let mut sweep = end - start;
    while sweep > std::f64::consts::PI {
        sweep -= std::f64::consts::TAU;
    }
    while sweep < -std::f64::consts::PI {
        sweep += std::f64::consts::TAU;
    }
    let step = std::f64::consts::TAU / ARC_SEGMENTS_PER_TURN;
    let steps = ((sweep.abs() / step).ceil() as usize).clamp(3, 16);
    for i in 1..=steps {
        let angle = start + sweep * (i as f64 / steps as f64);
        out.push(centre + Vec2::new(radius * angle.cos(), radius * angle.sin()));
    }
}

/// The ring without consecutive repeats, and without a closing point that
/// only repeats the first.
fn without_repeats(points: &[Vec2]) -> Vec<Vec2> {
    let mut ring: Vec<Vec2> = Vec::new();
    for &point in points {
        match ring.last() {
            Some(&last) if (last - point).len() < SAME_POINT_PT => {}
            _ => ring.push(point),
        }
    }
    if ring.len() > 1 {
        let first = ring[0];
        let last = *ring.last().expect("more than one point");
        if (last - first).len() < SAME_POINT_PT {
            ring.pop();
        }
    }
    ring
}

/// Twice the signed area, halved — positive for one direction round the ring,
/// negative for the other.
fn signed_area(ring: &[Vec2]) -> f64 {
    let mut twice = 0.0;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        twice += a.x * b.y - b.x * a.y;
    }
    twice * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(clockwise: bool) -> Vec<Vec2> {
        let mut ring = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(0.0, 100.0),
        ];
        if clockwise {
            ring.reverse();
        }
        ring
    }

    fn bounds(points: &[Vec2]) -> (f64, f64, f64, f64) {
        points.iter().fold(
            (
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ),
            |(min_x, max_x, min_y, max_y), p| {
                (
                    min_x.min(p.x),
                    max_x.max(p.x),
                    min_y.min(p.y),
                    max_y.max(p.y),
                )
            },
        )
    }

    #[test]
    fn a_square_becomes_a_scalloped_ring_that_closes_on_itself() {
        let ring = outline(&square(false));
        assert!(
            ring.len() > 4 * 4,
            "too few points to be scalloped: {}",
            ring.len()
        );
        // Tessellated, so the last point is the last point of the closing
        // scallop and the ring is completed by `closed: true` — it must end
        // next to where it began rather than back inside the shape.
        let gap = (ring[ring.len() - 1] - ring[0]).len();
        assert!(
            gap < SCALLOP_PITCH_PT,
            "the ring does not come back round: {gap} pt"
        );
    }

    #[test]
    fn every_scallop_stands_outside_the_polygon() {
        let ring = outline(&square(false));
        let (min_x, max_x, min_y, max_y) = bounds(&ring);
        // A 100 pt side is five chords of 20 pt, and each bump is 0.32 of its
        // chord: about 6.4 pt out. Every side must reach past the polygon by
        // about that much.
        for (side, extent) in [
            ("left", -min_x),
            ("right", max_x - 100.0),
            ("top", -min_y),
            ("bottom", max_y - 100.0),
        ] {
            assert!(
                extent > 4.0 && extent < 7.5,
                "the {side} scallops stand {extent} pt out, not about 6.4"
            );
        }
        // And nothing from the other side leaks into the polygon's interior.
        assert!(min_x >= -7.5 && max_x <= 107.5 && min_y >= -7.5 && max_y <= 107.5);
    }

    #[test]
    fn the_ring_going_the_other_way_round_bulges_out_too() {
        let forwards = outline(&square(false));
        let backwards = outline(&square(true));
        let (min_x, max_x, min_y, max_y) = bounds(&backwards);
        assert!(
            min_x < -4.0,
            "clockwise input bulged inward on the left: {min_x}"
        );
        assert!(
            max_x > 104.0,
            "clockwise input bulged inward on the right: {max_x}"
        );
        assert!(
            min_y < -4.0,
            "clockwise input bulged inward on the top: {min_y}"
        );
        assert!(
            max_y > 104.0,
            "clockwise input bulged inward on the bottom: {max_y}"
        );
        assert!(
            (forwards.len() as i64 - backwards.len() as i64).abs() <= 2,
            "the direction of travel changed how many points the cloud has"
        );
    }

    #[test]
    fn fewer_than_three_points_enclose_no_cloud() {
        let points = [Vec2::new(0.0, 0.0), Vec2::new(50.0, 50.0)];
        assert!(outline(&points).is_empty());
        assert!(outline(&[]).is_empty());
    }

    #[test]
    fn clicks_in_a_line_enclose_no_cloud() {
        let points = [
            Vec2::new(0.0, 0.0),
            Vec2::new(50.0, 0.0),
            Vec2::new(100.0, 0.0),
        ];
        assert!(outline(&points).is_empty());
    }

    #[test]
    fn a_click_that_repeats_the_last_one_does_not_make_a_zero_edge() {
        let plain = outline(&square(false));
        let repeated = outline(&[
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(0.0, 100.0),
            Vec2::new(0.0, 100.0),
        ]);
        assert_eq!(plain.len(), repeated.len());
    }

    #[test]
    fn a_closing_click_on_the_first_point_does_not_add_an_edge() {
        let plain = outline(&square(false));
        let closed = outline(&[
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(0.0, 100.0),
            Vec2::new(0.0, 0.0),
        ]);
        assert_eq!(plain.len(), closed.len());
    }

    #[test]
    fn a_long_edge_gets_more_scallops_than_a_short_one() {
        let mut wide = square(false);
        wide[1] = Vec2::new(400.0, 0.0);
        wide[2] = Vec2::new(400.0, 100.0);
        let short = outline(&square(false));
        let long = outline(&wide);
        assert!(
            long.len() > short.len(),
            "a 400 pt edge was not given more bumps"
        );
    }
}

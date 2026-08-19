//! Turning a drawn stroke into a shape.
//!
//! Pure geometry: no PDFium, no I/O, no allocation beyond a couple of small
//! vectors — so it lives in the core and is testable on the host, and so it can
//! be called on the gesture's own thread without queueing behind a render.
//!
//! ## The failure mode this is shaped around
//!
//! **Over-eager snapping is worse than no snapping.** A deliberate squiggle
//! turning into a circle is infuriating in a way a squiggle staying a squiggle
//! never is: the user loses work they meant, and the only way to get it back is
//! to draw it again and hope. So every branch below defaults to
//! [`Shape::Freehand`] when it is not sure, and the residual test refuses rather
//! than picking the nearer of two bad fits.
//!
//! The UI adds the other half of that guarantee: the shape only replaces the
//! stroke after a deliberate dwell at the end of the drag, with the snapped
//! result shown during it, so recognition never surprises anyone.
//!
//! ## What is not here
//!
//! A corpus of **real** strokes. Every stroke in the tests below is generated,
//! and generated strokes — even jittered ones — are cleaner than a finger on
//! glass: they have even spacing, no hesitation, no overshoot at the corners and
//! no double-back at the end. A recogniser tuned only against them will pass here
//! and misbehave on a device. `examples/dump_strokes.rs` and the app's stroke
//! recorder exist to collect the real thing; until that corpus is committed, the
//! numbers below are provisional and this comment is the honest statement of it.

use crate::document::{Point, Rect};
use crate::render::markup::Shape;

/// Points every stroke is resampled to before anything is measured.
///
/// Resampling is not a tidying step — it is what makes the measurements mean
/// anything. Raw touch samples are spaced by *speed*: a slow corner gets fifty
/// points and a fast edge gets three, so an unresampled stroke's every metric is
/// dominated by how the hand moved rather than by the shape it drew.
const RESAMPLE_POINTS: usize = 64;

/// How close the ends must be, relative to the bounding box's diagonal, for a
/// stroke to count as closed.
const CLOSED_FRACTION: f32 = 0.20;

/// End-to-end distance over path length, above which an open stroke is a line.
const STRAIGHTNESS: f32 = 0.95;

/// Worst fit still accepted, as a fraction of the bounding diagonal.
///
/// Past this the answer is [`Shape::Freehand`] — refusing to guess, rather than
/// picking whichever of two poor fits is marginally less poor.
const SNAP_THRESHOLD: f32 = 0.07;

/// `4πA/P²`: 1.0 for a circle, about 0.785 for a square.
const CIRCULARITY_OF_A_CIRCLE: f32 = 0.87;

/// Above this many corners after simplification, a closed stroke is not a
/// rectangle. Four corners plus a start/end pair that did not quite meet is
/// normal, so this is not 4.
const MAX_RECT_CORNERS: usize = 7;

/// How far a rectangle may be off-axis and still snap, in radians (about 8°).
///
/// The shape vocabulary has no rotated rectangle, so snapping one would quietly
/// straighten a deliberately angled box. Past this it stays freehand, which is at
/// least what was drawn.
const MAX_RECT_TILT: f32 = 0.14;

/// Recognise a stroke, or decline to.
///
/// `points` are in page points, in the order they were drawn.
pub fn recognise(points: &[Point]) -> Shape {
    let freehand = || Shape::Freehand {
        points: points.to_vec(),
    };

    if points.len() < 3 {
        return freehand();
    }

    let stroke = resample(points, RESAMPLE_POINTS);
    let bounds = bounding_box(&stroke);
    let diagonal = (bounds.width().powi(2) + bounds.height().powi(2)).sqrt();
    if diagonal <= f32::EPSILON {
        return freehand();
    }

    let first = stroke[0];
    let last = stroke[stroke.len() - 1];
    let gap = distance(first, last);

    if gap >= CLOSED_FRACTION * diagonal {
        // Open. The only thing an open stroke can be is a line.
        let length = path_length(&stroke);
        return if length > 0.0 && gap / length > STRAIGHTNESS {
            Shape::Line {
                from: *points.first().expect("checked non-empty"),
                to: *points.last().expect("checked non-empty"),
            }
        } else {
            freehand()
        };
    }

    // Closed. Circularity is the primary discriminator — one number that is 1.0
    // for a circle and 0.785 for a square. The fit residuals and the corner count
    // are cross-checks on it, not independent votes.
    let area = shoelace_area(&stroke);
    let perimeter = path_length(&stroke) + gap;
    if perimeter <= 0.0 {
        return freehand();
    }
    let circularity = 4.0 * std::f32::consts::PI * area / (perimeter * perimeter);

    let circle_residual = circle_fit_residual(&stroke) / diagonal;
    let (rect_residual, tilt) = rect_fit_residual(&stroke);
    let rect_residual = rect_residual / diagonal;

    if circle_residual.min(rect_residual) > SNAP_THRESHOLD {
        return freehand();
    }

    let corners = simplify(&stroke, 0.02 * diagonal).len();

    if circle_residual <= rect_residual {
        if circularity >= CIRCULARITY_OF_A_CIRCLE {
            Shape::Ellipse { rect: bounds }
        } else {
            freehand()
        }
    } else if corners <= MAX_RECT_CORNERS
        && circularity < CIRCULARITY_OF_A_CIRCLE
        && tilt <= MAX_RECT_TILT
    {
        Shape::Rect { rect: bounds }
    } else {
        freehand()
    }
}

// ------------------------------------------------------------------ geometry --

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn path_length(points: &[Point]) -> f32 {
    points.windows(2).map(|p| distance(p[0], p[1])).sum()
}

fn bounding_box(points: &[Point]) -> Rect {
    let mut bounds = Rect {
        left: f32::MAX,
        top: f32::MAX,
        right: f32::MIN,
        bottom: f32::MIN,
    };
    for p in points {
        bounds.left = bounds.left.min(p.x);
        bounds.right = bounds.right.max(p.x);
        bounds.top = bounds.top.min(p.y);
        bounds.bottom = bounds.bottom.max(p.y);
    }
    bounds
}

trait RectExt {
    fn width(&self) -> f32;
    fn height(&self) -> f32;
}

impl RectExt for Rect {
    fn width(&self) -> f32 {
        self.right - self.left
    }
    fn height(&self) -> f32 {
        self.bottom - self.top
    }
}

/// Evenly spaced points along the stroke, endpoints preserved.
fn resample(points: &[Point], count: usize) -> Vec<Point> {
    let total = path_length(points);
    if total <= 0.0 || count < 2 {
        return points.to_vec();
    }

    let step = total / (count - 1) as f32;
    let mut out = Vec::with_capacity(count);
    out.push(points[0]);

    let mut travelled = 0.0f32;
    let mut target = step;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let segment = distance(a, b);
        if segment <= 0.0 {
            continue;
        }
        while travelled + segment >= target && out.len() < count - 1 {
            let t = (target - travelled) / segment;
            out.push(Point {
                x: a.x + t * (b.x - a.x),
                y: a.y + t * (b.y - a.y),
            });
            target += step;
        }
        travelled += segment;
    }

    out.push(*points.last().expect("checked non-empty"));
    out
}

/// Twice-the-signed-area formula, made positive: the stroke may be drawn either
/// way round and the direction carries no meaning here.
fn shoelace_area(points: &[Point]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    (sum / 2.0).abs()
}

/// Mean distance from the best-fit circle, in the stroke's own units.
///
/// Kåsa's algebraic fit: solving for the centre directly, which is exact for a
/// clean circle and close enough for a drawn one, with none of the iteration a
/// geometric fit needs.
fn circle_fit_residual(points: &[Point]) -> f32 {
    let n = points.len() as f32;
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    for p in points {
        sx += p.x;
        sy += p.y;
    }
    let (mx, my) = (sx / n, sy / n);

    let (mut suu, mut suv, mut svv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in points {
        let (u, v) = (p.x - mx, p.y - my);
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }

    let determinant = suu * svv - suv * suv;
    if determinant.abs() < f32::EPSILON {
        return f32::MAX;
    }
    let b1 = (suuu + suvv) / 2.0;
    let b2 = (svvv + svuu) / 2.0;
    let uc = (b1 * svv - b2 * suv) / determinant;
    let vc = (b2 * suu - b1 * suv) / determinant;

    let centre = Point {
        x: uc + mx,
        y: vc + my,
    };
    let radius = points.iter().map(|&p| distance(p, centre)).sum::<f32>() / n;
    points
        .iter()
        .map(|&p| (distance(p, centre) - radius).abs())
        .sum::<f32>()
        / n
}

/// Mean distance from the minimum-area enclosing rectangle, and how far that
/// rectangle is rotated off the axes.
///
/// Rotating calipers over the convex hull: the minimum-area rectangle always has
/// a side flush with a hull edge, so trying each edge as the rectangle's
/// orientation is exhaustive rather than a search.
fn rect_fit_residual(points: &[Point]) -> (f32, f32) {
    let hull = convex_hull(points);
    if hull.len() < 3 {
        return (f32::MAX, 0.0);
    }

    let mut best = (f32::MAX, 0.0f32, 0.0f32);
    for i in 0..hull.len() {
        let a = hull[i];
        let b = hull[(i + 1) % hull.len()];
        let edge = distance(a, b);
        if edge <= 0.0 {
            continue;
        }
        let (cos, sin) = ((b.x - a.x) / edge, (b.y - a.y) / edge);

        let (mut min_u, mut max_u, mut min_v, mut max_v) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in &hull {
            let u = p.x * cos + p.y * sin;
            let v = -p.x * sin + p.y * cos;
            min_u = min_u.min(u);
            max_u = max_u.max(u);
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
        let area = (max_u - min_u) * (max_v - min_v);
        if area < best.0 {
            best = (area, cos, sin);
        }
    }

    let (_, cos, sin) = best;
    let (mut min_u, mut max_u, mut min_v, mut max_v) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in points {
        let u = p.x * cos + p.y * sin;
        let v = -p.x * sin + p.y * cos;
        min_u = min_u.min(u);
        max_u = max_u.max(u);
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }

    let residual = points
        .iter()
        .map(|p| {
            let u = p.x * cos + p.y * sin;
            let v = -p.x * sin + p.y * cos;
            // Distance to the nearest side, which for a point inside the
            // rectangle is what "off the outline" means.
            (u - min_u)
                .min(max_u - u)
                .min(v - min_v)
                .min(max_v - v)
                .abs()
        })
        .sum::<f32>()
        / points.len() as f32;

    // How far the best orientation is from an axis, folded into 0..45°.
    let angle = sin.atan2(cos).rem_euclid(std::f32::consts::FRAC_PI_2);
    let tilt = angle.min(std::f32::consts::FRAC_PI_2 - angle);

    (residual, tilt)
}

/// Andrew's monotone chain.
fn convex_hull(points: &[Point]) -> Vec<Point> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| {
        a.x.partial_cmp(&b.x)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
    });
    sorted.dedup_by(|a, b| a.x == b.x && a.y == b.y);
    if sorted.len() < 3 {
        return sorted;
    }

    let cross = |o: Point, a: Point, b: Point| {
        (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
    };

    let mut hull: Vec<Point> = Vec::with_capacity(sorted.len() * 2);
    for &p in sorted.iter().chain(sorted.iter().rev()) {
        // The lower and upper chains are built by the same rule; restarting the
        // "keep turning left" test at the halfway point is what separates them.
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull
}

/// Douglas–Peucker. The number of points left is the corner count.
fn simplify(points: &[Point], epsilon: f32) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }

    let (first, last) = (points[0], points[points.len() - 1]);
    let mut furthest = 0usize;
    let mut furthest_distance = 0.0f32;
    for (i, &p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let d = perpendicular_distance(p, first, last);
        if d > furthest_distance {
            furthest = i;
            furthest_distance = d;
        }
    }

    if furthest_distance <= epsilon {
        return vec![first, last];
    }

    let mut left = simplify(&points[..=furthest], epsilon);
    let right = simplify(&points[furthest..], epsilon);
    left.pop();
    left.extend(right);
    left
}

fn perpendicular_distance(p: Point, a: Point, b: Point) -> f32 {
    let length = distance(a, b);
    if length <= 0.0 {
        return distance(p, a);
    }
    ((b.x - a.x) * (a.y - p.y) - (a.x - p.x) * (b.y - a.y)).abs() / length
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic pseudo-random source.
    ///
    /// Seeded rather than random: a recogniser test that fails one run in fifty is
    /// worse than no test, because it trains everyone to re-run it.
    struct Wobble(u32);

    impl Wobble {
        fn next(&mut self) -> f32 {
            // xorshift, then mapped to -1..1.
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 % 2000) as f32 / 1000.0 - 1.0
        }
    }

    fn circle(cx: f32, cy: f32, r: f32, jitter: f32, seed: u32) -> Vec<Point> {
        let mut wobble = Wobble(seed);
        (0..48)
            .map(|i| {
                let t = i as f32 / 48.0 * std::f32::consts::TAU;
                Point {
                    x: cx + r * t.cos() + wobble.next() * jitter,
                    y: cy + r * t.sin() + wobble.next() * jitter,
                }
            })
            .collect()
    }

    fn rectangle(rect: Rect, jitter: f32, seed: u32) -> Vec<Point> {
        let mut wobble = Wobble(seed);
        let corners = [
            (rect.left, rect.top),
            (rect.right, rect.top),
            (rect.right, rect.bottom),
            (rect.left, rect.bottom),
            (rect.left, rect.top),
        ];
        let mut points = Vec::new();
        for pair in corners.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            for step in 0..12 {
                let t = step as f32 / 12.0;
                points.push(Point {
                    x: from.0 + t * (to.0 - from.0) + wobble.next() * jitter,
                    y: from.1 + t * (to.1 - from.1) + wobble.next() * jitter,
                });
            }
        }
        points.push(Point {
            x: rect.left,
            y: rect.top,
        });
        points
    }

    fn square(side: f32) -> Rect {
        Rect {
            left: 0.0,
            top: 0.0,
            right: side,
            bottom: side,
        }
    }

    #[test]
    fn a_clean_circle_is_an_ellipse() {
        assert!(matches!(
            recognise(&circle(100.0, 100.0, 40.0, 0.0, 1)),
            Shape::Ellipse { .. }
        ));
    }

    #[test]
    fn a_wobbly_circle_is_still_an_ellipse() {
        // Two points of wobble on a 40 pt radius: a steady hand, not a machine.
        assert!(
            matches!(
                recognise(&circle(100.0, 100.0, 40.0, 2.0, 7)),
                Shape::Ellipse { .. }
            ),
            "a hand-drawn circle should still snap",
        );
    }

    #[test]
    fn a_clean_rectangle_is_a_rectangle() {
        assert!(matches!(
            recognise(&rectangle(square(80.0), 0.0, 1)),
            Shape::Rect { .. }
        ));
    }

    #[test]
    fn a_wobbly_rectangle_is_still_a_rectangle() {
        assert!(
            matches!(
                recognise(&rectangle(square(120.0), 2.0, 11)),
                Shape::Rect { .. }
            ),
            "a hand-drawn box should still snap",
        );
    }

    #[test]
    fn a_straight_drag_is_a_line() {
        let points: Vec<Point> = (0..20)
            .map(|i| Point {
                x: i as f32 * 5.0,
                y: 40.0 + i as f32 * 0.1,
            })
            .collect();
        assert!(matches!(recognise(&points), Shape::Line { .. }));
    }

    #[test]
    fn a_line_keeps_the_ends_that_were_drawn_not_the_resampled_ones() {
        let points = vec![
            Point { x: 10.0, y: 10.0 },
            Point { x: 50.0, y: 10.5 },
            Point { x: 90.0, y: 11.0 },
        ];
        match recognise(&points) {
            Shape::Line { from, to } => {
                assert_eq!(from, points[0]);
                assert_eq!(to, points[2]);
            }
            other => panic!("expected a line, got {other:?}"),
        }
    }

    #[test]
    fn a_deliberate_squiggle_stays_freehand() {
        // The criterion that matters most. Over-eager recognition is the failure
        // mode: a squiggle that stays a squiggle costs nothing, and a squiggle
        // turned into a circle costs the user their drawing.
        let mut wobble = Wobble(3);
        let points: Vec<Point> = (0..60)
            .map(|i| {
                let t = i as f32;
                Point {
                    x: 20.0 + t * 1.5 + wobble.next() * 6.0,
                    y: 50.0 + (t / 4.0).sin() * 25.0 + wobble.next() * 6.0,
                }
            })
            .collect();
        assert!(
            matches!(recognise(&points), Shape::Freehand { .. }),
            "a squiggle was snapped to something it is not",
        );
    }

    #[test]
    fn a_random_walk_is_never_a_shape() {
        for seed in [1u32, 9, 17, 42, 99, 256, 1013, 7777] {
            let mut wobble = Wobble(seed);
            let mut at = Point { x: 100.0, y: 100.0 };
            let points: Vec<Point> = (0..64)
                .map(|_| {
                    at = Point {
                        x: at.x + wobble.next() * 12.0,
                        y: at.y + wobble.next() * 12.0,
                    };
                    at
                })
                .collect();
            assert!(
                matches!(recognise(&points), Shape::Freehand { .. }),
                "seed {seed} produced a shape from a random walk",
            );
        }
    }

    #[test]
    fn recognising_a_circle_survives_rotation() {
        // A circle is rotationally symmetric, so where the stroke *starts* must
        // not change the answer. It is exactly the sort of thing a corner-count
        // heuristic gets wrong.
        let base = circle(0.0, 0.0, 50.0, 1.5, 5);
        for turn in 0..8 {
            let angle = turn as f32 * std::f32::consts::TAU / 8.0;
            let rotated: Vec<Point> = base
                .iter()
                .map(|p| Point {
                    x: p.x * angle.cos() - p.y * angle.sin() + 200.0,
                    y: p.x * angle.sin() + p.y * angle.cos() + 200.0,
                })
                .collect();
            assert!(
                matches!(recognise(&rotated), Shape::Ellipse { .. }),
                "a circle rotated by {turn}/8 of a turn stopped being one",
            );
        }
    }

    #[test]
    fn recognising_a_circle_survives_scale() {
        for radius in [8.0f32, 20.0, 50.0, 200.0, 800.0] {
            assert!(
                matches!(
                    recognise(&circle(0.0, 0.0, radius, radius * 0.04, 13)),
                    Shape::Ellipse { .. }
                ),
                "a circle of radius {radius} was not recognised",
            );
        }
    }

    #[test]
    fn a_rectangle_drawn_at_an_angle_stays_freehand() {
        // The shape vocabulary has no rotated rectangle. Snapping one would
        // silently straighten a deliberately angled box — a worse outcome than
        // leaving it as drawn.
        let upright = rectangle(square(100.0), 1.0, 4);
        let angle = 0.6f32;
        let tilted: Vec<Point> = upright
            .iter()
            .map(|p| Point {
                x: p.x * angle.cos() - p.y * angle.sin(),
                y: p.x * angle.sin() + p.y * angle.cos(),
            })
            .collect();
        assert!(matches!(recognise(&tilted), Shape::Freehand { .. }));
    }

    #[test]
    fn a_two_point_stroke_is_freehand_rather_than_a_panic() {
        assert!(matches!(
            recognise(&[Point { x: 0.0, y: 0.0 }, Point { x: 1.0, y: 1.0 }]),
            Shape::Freehand { .. }
        ));
        assert!(matches!(recognise(&[]), Shape::Freehand { .. }));
    }

    #[test]
    fn a_stroke_that_never_moved_is_freehand() {
        let stuck = vec![Point { x: 5.0, y: 5.0 }; 40];
        assert!(matches!(recognise(&stuck), Shape::Freehand { .. }));
    }

    #[test]
    fn resampling_keeps_the_ends_and_evens_the_spacing() {
        let uneven = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: 0.0 },
            Point { x: 2.0, y: 0.0 },
            Point { x: 100.0, y: 0.0 },
        ];
        let even = resample(&uneven, 5);
        assert_eq!(even.len(), 5);
        assert_eq!(even[0], uneven[0]);
        assert_eq!(*even.last().unwrap(), *uneven.last().unwrap());
        for pair in even.windows(2) {
            assert!((distance(pair[0], pair[1]) - 25.0).abs() < 0.01);
        }
    }

    #[test]
    fn recognition_is_fast_enough_to_run_on_the_gesture_thread() {
        let stroke = circle(100.0, 100.0, 60.0, 2.0, 21);
        let started = std::time::Instant::now();
        for _ in 0..100 {
            let _ = recognise(&stroke);
        }
        let each = started.elapsed() / 100;
        assert!(
            each < std::time::Duration::from_millis(5),
            "recognition took {each:?} per stroke",
        );
    }
}

//! Filling a region with a hatch pattern.
//!
//! **A hatch is not a shape, it is an instruction.** The file gives a boundary
//! and a recipe — a family of parallel lines at an angle, spaced by an offset,
//! broken into dashes — and every viewer has to run the recipe itself. There is
//! nothing to draw until it has been run.
//!
//! Which is why this is here rather than in the reader: it is arithmetic with
//! several ways to be quietly wrong, and all of them look like *some* kind of
//! hatching. Lines at the wrong angle still read as hatching. Lines spaced by
//! the offset's length rather than its perpendicular component still read as
//! hatching, at the wrong density. Dashes measured from the region rather than
//! from the pattern's own origin still read as hatching, and stop two adjacent
//! regions of the same pattern from lining up — which on a section drawing is
//! how one material silently becomes two.

use super::model::{Entity, Point, Shape, Vertex};

/// A region to fill, and what to fill it with.
///
/// The two formats state this differently and mean the same thing, so both
/// readers build one of these and neither knows how a hatch is drawn.
#[derive(Debug, Clone, Default)]
pub struct Hatch {
    /// Closed boundaries in drawing units. Islands included: a point inside an
    /// odd number of them is inside the region.
    pub loops: Vec<Vec<Point>>,
    /// Solid throughout, rather than lined.
    pub solid: bool,
    /// The recipe, where there is one.
    pub lines: Vec<PatternLine>,
}

/// An arc or elliptical arc as points, from `from` to `to`.
///
/// **The direction flag decides which way round**, not which angle is larger.
/// A boundary arc that runs the wrong way round leaves a loop that crosses
/// itself, and a self-crossing loop under the even-odd rule fills the parts
/// that should be empty and empties the parts that should be filled.
#[allow(clippy::too_many_arguments)]
pub fn arc_points(
    out: &mut Vec<Point>,
    centre: Point,
    radius: f64,
    ratio: f64,
    tilt: f64,
    from: f64,
    to: f64,
    anticlockwise: bool,
) {
    let mut sweep = to - from;
    if anticlockwise {
        while sweep <= 1e-9 {
            sweep += std::f64::consts::TAU;
        }
    } else {
        while sweep >= -1e-9 {
            sweep -= std::f64::consts::TAU;
        }
    }
    let steps = ((sweep.abs() / 0.15).ceil() as usize).clamp(2, 128);
    let (tilt_sin, tilt_cos) = tilt.sin_cos();
    for step in 0..=steps {
        let angle = from + sweep * step as f64 / steps as f64;
        let (x, y) = (radius * angle.cos(), radius * ratio * angle.sin());
        out.push(Point::new(
            centre.x + x * tilt_cos - y * tilt_sin,
            centre.y + x * tilt_sin + y * tilt_cos,
        ));
    }
}

/// What a hatch puts on the sheet: one shape, whatever it fills.
///
/// **The instruction, not the strokes** — see [`Shape::Hatch`] for why. A solid
/// fill has no recipe to keep, so it goes straight in as the region it is.
pub fn shape_of(hatch: &Hatch, layer: u16) -> Option<Entity> {
    if hatch.loops.is_empty() {
        return None;
    }
    let shape = if hatch.solid || hatch.lines.is_empty() {
        // A pattern with no definition lines is a solid fill: that is what
        // SOLID hatches, and gradient ones, come through as.
        Shape::Fill { loops: hatch.loops.clone() }
    } else {
        Shape::Hatch { loops: hatch.loops.clone(), lines: hatch.lines.clone() }
    };
    Some(Entity { layer, shape })
}

/// A boundary as points, with any bowed segments opened out.
///
/// **A bulge is the tangent of a quarter of the included angle**, the same
/// reading [`super::raster`] uses for a polyline — a round column hatched
/// through gives a boundary of four bulges and nothing else, and taking the
/// bulge for a sag or a radius fills a circle-ish region that is not the one
/// the drawing has.
pub fn flattened(vertices: &[Vertex], closed: bool) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(vertices.len());
    if vertices.is_empty() {
        return out;
    }
    out.push(vertices[0].at);

    let last = if closed { vertices.len() } else { vertices.len() - 1 };
    for index in 0..last {
        let from = vertices[index];
        let to = vertices[(index + 1) % vertices.len()];
        if from.bulge.abs() < 1e-12 {
            out.push(to.at);
            continue;
        }

        let included = 4.0 * from.bulge.atan();
        let (dx, dy) = (to.at.x - from.at.x, to.at.y - from.at.y);
        let chord = dx.hypot(dy);
        let half = (included / 2.0).sin();
        if chord < 1e-12 || half.abs() < 1e-12 {
            out.push(to.at);
            continue;
        }
        let radius = chord / (2.0 * half);
        let away = radius * (included / 2.0).cos();
        let centre = Point::new(
            (from.at.x + to.at.x) / 2.0 - away * dy / chord,
            (from.at.y + to.at.y) / 2.0 + away * dx / chord,
        );
        let start = (from.at.y - centre.y).atan2(from.at.x - centre.x);

        // A fixed count rather than one judged by how big the arc is on screen:
        // a boundary is flattened once, when the file is read, and there is no
        // view yet to judge against.
        let steps = ((included.abs() / 0.15).ceil() as usize).clamp(2, 64);
        for step in 1..=steps {
            let angle = start + included * step as f64 / steps as f64;
            out.push(Point::new(
                centre.x + radius.abs() * angle.cos(),
                centre.y + radius.abs() * angle.sin(),
            ));
        }
    }

    // **A loop is closed by being a loop, not by repeating its first point.**
    // Everything that consumes one wraps round with a modulus, so a repeated
    // point is a zero-length edge: harmless, and one more thing for a reader to
    // wonder about.
    if closed && out.len() > 1 {
        let (first, last) = (out[0], out[out.len() - 1]);
        if (first.x - last.x).abs() < 1e-12 && (first.y - last.y).abs() < 1e-12 {
            out.pop();
        }
    }
    out
}

/// One line of a pattern definition, as the file states it.
#[derive(Debug, Clone)]
pub struct PatternLine {
    /// Anticlockwise from +x, in radians.
    pub angle: f64,
    /// A point the first line of the family passes through.
    pub base: Point,
    /// How far each line is from the one before it: `x` along the line, `y`
    /// across it. **The across part is what sets the spacing** — a pattern with
    /// a large along-offset and a small across-offset is a dense hatch that is
    /// staggered, not a sparse one.
    pub offset: Point,
    /// Dash, gap, dash, gap — positive draws, negative skips, zero is a dot.
    /// Empty means an unbroken line.
    pub dashes: Vec<f64>,
}

/// The most segments one hatch may produce.
///
/// A pattern is defined in drawing units, so a region a hundred metres across
/// filled with a pattern spaced at a millimetre asks for a hundred thousand
/// lines per family. Files like that exist — usually because somebody hatched
/// at the wrong scale — and without a bound one of them takes the app with it.
pub const MOST_SEGMENTS: usize = 20_000;

/// Run the recipe: the strokes that fill `loops` with `lines`.
///
/// `loops` are closed polygons in drawing units, already flattened. Islands are
/// handled by the even-odd rule, which is what a hatch means by a hole: a point
/// inside an odd number of loops is inside the region.
pub fn strokes(loops: &[Vec<Point>], lines: &[PatternLine], most: usize) -> Vec<(Point, Point)> {
    let Some((low, high)) = extent(loops) else { return Vec::new() };
    let mut out = Vec::new();

    for line in lines {
        if out.len() >= most {
            break;
        }
        one_family(&mut out, loops, line, low, high, most);
    }

    out
}

fn extent(loops: &[Vec<Point>]) -> Option<(Point, Point)> {
    let mut low = Point::new(f64::MAX, f64::MAX);
    let mut high = Point::new(f64::MIN, f64::MIN);
    let mut any = false;
    for ring in loops {
        for at in ring {
            low.x = low.x.min(at.x);
            low.y = low.y.min(at.y);
            high.x = high.x.max(at.x);
            high.y = high.y.max(at.y);
            any = true;
        }
    }
    any.then_some((low, high))
}

fn one_family(
    out: &mut Vec<(Point, Point)>,
    loops: &[Vec<Point>],
    line: &PatternLine,
    low: Point,
    high: Point,
    most: usize,
) {
    let (sin, cos) = line.angle.sin_cos();
    // Along the lines, and across them.
    let along = Point::new(cos, sin);
    let across = Point::new(-sin, cos);

    // **The spacing is the offset's component across the lines, not its
    // length.** Taking the length instead draws the same pattern too sparse by
    // however much the offset is staggered along — which on a 45° hatch is a
    // factor of about one and a half, close enough to look deliberate.
    let step = line.offset.y;
    if step.abs() < 1e-9 {
        // Every line of the family would be the same line.
        return;
    }

    // How many steps from the base line reach each corner of the region.
    let mut lowest = f64::MAX;
    let mut highest = f64::MIN;
    for corner in [
        Point::new(low.x, low.y),
        Point::new(high.x, low.y),
        Point::new(low.x, high.y),
        Point::new(high.x, high.y),
    ] {
        let away = (corner.x - line.base.x) * across.x + (corner.y - line.base.y) * across.y;
        let steps = away / step;
        lowest = lowest.min(steps);
        highest = highest.max(steps);
    }
    if !lowest.is_finite() || !highest.is_finite() {
        return;
    }

    // The diagonal, which is as far as any line inside the region can run.
    let reach = (high.x - low.x).hypot(high.y - low.y) + 1.0;
    let first = lowest.floor() as i64;
    let last = highest.ceil() as i64;

    // A pattern so fine that it would need more lines than the whole hatch is
    // allowed is not drawn finely and then truncated half way down the region:
    // that leaves a hatch that fades out, which reads as a drawing fault. It is
    // left to the caller's count instead, which reports it.
    if last.saturating_sub(first) as usize > most {
        return;
    }

    for step_number in first..=last {
        if out.len() >= most {
            return;
        }
        let k = step_number as f64;
        // The base moves along as well as across: that is what staggers a
        // brick pattern instead of stacking every course on the one below.
        let origin = Point::new(
            line.base.x + k * (line.offset.x * along.x + step * across.x),
            line.base.y + k * (line.offset.x * along.y + step * across.y),
        );

        // The whole line, long enough to cross the region from any angle.
        let from = Point::new(origin.x - along.x * reach, origin.y - along.y * reach);
        let to = Point::new(origin.x + along.x * reach, origin.y + along.y * reach);

        // **Both measured from the origin, not from where the sweep started.**
        // `inside` answers in distances along the ray it was given, which
        // begins a whole diagonal behind the origin; `dashed` steps out from
        // the origin, because that is what the dash pattern is in phase with.
        // Handing one's numbers to the other without this shift put every
        // stroke a diagonal's length away from the region it belongs to — a
        // hatch of the right pattern and the right density, drawn beside the
        // thing it was filling.
        for (start, end) in inside(loops, from, to) {
            dashed(out, &line.dashes, origin, along, start - reach, end - reach, most);
        }
    }
}

/// The parts of a line that are inside the region, as distances along it.
///
/// Even-odd: crossings sorted along the line, and the region is between the
/// first and second, the third and fourth, and so on. That is what makes a
/// hatch with a hole in it come out with a hole in it.
fn inside(loops: &[Vec<Point>], from: Point, to: Point) -> Vec<(f64, f64)> {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = dx.hypot(dy);
    if length < 1e-12 {
        return Vec::new();
    }

    let mut crossings: Vec<f64> = Vec::new();
    for ring in loops {
        if ring.len() < 2 {
            continue;
        }
        for at in 0..ring.len() {
            let a = ring[at];
            let b = ring[(at + 1) % ring.len()];
            let (ex, ey) = (b.x - a.x, b.y - a.y);

            let denominator = dx * ey - dy * ex;
            if denominator.abs() < 1e-12 {
                continue;
            }
            let along = ((a.x - from.x) * ey - (a.y - from.y) * ex) / denominator;
            let across = ((a.x - from.x) * dy - (a.y - from.y) * dx) / denominator;
            // **Half-open on the edge.** Counting both ends of every edge means
            // a line passing exactly through a corner crosses twice, and the
            // region flips inside-out from there to the far side of the hatch.
            if (0.0..1.0).contains(&across) {
                crossings.push(along * length);
            }
        }
    }

    crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    crossings
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .filter(|(a, b)| b - a > 1e-9)
        .collect()
}

/// Lay the dash pattern along one span.
///
/// **Measured from the pattern's own origin, not from the span.** Starting each
/// span's dashes at its own beginning lines the pattern up with the boundary
/// rather than with itself, so two neighbouring regions of the same hatch do
/// not match along the edge they share — which reads as two different materials.
fn dashed(
    out: &mut Vec<(Point, Point)>,
    dashes: &[f64],
    origin: Point,
    along: Point,
    start: f64,
    end: f64,
    most: usize,
) {
    let at = |distance: f64| {
        Point::new(origin.x + along.x * distance, origin.y + along.y * distance)
    };

    let period: f64 = dashes.iter().map(|d| d.abs().max(DOT)).sum();
    if dashes.is_empty() || period < 1e-9 {
        out.push((at(start), at(end)));
        return;
    }

    // Back up to the start of the repeat that contains `start`, so the pattern
    // is in phase with the origin however far along the span begins.
    let cycles = (start / period).floor();
    let mut distance = cycles * period;

    while distance < end {
        for dash in dashes {
            if out.len() >= most {
                return;
            }
            let length = dash.abs().max(DOT);
            let (from, to) = (distance, distance + length);
            distance = to;

            if *dash < 0.0 {
                continue; // a gap
            }
            let (from, to) = (from.max(start), to.min(end));
            if to > from {
                out.push((at(from), at(to)));
            }
            if distance >= end {
                return;
            }
        }
    }
}

/// A dot has no length, so it is drawn as the shortest thing that shows.
const DOT: f64 = 1e-3;

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: f64) -> Vec<Vec<Point>> {
        vec![vec![
            Point::new(0.0, 0.0),
            Point::new(size, 0.0),
            Point::new(size, size),
            Point::new(0.0, size),
        ]]
    }

    /// Horizontal lines every two units across a ten-unit square.
    fn every_two() -> PatternLine {
        PatternLine {
            angle: 0.0,
            base: Point::new(0.0, 0.0),
            offset: Point::new(0.0, 2.0),
            dashes: Vec::new(),
        }
    }

    /// The lines land at the stated spacing, and run the full width.
    ///
    /// **They also start and end exactly on the boundary**, which is the half
    /// of this worth guarding: the crossings are worked out along a ray that
    /// begins a whole diagonal behind the pattern's origin, and the dashes are
    /// laid from the origin itself. Measuring one in the other's units put
    /// every stroke a diagonal away — the right pattern at the right density,
    /// drawn beside the region instead of inside it — and nothing about the
    /// count alone would have caught it.
    #[test]
    fn a_square_fills_with_lines_at_the_stated_spacing() {
        let made = strokes(&square(10.0), &[every_two()], MOST_SEGMENTS);

        // y = 2, 4, 6, 8. The lines at y = 0 and y = 10 lie exactly along the
        // edges — see `a_line_along_an_edge_is_not_drawn`.
        assert_eq!(4, made.len(), "{made:?}");
        for (from, to) in &made {
            assert!((from.y - to.y).abs() < 1e-9, "not horizontal: {from:?} {to:?}");
            assert!((from.x - 0.0).abs() < 1e-6 && (to.x - 10.0).abs() < 1e-6, "{from:?} {to:?}");
        }
    }

    /// A pattern line lying exactly along a boundary edge is left out.
    ///
    /// **A decision, not an oversight.** Such a line grazes the region rather
    /// than crossing it: it meets the boundary an odd number of times, so
    /// there is no pair of crossings saying where it goes in and comes out.
    /// Drawing it anyway would mean guessing, and the guess is a stroke laid
    /// exactly on top of an outline that the boundary geometry has already
    /// drawn — invisible when it is right and a stray line when it is wrong.
    #[test]
    fn a_line_along_an_edge_is_not_drawn() {
        let made = strokes(&square(10.0), &[every_two()], MOST_SEGMENTS);

        for (from, _) in &made {
            assert!(from.y > 1e-9 && from.y < 10.0 - 1e-9, "a line grazed the edge at {from:?}");
        }
    }

    /// **The spacing is the offset across the lines, not its length.**
    ///
    /// A staggered pattern — a large step along the lines and a small one
    /// across — is dense. Using the offset's length instead spreads it by
    /// however much it is staggered, which still looks like hatching and is
    /// simply the wrong pattern.
    #[test]
    fn a_staggered_offset_does_not_thin_the_pattern() {
        let staggered = PatternLine {
            angle: 0.0,
            base: Point::new(0.0, 0.0),
            offset: Point::new(8.0, 2.0),
            dashes: Vec::new(),
        };

        let plain = strokes(&square(10.0), &[every_two()], MOST_SEGMENTS);
        let made = strokes(&square(10.0), &[staggered], MOST_SEGMENTS);

        assert_eq!(plain.len(), made.len(), "the stagger changed the density");
    }

    /// A hole in the region is a hole in the hatching.
    #[test]
    fn an_island_is_left_unhatched() {
        let mut loops = square(10.0);
        loops.push(vec![
            Point::new(4.0, 4.0),
            Point::new(6.0, 4.0),
            Point::new(6.0, 6.0),
            Point::new(4.0, 6.0),
        ]);

        let made = strokes(&loops, &[every_two()], MOST_SEGMENTS);

        // The line at y = 4 runs along the island's edge; the one at y = 6 too.
        // What matters is that some line comes back as two pieces rather than
        // one, which is the hole.
        let split = made.iter().filter(|(from, _)| (from.y - 4.0).abs() < 1e-9).count();
        assert!(split >= 1, "{made:?}");

        // And nothing is drawn straight through the middle of the island.
        let through = made.iter().any(|(from, to)| {
            (from.y - 5.0).abs() < 1e-9 && from.x < 4.5 && to.x > 5.5
        });
        assert!(!through, "a line ran through the island: {made:?}");
    }

    /// Dashes break the lines up, and gaps are not drawn.
    #[test]
    fn a_dashed_pattern_comes_out_in_pieces() {
        // A spacing that keeps every line clear of the edges, so this measures
        // the dashes rather than the grazing rule.
        let dashed = PatternLine {
            angle: 0.0,
            base: Point::new(0.0, 0.0),
            offset: Point::new(0.0, 2.0),
            dashes: vec![1.0, -1.0],
        };

        let made = strokes(&square(10.0), &[dashed], MOST_SEGMENTS);

        // Four lines, each broken into five one-unit dashes across ten units.
        assert_eq!(20, made.len(), "not broken up as expected: {made:?}");
        for (from, to) in &made {
            let length = (to.x - from.x).hypot(to.y - from.y);
            assert!(length <= 1.0 + 1e-6, "a dash ran {length} long");
        }
    }

    /// **Dashes are in phase with the pattern, not with the region.**
    ///
    /// Two regions of the same hatch side by side have to line up along the
    /// edge they share. Starting each span's dashes at its own beginning makes
    /// them line up with the boundary instead, and the join reads as a change
    /// of material.
    #[test]
    fn dashes_line_up_between_two_regions() {
        let pattern = PatternLine {
            angle: 0.0,
            base: Point::new(0.0, 0.0),
            offset: Point::new(0.0, 5.0),
            dashes: vec![1.0, -1.0],
        };

        let left = strokes(&square(10.0), &[pattern.clone()], MOST_SEGMENTS);
        // The same pattern over a region that starts at x = 10.
        let right_loops = vec![vec![
            Point::new(10.0, 0.0),
            Point::new(20.0, 0.0),
            Point::new(20.0, 10.0),
            Point::new(10.0, 10.0),
        ]];
        let right = strokes(&right_loops, &[pattern], MOST_SEGMENTS);

        // Every dash in both regions begins at an even distance along.
        for (from, _) in left.iter().chain(right.iter()) {
            let into = from.x.rem_euclid(2.0);
            assert!(into < 1e-6 || (into - 2.0).abs() < 1e-6, "out of phase at {from:?}");
        }
    }

    /// A pattern too fine for the region is refused, not truncated.
    ///
    /// Drawing as many lines as the budget allows and stopping leaves a hatch
    /// that fades out half way down, which reads as a fault in the drawing
    /// rather than as a limit.
    #[test]
    fn an_impossibly_fine_pattern_draws_nothing() {
        let far_too_fine = PatternLine {
            angle: 0.0,
            base: Point::new(0.0, 0.0),
            offset: Point::new(0.0, 0.0001),
            dashes: Vec::new(),
        };

        let made = strokes(&square(1000.0), &[far_too_fine], 1_000);

        assert!(made.is_empty(), "{} lines came back", made.len());
    }

    /// A pattern whose lines never step across itself would repeat for ever.
    #[test]
    fn a_pattern_with_no_spacing_draws_nothing() {
        let nowhere = PatternLine {
            angle: 0.0,
            base: Point::new(0.0, 0.0),
            offset: Point::new(1.0, 0.0),
            dashes: Vec::new(),
        };

        assert!(strokes(&square(10.0), &[nowhere], MOST_SEGMENTS).is_empty());
    }

    /// Angled hatching runs at the angle it was given.
    #[test]
    fn a_pattern_runs_at_its_own_angle() {
        let at_45 = PatternLine {
            angle: std::f64::consts::FRAC_PI_4,
            base: Point::new(0.0, 0.0),
            offset: Point::new(0.0, 2.0),
            dashes: Vec::new(),
        };

        let made = strokes(&square(10.0), &[at_45], MOST_SEGMENTS);

        assert!(!made.is_empty());
        for (from, to) in &made {
            let (dx, dy) = (to.x - from.x, to.y - from.y);
            assert!((dx - dy).abs() < 1e-6, "not at 45 degrees: {from:?} {to:?}");
        }
    }
}

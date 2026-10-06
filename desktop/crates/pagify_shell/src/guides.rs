//! Reference lines for moving something: the lines through the edges and the
//! middles of the other things on the page, shown while a thing is dragged near
//! them, and the pull that makes it land exactly on one.
//!
//! **Asked for with a screenshot of what a design program does.** Thin grey
//! lines run through the edges and centres of the things near the one being
//! moved; where one of its own edges or its middle lines up with one of them
//! the line turns green, and the thing sits exactly there. Only while it is
//! being moved — they are a measuring aid, not part of the page.
//!
//! Window-free, so every rule here is tested without a window: given the
//! rectangle being moved and the rectangles of everything else, [`align`]
//! answers how far to nudge it so it sits on a line, and which lines to draw.
//! All of it in page points, on the page's own top-left origin.

use pdf_core::document::Rect;

/// One line, across the page. `at` is where, in page points: an `x` for a
/// vertical line, a `y` for a horizontal one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub at: f32,
    /// Whether a side or the middle of the thing being moved is exactly on it.
    pub exact: bool,
}

/// What to draw.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Guides {
    /// Lines of constant `x`.
    pub vertical: Vec<Line>,
    /// Lines of constant `y`.
    pub horizontal: Vec<Line>,
}

impl Guides {
    pub fn is_empty(&self) -> bool {
        self.vertical.is_empty() && self.horizontal.is_empty()
    }
}

/// How close two positions have to be to count as the same one.
const SAME: f32 = 0.25;

/// The most grey lines drawn on either axis. A page full of small things has a
/// line through nearly every point; the nearest few are all that helps.
const MOST_NEAR: usize = 6;

/// The rectangle being moved, nudged onto the nearest line, and the lines.
///
/// * `moving` — where it is now, after the drag.
/// * `others` — everything else on the page, the page's own edges and middle
///   excepted (they are added here from `page`).
/// * `page` — its width and height.
/// * `snap` — how close, at most, a side or the middle of `moving` has to be to
///   a line to be pulled onto it, in page points.
/// * `near` — how close to show a grey line, at most. At least `snap`.
///
/// Returns the nudge `(dx, dy)` to add to the drag, and the lines. The lines
/// are those of `moving` **after** the nudge: with the nudge applied, the exact
/// ones are the lines it now sits on.
pub fn align(moving: Rect, others: &[Rect], page: (f32, f32), snap: f32, near: f32) -> ((f32, f32), Guides) {
    let near = near.max(snap);
    let xs = |r: &Rect| [r.left.min(r.right), (r.left + r.right) / 2.0, r.left.max(r.right)];
    let ys = |r: &Rect| [r.top.min(r.bottom), (r.top + r.bottom) / 2.0, r.top.max(r.bottom)];

    let mut x_targets: Vec<f32> = vec![0.0, page.0 / 2.0, page.0];
    let mut y_targets: Vec<f32> = vec![0.0, page.1 / 2.0, page.1];
    for other in others {
        x_targets.extend(xs(other));
        y_targets.extend(ys(other));
    }

    let (dx, vertical) = along(xs(&moving), &x_targets, snap, near);
    let (dy, horizontal) = along(ys(&moving), &y_targets, snap, near);
    ((dx, dy), Guides { vertical, horizontal })
}

/// One axis: the three positions of the thing against every target.
fn along(mine: [f32; 3], targets: &[f32], snap: f32, near: f32) -> (f32, Vec<Line>) {
    // The pull: the smallest gap within `snap`.
    let mut pull: Option<f32> = None;
    for &m in &mine {
        for &t in targets {
            let gap = t - m;
            if gap.abs() <= snap && pull.map_or(true, |p| gap.abs() < p.abs()) {
                pull = Some(gap);
            }
        }
    }
    let nudge = pull.unwrap_or(0.0);

    // The lines, for where it will be.
    let placed = [mine[0] + nudge, mine[1] + nudge, mine[2] + nudge];
    let mut lines: Vec<Line> = Vec::new();
    for &t in targets {
        let closest = placed.iter().map(|m| (t - m).abs()).fold(f32::MAX, f32::min);
        if closest > near {
            continue;
        }
        let exact = closest < SAME;
        match lines.iter_mut().find(|l| (l.at - t).abs() < SAME) {
            Some(existing) => existing.exact |= exact,
            None => lines.push(Line { at: t, exact }),
        }
    }
    // Every exact line; of the grey ones only the nearest few.
    let mut grey: Vec<Line> = lines.iter().copied().filter(|l| !l.exact).collect();
    grey.sort_by(|a, b| {
        let d = |l: &Line| placed.iter().map(|m| (l.at - m).abs()).fold(f32::MAX, f32::min);
        d(a).total_cmp(&d(b))
    });
    grey.truncate(MOST_NEAR);
    let mut out: Vec<Line> = lines.into_iter().filter(|l| l.exact).collect();
    out.extend(grey);
    out.sort_by(|a, b| a.at.total_cmp(&b.at));
    (nudge, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect { left, top, right, bottom }
    }

    const PAGE: (f32, f32) = (600.0, 800.0);

    #[test]
    fn nothing_near_nothing_to_show_and_no_pull() {
        let moving = rect(100.0, 100.0, 140.0, 120.0);
        let ((dx, dy), guides) = align(moving, &[rect(400.0, 500.0, 450.0, 520.0)], PAGE, 4.0, 20.0);
        assert_eq!((dx, dy), (0.0, 0.0));
        // Only the page's own edges are anywhere near, if at all: 100 pt from the left one.
        assert!(guides.vertical.iter().all(|l| !l.exact), "{guides:?}");
        assert!(guides.horizontal.iter().all(|l| !l.exact), "{guides:?}");
    }

    #[test]
    fn a_side_close_to_another_things_side_is_pulled_onto_it_and_the_line_is_exact() {
        // Its left edge is 3 pt right of the other thing's left edge at x = 200.
        let moving = rect(203.0, 300.0, 260.0, 330.0);
        let other = rect(200.0, 100.0, 280.0, 150.0);
        let ((dx, dy), guides) = align(moving, &[other], PAGE, 5.0, 20.0);
        assert!((dx + 3.0).abs() < 1e-4, "dx = {dx}");
        assert_eq!(dy, 0.0);
        let on = guides.vertical.iter().find(|l| (l.at - 200.0).abs() < 0.01).expect("no line at x = 200");
        assert!(on.exact);
    }

    #[test]
    fn middles_align_too() {
        // Centre x = 240 against the other's centre 240; centre y of each differ by 2.
        let moving = rect(220.0, 302.0, 260.0, 322.0);
        let other = rect(200.0, 100.0, 280.0, 140.0);
        let ((dx, _), guides) = align(moving, &[other], PAGE, 5.0, 20.0);
        assert!(dx.abs() < 1e-4, "the middles already agree: {dx}");
        assert!(guides.vertical.iter().any(|l| (l.at - 240.0).abs() < 0.01 && l.exact));
    }

    #[test]
    fn it_snaps_to_the_nearest_of_several_lines() {
        // 2 pt from x = 202 and 4 pt from x = 200: the nearer one wins.
        let moving = rect(198.0, 300.0, 240.0, 320.0);
        let others = [rect(200.0, 0.0, 210.0, 10.0), rect(202.0, 20.0, 210.0, 30.0)];
        let ((dx, _), _) = align(moving, &others, PAGE, 5.0, 20.0);
        assert!((dx - 2.0).abs() < 1e-4 || (dx - 4.0).abs() < 1e-4, "{dx}");
        // Left edge 198 -> 200 is 2; to 202 is 4; so 2.
        assert!((dx - 2.0).abs() < 1e-4, "{dx}");
    }

    #[test]
    fn the_pages_own_edge_and_middle_are_lines_too() {
        let moving = rect(2.0, 300.0, 52.0, 330.0);
        let ((dx, _), guides) = align(moving, &[], PAGE, 5.0, 20.0);
        assert!((dx + 2.0).abs() < 1e-4, "pulled onto the page's left edge: {dx}");
        assert!(guides.vertical.iter().any(|l| l.at == 0.0 && l.exact));
        // The middle of the page, x = 300.
        let moving = rect(280.0, 300.0, 320.0, 330.0);
        let ((dx, _), guides) = align(moving, &[], PAGE, 5.0, 20.0);
        assert!(dx.abs() < 1e-4, "{dx}");
        assert!(guides.vertical.iter().any(|l| l.at == 300.0 && l.exact));
    }

    #[test]
    fn a_line_beyond_the_snap_but_within_reach_is_shown_grey_and_does_not_pull() {
        let moving = rect(110.0, 300.0, 150.0, 320.0);
        let other = rect(100.0, 100.0, 105.0, 140.0);
        let ((dx, _), guides) = align(moving, &[other], PAGE, 3.0, 30.0);
        assert_eq!(dx, 0.0);
        let line = guides.vertical.iter().find(|l| (l.at - 105.0).abs() < 0.01).expect("the near line was not shown");
        assert!(!line.exact);
    }

    #[test]
    fn only_the_nearest_grey_lines_are_kept() {
        let others: Vec<Rect> = (0..30).map(|i| rect(100.0 + i as f32, 0.0, 100.5 + i as f32, 5.0)).collect();
        let moving = rect(110.0, 300.0, 120.0, 310.0);
        let (_, guides) = align(moving, &others, PAGE, 0.0, 50.0);
        assert!(guides.vertical.iter().filter(|l| !l.exact).count() <= MOST_NEAR);
    }

    #[test]
    fn two_things_with_the_same_edge_give_one_line() {
        let moving = rect(203.0, 300.0, 260.0, 330.0);
        let others = [rect(200.0, 0.0, 250.0, 10.0), rect(200.0, 20.0, 260.0, 30.0)];
        let (_, guides) = align(moving, &others, PAGE, 5.0, 20.0);
        assert_eq!(guides.vertical.iter().filter(|l| (l.at - 200.0).abs() < 0.01).count(), 1);
    }

    #[test]
    fn a_rectangle_given_backwards_is_read_the_right_way_round() {
        let ((dx, _), _) = align(rect(260.0, 330.0, 203.0, 300.0), &[rect(200.0, 100.0, 280.0, 150.0)], PAGE, 5.0, 20.0);
        assert!((dx + 3.0).abs() < 1e-4, "{dx}");
    }

    #[test]
    fn no_snap_distance_means_lines_are_shown_but_nothing_is_pulled() {
        let moving = rect(203.0, 300.0, 260.0, 330.0);
        let ((dx, dy), guides) = align(moving, &[rect(200.0, 100.0, 280.0, 150.0)], PAGE, 0.0, 20.0);
        assert_eq!((dx, dy), (0.0, 0.0));
        assert!(!guides.vertical.is_empty());
        assert!(guides.vertical.iter().all(|l| !l.exact));
    }
}

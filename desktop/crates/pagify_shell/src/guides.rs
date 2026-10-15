//! Reference lines for moving something, and the pull that makes it land
//! exactly on a line of another thing.
//!
//! **Asked for with a screenshot of what a design program does, then cut down
//! (the screenshot showed a grey line through every edge near the thing).** The
//! lines drawn are the thing's own: its two outermost sides and its middle, on
//! each axis, six at most. One turns green when it is exactly on an edge or a
//! middle of something else on the page, and the thing is pulled onto it; the
//! other things' lines are never drawn themselves. Only while it is being
//! moved — they are a measuring aid, not part of the page.
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
    /// Whether it is exactly on an edge or a middle of something else.
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

/// The rectangle being moved, nudged onto the nearest line, and the lines.
///
/// * `moving` — where it is now, after the drag.
/// * `others` — everything else on the page, the page's own edges and middle
///   excepted (they are added here from `page`).
/// * `page` — its width and height.
/// * `snap` — how close, at most, a side or the middle of `moving` has to be to
///   a line to be pulled onto it, in page points.
///
/// Returns the nudge `(dx, dy)` to add to the drag, and the lines. The lines
/// are those of `moving` **after** the nudge — its sides and its middle — and
/// the exact ones are those it now sits on something else's.
pub fn align(moving: Rect, others: &[Rect], page: (f32, f32), snap: f32) -> ((f32, f32), Guides) {
    let xs =|r: &Rect| [r.left.min(r.right), (r.left + r.right) / 2.0, r.left.max(r.right)];
    let ys = |r: &Rect| [r.top.min(r.bottom), (r.top + r.bottom) / 2.0, r.top.max(r.bottom)];

    let mut x_targets: Vec<f32> = vec![0.0, page.0 / 2.0, page.0];
    let mut y_targets: Vec<f32> = vec![0.0, page.1 / 2.0, page.1];
    for other in others {
        x_targets.extend(xs(other));
        y_targets.extend(ys(other));
    }

    let (dx, vertical) = along(xs(&moving), &x_targets, snap);
    let (dy, horizontal) = along(ys(&moving), &y_targets, snap);
    ((dx, dy), Guides { vertical, horizontal })
}

/// One axis: the three positions of the thing against every target.
fn along(mine: [f32; 3], targets: &[f32], snap: f32) -> (f32, Vec<Line>) {
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

    // The lines: where it will be, one for each of its positions. A thing with no
    // width has all three in one place, which is one line.
    let mut lines: Vec<Line> = Vec::new();
    for m in mine {
        let at = m + nudge;
        let exact = targets.iter().any(|t| (t - at).abs() < SAME);
        match lines.iter_mut().find(|l| (l.at - at).abs() < SAME) {
            Some(existing) => existing.exact |= exact,
            None => lines.push(Line { at, exact }),
        }
    }
    (nudge, lines)
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
        let ((dx, dy), guides) = align(moving, &[rect(400.0, 500.0, 450.0, 520.0)], PAGE, 4.0);
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
        let ((dx, dy), guides) = align(moving, &[other], PAGE, 5.0);
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
        let ((dx, _), guides) = align(moving, &[other], PAGE, 5.0);
        assert!(dx.abs() < 1e-4, "the middles already agree: {dx}");
        assert!(guides.vertical.iter().any(|l| (l.at - 240.0).abs() < 0.01 && l.exact));
    }

    #[test]
    fn it_snaps_to_the_nearest_of_several_lines() {
        // 2 pt from x = 202 and 4 pt from x = 200: the nearer one wins.
        let moving = rect(198.0, 300.0, 240.0, 320.0);
        let others = [rect(200.0, 0.0, 210.0, 10.0), rect(202.0, 20.0, 210.0, 30.0)];
        let ((dx, _), _) = align(moving, &others, PAGE, 5.0);
        assert!((dx - 2.0).abs() < 1e-4 || (dx - 4.0).abs() < 1e-4, "{dx}");
        // Left edge 198 -> 200 is 2; to 202 is 4; so 2.
        assert!((dx - 2.0).abs() < 1e-4, "{dx}");
    }

    #[test]
    fn the_pages_own_edge_and_middle_are_lines_too() {
        let moving = rect(2.0, 300.0, 52.0, 330.0);
        let ((dx, _), guides) = align(moving, &[], PAGE, 5.0);
        assert!((dx + 2.0).abs() < 1e-4, "pulled onto the page's left edge: {dx}");
        assert!(guides.vertical.iter().any(|l| l.at == 0.0 && l.exact));
        // The middle of the page, x = 300.
        let moving = rect(280.0, 300.0, 320.0, 330.0);
        let ((dx, _), guides) = align(moving, &[], PAGE, 5.0);
        assert!(dx.abs() < 1e-4, "{dx}");
        assert!(guides.vertical.iter().any(|l| l.at == 300.0 && l.exact));
    }

    #[test]
    fn another_things_edge_near_it_is_not_drawn_and_does_not_pull_beyond_the_snap() {
        // The reported clutter: a line through every edge near the thing.
        let moving = rect(110.0, 300.0, 150.0, 320.0);
        let other = rect(100.0, 100.0, 105.0, 140.0);
        let ((dx, _), guides) = align(moving, &[other], PAGE, 3.0);
        assert_eq!(dx, 0.0);
        assert!(guides.vertical.iter().all(|l| (l.at - 105.0).abs() > 0.01), "{guides:?}");
    }

    #[test]
    fn only_its_own_sides_and_middle_are_drawn_however_much_is_near() {
        let others: Vec<Rect> = (0..30).map(|i| rect(100.0 + i as f32, 0.0, 100.5 + i as f32, 5.0)).collect();
        let moving = rect(110.0, 300.0, 130.0, 310.0);
        let (_, guides) = align(moving, &others, PAGE, 0.0);
        let at: Vec<f32> = guides.vertical.iter().map(|l| l.at).collect();
        assert_eq!(at, vec![110.0, 120.0, 130.0]);
        let at: Vec<f32> = guides.horizontal.iter().map(|l| l.at).collect();
        assert_eq!(at, vec![300.0, 305.0, 310.0]);
    }

    #[test]
    fn a_thing_with_no_width_is_one_line_not_three() {
        let (_, guides) = align(rect(150.0, 300.0, 150.0, 340.0), &[], PAGE, 0.0);
        assert_eq!(guides.vertical.len(), 1);
        assert_eq!(guides.horizontal.len(), 3);
    }

    #[test]
    fn two_things_with_the_same_edge_give_one_line() {
        // Its left edge is 3 pt off the one edge both others share; nothing else is as near.
        let moving = rect(203.0, 300.0, 240.0, 330.0);
        let others = [rect(200.0, 0.0, 250.0, 10.0), rect(200.0, 20.0, 260.0, 30.0)];
        let (_, guides) = align(moving, &others, PAGE, 5.0);
        assert_eq!(guides.vertical.iter().filter(|l| (l.at - 200.0).abs() < 0.01).count(), 1);
    }

    #[test]
    fn a_rectangle_given_backwards_is_read_the_right_way_round() {
        let ((dx, _), _) = align(rect(260.0, 330.0, 203.0, 300.0), &[rect(200.0, 100.0, 280.0, 150.0)], PAGE, 5.0);
        assert!((dx + 3.0).abs() < 1e-4, "{dx}");
    }

    #[test]
    fn no_snap_distance_means_lines_are_shown_but_nothing_is_pulled() {
        let moving = rect(203.0, 300.0, 260.0, 330.0);
        let ((dx, dy), guides) = align(moving, &[rect(200.0, 100.0, 280.0, 150.0)], PAGE, 0.0);
        assert_eq!((dx, dy), (0.0, 0.0));
        assert!(!guides.vertical.is_empty());
        assert!(guides.vertical.iter().all(|l| !l.exact));
    }
}

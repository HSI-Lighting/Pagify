//! Putting a thing against, or in the middle of, the page: **Align to page**.
//!
//! Asked for by the owner: "align text or object with respect to the page, i.e. middle of the
//! page from the side margins, middle of the page from top margin etc." The reference is the
//! page's own edges (the owner chose that over settable margins), so *Centre horizontally* puts
//! the middle of the thing midway between the left and right edges.
//!
//! Window-free: given where the thing shows and the page, [`delta`] answers how far to move it.
//! All of it in page points, y down, in the frame the page is shown in (a turned page's
//! included: every read and write the app makes is already in that frame).

use pdf_core::document::Rect;

/// Where on the page to put the thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignTo {
    Left,
    CentreH,
    Right,
    Top,
    Middle,
    Bottom,
}

impl AlignTo {
    /// The six, in the order a panel lists them: across, then down.
    pub const ALL: [AlignTo; 6] =
        [AlignTo::Left, AlignTo::CentreH, AlignTo::Right, AlignTo::Top, AlignTo::Middle, AlignTo::Bottom];

    /// What a button says.
    pub fn label(self) -> &'static str {
        match self {
            AlignTo::Left => "Left",
            AlignTo::CentreH => "Centre",
            AlignTo::Right => "Right",
            AlignTo::Top => "Top",
            AlignTo::Middle => "Middle",
            AlignTo::Bottom => "Bottom",
        }
    }

    /// What a button's hover says.
    pub fn hint(self) -> &'static str {
        match self {
            AlignTo::Left => "Against the left edge of the page",
            AlignTo::CentreH => "Centred between the left and right edges of the page",
            AlignTo::Right => "Against the right edge of the page",
            AlignTo::Top => "Against the top edge of the page",
            AlignTo::Middle => "Centred between the top and bottom edges of the page",
            AlignTo::Bottom => "Against the bottom edge of the page",
        }
    }
}

/// How far to move `shown` (the rectangle the thing is seen in) to put it where `how` says on
/// `page`. One axis only: the other is `0`, so a thing aligned across keeps its height.
pub fn delta(shown: Rect, page: (f32, f32), how: AlignTo) -> (f32, f32) {
    let (left, right) = (shown.left.min(shown.right), shown.left.max(shown.right));
    let (top, bottom) = (shown.top.min(shown.bottom), shown.top.max(shown.bottom));
    match how {
        AlignTo::Left => (-left, 0.0),
        AlignTo::CentreH => (page.0 / 2.0 - (left + right) / 2.0, 0.0),
        AlignTo::Right => (page.0 - right, 0.0),
        AlignTo::Top => (0.0, -top),
        AlignTo::Middle => (0.0, page.1 / 2.0 - (top + bottom) / 2.0),
        AlignTo::Bottom => (0.0, page.1 - bottom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect { left, top, right, bottom }
    }

    /// A4, as the owner's example.
    const A4: (f32, f32) = (595.2, 841.8);

    fn after(shown: Rect, how: AlignTo) -> Rect {
        let (dx, dy) = delta(shown, A4, how);
        rect(shown.left + dx, shown.top + dy, shown.right + dx, shown.bottom + dy)
    }

    #[test]
    fn each_of_the_six_puts_the_thing_where_it_says() {
        let square = rect(100.0, 100.0, 150.0, 180.0);
        assert_eq!(after(square, AlignTo::Left).left, 0.0);
        assert_eq!(after(square, AlignTo::Right).right, A4.0);
        assert_eq!(after(square, AlignTo::Top).top, 0.0);
        assert_eq!(after(square, AlignTo::Bottom).bottom, A4.1);
        let across = after(square, AlignTo::CentreH);
        assert!(((across.left + across.right) / 2.0 - A4.0 / 2.0).abs() < 1e-3, "{across:?}");
        let down = after(square, AlignTo::Middle);
        assert!(((down.top + down.bottom) / 2.0 - A4.1 / 2.0).abs() < 1e-3, "{down:?}");
    }

    #[test]
    fn a_move_across_leaves_the_height_and_a_move_down_the_width() {
        let square = rect(100.0, 100.0, 150.0, 180.0);
        for how in [AlignTo::Left, AlignTo::CentreH, AlignTo::Right] {
            assert_eq!(delta(square, A4, how).1, 0.0, "{how:?} moved it up or down");
        }
        for how in [AlignTo::Top, AlignTo::Middle, AlignTo::Bottom] {
            assert_eq!(delta(square, A4, how).0, 0.0, "{how:?} moved it across");
        }
    }

    #[test]
    fn centring_is_right_whichever_corner_the_rectangle_is_given_from() {
        let upright = rect(100.0, 100.0, 150.0, 180.0);
        let flipped = rect(150.0, 180.0, 100.0, 100.0);
        assert_eq!(delta(upright, A4, AlignTo::CentreH), delta(flipped, A4, AlignTo::CentreH));
        assert_eq!(delta(upright, A4, AlignTo::Middle), delta(flipped, A4, AlignTo::Middle));
        assert_eq!(delta(upright, A4, AlignTo::Right), delta(flipped, A4, AlignTo::Right));
        assert_eq!(delta(upright, A4, AlignTo::Bottom), delta(flipped, A4, AlignTo::Bottom));
    }

    #[test]
    fn a_thing_already_there_does_not_move() {
        let centred = rect(272.6, 400.0, 322.6, 440.0);
        assert!(delta(centred, A4, AlignTo::CentreH).0.abs() < 1e-3);
        assert_eq!(delta(rect(0.0, 5.0, 40.0, 9.0), A4, AlignTo::Left), (0.0, 0.0));
    }

    #[test]
    fn a_thing_wider_than_the_page_is_centred_all_the_same() {
        let wide = rect(-50.0, 10.0, 700.0, 20.0);
        let (dx, _) = delta(wide, A4, AlignTo::CentreH);
        assert!(((wide.left + dx + wide.right + dx) / 2.0 - A4.0 / 2.0).abs() < 1e-3);
    }
}

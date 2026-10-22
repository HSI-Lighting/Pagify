//! A page that carries its own `/Rotate`.
//!
//! **Two frames, one page.** The engine reads and writes in the *unrotated*
//! frame: the crop box's top-left corner, y down, exactly as the page's content
//! stream lays it out. PDFium draws the page *turned* by `/Rotate`, and the
//! app's canvas, its strip layout and every click are in that **displayed**
//! frame. For `/Rotate 0` they are the same frame and nothing here does
//! anything. For a turned page they are not, and a sheet drawn sideways and
//! turned upright by `/Rotate 270` (every AutoCAD plot, most scans) had every
//! text box, hover and marquee measured in one frame and drawn in the other.
//!
//! [`Frame`] is the one place the two are related. `Session` converts what the
//! engine reports *into* the displayed frame and what the app asks for *back*
//! into the unrotated one, so the engine's byte-level code, which is verified in
//! its own frame, never has to know a page is turned.
//!
//! `turns` is `/Rotate` in quarter turns clockwise, as `Document::page_rotation`
//! gives it. With the unrotated page `width` × `height`:
//!
//! | turns | displayed x, y of the unrotated point (x, y) |
//! |-------|----------------------------------------------|
//! | 0     | `x, y`                                       |
//! | 1     | `height - y, x`                              |
//! | 2     | `width - x, height - y`                      |
//! | 3     | `y, width - x`                               |

use super::{Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    turns: u8,
    /// The unrotated page's size, which is what the engine's own frame spans.
    width: f32,
    height: f32,
}

impl Frame {
    /// The frame of a page that is not turned.
    pub const NONE: Frame = Frame { turns: 0, width: 0.0, height: 0.0 };

    /// From a page's `/Rotate` and the size PDFium reports for it, which is the
    /// **displayed** size (it already swaps width and height for a quarter turn).
    pub fn new(turns: u8, displayed_width: f32, displayed_height: f32) -> Frame {
        let turns = turns % 4;
        if turns == 0 {
            return Frame::NONE;
        }
        let (width, height) = if turns % 2 == 1 {
            (displayed_height, displayed_width)
        } else {
            (displayed_width, displayed_height)
        };
        Frame { turns, width, height }
    }

    /// Whether this frame changes nothing. Every conversion is then a plain
    /// return of its argument, so a page that is not turned cannot be affected.
    pub fn is_identity(&self) -> bool {
        self.turns == 0
    }

    pub fn turns(&self) -> u8 {
        self.turns
    }

    /// Whether a quarter turn swaps the page's width and height.
    pub fn swaps_axes(&self) -> bool {
        self.turns % 2 == 1
    }

    /// An unrotated point, in the displayed frame.
    pub fn point(&self, p: Point) -> Point {
        let (x, y) = (p.x, p.y);
        match self.turns {
            1 => Point { x: self.height - y, y: x },
            2 => Point { x: self.width - x, y: self.height - y },
            3 => Point { x: y, y: self.width - x },
            _ => p,
        }
    }

    /// A displayed point, in the unrotated frame: the exact inverse of
    /// [`Frame::point`].
    pub fn point_back(&self, p: Point) -> Point {
        let (x, y) = (p.x, p.y);
        match self.turns {
            1 => Point { x: y, y: self.height - x },
            2 => Point { x: self.width - x, y: self.height - y },
            3 => Point { x: self.width - y, y: x },
            _ => p,
        }
    }

    /// A direction (no translation), unrotated to displayed.
    pub fn vector(&self, (dx, dy): (f32, f32)) -> (f32, f32) {
        match self.turns {
            1 => (-dy, dx),
            2 => (-dx, -dy),
            3 => (dy, -dx),
            _ => (dx, dy),
        }
    }

    /// A direction, displayed to unrotated.
    pub fn vector_back(&self, (dx, dy): (f32, f32)) -> (f32, f32) {
        match self.turns {
            1 => (dy, -dx),
            2 => (-dx, -dy),
            3 => (-dy, dx),
            _ => (dx, dy),
        }
    }

    /// A rectangle: its corners carried over and put in order again, since a
    /// quarter turn exchanges which corner is the top-left.
    pub fn rect(&self, r: Rect) -> Rect {
        if self.is_identity() {
            return r;
        }
        let a = self.point(Point { x: r.left, y: r.top });
        let b = self.point(Point { x: r.right, y: r.bottom });
        ordered(a, b)
    }

    pub fn rect_back(&self, r: Rect) -> Rect {
        if self.is_identity() {
            return r;
        }
        let a = self.point_back(Point { x: r.left, y: r.top });
        let b = self.point_back(Point { x: r.right, y: r.bottom });
        ordered(a, b)
    }

    /// The same for the layout module's own rectangle type.
    pub fn layout_rect(&self, r: super::layout::Rect) -> super::layout::Rect {
        if self.is_identity() {
            return r;
        }
        let o = self.rect(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom });
        super::layout::Rect { left: o.left, top: o.top, right: o.right, bottom: o.bottom }
    }

    /// How far a turned page's content has to be turned, in radians, to read as
    /// the page shows it: the displayed frame has y down, so a clockwise page
    /// turn is a positive angle there.
    pub fn quarter_radians(&self) -> f32 {
        self.turns as f32 * std::f32::consts::FRAC_PI_2
    }
}

/// What the engine reports about a page, carried into the displayed frame.
///
/// Each method is a no-op for a page that is not turned. They convert **in
/// place** because the callers hold the engine's own vectors.
impl Frame {
    pub fn text_run(&self, run: &mut super::TextRun) {
        if self.is_identity() {
            return;
        }
        run.rect = self.rect(run.rect);
        run.origin = self.point(run.origin);
    }

    /// The direction a run's text takes **on the page as shown**: the thing
    /// "is this text level?" has to be asked of.
    pub fn run_style(&self, style: &mut super::RunStyle) {
        style.axis = self.vector(style.axis);
    }

    pub fn drawn(&self, object: &mut super::DrawnObject) {
        object.rect = self.rect(object.rect);
    }

    pub fn page_image(&self, image: &mut super::PageImage) {
        image.rect = self.rect(image.rect);
    }

    pub fn locked(&self, item: &mut super::LockedItem) {
        item.rect = self.rect(item.rect);
    }

    pub fn link(&self, link: &mut super::InternalLink) {
        link.rect = self.rect(link.rect);
    }

    pub fn sensitive(&self, found: &mut super::SensitiveOnPage) {
        found.area = self.rect(found.area);
    }

    pub fn signature(&self, mark: &mut super::SignatureMark) {
        for stroke in &mut mark.strokes {
            for p in stroke {
                *p = self.point(*p);
            }
        }
    }

    /// A placed picture as the page shows it: its pixels turned with the page and
    /// its rectangle carried over. Its own rotation, about its centre, is the
    /// same in both frames and is left alone.
    pub fn image_mark(&self, rect: &mut Rect, rgba: &mut Vec<u8>, width: &mut u32, height: &mut u32) {
        if self.is_identity() {
            return;
        }
        *rect = self.rect(*rect);
        let (pixels, w, h) = turn_pixels(rgba, *width, *height, self.turns);
        *rgba = pixels;
        *width = w;
        *height = h;
    }

    pub fn lifted(&self, lifted: &mut super::Lifted) {
        lifted.rect = self.rect(lifted.rect);
    }

    /// A page's characters: four numbers (left, top, right, bottom) each.
    pub fn characters(&self, page: &mut super::PageCharacters) {
        if self.is_identity() {
            return;
        }
        for b in page.boxes.chunks_exact_mut(4) {
            let r = self.rect(Rect { left: b[0], top: b[1], right: b[2], bottom: b[3] });
            b.copy_from_slice(&[r.left, r.top, r.right, r.bottom]);
        }
    }

}

/// A picture's pixels turned `quarters` times clockwise. Returns the new bytes
/// and the new width and height.
pub fn turn_pixels(rgba: &[u8], width: u32, height: u32, quarters: u8) -> (Vec<u8>, u32, u32) {
    let (w, h) = (width as usize, height as usize);
    if quarters % 4 == 0 || rgba.len() != w * h * 4 {
        return (rgba.to_vec(), width, height);
    }
    let mut now = (rgba.to_vec(), w, h);
    for _ in 0..quarters % 4 {
        let (src, sw, sh) = (&now.0, now.1, now.2);
        let mut out = vec![0u8; src.len()];
        // Clockwise: the pixel at (x, y) goes to (sh - 1 - y, x) in an sh-wide image.
        for y in 0..sh {
            for x in 0..sw {
                let to = (x * sh + (sh - 1 - y)) * 4;
                let from = (y * sw + x) * 4;
                out[to..to + 4].copy_from_slice(&src[from..from + 4]);
            }
        }
        now = (out, sh, sw);
    }
    (now.0, now.1 as u32, now.2 as u32)
}

fn ordered(a: Point, b: Point) -> Rect {
    Rect {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x),
        bottom: a.y.max(b.y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AA125 sheet: 1684 x 2384 unrotated, `/Rotate 270`, so 2384 x 1684 shown.
    fn aa125() -> Frame {
        Frame::new(3, 2384.0, 1684.0)
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_page_that_is_not_turned_is_left_alone() {
        let f = Frame::new(0, 612.0, 792.0);
        let r = Rect { left: 1.0, top: 2.0, right: 3.0, bottom: 4.0 };
        assert!(f.is_identity());
        assert_eq!(f.rect(r), r);
        assert_eq!(f.rect_back(r), r);
        assert_eq!(f.point(Point { x: 5.0, y: 6.0 }), Point { x: 5.0, y: 6.0 });
        assert_eq!(f.vector((0.0, 1.0)), (0.0, 1.0));
    }

    #[test]
    fn the_real_label_lands_where_the_page_shows_it() {
        // Measured on AA125 page 1: the "142" label's unrotated rect, and the
        // axis its text matrix gave. On screen it is level and about 15 wide.
        let f = aa125();
        let r = f.rect(Rect { left: 672.0, top: 1648.0, right: 680.0, bottom: 1663.0 });
        assert!(near(r.left, 1648.0) && near(r.right, 1663.0), "{r:?}");
        assert!(near(r.top, 1004.0) && near(r.bottom, 1012.0), "{r:?}");
        assert!(r.width_or_zero() > r.height_or_zero(), "level text is wider than tall");
        let (x, y) = f.vector((0.0, 1.0));
        assert!(near(x, 1.0) && near(y, 0.0), "the text now runs along +x: ({x}, {y})");
    }

    #[test]
    fn displayed_size_is_the_swapped_unrotated_size() {
        for turns in 0..4u8 {
            let f = Frame::new(turns, 2384.0, 1684.0);
            let corner = f.point(Point { x: 0.0, y: 0.0 });
            let far = f.point(Point { x: if turns == 0 { 2384.0 } else if f.swaps_axes() { 1684.0 } else { 2384.0 }, y: if turns == 0 { 1684.0 } else if f.swaps_axes() { 2384.0 } else { 1684.0 } });
            // The unrotated page's two opposite corners are the displayed
            // page's two opposite corners, in some order.
            let (lo, hi) = (corner.x.min(far.x), corner.x.max(far.x));
            let (lo_y, hi_y) = (corner.y.min(far.y), corner.y.max(far.y));
            assert!(near(lo, 0.0) && near(lo_y, 0.0), "turns {turns}");
            assert!(near(hi, 2384.0) && near(hi_y, 1684.0), "turns {turns}: {hi} x {hi_y}");
        }
    }

    #[test]
    fn every_conversion_round_trips() {
        for turns in 0..4u8 {
            let f = Frame::new(turns, 612.0, 792.0);
            for p in [Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 30.0 }, Point { x: 611.0, y: 5.5 }] {
                let back = f.point_back(f.point(p));
                assert!(near(back.x, p.x) && near(back.y, p.y), "turns {turns}: {p:?} -> {back:?}");
                let there = f.point(f.point_back(p));
                assert!(near(there.x, p.x) && near(there.y, p.y), "turns {turns}: inverse first");
            }
            let r = Rect { left: 10.0, top: 20.0, right: 70.0, bottom: 45.0 };
            let again = f.rect_back(f.rect(r));
            assert!(near(again.left, r.left) && near(again.right, r.right), "turns {turns}");
            assert!(near(again.top, r.top) && near(again.bottom, r.bottom), "turns {turns}");
            let v = f.vector_back(f.vector((0.6, -0.8)));
            assert!(near(v.0, 0.6) && near(v.1, -0.8), "turns {turns}: vector");
        }
    }

    #[test]
    fn a_rect_stays_ordered_and_keeps_its_area() {
        for turns in 1..4u8 {
            let f = Frame::new(turns, 612.0, 792.0);
            let r = f.rect(Rect { left: 10.0, top: 20.0, right: 70.0, bottom: 45.0 });
            assert!(r.left < r.right && r.top < r.bottom, "turns {turns}: {r:?}");
            assert!(near((r.right - r.left) * (r.bottom - r.top), 60.0 * 25.0), "turns {turns}");
        }
    }

    #[test]
    fn a_vector_turns_with_the_page_and_keeps_its_length() {
        // One clockwise quarter turn takes "right" to "down" on screen.
        let f = Frame::new(1, 612.0, 792.0);
        let (x, y) = f.vector((1.0, 0.0));
        assert!(near(x, 0.0) && near(y, 1.0), "({x}, {y})");
        let (x, y) = Frame::new(2, 612.0, 792.0).vector((1.0, 0.0));
        assert!(near(x, -1.0) && near(y, 0.0), "({x}, {y})");
    }

    trait Extent {
        fn width_or_zero(&self) -> f32;
        fn height_or_zero(&self) -> f32;
    }
    impl Extent for Rect {
        fn width_or_zero(&self) -> f32 {
            self.right - self.left
        }
        fn height_or_zero(&self) -> f32 {
            self.bottom - self.top
        }
    }
}

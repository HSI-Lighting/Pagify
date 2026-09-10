//! The reader: where the pages sit, which ones matter, and what is selected.
//!
//! Build plan phase 2. All of it is arithmetic over page sizes and character
//! boxes, so all of it is here rather than in the app — the scroll position
//! that decides which page to rasterise is exactly the kind of logic that
//! becomes untestable the moment it lives inside a frame callback.

use std::ops::Range;

use pdf_core::document::search::SearchIndex;
use pdf_core::document::PageCharacters;
use pdf_core::PageSize;

/// Space between pages in the continuous strip, in page points.
pub const PAGE_GAP_PT: f32 = 12.0;

/// How pages are arranged down the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// One page per row, all of them. The reading default.
    #[default]
    Single,
    /// Two pages per row, as an opened book.
    Facing,
    /// Two per row, but page one alone — a cover has no facing page, and
    /// pairing it with page two puts every spread on the wrong side of the
    /// fold for the rest of the document.
    FacingWithCover,
}

impl Layout {
    fn per_row(self) -> usize {
        match self {
            Layout::Single => 1,
            Layout::Facing | Layout::FacingWithCover => 2,
        }
    }

    /// Which row a page belongs to.
    fn row_of(self, page: usize) -> usize {
        match self {
            Layout::Single => page,
            Layout::Facing => page / 2,
            // Page 0 has a row to itself; everything after shifts by one.
            Layout::FacingWithCover => {
                if page == 0 {
                    0
                } else {
                    1 + (page - 1) / 2
                }
            }
        }
    }
}

/// Where every page sits in one continuous vertical strip.
///
/// Built in page points, not pixels. Zoom then scales the whole strip, so
/// changing zoom does not require relaying out — and, more importantly, the
/// scroll position stays meaningful across a zoom change, which is what stops
/// the view jumping to a different page when someone zooms in.
///
/// **Rows, not pages.** A page's vertical position is its *row's*, and a row
/// holds one page or two. Facing pages are not a drawing trick applied at the
/// last moment: they change which page is next to which, what "the page you are
/// looking at" means, and how far a scroll travels. Putting that in the layout
/// keeps every one of those answers in one place.
#[derive(Debug, Clone)]
pub struct Strip {
    /// Top of each page, in strip points. Pages sharing a row share a top.
    tops: Vec<f32>,
    /// Left of each page within the strip's width.
    lefts: Vec<f32>,
    sizes: Vec<(f32, f32)>,
    gap: f32,
    total: f32,
    width: f32,
    layout: Layout,
}

impl Strip {
    pub fn new(sizes: &[PageSize], gap: f32) -> Self {
        Self::with_layout(sizes, gap, Layout::Single)
    }

    pub fn with_layout(sizes: &[PageSize], gap: f32, layout: Layout) -> Self {
        let count = sizes.len();
        let per_row = layout.per_row();

        // Group pages into rows first: a row's height is its tallest page, and
        // a row's width is what the strip has to be wide enough for.
        let mut rows: Vec<Vec<usize>> = Vec::new();
        for page in 0..count {
            let row = layout.row_of(page);
            while rows.len() <= row {
                rows.push(Vec::new());
            }
            rows[row].push(page);
        }

        let row_width = |row: &Vec<usize>| -> f32 {
            let pages: f32 = row.iter().map(|p| sizes[*p].width_pt).sum();
            pages + gap * (row.len().saturating_sub(1)) as f32
        };
        let width = rows.iter().map(row_width).fold(0.0, f32::max).max(
            // A one-page document laid out facing is still as wide as a spread
            // would be, or the page jumps sideways when the second arrives.
            if per_row > 1 && count == 1 { sizes[0].width_pt } else { 0.0 },
        );

        let mut tops = vec![0.0; count];
        let mut lefts = vec![0.0; count];
        let mut cursor = 0.0;

        for row in &rows {
            let height = row.iter().map(|p| sizes[*p].height_pt).fold(0.0, f32::max);
            // Each row centred in the strip, so a narrow page among wide ones
            // sits under them rather than against one edge.
            let mut x = (width - row_width(row)) / 2.0;
            for page in row {
                tops[*page] = cursor;
                lefts[*page] = x;
                x += sizes[*page].width_pt + gap;
            }
            cursor += height + gap;
        }

        // The trailing gap is not part of the document.
        let total = (cursor - gap).max(0.0);

        Strip {
            tops,
            lefts,
            sizes: sizes.iter().map(|s| (s.width_pt, s.height_pt)).collect(),
            gap,
            total,
            width,
            layout,
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn page_count(&self) -> usize {
        self.tops.len()
    }

    /// Total height of the strip in points.
    pub fn height_pt(&self) -> f32 {
        self.total
    }

    /// How wide the strip is — a page in `Single`, a spread in the others.
    pub fn width_pt(&self) -> f32 {
        self.width
    }

    pub fn top_of(&self, page: usize) -> Option<f32> {
        self.tops.get(page).copied()
    }

    /// Where a page sits across the strip. Zero in `Single`, where every page
    /// is centred on its own.
    pub fn left_of(&self, page: usize) -> Option<f32> {
        self.lefts.get(page).copied()
    }

    pub fn size_of(&self, page: usize) -> Option<(f32, f32)> {
        self.sizes.get(page).copied()
    }

    /// Every page overlapping the window `[top, bottom)` in strip points.
    ///
    /// Returns an empty range for a window entirely inside a gap, which is a
    /// real position to be scrolled to and not an error.
    pub fn visible(&self, top: f32, bottom: f32) -> Range<usize> {
        if self.tops.is_empty() || bottom <= top {
            return 0..0;
        }

        let mut first = None;
        let mut last = None;

        for (index, page_top) in self.tops.iter().enumerate() {
            let page_bottom = page_top + self.sizes[index].1;
            if page_bottom > top && *page_top < bottom {
                first.get_or_insert(index);
                last = Some(index);
            }
        }

        match (first, last) {
            (Some(f), Some(l)) => f..l + 1,
            _ => 0..0,
        }
    }

    /// The page a point in the strip falls on, or the nearer one when the point
    /// is in a gap. Used to answer "which page am I looking at" for the status
    /// line, where "none, you are between two" is not a useful answer.
    pub fn page_at(&self, y: f32) -> usize {
        if self.tops.is_empty() {
            return 0;
        }

        for (index, top) in self.tops.iter().enumerate() {
            if y < top + self.sizes[index].1 + self.gap / 2.0 {
                return index;
            }
        }
        self.tops.len() - 1
    }

    /// The scroll offset that puts `page` at the top of the window.
    pub fn scroll_to(&self, page: usize) -> f32 {
        self.tops.get(page).copied().unwrap_or(0.0)
    }
}

/// What to rasterise ahead of the viewport, and in what order.
///
/// The cache exists so a scroll resolves to a copy rather than a render, and it
/// only pays off if the right pages are in it. `radius` pages either side of
/// what is visible, nearest first — because a prefetch that arrives after the
/// user has already scrolled past is wasted work, and the nearest page is the
/// one they are most likely to reach first.
pub fn prefetch_targets(visible: &Range<usize>, page_count: usize, radius: usize) -> Vec<usize> {
    if page_count == 0 || visible.is_empty() {
        return Vec::new();
    }

    let mut targets = Vec::new();
    for distance in 1..=radius {
        // Forward first at each distance: reading goes down the document, so
        // the page below is likelier than the page above.
        if let Some(after) = visible.end.checked_add(distance - 1) {
            if after < page_count {
                targets.push(after);
            }
        }
        if let Some(before) = visible.start.checked_sub(distance) {
            targets.push(before);
        }
    }
    targets
}

// ---------------------------------------------------------------------------
// Text selection
// ---------------------------------------------------------------------------

/// A rectangle in page space: points, top-left origin, y down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }

    fn union(self, other: Rect) -> Rect {
        Rect {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// A page's characters with their boxes, in reading order.
///
/// `pdf_core` hands back a flat `Vec<f32>` of four floats per code unit, which
/// is the right shape to cross a language boundary and the wrong one to work
/// with. This unpacks it once.
pub struct Characters {
    text: Vec<char>,
    boxes: Vec<Rect>,
}

impl Characters {
    pub fn new(raw: PageCharacters) -> Self {
        let text: Vec<char> = raw.text.chars().collect();
        let boxes: Vec<Rect> = raw
            .boxes
            .chunks_exact(4)
            .map(|b| Rect { left: b[0], top: b[1], right: b[2], bottom: b[3] })
            .collect();

        // A box per code unit is the contract. If they ever disagree, trust the
        // shorter: a selection over characters with no box cannot be drawn, and
        // guessing a box puts a highlight somewhere there is no text.
        let n = text.len().min(boxes.len());
        Characters { text: text[..n].to_vec(), boxes: boxes[..n].to_vec() }
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The character whose box contains this point, if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.boxes.iter().position(|b| b.contains(x, y))
    }

    /// The character nearest a point, for a drag that has wandered into a
    /// margin. A selection that stops dead when the pointer leaves the text is
    /// worse than one that keeps up.
    pub fn nearest(&self, x: f32, y: f32) -> Option<usize> {
        if let Some(hit) = self.hit(x, y) {
            return Some(hit);
        }

        self.boxes
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                distance_to(a, x, y)
                    .partial_cmp(&distance_to(b, x, y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, _)| index)
    }

    /// The characters between two points, in document order.
    ///
    /// Ordered by *character index*, not by position, which is what makes a
    /// backwards drag select the same run as a forwards one — and what makes a
    /// selection across a two-column page follow the reading order rather than
    /// sweeping a rectangle through both columns.
    pub fn range_between(&self, from: (f32, f32), to: (f32, f32)) -> Option<Range<usize>> {
        let a = self.nearest(from.0, from.1)?;
        let b = self.nearest(to.0, to.1)?;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        Some(start..(end + 1).min(self.text.len()))
    }

    /// The whole page as a string, in character order.
    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    /// Every match, as **character** ranges that index this page's boxes.
    ///
    /// Searching `Page::text()` instead would be easier and wrong: that string
    /// and this one are built differently, so a byte offset into it does not
    /// address a box here. A highlight drawn from the wrong index lands on the
    /// wrong word — which looks like a broken search rather than a broken
    /// conversion, and is much harder to trace.
    ///
    /// Matching is NFKC and case-folded, so `final` finds `ﬁnal` and `Lamp`
    /// finds `LAMP`, while the text this returns ranges over stays exactly as
    /// the document has it.
    pub fn find(&self, needle: &str) -> Vec<Range<usize>> {
        if needle.trim().is_empty() || self.text.is_empty() {
            return Vec::new();
        }

        // Built from our own characters, so byte offsets map back by
        // construction rather than by assumption.
        let joined: String = self.text.iter().collect();
        let index = SearchIndex::new(&joined);

        let mut byte_to_char = vec![0usize; joined.len() + 1];
        for (character, (byte, _)) in joined.char_indices().enumerate() {
            byte_to_char[byte] = character;
        }
        byte_to_char[joined.len()] = self.text.len();
        // Fill the interior bytes of multi-byte characters, so a range landing
        // mid-character still resolves rather than reading a zero.
        let mut last = 0;
        for slot in byte_to_char.iter_mut() {
            if *slot == 0 && last != 0 {
                *slot = last;
            } else {
                last = *slot;
            }
        }

        index
            .find(needle)
            .into_iter()
            .filter_map(|hit| {
                let from = *byte_to_char.get(hit.start)?;
                let to = *byte_to_char.get(hit.end)?;
                (from < to).then_some(from..to)
            })
            .collect()
    }

    pub fn text_of(&self, range: Range<usize>) -> String {
        self.text
            .get(range)
            .map(|slice| slice.iter().collect())
            .unwrap_or_default()
    }

    /// One rect per line covered, which is the shape a highlight wants — a
    /// selection spanning three lines is one mark, not three.
    ///
    /// Lines are found by a break in vertical position rather than by looking
    /// for newlines, because extracted text often has none.
    pub fn line_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let Some(boxes) = self.boxes.get(range) else {
            return Vec::new();
        };
        if boxes.is_empty() {
            return Vec::new();
        }

        let mut lines: Vec<Rect> = Vec::new();
        let mut current = boxes[0];

        for box_ in &boxes[1..] {
            // Same line when the vertical spans overlap at all. Comparing tops
            // alone breaks on mixed font sizes, where a larger glyph on the
            // same baseline starts higher.
            let overlaps = box_.top < current.bottom && box_.bottom > current.top;
            if overlaps {
                current = current.union(*box_);
            } else {
                lines.push(current);
                current = *box_;
            }
        }
        lines.push(current);
        lines
    }
}

fn distance_to(rect: &Rect, x: f32, y: f32) -> f32 {
    let dx = (rect.left - x).max(0.0).max(x - rect.right);
    let dy = (rect.top - y).max(0.0).max(y - rect.bottom);
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(heights: &[f32]) -> Vec<PageSize> {
        heights
            .iter()
            .map(|h| PageSize { width_pt: 600.0, height_pt: *h })
            .collect()
    }

    #[test]
    fn the_strip_stacks_pages_with_gaps_but_not_a_trailing_one() {
        let strip = Strip::new(&sizes(&[100.0, 200.0, 50.0]), 10.0);

        assert_eq!(strip.top_of(0), Some(0.0));
        assert_eq!(strip.top_of(1), Some(110.0));
        assert_eq!(strip.top_of(2), Some(320.0));
        assert_eq!(strip.height_pt(), 370.0, "a trailing gap is not part of the document");
    }

    #[test]
    fn an_empty_document_has_no_height_and_no_visible_pages() {
        let strip = Strip::new(&[], 10.0);
        assert_eq!(strip.height_pt(), 0.0);
        assert!(strip.visible(0.0, 100.0).is_empty());
        assert_eq!(strip.page_at(0.0), 0, "and answers page queries without panicking");
    }

    #[test]
    fn visibility_includes_every_page_the_window_touches() {
        let strip = Strip::new(&sizes(&[100.0, 100.0, 100.0]), 10.0);

        assert_eq!(strip.visible(0.0, 50.0), 0..1);
        // Straddling the gap between pages 0 and 1.
        assert_eq!(strip.visible(90.0, 130.0), 0..2);
        assert_eq!(strip.visible(0.0, 1000.0), 0..3);
        // A tall window past the end still stops at the last page.
        assert_eq!(strip.visible(300.0, 900.0), 2..3);
    }

    #[test]
    fn a_window_entirely_inside_a_gap_sees_nothing() {
        let strip = Strip::new(&sizes(&[100.0, 100.0]), 40.0);
        assert!(strip.visible(105.0, 135.0).is_empty(), "a gap is a real place to be");
    }

    #[test]
    fn prefetch_reaches_out_from_the_visible_run_nearest_first_and_forwards_first() {
        // Visible 5..7 (pages 5 and 6) in a 20-page document.
        let targets = prefetch_targets(&(5..7), 20, 2);
        assert_eq!(targets, vec![7, 4, 8, 3]);
    }

    #[test]
    fn prefetch_does_not_run_off_either_end() {
        assert_eq!(prefetch_targets(&(0..1), 2, 3), vec![1]);
        assert_eq!(prefetch_targets(&(0..1), 1, 2), Vec::<usize>::new());
        assert!(prefetch_targets(&(0..0), 10, 2).is_empty(), "nothing visible, nothing to guess");
        assert!(prefetch_targets(&(0..1), 0, 2).is_empty());
    }

    // -- text selection ------------------------------------------------------

    /// "ab" on one line, "cd" on the next.
    fn two_lines() -> Characters {
        Characters::new(PageCharacters {
            text: "abcd".into(),
            boxes: vec![
                0.0, 0.0, 10.0, 10.0,
                10.0, 0.0, 20.0, 10.0,
                0.0, 20.0, 10.0, 30.0,
                10.0, 20.0, 20.0, 30.0,
            ],
        })
    }

    #[test]
    fn a_character_is_found_under_a_point() {
        let chars = two_lines();
        assert_eq!(chars.hit(5.0, 5.0), Some(0));
        assert_eq!(chars.hit(15.0, 25.0), Some(3));
        assert_eq!(chars.hit(500.0, 500.0), None);
    }

    #[test]
    fn a_drag_into_the_margin_still_selects() {
        let chars = two_lines();
        assert_eq!(chars.nearest(500.0, 25.0), Some(3), "off to the right of the last line");
        assert_eq!(chars.nearest(-50.0, 5.0), Some(0));
    }

    #[test]
    fn selection_follows_reading_order_not_drag_direction() {
        let chars = two_lines();
        let forwards = chars.range_between((5.0, 5.0), (15.0, 25.0)).unwrap();
        let backwards = chars.range_between((15.0, 25.0), (5.0, 5.0)).unwrap();

        assert_eq!(forwards, backwards, "a backwards drag selects the same run");
        assert_eq!(chars.text_of(forwards), "abcd");
    }

    #[test]
    fn a_selection_across_lines_yields_one_rect_per_line() {
        let chars = two_lines();
        let rects = chars.line_rects(0..4);

        assert_eq!(rects.len(), 2, "two lines covered, so two rects");
        assert_eq!(rects[0], Rect { left: 0.0, top: 0.0, right: 20.0, bottom: 10.0 });
        assert_eq!(rects[1], Rect { left: 0.0, top: 20.0, right: 20.0, bottom: 30.0 });
    }

    #[test]
    fn a_taller_glyph_on_the_same_baseline_stays_on_the_same_line() {
        // The bug a top-coordinate comparison would have: a capital or a larger
        // font on the same line starts higher and would open a second rect.
        let chars = Characters::new(PageCharacters {
            text: "Ab".into(),
            boxes: vec![0.0, 0.0, 10.0, 12.0, 10.0, 4.0, 18.0, 12.0],
        });

        assert_eq!(chars.line_rects(0..2).len(), 1);
    }

    #[test]
    fn characters_without_boxes_are_dropped_rather_than_guessed() {
        // A highlight drawn from a guessed box lands where there is no text.
        let chars = Characters::new(PageCharacters {
            text: "abcd".into(),
            boxes: vec![0.0, 0.0, 10.0, 10.0],
        });
        assert_eq!(chars.len(), 1);
        assert_eq!(chars.text_of(0..1), "a");
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    /// "the final draft" laid out one character per 10pt.
    fn page(text: &str) -> Characters {
        let mut boxes = Vec::new();
        for (i, _) in text.chars().enumerate() {
            let x = i as f32 * 10.0;
            boxes.extend_from_slice(&[x, 0.0, x + 9.0, 12.0]);
        }
        Characters::new(PageCharacters { text: text.into(), boxes })
    }

    #[test]
    fn a_hit_indexes_this_pages_own_boxes() {
        // The whole reason this does not search `Page::text()`: a range has to
        // address the boxes it will be drawn over.
        let chars = page("the final draft");
        let hits = chars.find("final");

        assert_eq!(hits.len(), 1);
        assert_eq!(chars.text_of(hits[0].clone()), "final");

        // And the rect covers exactly those characters, not somewhere else.
        let rects = chars.line_rects(hits[0].clone());
        assert_eq!(rects.len(), 1);
        assert!((rects[0].left - 40.0).abs() < 0.01, "highlight at {}", rects[0].left);
    }

    #[test]
    fn search_is_case_and_ligature_insensitive() {
        assert_eq!(page("LAMP lamp Lamp").find("lamp").len(), 3);
        assert_eq!(page("the ﬁnal draft").find("final").len(), 1);
    }

    #[test]
    fn a_ligature_hit_still_lands_on_the_right_box() {
        // `ﬁ` is one character here and two after folding, so a naive offset
        // would put the highlight one box to the right of the word.
        let chars = page("aﬁx");
        let hits = chars.find("fi");
        assert_eq!(hits.len(), 1);
        assert_eq!(chars.text_of(hits[0].clone()), "ﬁ", "landed on the wrong character");
    }

    #[test]
    fn nothing_matches_nothing() {
        assert!(page("some text").find("absent").is_empty());
        assert!(page("some text").find("").is_empty());
        assert!(page("").find("anything").is_empty());
    }

    #[test]
    fn every_occurrence_is_found() {
        let chars = page("ip65 ip65 ip65");
        assert_eq!(chars.find("ip65").len(), 3);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn sizes(n: usize) -> Vec<PageSize> {
        (0..n).map(|_| PageSize { width_pt: 200.0, height_pt: 300.0 }).collect()
    }

    #[test]
    fn single_stacks_every_page_on_its_own_row() {
        let strip = Strip::with_layout(&sizes(3), 10.0, Layout::Single);
        assert_eq!(strip.top_of(0), Some(0.0));
        assert_eq!(strip.top_of(1), Some(310.0));
        assert_eq!(strip.top_of(2), Some(620.0));
        assert_eq!(strip.width_pt(), 200.0);
    }

    #[test]
    fn facing_puts_two_pages_side_by_side() {
        let strip = Strip::with_layout(&sizes(4), 10.0, Layout::Facing);
        assert_eq!(strip.top_of(0), strip.top_of(1), "the first two should share a row");
        assert_eq!(strip.top_of(2), strip.top_of(3), "the next two should share a row");
        assert_ne!(strip.top_of(0), strip.top_of(2), "all four ended up on one row");

        assert!(
            strip.left_of(1).unwrap() > strip.left_of(0).unwrap(),
            "the second page is not to the right of the first"
        );
        assert_eq!(strip.width_pt(), 410.0, "the strip is not a spread wide");
    }

    /// A cover has no facing page. Pairing it with page two puts every spread
    /// in the document on the wrong side of the fold.
    #[test]
    fn a_separate_cover_shifts_every_spread_by_one() {
        let strip = Strip::with_layout(&sizes(5), 10.0, Layout::FacingWithCover);
        assert_ne!(strip.top_of(0), strip.top_of(1), "the cover was paired with page two");
        assert_eq!(strip.top_of(1), strip.top_of(2), "pages two and three should face");
        assert_eq!(strip.top_of(3), strip.top_of(4), "pages four and five should face");
    }

    /// Height is the tallest page in the row, or a short page beside a tall one
    /// would have the next row drawn over it.
    #[test]
    fn a_row_is_as_tall_as_its_tallest_page() {
        let mixed = vec![
            PageSize { width_pt: 200.0, height_pt: 300.0 },
            PageSize { width_pt: 200.0, height_pt: 500.0 },
            PageSize { width_pt: 200.0, height_pt: 300.0 },
        ];
        let strip = Strip::with_layout(&mixed, 10.0, Layout::Facing);
        assert_eq!(strip.top_of(2), Some(510.0), "the third page overlaps the tall one");
    }

    /// A narrow page among wide ones sits under them rather than against an
    /// edge — the same reason a single page is centred in the window.
    #[test]
    fn a_narrow_row_is_centred_in_the_strip() {
        let mixed = vec![
            PageSize { width_pt: 400.0, height_pt: 300.0 },
            PageSize { width_pt: 400.0, height_pt: 300.0 },
            PageSize { width_pt: 200.0, height_pt: 300.0 },
        ];
        let strip = Strip::with_layout(&mixed, 10.0, Layout::Facing);
        // Row two holds one 200pt page in an 810pt strip.
        assert_eq!(strip.left_of(2), Some(305.0));
    }

    /// Whatever the layout, every page must be somewhere and the strip must be
    /// tall enough to hold it.
    #[test]
    fn every_page_is_inside_the_strip_in_every_layout() {
        for layout in [Layout::Single, Layout::Facing, Layout::FacingWithCover] {
            let strip = Strip::with_layout(&sizes(7), 10.0, layout);
            for page in 0..7 {
                let top = strip.top_of(page).expect("a top");
                let (w, h) = strip.size_of(page).expect("a size");
                let left = strip.left_of(page).expect("a left");
                assert!(top + h <= strip.height_pt() + 0.01, "{layout:?}: page {page} hangs off the bottom");
                assert!(left + w <= strip.width_pt() + 0.01, "{layout:?}: page {page} hangs off the side");
                assert!(left >= -0.01, "{layout:?}: page {page} starts left of the strip");
            }
        }
    }

    #[test]
    fn an_empty_document_lays_out_without_panicking() {
        for layout in [Layout::Single, Layout::Facing, Layout::FacingWithCover] {
            let strip = Strip::with_layout(&[], 10.0, layout);
            assert_eq!(strip.page_count(), 0);
            assert_eq!(strip.height_pt(), 0.0);
        }
    }
}

// The placement rules are adapted from PdfCraft (https://github.com/storytold/pdfcraft,
// crates/edit/src/text.rs `edit_block` and crates/ui-egui/src/edit_text_ui.rs `editor_job` @0dc77fe).
// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors. MIT License; see THIRD_PARTY_NOTICES.md.

//! Left, centre, right and justify for a page's own paragraphs, as plain arithmetic.
//!
//! **The rules only.** Where each line of a paragraph goes for an alignment, which lines are
//! stretched to fill the box, and what alignment a paragraph already has. Nothing here reads a
//! page or writes one: the app moves lines whose words are unchanged (they keep their codes and
//! spacing) and writes only the lines that were retyped.

/// How a paragraph's lines sit in its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineAlign {
    #[default]
    Left,
    Center,
    Right,
    /// Every line but the last fills the box; the last sits at the left.
    Justify,
}

/// Where a line shows its ink, in page points: its first mark's left edge and its last mark's
/// right edge (`Characters::ink_left_in` / `ink_right_in`), not its text box, which can run past.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineInk {
    pub left: f32,
    pub right: f32,
}

/// The alignment a paragraph's lines already have.
///
/// `justified` is the caller's own finding (the app's `paragraph_is_justified`, which looks at the
/// ink too) and wins. Otherwise, with two lines or more: **centred** when the lines start at
/// different places (by more than half the type size) and every line's middle is within the type
/// size of the paragraph's middle, as PdfCraft tells a centred paragraph; **right** when they start
/// at different places and every line ends within half the type size of the paragraph's right;
/// **left** otherwise. One line says nothing, and is taken as left.
pub fn detect(lines: &[LineInk], size: f32, justified: bool) -> LineAlign {
    if justified {
        return LineAlign::Justify;
    }
    if lines.len() < 2 {
        return LineAlign::Left;
    }
    let size = size.abs().max(0.01);
    let (left, right) = lines.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), line| (l.min(line.left), r.max(line.right)));
    let middle = (left + right) / 2.0;
    let starts = lines.iter().map(|line| line.left).fold(f32::NEG_INFINITY, f32::max) - left;
    if starts <= 0.5 * size {
        return LineAlign::Left;
    }
    if lines.iter().all(|line| ((line.left + line.right) / 2.0 - middle).abs() <= size) {
        return LineAlign::Center;
    }
    if lines.iter().all(|line| (right - line.right).abs() <= 0.5 * size) {
        return LineAlign::Right;
    }
    LineAlign::Left
}

/// Where each line's left edge goes in a box `box_width` wide starting at `box_left`, for lines
/// `widths` wide: the free space on both sides for centre, all of it on the left for right, none
/// for left and for justify (a justified line fills the box from its left, and its last line sits
/// at the left). A line wider than the box starts at the box's left.
pub fn offsets(widths: &[f32], box_left: f32, box_width: f32, align: LineAlign) -> Vec<f32> {
    widths
        .iter()
        .map(|width| {
            let free = (box_width - width).max(0.0);
            box_left
                + match align {
                    LineAlign::Center => free / 2.0,
                    LineAlign::Right => free,
                    LineAlign::Left | LineAlign::Justify => 0.0,
                }
        })
        .collect()
}

/// The width each line is to be stretched to: `Some(box_width)` for every line but the last of a
/// justified paragraph that has a gap to widen, `None` for every other line. `gaps[i]` is how
/// many word gaps line *i* has.
pub fn stretch_to(lines: usize, box_width: f32, align: LineAlign, gaps: &[usize]) -> Vec<Option<f32>> {
    (0..lines)
        .map(|line| {
            let last = line + 1 == lines;
            let has_gap = gaps.get(line).copied().unwrap_or(0) > 0;
            (align == LineAlign::Justify && !last && has_gap).then_some(box_width)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(left: f32, width: f32) -> LineInk {
        LineInk { left, right: left + width }
    }

    #[test]
    fn a_ragged_left_paragraph_reads_as_left() {
        assert_eq!(detect(&[ink(10.0, 100.0), ink(10.0, 60.0), ink(10.0, 80.0)], 12.0, false), LineAlign::Left);
    }

    #[test]
    fn rows_centred_on_one_axis_read_as_centre() {
        // Middles at 60, 60, 60.
        assert_eq!(detect(&[ink(10.0, 100.0), ink(30.0, 60.0), ink(20.0, 80.0)], 12.0, false), LineAlign::Center);
    }

    #[test]
    fn rows_ending_together_read_as_right() {
        assert_eq!(detect(&[ink(10.0, 100.0), ink(50.0, 60.0), ink(30.0, 80.0)], 12.0, false), LineAlign::Right);
    }

    #[test]
    fn the_justified_flag_wins() {
        assert_eq!(detect(&[ink(10.0, 100.0), ink(30.0, 60.0)], 12.0, true), LineAlign::Justify);
    }

    #[test]
    fn one_line_reads_as_left() {
        assert_eq!(detect(&[ink(40.0, 20.0)], 12.0, false), LineAlign::Left);
    }

    #[test]
    fn centre_puts_the_free_space_on_both_sides() {
        assert_eq!(offsets(&[60.0], 0.0, 100.0, LineAlign::Center), vec![20.0]);
        assert_eq!(offsets(&[60.0], 50.0, 100.0, LineAlign::Center), vec![70.0]);
    }

    #[test]
    fn right_puts_it_on_the_left_and_left_and_justify_put_none() {
        assert_eq!(offsets(&[60.0], 0.0, 100.0, LineAlign::Right), vec![40.0]);
        assert_eq!(offsets(&[60.0, 90.0], 5.0, 100.0, LineAlign::Left), vec![5.0, 5.0]);
        assert_eq!(offsets(&[60.0, 90.0], 5.0, 100.0, LineAlign::Justify), vec![5.0, 5.0]);
        assert_eq!(offsets(&[120.0], 5.0, 100.0, LineAlign::Center), vec![5.0], "a line wider than the box starts at its left");
    }

    #[test]
    fn justify_stretches_every_line_but_the_last() {
        assert_eq!(stretch_to(3, 100.0, LineAlign::Justify, &[2, 3, 1]), vec![Some(100.0), Some(100.0), None]);
        assert_eq!(stretch_to(2, 100.0, LineAlign::Center, &[2, 3]), vec![None, None]);
    }

    #[test]
    fn a_line_with_no_gap_is_not_stretched() {
        assert_eq!(stretch_to(2, 100.0, LineAlign::Justify, &[0, 4]), vec![None, None]);
    }
}

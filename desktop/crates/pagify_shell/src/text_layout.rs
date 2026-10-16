//! Where each run of a text box's rows goes on the page, in points — what the box's
//! writer puts there.
//!
//! **The layout places, it does not wrap.** Which characters make up each row, and
//! where its baseline is, come in from what the box showed (`RowIn`), so the page
//! has the rows the person saw. Within a row every run is measured with its own
//! face's advances, so a run's look moves the ones after it along.

use std::ops::Range;

pub use pdf_core::text::LineMetrics;

use crate::rich_text::{CharLook, RichText, Script};

/// A superscript or subscript is drawn at this share of its size…
pub const SCRIPT_SCALE: f32 = 0.58;
/// …raised by this many of its size's ems,
pub const SUPER_RISE_EM: f32 = 0.33;
/// or lowered by this many.
pub const SUB_DROP_EM: f32 = 0.14;

/// What the layout needs to know of a face.
pub trait Measure {
    /// One advance per glyph, as shaped, in ems.
    fn advances_em(&self, face: &str, text: &str) -> Vec<f32>;
    fn lines(&self, face: &str) -> LineMetrics;
}

/// The real faces, as `pdf_core::text` has them registered.
pub struct FaceMeasure;

impl Measure for FaceMeasure {
    fn advances_em(&self, face: &str, text: &str) -> Vec<f32> {
        pdf_core::text::shape(face, text).map(|s| s.glyphs.iter().map(|g| g.advance).collect()).unwrap_or_default()
    }

    fn lines(&self, face: &str) -> LineMetrics {
        pdf_core::text::line_metrics(face).unwrap_or(LineMetrics::FALLBACK)
    }
}

/// One row as the box showed it: its characters, and its baseline in points from
/// the top of the page.
#[derive(Debug, Clone, PartialEq)]
pub struct RowIn {
    pub chars: Range<usize>,
    pub baseline_pt: f32,
}

/// Where a row sits between the box's edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAlign {
    Left,
    Center,
    Right,
}

/// One run of one row, placed.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedSpan {
    pub chars: Range<usize>,
    pub text: String,
    pub look: CharLook,
    /// Where it starts.
    pub x_pt: f32,
    /// Its baseline, rise included; y counts downwards.
    pub baseline_pt: f32,
    /// The size it is drawn at, a superscript's or subscript's scale included.
    pub size_pt: f32,
    /// One per glyph, its letter spacing and width included.
    pub advances_pt: Vec<f32>,
    /// Its width, as a share of its own.
    pub scale_x: f32,
}

/// An underline or a strike-through.
#[derive(Debug, Clone, PartialEq)]
pub struct LineRule {
    pub x0_pt: f32,
    pub x1_pt: f32,
    pub y_pt: f32,
    pub thickness_pt: f32,
    pub color: [u8; 4],
}

/// A row's runs and its lines.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedRow {
    pub spans: Vec<PlacedSpan>,
    pub rules: Vec<LineRule>,
}

/// Place every run of every row in a box `width_pt` wide whose left edge is at
/// `left_pt`.
pub fn place_rows(
    rich: &RichText,
    rows: &[RowIn],
    left_pt: f32,
    width_pt: f32,
    align: RowAlign,
    measure: &dyn Measure,
) -> Vec<PlacedRow> {
    let chars: Vec<char> = rich.text().chars().collect();
    rows.iter().map(|row| place_row(rich, &chars, row, left_pt, width_pt, align, measure)).collect()
}

fn place_row(
    rich: &RichText,
    chars: &[char],
    row: &RowIn,
    left_pt: f32,
    width_pt: f32,
    align: RowAlign,
    measure: &dyn Measure,
) -> PlacedRow {
    // A row's trailing spaces (and its newline) are neither measured nor underlined:
    // a right-aligned or centred row lines up by its last letter.
    let mut end = row.chars.end.min(chars.len());
    while end > row.chars.start && chars[end - 1].is_whitespace() {
        end -= 1;
    }

    let mut spans: Vec<PlacedSpan> = rich
        .spans(row.chars.start..end)
        .into_iter()
        .map(|(range, look)| {
            let text: String = chars[range.clone()].iter().collect();
            let size_pt = if look.script == Script::Normal { look.size } else { look.size * SCRIPT_SCALE };
            let rise = match look.script {
                Script::Normal => 0.0,
                Script::Super => SUPER_RISE_EM * look.size,
                Script::Sub => -SUB_DROP_EM * look.size,
            } + look.baseline_pt;
            let scale_x = look.width_pct / 100.0;
            let advances_pt = measure
                .advances_em(&look.face, &text)
                .iter()
                .map(|advance| advance * size_pt * scale_x + look.letter_spacing_pt)
                .collect();
            PlacedSpan {
                chars: range,
                text,
                look: look.clone(),
                x_pt: 0.0,
                baseline_pt: row.baseline_pt - rise,
                size_pt,
                advances_pt,
                scale_x,
            }
        })
        .collect();

    // No spacing after the row's last glyph: the row ends at its last letter.
    if let Some(last) = spans.last_mut() {
        let spacing = last.look.letter_spacing_pt;
        if let Some(advance) = last.advances_pt.last_mut() {
            *advance -= spacing;
        }
    }

    let row_width: f32 = spans.iter().map(span_width).sum();
    let mut x = match align {
        RowAlign::Left => left_pt,
        RowAlign::Center => left_pt + (width_pt - row_width) / 2.0,
        RowAlign::Right => left_pt + width_pt - row_width,
    };
    for span in &mut spans {
        span.x_pt = x;
        x += span_width(span);
    }

    let mut rules = rules_of(&spans, row.baseline_pt, measure, |look| look.underline, |m| (m.underline_position, m.underline_thickness));
    rules.extend(rules_of(&spans, row.baseline_pt, measure, |look| look.strike, |m| (m.strikeout_position, m.strikeout_thickness)));
    PlacedRow { spans, rules }
}

fn span_width(span: &PlacedSpan) -> f32 {
    span.advances_pt.iter().sum()
}

/// One rule per stretch of adjacent spans that `marked` and share a colour, at the
/// line the stretch's largest span's face gives (`which`), on the row's baseline —
/// a raised span does not raise it.
fn rules_of(
    spans: &[PlacedSpan],
    baseline_pt: f32,
    measure: &dyn Measure,
    marked: impl Fn(&CharLook) -> bool,
    which: impl Fn(&LineMetrics) -> (f32, f32),
) -> Vec<LineRule> {
    let mut rules = Vec::new();
    let mut i = 0;
    while i < spans.len() {
        if !marked(&spans[i].look) {
            i += 1;
            continue;
        }
        let color = spans[i].look.color;
        let mut j = i;
        while j + 1 < spans.len() && marked(&spans[j + 1].look) && spans[j + 1].look.color == color {
            j += 1;
        }
        let stretch = &spans[i..=j];
        let largest = stretch.iter().max_by(|a, b| a.size_pt.total_cmp(&b.size_pt)).expect("a stretch has a span");
        let (position, thickness) = which(&measure.lines(&largest.look.face));
        let last = &stretch[stretch.len() - 1];
        rules.push(LineRule {
            x0_pt: stretch[0].x_pt,
            x1_pt: last.x_pt + span_width(last),
            y_pt: baseline_pt - position * largest.size_pt,
            thickness_pt: thickness * largest.size_pt,
            color,
        });
        i = j + 1;
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich_text::{CharChange, CharLook, RichText, Script};

    /// Every glyph half an em wide, and lines where a test can count on them.
    struct FixedMeasure;

    impl Measure for FixedMeasure {
        fn advances_em(&self, _face: &str, text: &str) -> Vec<f32> {
            vec![0.5; text.chars().count()]
        }
        fn lines(&self, _face: &str) -> LineMetrics {
            LineMetrics {
                ascent: 0.8,
                descent: 0.2,
                underline_position: -0.1,
                underline_thickness: 0.05,
                strikeout_position: 0.3,
                strikeout_thickness: 0.05,
            }
        }
    }

    fn rich(text: &str, size: f32) -> RichText {
        let mut r = RichText::new(CharLook::plain("Face", size, [0, 0, 0, 255]));
        r.edit(text);
        r
    }

    fn change(r: &mut RichText, range: Range<usize>, change: CharChange) {
        r.set_char(range, &change);
    }

    fn one_row(r: &RichText, align: RowAlign) -> PlacedRow {
        let rows = [RowIn { chars: 0..r.char_count(), baseline_pt: 100.0 }];
        place_rows(r, &rows, 10.0, 200.0, align, &FixedMeasure).remove(0)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn spans_follow_one_another_along_the_row() {
        let mut r = rich("abcd", 10.0);
        change(&mut r, 2..4, CharChange { size: Some(20.0), ..Default::default() });
        let row = one_row(&r, RowAlign::Left);
        assert_eq!(row.spans.len(), 2);
        assert!(close(row.spans[0].x_pt, 10.0));
        assert!(close(row.spans[1].x_pt, 10.0 + 2.0 * 0.5 * 10.0), "{}", row.spans[1].x_pt);
    }

    #[test]
    fn centre_and_right_alignment_place_the_row_in_the_box() {
        let r = rich("abcd", 10.0);
        assert!(close(one_row(&r, RowAlign::Center).spans[0].x_pt, 10.0 + 90.0));
        assert!(close(one_row(&r, RowAlign::Right).spans[0].x_pt, 10.0 + 180.0));
    }

    #[test]
    fn superscript_is_smaller_and_raised_and_subscript_lowered() {
        let mut r = rich("x", 12.0);
        change(&mut r, 0..1, CharChange { script: Some(Script::Super), ..Default::default() });
        let up = &one_row(&r, RowAlign::Left).spans[0];
        assert!(close(up.size_pt, 12.0 * 0.58), "{}", up.size_pt);
        assert!(close(up.baseline_pt, 100.0 - 0.33 * 12.0), "{}", up.baseline_pt);
        change(&mut r, 0..1, CharChange { script: Some(Script::Sub), ..Default::default() });
        let down = &one_row(&r, RowAlign::Left).spans[0];
        assert!(close(down.baseline_pt, 100.0 + 0.14 * 12.0), "{}", down.baseline_pt);
    }

    #[test]
    fn letter_spacing_goes_after_every_glyph_but_the_rows_last() {
        let mut r = rich("abc", 10.0);
        change(&mut r, 0..0, CharChange { letter_spacing_pt: Some(2.0), ..Default::default() });
        assert_eq!(one_row(&r, RowAlign::Left).spans[0].advances_pt, vec![7.0, 7.0, 5.0]);
    }

    #[test]
    fn character_width_scales_the_advances() {
        let mut r = rich("ab", 10.0);
        change(&mut r, 0..0, CharChange { width_pct: Some(50.0), ..Default::default() });
        let span = &one_row(&r, RowAlign::Left).spans[0];
        assert_eq!(span.advances_pt, vec![2.5, 2.5]);
        assert!(close(span.scale_x, 0.5));
    }

    #[test]
    fn baseline_shift_moves_the_span_up() {
        let mut r = rich("ab", 10.0);
        change(&mut r, 0..0, CharChange { baseline_pt: Some(2.0), ..Default::default() });
        assert!(close(one_row(&r, RowAlign::Left).spans[0].baseline_pt, 98.0));
    }

    #[test]
    fn an_underline_is_one_rule_per_stretch_at_the_fonts_position() {
        let mut r = rich("abcd", 10.0);
        change(&mut r, 2..4, CharChange { size: Some(20.0), ..Default::default() });
        change(&mut r, 0..0, CharChange { underline: Some(true), ..Default::default() });
        let row = one_row(&r, RowAlign::Left);
        assert_eq!(row.rules.len(), 1, "{:?}", row.rules);
        let rule = &row.rules[0];
        assert!(close(rule.x0_pt, 10.0));
        assert!(close(rule.x1_pt, 10.0 + 10.0 + 20.0), "{}", rule.x1_pt);
        assert!(close(rule.y_pt, 100.0 + 0.1 * 20.0), "{}", rule.y_pt);
        assert!(close(rule.thickness_pt, 0.05 * 20.0));

        change(&mut r, 2..4, CharChange { color: Some([200, 0, 0, 255]), ..Default::default() });
        assert_eq!(one_row(&r, RowAlign::Left).rules.len(), 2, "two colours, two rules");
    }

    #[test]
    fn a_strikethrough_sits_at_the_fonts_strikeout_height() {
        let mut r = rich("ab", 10.0);
        change(&mut r, 0..0, CharChange { strike: Some(true), ..Default::default() });
        let row = one_row(&r, RowAlign::Left);
        assert_eq!(row.rules.len(), 1);
        assert!(close(row.rules[0].y_pt, 100.0 - 0.3 * 10.0), "{}", row.rules[0].y_pt);
    }

    #[test]
    fn a_row_without_decoration_has_no_rules() {
        assert!(one_row(&rich("ab", 10.0), RowAlign::Left).rules.is_empty());
    }

    #[test]
    fn trailing_spaces_are_not_measured_or_underlined() {
        let mut r = rich("ab  ", 10.0);
        change(&mut r, 0..0, CharChange { underline: Some(true), ..Default::default() });
        let row = one_row(&r, RowAlign::Right);
        assert!(close(row.spans[0].x_pt, 10.0 + 200.0 - 10.0), "{}", row.spans[0].x_pt);
        assert_eq!(row.spans[0].text, "ab");
        assert!(close(row.rules[0].x1_pt, 210.0), "{}", row.rules[0].x1_pt);
    }
}

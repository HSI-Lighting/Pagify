//! What kind of text, if any, a page actually has.
//!
//! The engine could not previously tell **"this page has no text"** from
//! **"this page has text I could not map to Unicode."** Both arrive as an empty
//! or short result, and they want opposite responses: the first wants OCR, the
//! second wants encoding repair, where OCR would be both wasteful and a quality
//! loss. Everything in the extraction plan dispatches off this distinction, so
//! it is drawn here first.
//!
//! ## Raw counts, not just a verdict
//!
//! The verdict is a threshold applied to measurements, and the thresholds are
//! guesses until a corpus says otherwise. So every count is returned alongside
//! it. A caller that disagrees can apply its own rule; a fixed threshold buried
//! in an engine is the kind of decision that is invisible until it is wrong.

use serde::{Deserialize, Serialize};

/// What a page's text layer is like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageTextKind {
    /// Characters present and mapping cleanly. Extraction works today.
    Native,
    /// Characters present, but too many of them have no Unicode mapping.
    /// Wants encoding repair, not recognition.
    Unmappable,
    /// No usable characters and an image covering most of the page.
    Scanned,
    /// Characters *and* a large image — a scan that already carries an OCR
    /// layer, or text over a photograph. Extraction works; the image is not a
    /// reason to re-recognise it.
    Hybrid,
    /// No characters, negligible image, but a crowd of small filled paths:
    /// type converted to outlines. Looks like text, selects like nothing.
    Outlined,
    /// Neither text nor image of consequence.
    Empty,
}

/// The measurements behind the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageClassification {
    pub kind: PageTextKind,
    /// Characters PDFium reports, including unmappable ones.
    pub chars: usize,
    /// How many of those have no Unicode mapping.
    pub unmappable: usize,
    /// Fraction of the page covered by image objects, clamped to 1.
    pub image_coverage: f32,
    /// Path objects on the page.
    pub paths: usize,
    /// Path objects small enough to be a glyph rather than a rule or a frame.
    pub glyph_paths: usize,
    /// Characters not on the horizontal — a stamp, a rotated table header.
    pub rotated_chars: usize,
}

impl PageClassification {
    /// Above this fraction of unmappable characters, the text layer is not
    /// usable as text. Deliberately not 0: a single unmapped bullet or ligature
    /// in a page of prose is normal and repairing it is not worth a mode change.
    pub const UNMAPPABLE_RATIO: f32 = 0.20;

    /// Below this many characters a page is treated as having no text layer.
    /// A stray page number or a colophon on a scan should not stop it being a
    /// scan.
    pub const MIN_USEFUL_CHARS: usize = 8;

    /// Image coverage above this makes a page a scan rather than an
    /// illustration on a text page.
    pub const SCAN_COVERAGE: f32 = 0.60;

    /// Enough glyph-sized paths to be words rather than decoration.
    ///
    /// Low, because it is paired with the ratio below. A single outlined
    /// headline is still outlined type — the page reads as text and selects as
    /// nothing — and a count high enough to exclude a logo on its own would
    /// exclude the headline too.
    pub const OUTLINE_PATHS: usize = 12;

    /// …and they must be most of what is on the page.
    ///
    /// This is what a raw count cannot say. Twelve small paths among four
    /// hundred is a chart's tick marks; twelve among fourteen is a word. The
    /// first version counted only, called a page of outlined type `Empty`
    /// because it held thirteen letters, and would equally have called a
    /// scatter plot outlined type.
    pub const OUTLINE_SHARE: f32 = 0.75;

    /// Segments above which a single path is a line of type rather than a
    /// shape.
    ///
    /// Measured on a real report: background rectangles 5 segments, rounded
    /// boxes 22–34, every line of outlined text between 290 and 5174. The
    /// threshold sits in the gap, nearer the shapes than the text.
    pub const TYPE_SEGMENTS: usize = 64;

    pub fn unmappable_ratio(&self) -> f32 {
        if self.chars == 0 {
            0.0
        } else {
            self.unmappable as f32 / self.chars as f32
        }
    }

    /// Decide the verdict from the counts. Separated from the measuring so it
    /// can be tested against invented numbers, without a PDF or a PDFium.
    pub fn verdict(&self) -> PageTextKind {
        let big_image = self.image_coverage >= Self::SCAN_COVERAGE;

        if self.chars >= Self::MIN_USEFUL_CHARS {
            // The *ratio* is judged before the survivor count, and the order
            // matters. A page of 500 characters where 480 fail to map still
            // leaves 20 that worked — enough to pass a "does it have text?"
            // count, and nowhere near enough to be a text layer. Checking the
            // survivors first calls that page Native and sends it to nobody.
            if self.unmappable_ratio() > Self::UNMAPPABLE_RATIO {
                return PageTextKind::Unmappable;
            }

            let usable = self.chars - self.unmappable;
            if usable >= Self::MIN_USEFUL_CHARS {
                // `Hybrid` means part of the page reads and part of it does
                // not. A scan behind a caption is the obvious case; **text
                // drawn as outlines beside real text is the same case**, and
                // was previously unreachable — the outline test below only ran
                // for pages with no usable text at all, so a page with one
                // readable line and forty outlined ones came back `Native` with
                // nothing to say about the forty.
                //
                // That is the worst answer available. `Outlined` sends a page
                // to recognition and `Native` at least reads what is there;
                // `Native` on a page that is two thirds outlines reads a third
                // of it and reports success.
                let outlined_text = self.glyph_paths >= Self::OUTLINE_PATHS;
                return if big_image || outlined_text {
                    PageTextKind::Hybrid
                } else {
                    PageTextKind::Native
                };
            }
        }

        if big_image {
            return PageTextKind::Scanned;
        }

        let glyph_share = if self.paths == 0 {
            0.0
        } else {
            self.glyph_paths as f32 / self.paths as f32
        };
        if self.glyph_paths >= Self::OUTLINE_PATHS && glyph_share >= Self::OUTLINE_SHARE {
            return PageTextKind::Outlined;
        }

        PageTextKind::Empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(chars: usize, unmappable: usize) -> PageClassification {
        PageClassification {
            kind: PageTextKind::Empty,
            chars,
            unmappable,
            image_coverage: 0.0,
            paths: 0,
            glyph_paths: 0,
            rotated_chars: 0,
        }
    }

    #[test]
    fn clean_text_is_native() {
        assert_eq!(counts(500, 0).verdict(), PageTextKind::Native);
        // A couple of unmapped ligatures in a page of prose is not a mode change.
        assert_eq!(counts(500, 3).verdict(), PageTextKind::Native);
    }

    /// The defect a real report exposed: a page whose prose is drawn as glyph
    /// outlines beside a little real text was called `Native`, and the reader
    /// got a third of the page with no indication the rest existed.
    #[test]
    fn text_beside_outlined_text_is_hybrid_not_native() {
        let mut page = counts(384, 1);
        page.paths = 50;
        page.glyph_paths = 29;
        assert_eq!(page.verdict(), PageTextKind::Hybrid);
    }

    /// The other side of it. Bullets, icons and a logo are small filled paths
    /// too, and calling every page that has a few of them `Hybrid` would make
    /// the warning worthless.
    #[test]
    fn a_native_page_with_a_few_ornaments_stays_native() {
        let mut page = counts(1200, 0);
        page.paths = 20;
        page.glyph_paths = PageClassification::OUTLINE_PATHS - 1;
        assert_eq!(page.verdict(), PageTextKind::Native);
    }

    /// `Hybrid` must not swallow the case it was built for.
    #[test]
    fn text_over_a_scan_is_still_hybrid() {
        let mut page = counts(400, 0);
        page.image_coverage = 0.9;
        assert_eq!(page.verdict(), PageTextKind::Hybrid);
    }

    /// A page that is *entirely* outlines still has no text to read, and
    /// belongs in recognition rather than in the hybrid warning.
    #[test]
    fn a_page_that_is_all_outlines_is_still_outlined() {
        let mut page = counts(0, 0);
        page.paths = 40;
        page.glyph_paths = 38;
        assert_eq!(page.verdict(), PageTextKind::Outlined);
    }

    #[test]
    fn mostly_unmappable_text_is_not_an_empty_page() {
        // The distinction the whole classifier exists for. This page wants
        // encoding repair; calling it Scanned would send it to OCR and lose
        // quality it did not need to lose.
        let page = counts(500, 480);
        assert_eq!(page.verdict(), PageTextKind::Unmappable);
        assert!(page.unmappable_ratio() > 0.9);
    }

    #[test]
    fn a_scan_is_a_scan_even_with_a_page_number_on_it() {
        let mut page = counts(3, 0);
        page.image_coverage = 0.95;
        assert_eq!(page.verdict(), PageTextKind::Scanned);
    }

    #[test]
    fn a_scan_that_already_has_an_ocr_layer_is_hybrid_not_scanned() {
        // Re-recognising this would be wasted work and would overwrite a text
        // layer that is very likely better than what we would produce.
        let mut page = counts(1200, 0);
        page.image_coverage = 0.98;
        assert_eq!(page.verdict(), PageTextKind::Hybrid);
    }

    #[test]
    fn outlined_type_is_told_apart_from_an_empty_page() {
        let mut page = counts(0, 0);
        page.paths = 300;
        page.glyph_paths = 280;
        assert_eq!(page.verdict(), PageTextKind::Outlined);

        // A page with a few rules and a border is empty, not outlined.
        let mut ruled = counts(0, 0);
        ruled.paths = 12;
        ruled.glyph_paths = 4;
        assert_eq!(ruled.verdict(), PageTextKind::Empty);
    }

    #[test]
    fn a_single_outlined_headline_is_still_outlined_type() {
        // It reads as text and selects as nothing, which is the whole
        // definition. A threshold high enough to exclude a logo on its own
        // would exclude this too.
        let mut headline = counts(0, 0);
        headline.paths = 14;
        headline.glyph_paths = 13;
        assert_eq!(headline.verdict(), PageTextKind::Outlined);
    }

    #[test]
    fn a_chart_full_of_small_marks_is_not_outlined_type() {
        // What the ratio is for. Tick marks and data points are small paths in
        // quantity, and a count alone cannot tell them from letters.
        let mut chart = counts(0, 0);
        chart.paths = 400;
        chart.glyph_paths = 60;
        assert_eq!(chart.verdict(), PageTextKind::Empty);
    }

    #[test]
    fn a_blank_page_is_empty() {
        assert_eq!(counts(0, 0).verdict(), PageTextKind::Empty);
        // And a page with a small decorative image is still empty, not a scan.
        let mut decorated = counts(0, 0);
        decorated.image_coverage = 0.05;
        assert_eq!(decorated.verdict(), PageTextKind::Empty);
    }

    #[test]
    fn the_ratio_does_not_divide_by_zero() {
        assert_eq!(counts(0, 0).unmappable_ratio(), 0.0);
    }
}

/// Whether a path is shaped like type rather than like a rule.
///
/// **Extracted so that redaction and classification cannot disagree.** Both have
/// to answer the same question — *are these letters?* — and two thresholds
/// tuned separately would eventually give a page that classifies as outlined
/// type and redacts as decoration, or the reverse.
///
/// Two shapes of the same thing, because producers disagree about how much text
/// goes into one path object:
///
/// - **One path per glyph.** Small in both directions and not a hairline: a
///   table's border is long and thin, a letter is small and roughly square.
/// - **One path per run.** A whole line of outlined text is as wide as the
///   column, so no width test can see it. What separates a line of letters from
///   a line is the segment count.
pub fn looks_like_type(
    width: f32,
    height: f32,
    segments: usize,
    page_width: f32,
    page_height: f32,
) -> bool {
    if width <= 0.5 || height <= 0.5 || height >= page_height * 0.10 {
        return false;
    }
    width < page_width * 0.10 || segments >= PageClassification::TYPE_SEGMENTS
}

#[cfg(test)]
mod type_shape_tests {
    use super::looks_like_type;

    const W: f32 = 595.0;
    const H: f32 = 842.0;

    #[test]
    fn a_letter_sized_path_looks_like_type() {
        assert!(looks_like_type(6.0, 9.0, 30, W, H));
    }

    /// A table rule: long, and thin enough to fail the height floor.
    #[test]
    fn a_hairline_rule_does_not() {
        assert!(!looks_like_type(400.0, 0.4, 4, W, H));
    }

    /// A full-page background panel fails on height.
    #[test]
    fn a_background_panel_does_not() {
        assert!(!looks_like_type(W, H, 5, W, H));
    }

    /// **The case the width test cannot see.** A whole line of outlined type is
    /// as wide as its column, so only the segment count gives it away.
    #[test]
    fn a_whole_line_of_outlined_type_is_caught_by_its_segments() {
        assert!(looks_like_type(400.0, 11.0, 900, W, H), "a line of letters");
        assert!(!looks_like_type(400.0, 11.0, 22, W, H), "a rounded box");
    }
}

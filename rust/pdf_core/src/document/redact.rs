//! Redaction — removing content so that it is *gone*, not hidden.
//!
//! The distinction this module exists to hold: a black rectangle drawn over a
//! phone number is a drawing of a redaction. The number is still in the content
//! stream, still selectable, still in every text extraction. Redaction here
//! means the text objects are deleted from the page and the file is rewritten
//! without them.
//!
//! Two rules follow from that and shape everything below.
//!
//! **A character the rectangle touches at all is removed.** Not "mostly
//! covered", not "centre inside" — any intersection. The alternative fails in
//! the one direction that matters: a half-covered character left behind is
//! invisible on screen and perfectly readable to anything that reads the text
//! layer, which is precisely the leak the feature exists to prevent. Erring the
//! other way costs a partly-obscured glyph.
//!
//! **Nothing happens until the whole page has been surveyed.** Removal is
//! planned first and applied second, so a rectangle that turns out to cover
//! something this code cannot clear leaves the page exactly as it was rather
//! than half-redacted.
//!
//! # What this pass cannot clear, measured
//!
//! An image crossing the rectangle cannot be cleared without re-encoding it, so
//! it refuses. That sounds like an edge case and is not — a mid-page rectangle
//! on every page of four real documents:
//!
//! ```text
//!                                  pages   clears   refused   because
//!   Apple licence, prose             117      117         0   —
//!   Handover certificate               2        2         0   —
//!   HSI catalogue 2026               149       33       116   image 101, image+outlined 12, outlined 3
//!   Wiring schematic, CAD              1        0         1   outlined type
//! ```
//!
//! Prose redacts cleanly and always did. Images block 113 of 149 catalogue
//! pages — but *why* splits into situations that refuse identically and want
//! different answers, so the same run was asked which:
//!
//! ```text
//!   text removed, image alongside   -> ask                    61   11,482 characters came out
//!   no text, image at the edge      -> ask                     6
//!   would only draw a mark          -> refused, re-encoding   34
//!   also blocked by outlined type   -> stays blocked          12
//! ```
//!
//! **67 of the 113 are a question, not a limit.** The text came out; what could
//! not be cleared is a photograph beside it, and whether that holds anything
//! worth hiding is for the person looking at the page. Hence
//! [`crate::document::DocumentMut::preview_redaction`], which reports instead of
//! refusing so they can be asked.
//!
//! The other 46 are not recoverable by asking, and two of the buckets say why
//! rather than being lumped together:
//!
//! - **34 would only draw a mark.** No text comes out because the words are the
//!   picture. Letting an acknowledgement through here produces a black rectangle
//!   over untouched words — see [`RedactionReport::would_only_draw_a_mark`],
//!   which refuses it and is not overridable.
//! - **12 are also blocked by outlined type**, which no answer about images
//!   fixes. This is the group vector glyph matching was expected to help, and
//!   it now does, in a specific and limited way — see below.
//!
//! Only the first group needs `FPDFImageObj_GetBitmap` / `SetBitmap`. Both exist,
//! so it is not large work; it is simply not in this pass.
//!
//! # What matching against a font actually unlocks
//!
//! [`crate::document::DocumentMut::redact`] takes an optional
//! [`crate::document::glyphs::Catalogue`]. Two distinct things follow from
//! giving it one, and only one of them needed matching at all:
//!
//! - **A path wholly inside the rectangle was always removable**, curves or
//!   not — nothing here ever needed to know a shape was a letter to remove it
//!   once it was entirely enclosed. What a `catalogue` unlocks for this case is
//!   only permission to *reach* it: a page classified wholesale
//!   [`crate::document::PageTextKind::Outlined`] refused outright before a
//!   `catalogue` existed, for every rectangle on it, whether or not that
//!   rectangle actually needed matching to clear.
//! - **A path only straddling the rectangle's edge** is where matching is the
//!   thing doing the work: confidently identified, the whole letter is removed;
//!   unidentified, it blocks exactly as before — there is no per-glyph
//!   splitting the way a native text run splits at a character boundary, so an
//!   unmatched shape is not guessed at.
//!
//! **Not re-measured against the real catalogue.** The 12-page figure above was
//! taken before this existed. What changes it now depends on how many of those
//! twelve had their outlined text wholly inside the rectangle (helped by the
//! first bullet alone, without needing a correct font) against how many needed
//! a straddling letter actually identified (helped only if the face given is
//! close enough to the one the document used) — a question the fixture this
//! module's own tests run against cannot answer, and the real file was not
//! available to re-run this measurement against when this landed. Recorded as
//! a gap rather than a number invented to fill it.
//!
//! Two over-blocking bugs were found by taking that measurement rather than
//! reasoning about it, and both were in this file: a form XObject was refused on
//! the strength of its bounding box, and then its children were refused on the
//! strength of being nested. Forms went from 299 reported to 154 to **0**.

use serde::{Deserialize, Serialize};

use super::{Color, Rect};

/// A request to clear a rectangle of a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Redaction {
    pub page_index: usize,
    /// Page points, top-left origin.
    pub area: Rect,
    /// Painted over the cleared area once it is empty. `None` leaves the page
    /// blank there, which is legitimate — a redaction that removes a line from a
    /// table often wants the space, not a mark.
    pub fill: Option<Color>,
    /// Refuse rather than half-clear.
    ///
    /// When the rectangle covers something this code cannot remove without
    /// destroying content outside it — an image crossing the boundary — the
    /// choice is to leave that content in the file or to refuse. Leaving it and
    /// reporting it is only safe if the caller reads the report. Defaults to
    /// refusing, because the failure mode of the other default is a document
    /// that everyone believes is redacted.
    pub require_complete: bool,
    /// The exact shapes asked for, when what was asked for is not a rectangle.
    ///
    /// **A text selection spanning two lines is not a rectangle.** It is the
    /// tail of one line and the head of the next, and the smallest rectangle
    /// holding both also holds the head of the first line and the tail of the
    /// last — the words on either side of what was actually picked. Sending
    /// only the union hides them too: measured on `two-column.pdf`, a
    /// 39-character selection took 68 characters and the word before it.
    ///
    /// So the caller may send the shapes themselves — one per line — and
    /// [`Redaction::area`] stays their union, which is what everything wanting
    /// a single rectangle still uses: the badge that undoes the lock, the
    /// survey, and the bounds a caller draws.
    ///
    /// Empty means the rectangle *is* the shape, which is what a dragged
    /// rectangle means and what every caller before this meant.
    #[serde(default)]
    pub parts: Vec<Rect>,
}

impl Redaction {
    /// A redaction that refuses to half-clear, with a black mark over the area.
    pub fn new(page_index: usize, area: Rect) -> Self {
        Redaction {
            page_index,
            area,
            fill: Some(Color { r: 0, g: 0, b: 0, a: 255 }),
            require_complete: true,
            parts: Vec::new(),
        }
    }

    /// The same request over an explicit set of shapes, with `area` their union.
    ///
    /// The union is computed here rather than trusted from the caller, because
    /// everything that reads `area` — the badge, the survey — depends on it
    /// actually containing the parts.
    pub fn over(page_index: usize, parts: Vec<Rect>) -> Option<Self> {
        let first = *parts.first()?;
        let area = parts.iter().fold(first, |acc, r| Rect {
            left: acc.left.min(r.left),
            top: acc.top.min(r.top),
            right: acc.right.max(r.right),
            bottom: acc.bottom.max(r.bottom),
        });
        Some(Redaction { parts, ..Redaction::new(page_index, area) })
    }

    /// The shapes to clear: the parts when there are any, the area when not.
    ///
    /// Every test of "is this inside what was asked for" goes through here, so
    /// that a request carrying parts and one carrying a bare rectangle take the
    /// same path and only the shape differs.
    pub fn shapes(&self) -> &[Rect] {
        if self.parts.is_empty() {
            std::slice::from_ref(&self.area)
        } else {
            &self.parts
        }
    }

    /// Whether any shape overlaps this rectangle.
    pub fn touches(&self, r: &Rect) -> bool {
        self.shapes().iter().any(|s| {
            r.left < s.right && r.right > s.left && r.top < s.bottom && r.bottom > s.top
        })
    }

    /// Whether any shape holds this point.
    pub fn holds(&self, x: f32, y: f32) -> bool {
        self.shapes()
            .iter()
            .any(|s| x >= s.left && x <= s.right && y >= s.top && y <= s.bottom)
    }

    /// Whether this rectangle spills outside every shape, within a tolerance.
    pub fn spills(&self, r: &Rect, slack: f32) -> bool {
        !self.shapes().iter().any(|s| {
            r.left >= s.left - slack
                && r.right <= s.right + slack
                && r.top >= s.top - slack
                && r.bottom <= s.bottom + slack
        })
    }
}

/// Content a rectangle covers that survived it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Uncleared {
    /// An image the rectangle crosses rather than contains. Its pixels under the
    /// rectangle are still in the file; removing the whole object would destroy
    /// the part outside.
    Image {
        object: usize,
        /// How much of the **redaction rectangle** this image lies under, from 0
        /// to 1.
        ///
        /// The difference between two situations that otherwise refuse
        /// identically. An image under nearly all of the rectangle, on a page
        /// where no text came out of it, means the words *are* the picture and
        /// only re-encoding will do. An image under part of it, on a page where
        /// text was removed cleanly, is a background or a photograph beside the
        /// text — the sensitive content did come out, and what remains is a
        /// question only the person looking at it can answer.
        ///
        /// Carried so a caller can tell those apart and ask, rather than refuse
        /// both the same way.
        covers: f32,
        /// The image may itself contain words.
        ///
        /// **The case an acknowledgement must not read as routine.** A page
        /// carrying real text *and* an image over most of it is a scanned figure
        /// beside a native caption. The caption's text comes out cleanly, so the
        /// discriminator that separates "ask" from "re-encode" says ask — and
        /// the figure keeps its own words, which is the one thing the person
        /// answering needs told.
        ///
        /// Read from the page classifier rather than guessed at: it is
        /// [`crate::document::PageTextKind::Hybrid`] reached through image
        /// coverage, which is exactly *text and a scan on one page*.
        may_hold_text: bool,
    },
    /// A path — a rule, a border, a filled panel — in the same position.
    /// Decorative, so it is reported and does not refuse.
    Path { object: usize },
    /// A path the rectangle covers that is shaped like **letters**: type
    /// converted to curves. Reported and refuses, because removing nothing
    /// while drawing a black mark is the one outcome redaction must never have.
    OutlinedText { object: usize },
    /// Nested content. A form XObject is a page in miniature and this code does
    /// not descend into one.
    Form { object: usize },
    /// An annotation crossing the boundary. Its `/Contents` may quote what was
    /// removed.
    Annotation { index: usize },
}

impl Uncleared {
    /// Whether this is bad enough to refuse the redaction over.
    ///
    /// **Images and nested content block; a path does not.** The line is drawn
    /// where it is because of what survives and what it costs to be strict:
    ///
    /// An image crossing the area is the scanned-page case, where the words
    /// *are* the picture — leaving it means the redaction removed nothing at
    /// all. Nested content may hold text this pass could not reach to remove.
    /// Both are genuine, and both are rare enough that refusing does not make
    /// the feature unusable.
    ///
    /// A path is different in degree, not in kind. It carries no text, so
    /// nothing is extractable from it — but every ruled table, underline and
    /// background panel is a path crossing any rectangle drawn on it, so
    /// blocking on one would refuse nearly every real redaction. It is reported
    /// and not enforced.
    ///
    /// **Outlined type is separated from decoration rather than lumped with
    /// it**, and that distinction is the whole reason this returns something
    /// other than `true`. Type converted to curves is a path by every
    /// structural test, and a page of it is common — one catalogue here has it
    /// on 92 of 95 pages. Treating those as decoration would mean a user drags
    /// a box over a name, sees a black rectangle, and ships a file where the
    /// name is still present as Bézier curves. So [`Uncleared::OutlinedText`]
    /// blocks and [`Uncleared::Path`] does not, and the two are told apart by
    /// [`crate::document::classify::looks_like_type`] — the same rule the page
    /// classifier uses, so they cannot disagree.
    pub fn blocks(&self) -> bool {
        !matches!(self, Uncleared::Path { .. })
    }

    /// How to say it to somebody deciding whether to publish the file.
    pub fn describe(&self) -> String {
        match self {
            Uncleared::Image { object, covers, may_hold_text: true } => format!(
                "object {object} lies under {:.0}% of the area and is part of a scanned \
                 image — it may contain words of its own, which cannot be removed",
                covers * 100.0
            ),
            Uncleared::Image { object, covers, may_hold_text: false } => format!(
                "an image (object {object}) lies under {:.0}% of the area and crosses its edge",
                covers * 100.0
            ),
            Uncleared::Path { object } => {
                format!("a drawing (object {object}) crosses the edge of the area")
            }
            Uncleared::OutlinedText { object } => format!(
                "object {object} is text drawn as curves, which this pass cannot remove"
            ),
            Uncleared::Form { object } => {
                format!("nested content (object {object}) overlaps the area")
            }
            Uncleared::Annotation { index } => {
                format!("annotation {index} crosses the edge of the area")
            }
        }
    }
}

/// What a redaction did.
///
/// Returned rather than logged because the only honest UI for redaction shows
/// the user what was destroyed before they publish.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionReport {
    /// Characters removed.
    pub characters: usize,
    /// Whole page objects deleted.
    pub objects: usize,
    /// Text destroyed that the rectangle did **not** cover, because its run
    /// could not be split — see [`Fate::Whole`].
    pub spilled: Vec<String>,
    /// What the rectangle covers and this pass left in place.
    pub uncleared: Vec<Uncleared>,
}

impl RedactionReport {
    /// An image under this much of the area, with no text coming out of it,
    /// means the visible content there **is** the image.
    ///
    /// Not a tuned number: below it, most of the rectangle is something other
    /// than the image, so a redaction that removed nothing removed nothing
    /// because there was nothing — which is an ordinary, honest outcome.
    pub const MOSTLY_IMAGE: f32 = 0.50;

    /// Whether going ahead would paint a mark and remove nothing.
    ///
    /// **The failure this whole module exists to prevent, arrived at from the
    /// other side.** A rectangle over a scanned figure takes no characters out —
    /// they are not characters — and an acknowledgement would let it through:
    /// the user confirms, a black rectangle appears, and the words underneath
    /// are untouched. That is a drawing of a redaction.
    ///
    /// Distinct from a rectangle over blank space, which also removes nothing
    /// and is perfectly honest. What separates them is whether something that
    /// can hold words is lying under the area: an image the area is mostly
    /// made of, **nested content this pass does not descend into, or type
    /// drawn as curves**. Found by audit: the last two were left out, so a
    /// card number drawn through a form XObject was "redacted" — a black mark
    /// painted, the number still extractable from the saved file.
    pub fn would_only_draw_a_mark(&self) -> bool {
        self.characters == 0
            && self.objects == 0
            && self.uncleared.iter().any(|u| match u {
                Uncleared::Image { covers, .. } => *covers >= Self::MOSTLY_IMAGE,
                Uncleared::Form { .. } | Uncleared::OutlinedText { .. } => true,
                Uncleared::Path { .. } | Uncleared::Annotation { .. } => false,
            })
    }

    /// Why going ahead would only draw a mark — what is under the area that
    /// this pass cannot take words out of. Empty when it would not.
    pub fn what_survives_a_mark(&self) -> Vec<String> {
        if !self.would_only_draw_a_mark() {
            return Vec::new();
        }
        let mut how: Vec<String> = self
            .uncleared
            .iter()
            .filter_map(|u| match u {
                Uncleared::Image { covers, .. } if *covers >= Self::MOSTLY_IMAGE => {
                    Some("the words are part of an image".to_string())
                }
                Uncleared::Form { object } => Some(format!(
                    "the words are inside nested content (object {object}) this pass does not reach"
                )),
                Uncleared::OutlinedText { object } => {
                    Some(format!("the words are drawn as curves (object {object})"))
                }
                _ => None,
            })
            .collect();
        how.sort();
        how.dedup();
        how
    }

    /// Whether the area is actually clear.
    pub fn is_complete(&self) -> bool {
        self.uncleared.is_empty()
    }

    /// What, if anything, would make a strict redaction refuse.
    pub fn blockers(&self) -> Vec<&Uncleared> {
        self.uncleared.iter().filter(|u| u.blocks()).collect()
    }

    /// Whether accepting the image objection is enough to let this go ahead.
    ///
    /// **The difference between what a confirmation dialog can buy and what it
    /// cannot.** An acknowledgement answers for images: the text came out, and
    /// whether the picture beside it matters is the user's call. It answers for
    /// nothing else. A page also blocked by outlined type stays blocked whatever
    /// the user says, because there the words themselves are the thing that did
    /// not come out.
    ///
    /// Measured on the 2026 catalogue: 116 pages refuse a mid-page rectangle,
    /// and 12 of them are blocked by an image *and* by outlined type. Counting
    /// those as recoverable would overstate what the dialog buys.
    pub fn only_images_are_in_the_way(&self) -> bool {
        let blockers = self.blockers();
        !blockers.is_empty() && blockers.iter().all(|u| matches!(u, Uncleared::Image { .. }))
    }
}

// ---------------------------------------------------------------- geometry --

/// Whether two rectangles share any area, in page points, top-left origin.
///
/// Touching edges do not count. A character whose right edge lands exactly on
/// the rectangle's left edge occupies no covered area, and treating that as a
/// hit would eat the neighbouring word on every redaction drawn to a text
/// boundary.
pub(crate) fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// How much of `area` lies under `other`, from 0 to 1.
pub(crate) fn covered_share(area: &Rect, other: &Rect) -> f32 {
    let width = (area.right - area.left).abs();
    let height = (area.bottom - area.top).abs();
    if width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    let overlap_w = (area.right.min(other.right) - area.left.max(other.left)).max(0.0);
    let overlap_h = (area.bottom.min(other.bottom) - area.top.max(other.top)).max(0.0);
    (overlap_w * overlap_h / (width * height)).clamp(0.0, 1.0)
}

/// Whether `inner` lies entirely within `outer`. Coincident edges count.
pub(crate) fn contains(outer: &Rect, inner: &Rect) -> bool {
    outer.left <= inner.left
        && outer.right >= inner.right
        && outer.top <= inner.top
        && outer.bottom >= inner.bottom
}

// ------------------------------------------------------------------ splitting --

/// One character of a run, judged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Marked {
    pub covered: bool,
    /// The left edge of this character, PDF space. Where a segment starting here
    /// has to be drawn.
    pub left: f32,
}

/// A stretch of a run that lives.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    /// Character offsets into the run's text — `char` positions, not bytes.
    pub start: usize,
    pub end: usize,
    /// Where to put it, PDF space. The first surviving character's own left
    /// edge, so the text that stays does not slide into the gap the redaction
    /// made.
    pub left: f32,
}

/// What has to happen to one text object.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Fate {
    /// The rectangle misses it.
    Untouched,
    /// Every character is covered, so the object goes.
    Whole,
    /// Some characters survive. The object is replaced by these.
    Split(Vec<Segment>),
}

/// Decide a run's fate from its judged characters.
pub(crate) fn fate(marks: &[Marked]) -> Fate {
    if marks.is_empty() || !marks.iter().any(|m| m.covered) {
        return Fate::Untouched;
    }
    if marks.iter().all(|m| m.covered) {
        return Fate::Whole;
    }

    let mut segments = Vec::new();
    let mut open: Option<Segment> = None;
    for (index, mark) in marks.iter().enumerate() {
        match (mark.covered, &mut open) {
            (false, None) => {
                open = Some(Segment { start: index, end: index + 1, left: mark.left })
            }
            (false, Some(segment)) => segment.end = index + 1,
            (true, slot @ Some(_)) => segments.push(slot.take().expect("just matched")),
            (true, None) => {}
        }
    }
    segments.extend(open);
    Fate::Split(segments)
}

/// Take a `char` range out of a string.
///
/// By `char` and not by byte, because the offsets come from PDFium's character
/// walk. Slicing `"café"[0..4]` by bytes cuts the `é` in half and panics; the
/// same expressed in characters is the whole word.
pub(crate) fn slice(text: &str, segment: &Segment) -> String {
    text.chars().take(segment.end).skip(segment.start).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect { left, top, right, bottom }
    }

    fn marks(pattern: &str) -> Vec<Marked> {
        pattern
            .chars()
            .enumerate()
            .map(|(i, c)| Marked { covered: c == '#', left: i as f32 * 10.0 })
            .collect()
    }

    // -- geometry ----------------------------------------------------------

    #[test]
    fn overlapping_rectangles_overlap() {
        assert!(overlaps(&rect(0.0, 0.0, 10.0, 10.0), &rect(5.0, 5.0, 15.0, 15.0)));
    }

    #[test]
    fn separate_rectangles_do_not() {
        assert!(!overlaps(&rect(0.0, 0.0, 10.0, 10.0), &rect(20.0, 0.0, 30.0, 10.0)));
    }

    /// **Why touching is not overlapping.** A redaction dragged to the exact
    /// left edge of a word shares an edge with the character before it and
    /// covers none of it. Counting that as a hit eats a character per boundary,
    /// on every redaction anyone draws carefully.
    #[test]
    fn touching_edges_do_not_overlap() {
        assert!(!overlaps(&rect(0.0, 0.0, 10.0, 10.0), &rect(10.0, 0.0, 20.0, 10.0)));
        assert!(!overlaps(&rect(0.0, 0.0, 10.0, 10.0), &rect(0.0, 10.0, 10.0, 20.0)));
    }

    /// Top-left origin: `top` is the smaller number. A test written for a
    /// bottom-left space passes on the x axis and silently fails on y, so the
    /// vertical case is asserted on its own.
    #[test]
    fn vertical_separation_is_read_in_a_top_left_space() {
        let above = rect(0.0, 0.0, 10.0, 5.0);
        let below = rect(0.0, 6.0, 10.0, 11.0);
        assert!(!overlaps(&above, &below));
        assert!(overlaps(&above, &rect(0.0, 4.0, 10.0, 11.0)));
    }

    #[test]
    fn containment_is_not_mere_overlap() {
        let outer = rect(0.0, 0.0, 100.0, 100.0);
        assert!(contains(&outer, &rect(10.0, 10.0, 20.0, 20.0)));
        assert!(!contains(&outer, &rect(90.0, 90.0, 110.0, 110.0)));
        assert!(overlaps(&outer, &rect(90.0, 90.0, 110.0, 110.0)));
    }

    /// An object exactly the size of the rectangle is contained by it, or a
    /// redaction drawn precisely around a word would refuse to remove it.
    #[test]
    fn a_coincident_rectangle_counts_as_contained() {
        let r = rect(10.0, 10.0, 20.0, 20.0);
        assert!(contains(&r, &r));
    }

    // -- splitting ---------------------------------------------------------

    #[test]
    fn a_run_the_rectangle_misses_is_untouched() {
        assert_eq!(fate(&marks(".....")), Fate::Untouched);
    }

    #[test]
    fn a_fully_covered_run_goes_whole() {
        assert_eq!(fate(&marks("#####")), Fate::Whole);
    }

    #[test]
    fn an_empty_run_is_untouched() {
        assert_eq!(fate(&[]), Fate::Untouched);
    }

    #[test]
    fn a_covered_head_leaves_the_tail() {
        let Fate::Split(segments) = fate(&marks("###..")) else { panic!("expected a split") };
        assert_eq!(segments, vec![Segment { start: 3, end: 5, left: 30.0 }]);
    }

    #[test]
    fn a_covered_tail_leaves_the_head() {
        let Fate::Split(segments) = fate(&marks("..###")) else { panic!("expected a split") };
        assert_eq!(segments, vec![Segment { start: 0, end: 2, left: 0.0 }]);
    }

    /// **The case the whole design is for.** A line reading
    /// `Contact john@acme.com for pricing` with only the address covered has to
    /// keep both halves, each where it was.
    #[test]
    fn a_covered_middle_leaves_both_ends() {
        let Fate::Split(segments) = fate(&marks("..###..")) else { panic!("expected a split") };
        assert_eq!(
            segments,
            vec![
                Segment { start: 0, end: 2, left: 0.0 },
                Segment { start: 5, end: 7, left: 50.0 },
            ]
        );
    }

    #[test]
    fn two_covered_stretches_leave_three_pieces() {
        let Fate::Split(segments) = fate(&marks(".#.#.")) else { panic!("expected a split") };
        assert_eq!(segments.len(), 3);
        assert_eq!(segments.iter().map(|s| s.start).collect::<Vec<_>>(), vec![0, 2, 4]);
    }

    /// **The reason a segment carries its own left edge.** Text that survives
    /// must stay where it was; rebuilt from the run's origin it would slide
    /// left into the space the redaction cleared, and the page would show a
    /// sentence that reads as though nothing had been taken out of it.
    #[test]
    fn a_surviving_tail_keeps_its_own_position() {
        let Fate::Split(segments) = fate(&marks("###..")) else { panic!("expected a split") };
        assert_eq!(segments[0].left, 30.0, "the tail must not slide into the gap");
    }

    // -- slicing -----------------------------------------------------------

    #[test]
    fn a_segment_slices_the_text_it_indexes() {
        assert_eq!(slice("abcdefg", &Segment { start: 2, end: 5, left: 0.0 }), "cde");
    }

    /// **Characters, not bytes.** The offsets come from a character walk, and
    /// `"café now"[0..5]` by bytes lands inside the `é`.
    #[test]
    fn slicing_counts_characters_not_bytes() {
        assert_eq!(slice("café now", &Segment { start: 0, end: 4, left: 0.0 }), "café");
        assert_eq!(slice("café now", &Segment { start: 5, end: 8, left: 0.0 }), "now");
    }

    #[test]
    fn slicing_past_the_end_stops_at_the_end() {
        assert_eq!(slice("ab", &Segment { start: 0, end: 99, left: 0.0 }), "ab");
    }

    // -- the report --------------------------------------------------------

    #[test]
    fn a_share_is_measured_against_the_rectangle_not_the_thing_covering_it() {
        let area = rect(0.0, 0.0, 10.0, 10.0);
        // A huge image over the whole rectangle covers all of it, however big it
        // is — the question is what the *redaction* fails to clear, not how much
        // of the image is involved.
        assert_eq!(covered_share(&area, &rect(-500.0, -500.0, 500.0, 500.0)), 1.0);
        assert_eq!(covered_share(&area, &rect(0.0, 0.0, 5.0, 10.0)), 0.5);
        assert_eq!(covered_share(&area, &rect(50.0, 50.0, 60.0, 60.0)), 0.0);
    }

    /// An acknowledgement answers for images and for nothing else.
    #[test]
    fn a_page_also_blocked_by_outlined_type_is_not_recovered_by_acknowledging_images() {
        let image = Uncleared::Image { object: 1, covers: 0.4, may_hold_text: false };

        let only_images = RedactionReport { uncleared: vec![image.clone()], ..Default::default() };
        assert!(only_images.only_images_are_in_the_way());

        let also_outlined = RedactionReport {
            uncleared: vec![image.clone(), Uncleared::OutlinedText { object: 2 }],
            ..Default::default()
        };
        assert!(
            !also_outlined.only_images_are_in_the_way(),
            "the dialog would have claimed a page it cannot unblock"
        );

        // A decorative path never blocked, so it does not count against this.
        let with_decoration = RedactionReport {
            uncleared: vec![image, Uncleared::Path { object: 3 }],
            ..Default::default()
        };
        assert!(with_decoration.only_images_are_in_the_way());

        // And a page with nothing in the way is not "recoverable" — it is done.
        assert!(!RedactionReport::default().only_images_are_in_the_way());
    }

    /// The scanned-figure warning has to read differently from the product-photo
    /// one, or the case that matters is delivered in the words used for the case
    /// that does not.
    #[test]
    fn an_image_that_may_hold_words_says_so() {
        let scanned = Uncleared::Image { object: 1, covers: 0.8, may_hold_text: true };
        let photo = Uncleared::Image { object: 1, covers: 0.8, may_hold_text: false };
        assert!(scanned.describe().contains("may contain words"));
        assert!(!photo.describe().contains("may contain words"));
        assert!(scanned.blocks() && photo.blocks());
    }

    /// **Acknowledging must not be able to buy a mark over untouched words.**
    #[test]
    fn a_redaction_that_would_remove_nothing_from_an_image_is_named_as_such() {
        let over_a_scan = RedactionReport {
            characters: 0,
            objects: 0,
            uncleared: vec![Uncleared::Image { object: 1, covers: 0.95, may_hold_text: true }],
            ..Default::default()
        };
        assert!(over_a_scan.would_only_draw_a_mark());

        // Text came out, so the mark marks something that was really removed.
        let caption_removed = RedactionReport {
            characters: 12,
            uncleared: vec![Uncleared::Image { object: 1, covers: 0.95, may_hold_text: true }],
            ..Default::default()
        };
        assert!(!caption_removed.would_only_draw_a_mark());

        // Blank space: also removes nothing, and is an honest outcome.
        assert!(!RedactionReport::default().would_only_draw_a_mark());

        // An image barely clipping the edge is not what the area is made of.
        let edge = RedactionReport {
            uncleared: vec![Uncleared::Image { object: 1, covers: 0.05, may_hold_text: false }],
            ..Default::default()
        };
        assert!(!edge.would_only_draw_a_mark());
    }

    #[test]
    fn a_report_with_nothing_left_behind_is_complete() {
        assert!(RedactionReport::default().is_complete());
    }

    /// A ruled table crosses every rectangle drawn on it. Blocking there would
    /// refuse nearly every real redaction, so a path reports without enforcing.
    #[test]
    fn a_path_reports_without_refusing_and_an_image_refuses() {
        assert!(!Uncleared::Path { object: 1 }.blocks());
        assert!(Uncleared::OutlinedText { object: 1 }.blocks());
        assert!(Uncleared::Image { object: 1, covers: 1.0, may_hold_text: false }.blocks());
        assert!(Uncleared::Form { object: 1 }.blocks());
        assert!(Uncleared::Annotation { index: 1 }.blocks());

        let report = RedactionReport {
            uncleared: vec![
                Uncleared::Path { object: 1 },
                Uncleared::Image { object: 2, covers: 0.25, may_hold_text: false },
            ],
            ..Default::default()
        };
        assert!(!report.is_complete(), "both are still reported");
        assert_eq!(report.blockers(), vec![&Uncleared::Image { object: 2, covers: 0.25, may_hold_text: false }]);
    }

    #[test]
    fn anything_left_behind_makes_it_incomplete() {
        let report = RedactionReport {
            uncleared: vec![Uncleared::Image { object: 3, covers: 0.5, may_hold_text: false }],
            ..Default::default()
        };
        assert!(!report.is_complete());
        assert!(report.uncleared[0].describe().contains("image"));
    }

    /// The safe default is the one you get without asking.
    #[test]
    fn a_redaction_refuses_to_half_clear_unless_told_otherwise() {
        let r = Redaction::new(0, rect(0.0, 0.0, 10.0, 10.0));
        assert!(r.require_complete);
        assert_eq!(r.fill, Some(Color { r: 0, g: 0, b: 0, a: 255 }));
    }
}

//! The document abstraction.
//!
//! Everything above this layer — the JNI bridge, the cache, the command stack —
//! talks to `dyn Document`. Mutation arrives as a separate trait implemented
//! against **PDFium, the single writer**, and deliberately behind no feature flag:
//! a flag around editing guarantees the default build never type-checks the
//! editing path, which is exactly how the old `editing` feature came to declare a
//! module nobody had written.

pub mod metadata;
pub mod pdfium_doc;

pub use metadata::DocumentMetadata;

use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::error::{PdfError, Result};
use crate::render::{Bitmap, RenderTarget};

/// A page's text with a box for every character of it.
///
/// What selection needs and runs cannot give. A run is a whole line, so a
/// selection built from runs can only start and end at a line — dragging across
/// half a sentence would copy all of both lines it touched. Characters make the
/// selection say what the user pointed at.
///
/// `text` and `boxes` are built from a single walk of the page and are aligned by
/// construction: four floats per character, in `text`'s own order. That alignment
/// is the whole contract, which is why the two arrive together rather than from
/// two calls that could be made against different states.
///
/// The alignment is in **UTF-16 code units**, not code points, because the
/// consumer is Kotlin and a Kotlin string is indexed that way. A character
/// outside the basic plane contributes its box twice, so `text.length` and
/// `boxes.len() / 4` agree on both sides of the boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageCharacters {
    pub text: String,
    /// Left, top, right, bottom — four per code unit, top-left origin.
    pub boxes: Vec<f32>,
}

/// A run of text on a page, with where it sits.
///
/// Coordinates are in points with the origin at the page's **top-left** and y
/// increasing downwards — deliberately not PDF's own bottom-left convention.
/// Every consumer of this is a UI that hit-tests a touch against these rects, and
/// flipping the axis once here is far safer than expecting each caller to
/// remember to do it.
///
/// A run is at most one line, and never spans two of them. Runs arrive from
/// [`Page::text_segments`] in the document's **character order**, which is the
/// order the text is read in; see that method for why callers must not throw
/// that ordering away.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSegment {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub text: String,
}

impl TextSegment {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }
}

/// Page dimensions in PostScript points (1/72"), as authored in the PDF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width_pt: f32,
    pub height_pt: f32,
}

impl PageSize {
    /// Pixel dimensions this page occupies at the given scale, clamped to at
    /// least 1x1 so a degenerate page can never produce a zero-sized bitmap.
    pub fn pixel_size(&self, scale: f32) -> (u32, u32) {
        let w = (self.width_pt * scale).round().max(1.0) as u32;
        let h = (self.height_pt * scale).round().max(1.0) as u32;
        (w, h)
    }
}

/// Quarter-turn page rotation applied at render time (not persisted to the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    None,
    Clockwise90,
    Clockwise180,
    Clockwise270,
}

impl Rotation {
    /// Rotation swaps the axes for odd quarter-turns, so callers sizing a target
    /// bitmap must ask through here rather than using the raw page size.
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::Clockwise90 | Rotation::Clockwise270)
    }

    pub fn from_quarter_turns(turns: i32) -> Self {
        match turns.rem_euclid(4) {
            1 => Rotation::Clockwise90,
            2 => Rotation::Clockwise180,
            3 => Rotation::Clockwise270,
            _ => Rotation::None,
        }
    }
}

/// What to draw, and how big. Kept separate from `RenderTarget` (where to draw)
/// so a request can be hashed into a cache key without involving the destination.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderRequest {
    /// Points-to-pixels multiplier. 1.0 renders at 72 dpi.
    pub scale: f32,
    pub rotation: Rotation,
    /// Draw annotation appearances (highlights, form field borders, stamps).
    pub render_annotations: bool,
    /// Draw interactive form field contents on top of the page.
    pub render_form_data: bool,
}

impl Default for RenderRequest {
    fn default() -> Self {
        RenderRequest {
            scale: 1.0,
            rotation: Rotation::None,
            render_annotations: true,
            render_form_data: true,
        }
    }
}

/// What to capture out of a page, and how sharply.
///
/// Separate from [`RenderRequest`] because the two are sized by opposite ends.
/// A screen render is sized by its destination — Kotlin measures the view and the
/// bitmap's dimensions decide the pixels. An export is sized by the crop and an
/// explicit scale, so the output resolution stays independent of the display.
///
/// `crop` is in page points with a top-left origin and y increasing downwards
/// (decision 4.4), the same space as [`Annotation`] geometry, and it is resolved
/// against the page rather than trusted: see [`crate::render::RegionPixels`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionRequest {
    pub crop: Rect,
    /// Points-to-pixels multiplier for the export. Lowered if it would breach the
    /// render ceiling; never raised.
    pub scale: f32,
    pub render_annotations: bool,
    pub render_form_data: bool,
}

impl Default for RegionRequest {
    fn default() -> Self {
        RegionRequest {
            crop: Rect {
                left: 0.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            },
            scale: 2.0,
            render_annotations: true,
            render_form_data: true,
        }
    }
}

/// A single page of an open document.
///
/// The `'_` lifetime on `Document::page` is a deviation from the original sketch:
/// PDFium page handles borrow their document, so a `Box<dyn Page>` detached from
/// the document's lifetime could not be made sound.
pub trait Page {
    fn size(&self) -> PageSize;

    /// Rasterise into a caller-owned buffer. The buffer is supplied rather than
    /// returned so the hot path can draw straight into a locked Android Bitmap
    /// with no intermediate allocation or copy.
    fn render_into(&self, request: &RenderRequest, target: &mut RenderTarget<'_>) -> Result<()>;

    /// Rasterise one region of the page, at a scale of the caller's choosing.
    ///
    /// Returns an owned bitmap rather than filling a supplied one because the
    /// caller cannot know the size in advance: the render ceiling may have
    /// lowered the scale, and the resolved [`crate::render::RegionPixels`] is what
    /// decides the dimensions.
    ///
    /// Nothing outside `crop` appears in the result. That is what makes this an
    /// export rather than a screenshot — see decision 4.8.
    fn render_region(&self, _request: &RegionRequest) -> Result<Bitmap> {
        Err(PdfError::Unsupported("region rendering"))
    }

    /// Extracted text in reading order, as PDFium's text page reports it.
    fn text(&self) -> Result<String>;

    /// Text runs with their positions, for selection and highlighting.
    ///
    /// Separate from [`Page::text`] because the two have very different costs and
    /// callers: the flat string is for search and copy, while this walks every
    /// run on the page and is only wanted when the user is actually selecting.
    ///
    /// **The order is part of the contract.** Runs come back in the document's
    /// character order — the order the page is read in — not sorted by position.
    /// A selection is the interval between two runs in that order, which is the
    /// only thing that distinguishes one column of a page from the column beside
    /// it; the two share a y band and cannot be told apart geometrically. A
    /// caller that sorts, filters into a map, or otherwise discards the ordering
    /// has thrown away the information selection depends on.
    fn text_segments(&self) -> Result<Vec<TextSegment>> {
        Ok(Vec::new())
    }

    /// The page's text with a box for every character.
    ///
    /// Separate from [`Page::text_segments`] because it costs more and is wanted
    /// less often: runs are enough to draw a highlight over a line, and only a
    /// selection — which has to start and end where a finger points, mid-line —
    /// needs to know where each character sits.
    fn characters(&self) -> Result<PageCharacters> {
        Ok(PageCharacters {
            text: String::new(),
            boxes: Vec::new(),
        })
    }
}

pub trait Document: Send + Sync {
    fn page_count(&self) -> usize;

    fn metadata(&self) -> Result<DocumentMetadata>;

    fn page(&self, index: usize) -> Result<Box<dyn Page + '_>>;

    /**
     * Page dimensions without loading the page.
     *
     * Split out from [`Page::size`] because loading a page parses its resources
     * and content-stream references, which on a large document is orders of
     * magnitude more expensive than reading two numbers out of the page tree.
     * Sizing happens constantly — measuring placeholders, choosing a render
     * scale, prefetching — so it must not drag a page load along with it.
     */
    fn page_size(&self, index: usize) -> Result<PageSize> {
        self.validate_page_index(index)?;
        Ok(self.page(index)?.size())
    }

    /// Marks already on a page, each with PDFium's index for it.
    ///
    /// Empty by default: a document that cannot report annotations is not an
    /// error, it simply has none to show. Types this engine does not model are
    /// skipped rather than guessed at — see [`IndexedAnnotation`] for why that
    /// makes the index, not the list position, the thing to address them by.
    fn annotations(&self, _page_index: usize) -> Result<Vec<IndexedAnnotation>> {
        Ok(Vec::new())
    }

    /// How many annotations a page carries, of any type.
    ///
    /// Separate from [`Document::annotations`] because it answers a different
    /// question and is far cheaper: it counts what is actually on the page,
    /// including the widgets and links this engine does not model. That makes it
    /// the honest check that a save really wrote something — reading back through
    /// our own model could only ever confirm that we can parse what we wrote.
    fn annotation_count(&self, _page_index: usize) -> Result<usize> {
        Ok(0)
    }

    /// `Some` only for implementations that can mutate and save the file.
    ///
    /// Every mutation reaches a document through here and then through a
    /// [`Command`](crate::command::Command); there is no other way in, which is
    /// what keeps batch processing and scripting cheap to add later.
    ///
    /// Not named `as_mut`: on a `Box<dyn Document>` that resolves to `Box`'s own
    /// inherent method instead, silently handing back the document rather than
    /// its mutation interface.
    fn as_document_mut(&mut self) -> Option<&mut dyn DocumentMut> {
        None
    }

    /// Bounds check shared by every implementation.
    fn validate_page_index(&self, index: usize) -> Result<()> {
        let count = self.page_count();
        if index >= count {
            return Err(crate::error::PdfError::PageOutOfRange { index, count });
        }
        Ok(())
    }
}

/// A page lifted out of a document, held so it can be put back.
///
/// Deleting a page in PDFium destroys it, so undo cannot work by remembering an
/// index — the content has to be kept. This owns that content, and it is
/// deliberately opaque: whether it ends up as a one-page scratch document or as
/// serialised object bytes is the implementation's business, and nothing above
/// the engine should be able to inspect or rebuild one.
///
/// It is **not** serialisable and never leaves the process. See [`UndoRecord`].
#[derive(Debug)]
pub struct RemovedPage {
    /// The page's own size, which is all any caller legitimately needs.
    pub size: PageSize,
    /// Engine-owned payload. Written and read by the PDFium implementation; the
    /// page tree has no other way to hand content back after a delete.
    #[allow(dead_code)]
    pub(crate) payload: Vec<u8>,
}

impl RemovedPage {
    #[allow(dead_code)] // The PDFium implementation is the first non-test caller.
    pub(crate) fn new(size: PageSize, payload: Vec<u8>) -> Self {
        RemovedPage { size, payload }
    }

    /// Hand the content to whoever is putting the page back, consuming self so a
    /// removed page cannot be restored twice from the same record.
    pub(crate) fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

/// Mutation of a document's structure.
///
/// Named for what it does, and deliberately free of annotation vocabulary: marks
/// on a page are objects, and they arrive as commands against the object layer
/// rather than as methods here. The previous trait was shaped the other way round
/// — `add_annotation`, `add_signature`, `remove_annotation` — which left the whole
/// page tree unreachable and made it useless for the write path.
///
/// Every method is reached through a [`Command`](crate::command::Command), which
/// is what makes undo, batch processing and scripting fall out later rather than
/// having to be retrofitted.
pub trait DocumentMut {
    // ------------------------------------------------------------- page tree --

    /// Reorder in place. `order[i]` is the index the page currently at `i` moves to.
    fn reorder_pages(&mut self, order: &[usize]) -> Result<()>;

    /// Remove a page, handing back its content so the deletion can be undone.
    fn delete_page(&mut self, index: usize) -> Result<RemovedPage>;

    /// Put a previously removed page back.
    fn insert_page(&mut self, at: usize, page: RemovedPage) -> Result<()>;

    fn insert_blank_page(&mut self, at: usize, size: PageSize) -> Result<()>;

    /// Persisted rotation, unlike the view rotation the reader applies at render
    /// time — this one survives a save.
    fn set_page_rotation(&mut self, index: usize, quarter_turns: u8) -> Result<()>;

    /// The rotation a page is currently at, in quarter turns.
    ///
    /// Read *before* a change so undo can restore it. Without it an undo record
    /// could only ever put back zero, which is correct exactly when the page was
    /// unrotated to begin with and silently wrong otherwise.
    fn page_rotation(&self, index: usize) -> Result<u8>;

    // ------------------------------------------------------------ annotation --

    /// Add a mark to a page, returning the index PDFium gave it.
    ///
    /// The index is returned rather than assumed, because it is what makes the
    /// addition reversible: undo removes exactly that annotation.
    fn add_annotation(&mut self, page_index: usize, annotation: &Annotation) -> Result<usize>;

    /// Remove a mark and discard it.
    ///
    /// Split from [`DocumentMut::take_annotation`] because the two have very
    /// different requirements and only one of them is hard. Undoing an *add* needs
    /// nothing back — the mark is being thrown away — while undoing a *remove*
    /// needs the mark itself, which means reading quad points, ink lists and
    /// colours back out of PDFium. Keeping them apart is what lets a drawn mark be
    /// undoable before that reading exists.
    fn remove_annotation(&mut self, page_index: usize, index: usize) -> Result<()>;

    /// Remove a mark and hand it back, so the removal can be undone.
    fn take_annotation(&mut self, page_index: usize, index: usize) -> Result<Annotation>;

    /// A new document holding copies of the given pages. Does not mutate self.
    fn extract_pages(&self, range: &[usize]) -> Result<Box<dyn Document>>;

    fn import_pages(&mut self, from: &dyn Document, range: &[usize], at: usize) -> Result<()>;

    // ----------------------------------------------------------- persistence --

    /// Append a delta, leaving the original bytes untouched.
    ///
    /// The default, and not for speed: a digital signature covers a byte range of
    /// the file, so a full rewrite breaks every existing signature and makes it
    /// impossible to add one that survives a later edit.
    ///
    /// Measured on the pinned PDFium via `examples/incremental_probe.rs`: with
    /// `FPDF_INCREMENTAL` the original survives as an exact prefix and a second
    /// `%%EOF` follows it; without the flag the file is rewritten 216 bytes
    /// shorter and the prefix is gone. Note that `pdfium-render`'s own
    /// `save_to_writer` hardcodes the flag to zero, so it takes the second path.
    fn save_incremental(&mut self, dest: &mut dyn Write) -> Result<()>;

    /// A rewritten, compacted copy. Offered as an explicit user action — never as
    /// the default save, because it is the path that destroys signatures.
    fn save_full_copy(&mut self, dest: &mut dyn Write) -> Result<()>;

    fn is_dirty(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// A point in page space, top-left origin with y increasing downwards.
///
/// A struct rather than a tuple so the JSON reads `{"x":1,"y":2}`. A bare pair
/// serialises as `[1,2]`, which is compact and unreadable the moment you are
/// staring at a malformed stroke wondering which number is which.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// A mark on a page.
///
/// **Coordinates are top-left origin with y increasing downwards**, matching the
/// text runs and the Kotlin model — *not* PDF's bottom-left convention. The
/// PDFium implementation flips once at the boundary, in both directions, so that
/// nothing above the engine has to think about it. That is the same bargain
/// `text_segments` already makes, and breaking it for annotations would put every
/// restored mark on the wrong half of its page.
///
/// The set is deliberately small: these three cover every tool the reader offers,
/// since a signature is ink with several strokes. Anything a document contains
/// that does not map onto one of them — a form widget, a link — is left alone
/// rather than modelled badly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Annotation {
    /// Text picked out with the highlighter: one rect per line covered, which is
    /// why this is a list rather than a rect. A selection spanning three lines is
    /// one annotation, so erasing it takes one action rather than three.
    Highlight { rects: Vec<Rect>, color: Color },
    /// A freehand stroke, or several — a signature is ink committed all at once.
    Ink {
        strokes: Vec<Vec<Point>>,
        color: Color,
        width: f32,
    },
    /// A note anchored to a point on the page.
    Note {
        rect: Rect,
        contents: String,
        color: Color,
    },
}

/// An annotation together with **PDFium's own index** for it on the page.
///
/// The index is not the annotation's position in the returned list, and the
/// difference is load-bearing: a page can hold annotations this engine does not
/// model, and those are skipped on read. Numbering our own results would address
/// the wrong annotation as soon as a page contained one — and a delete would take
/// out somebody's form field instead of their highlight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedAnnotation {
    pub index: usize,
    #[serde(flatten)]
    pub annotation: Annotation,
}

#[derive(Debug, Clone)]
pub struct Signature {
    pub rect: Rect,
    /// Rasterised visual appearance (PNG bytes).
    pub image: Vec<u8>,
    pub digital_signature: Option<DigitalSignature>,
}

#[derive(Debug, Clone)]
pub struct DigitalSignature {
    pub certificate: Vec<u8>,
    pub reason: String,
    pub location: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_size_scales_and_rounds() {
        let a4 = PageSize {
            width_pt: 595.0,
            height_pt: 842.0,
        };
        assert_eq!(a4.pixel_size(1.0), (595, 842));
        assert_eq!(a4.pixel_size(2.0), (1190, 1684));
    }

    #[test]
    fn pixel_size_never_collapses_to_zero() {
        let sliver = PageSize {
            width_pt: 0.4,
            height_pt: 800.0,
        };
        let (w, h) = sliver.pixel_size(0.01);
        assert_eq!(w, 1, "a zero-width bitmap would fail allocation downstream");
        assert_eq!(h, 8);
    }

    #[test]
    fn only_odd_quarter_turns_swap_axes() {
        assert!(!Rotation::None.swaps_axes());
        assert!(Rotation::Clockwise90.swaps_axes());
        assert!(!Rotation::Clockwise180.swaps_axes());
        assert!(Rotation::Clockwise270.swaps_axes());
    }

    fn segment(top: f32, bottom: f32) -> TextSegment {
        TextSegment {
            left: 10.0,
            top,
            right: 200.0,
            bottom,
            text: "run".into(),
        }
    }

    #[test]
    fn a_point_inside_a_run_is_detected() {
        let s = segment(100.0, 120.0);
        assert!(s.contains(50.0, 110.0));
        assert!(!s.contains(50.0, 130.0), "below the run");
        assert!(!s.contains(5.0, 110.0), "left of the run");
    }

    // The band tests that used to live here have been removed along with
    // `intersects_band`. They passed throughout the period in which highlighting
    // was visibly broken, because they asserted that a vertical band takes every
    // line it crosses — which it does, and which is the wrong question. On a
    // two-column page the band crosses both columns. Selection is now a range
    // over reading order, and is tested against runs extracted from a real
    // two-column page in `TextSelectionTest`.

    #[test]
    fn quarter_turns_wrap_in_both_directions() {
        assert_eq!(Rotation::from_quarter_turns(0), Rotation::None);
        assert_eq!(Rotation::from_quarter_turns(5), Rotation::Clockwise90);
        assert_eq!(Rotation::from_quarter_turns(-1), Rotation::Clockwise270);
    }
}

//! The document abstraction.
//!
//! Everything above this layer — the JNI bridge, the cache, the command stack —
//! talks to `dyn Document`. Mutation arrives as a separate trait implemented
//! against **PDFium, the single writer**, and deliberately behind no feature flag:
//! a flag around editing guarantees the default build never type-checks the
//! editing path, which is exactly how the old `editing` feature came to declare a
//! module nobody had written.

pub mod sensitive;
pub mod sensitivity;
pub mod blank;
pub mod classify;
pub mod glyphs;
pub mod layout;
pub mod search;
pub mod metadata;
mod outlined;
pub mod pdfium_doc;
pub mod redact;

pub use blank::{blank_document, Ruling};
pub use classify::{PageClassification, PageTextKind};
// Re-exported here because `DocumentMut` and `Command` both name them.
pub use crate::ocr::{RecognisedWord, Recogniser, TEXT_LAYER_ID};
pub use layout::{PageText, TextBlock, TextLine};
pub use metadata::DocumentMetadata;
pub use redact::{Redaction, RedactionReport, Uncleared};

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

/// What Fill & Sign can put on a page without a font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillMark {
    Tick,
    Cross,
    /// For the boxes that want filling in rather than ticking.
    Dot,
}

impl FillMark {
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "tick" | "check" | "checkmark" | "yes" => Some(FillMark::Tick),
            "cross" | "x" | "no" => Some(FillMark::Cross),
            "dot" | "bullet" | "filled" => Some(FillMark::Dot),
            _ => None,
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            FillMark::Tick => "tick",
            FillMark::Cross => "cross",
            FillMark::Dot => "dot",
        }
    }
}

/// Ink this program placed as somebody's signature, and where it sits.
///
/// **How a signature is told apart from a drawing.** Both are ink annotations,
/// and flattening a drawing because it looked like a signature would be a poor
/// way to lose somebody's markup. So a placed signature carries a key of this
/// engine's own naming it — written into the file, so it survives closing and
/// reopening, and readable by anything that can read a PDF.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureMark {
    /// PDFium's index for the annotation, which is what removes it.
    pub index: usize,
    /// What the signature was called when it was placed.
    pub name: String,
    pub strokes: Vec<Vec<Point>>,
    pub color: Color,
    pub width: f32,
}

/// Something worth a second look, and where it sits.
#[derive(Debug, Clone, PartialEq)]
pub struct SensitiveOnPage {
    pub kind: crate::document::sensitive::Kind,
    /// What was found, so a person can judge it before anything happens to it.
    pub text: String,
    pub area: Rect,
    pub page_index: usize,
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

    // Extended once, deliberately, rather than twice under two pressures:
    // layout reconstruction needs the size to derive a line tolerance that
    // scales with the type, and the editing path will need the same fields.
    //
    // Every field below is defaulted, so a bridge that has not been taught
    // about them reads the JSON exactly as before.
    /// Points, as rendered — including any vertical scale in the text matrix.
    #[serde(default)]
    pub font_size: f32,
    /// The font's own name, for the editor to match against.
    #[serde(default)]
    pub font_name: String,
    /// The run's direction, taken from the script of its characters rather than
    /// from the document's language field, which is often absent and often
    /// wrong.
    #[serde(default)]
    pub direction: crate::document::layout::Direction,
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

    /// The page's characters as placed glyphs, for layout reconstruction.
    ///
    /// Separate from [`Page::characters`] because it carries the angle of each
    /// character as well as its box. Reconstruction needs that: a stamp set at
    /// 45° across a drawing must be segregated before lines are grouped, or it
    /// merges into every horizontal line it crosses.
    fn glyphs(&self) -> Result<Vec<layout::Glyph>> {
        Ok(Vec::new())
    }

    /// What kind of text, if any, this page has.
    ///
    /// Cheap: one text-page load and one walk of the object list. Every later
    /// extraction tier dispatches on it, and the reader can finally say "this
    /// page has no text layer" rather than leaving someone to wonder why
    /// selection does nothing.
    fn classify(&self) -> Result<PageClassification> {
        Ok(PageClassification {
            kind: PageTextKind::Empty,
            chars: 0,
            unmappable: 0,
            image_coverage: 0.0,
            paths: 0,
            glyph_paths: 0,
            rotated_chars: 0,
        })
    }

    /// Read type converted to outlines by matching its paths against a
    /// candidate face's own glyphs.
    ///
    /// **The caller supplies the face.** A page with no text objects has no
    /// font this engine can reach for on its own — the document's original
    /// font was thrown away the moment the artwork was converted to curves.
    /// Build the catalogue from whatever face the document is believed to have
    /// used; see [`glyphs::Catalogue::from_font`] and its own note that a face
    /// this does not have will never resolve.
    ///
    /// Returns `Ok` with an empty result for a page that matched nothing at
    /// all — a logo, a ruled table, plain decoration — which is not a failure
    /// of this page or this catalogue. It is only ever a shape that is not a
    /// letter in the face it was compared against.
    fn recognise_outlined(&self, _catalogue: &glyphs::Catalogue) -> Result<layout::PageText> {
        Ok(layout::PageText {
            blocks: Vec::new(),
            source: layout::Source::Reconstructed,
            confidence: 0.0,
        })
    }

    /// [`Page::recognise_outlined`], as words rather than reassembled prose.
    ///
    /// The shape [`RecognisedWord`] already is — the same one OCR produces for
    /// a scanned page — so [`crate::command::Command::AddTextLayer`] can take
    /// either without knowing which one recognised the words. This is the
    /// entry point an "Extract Text" feature wants: a scanned page and an
    /// outlined one have the same actual problem, no native text layer to
    /// select, and this is what lets them share the one fix for it rather than
    /// needing a second selection mechanism built for outlined pages alone.
    fn recognise_outlined_words(&self, _catalogue: &glyphs::Catalogue) -> Result<Vec<RecognisedWord>> {
        Ok(Vec::new())
    }

    /// Whether [`Page::recognise_outlined_words`]' own output is worth trusting
    /// over running OCR instead.
    ///
    /// **Answers a different question from `recognise_outlined_words` itself.**
    /// That method cannot know whether the face it was given is the *right*
    /// one — a font it was not built from will still return *some* matches,
    /// just wrong or sparse ones, because a handful of simple shapes coincide
    /// across almost any two sans-serif faces. This is the check that catches
    /// that: it compares how many characters were actually recognised against
    /// [`PageClassification::glyph_paths`] — the count of type-shaped paths
    /// [`Page::classify`] already found, for free, deciding the page was
    /// `Outlined` in the first place.
    ///
    /// **Measured, not guessed**, on a real page matched against three cases:
    ///
    /// ```text
    ///                                    matched chars / glyph_paths
    ///   the page's own face                        0.84 – 0.87
    ///   a plausible but wrong face                      0.27
    ///   no usable face at all                           0.00
    /// ```
    ///
    /// The gap between a right and a wrong face is wide, so the threshold sits
    /// with a healthy margin on both sides rather than close to either
    /// measurement — a fixture built from one page of one font pair is a
    /// calibration point, not a corpus, and a threshold that only just cleared
    /// the wrong-face case would be one bad measurement away from being wrong
    /// the other way.
    fn outlined_words_are_trustworthy(&self, words: &[RecognisedWord]) -> Result<bool> {
        const MINIMUM_RATIO: f32 = 0.5;
        let glyph_paths = self.classify()?.glyph_paths;
        // Zero glyph-shaped paths means nothing here could genuinely have
        // matched — `recognise_outlined_words` filters through the same
        // `looks_like_type` test `glyph_paths` is counted from. Guarded
        // explicitly rather than folded into the division below: a `.max(1)`
        // on the denominator would have made this case look *more*
        // trustworthy the less there was to measure, which is the wrong
        // direction for a divide-by-zero guard to fail in.
        if glyph_paths == 0 {
            return Ok(false);
        }
        let matched: usize = words.iter().map(|w| w.text.chars().count()).sum();
        Ok(matched as f32 / glyph_paths as f32 >= MINIMUM_RATIO)
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

    /// Every run of text on a page, in the order the file stores them.
    ///
    /// A read, so it lives on `Document` rather than beside the write: showing
    /// somebody what is on a page should not require the power to change it.
    fn text_runs(&self, _page_index: usize) -> Result<Vec<TextRun>> {
        Ok(Vec::new())
    }

    /// Every image on a page, in the order the file stores them.
    ///
    /// A read, so it sits beside [`Document::text_runs`] rather than next to
    /// the write. What Lock needs to offer an image as something selectable,
    /// and what a right-click has to hit-test against.
    /// Things on a page somebody would not want to send out.
    ///
    /// **Candidates, not decisions.** See [`crate::document::sensitive`] — this
    /// finds what can be *checked*, and a person chooses what to do about it.
    /// Each comes back with the rectangle it occupies, ready to hand to
    /// [`DocumentMut::redact`] or to a lock.
    fn sensitive_on(&self, page_index: usize) -> Result<Vec<SensitiveOnPage>> {
        let characters = self.page(page_index)?.characters()?;

        let mut out = Vec::new();
        for found in crate::document::sensitive::scan(&characters.text) {
            // The boxes are per character and the finds are in characters, so
            // the two line up without conversion — which is the whole reason
            // `sensitive` counts in characters rather than bytes.
            let mut area =
                Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
            for index in found.at.clone() {
                let Some(b) = characters.boxes.get(index * 4..index * 4 + 4) else { continue };
                area.left = area.left.min(b[0]);
                area.top = area.top.min(b[1]);
                area.right = area.right.max(b[2]);
                area.bottom = area.bottom.max(b[3]);
            }
            if area.right <= area.left {
                // Found in the text but not on the page — nothing to point at,
                // so nothing to offer.
                continue;
            }
            out.push(SensitiveOnPage {
                kind: found.kind,
                text: found.text,
                area,
                page_index,
            });
        }
        Ok(out)
    }

    fn images_on(&self, _page_index: usize) -> Result<Vec<PageImage>> {
        Ok(Vec::new())
    }

    /// The bytes of the font one text run is drawn in, if it can be had.
    ///
    /// **What Obfuscate needs and [`crate::crypto::tweak::coverage`] cannot
    /// give it.** That function answers for fonts the *app* registered and
    /// returns [`crate::crypto::tweak::Coverage::Unknown`] for a document's
    /// own — correctly, since it has no way to open one. Taken literally that
    /// refuses every obfuscation, because `Unknown` is never safe.
    ///
    /// It matters because FF1 preserves the alphabet but not the characters: a
    /// part number using seven distinct digits can be replaced by one using all
    /// ten, and an embedded font is usually subset to the glyphs the document
    /// already needed. `pdfium_doc`'s own text-splitting names this exact
    /// failure and sidesteps it by only ever writing a subset of what was
    /// already drawn — which is not open to a feature whose whole job is
    /// writing something else.
    ///
    /// `None` where there is no font to read rather than an error: a page
    /// object that is not text has no font, and that is an answer rather than a
    /// fault. Note that PDFium substitutes for a non-embedded font, so bytes
    /// coming back do not prove the document carried them — see
    /// [`Document::run_font_is_embedded`].
    fn run_font_data(&self, _page_index: usize, _object: usize) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    /// Whether that font is embedded in the document rather than substituted.
    ///
    /// A substituted font is the *viewer's* answer to a name in the file, and
    /// another reader will substitute differently — so its coverage says
    /// nothing about what a third party will see.
    fn run_font_is_embedded(&self, _page_index: usize, _object: usize) -> Result<bool> {
        Ok(false)
    }

    /// Every text mark on a page, as the blobs the app stored beside them.
    ///
    /// Empty by default, and empty for every page this app has never written
    /// words onto. Text is page content rather than an annotation, so it appears
    /// in neither [`Document::annotations`] nor any index — these blobs are what
    /// make saved words a mark again instead of part of the page.
    fn text_marks(&self, _page_index: usize) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// What this document permits, when it is encrypted.
    ///
    /// `None` for a document with no password, and that is not the same as
    /// "everything permitted": permission bits in an unencrypted file are
    /// decoration, since nothing enforces them and every reader ignores them.
    /// Saying so is the honest answer; reporting "everything permitted" would
    /// suggest a restriction is possible and simply not in force.
    fn permissions(&self) -> Option<crate::pdf::encrypt::Permissions> {
        None
    }

    /// The face the last edit fell back to, if it was not the run's own.
    ///
    /// `None` when the words were written in the font that was already there,
    /// which is the ordinary case and the one nobody needs telling about.
    fn substituted_face(&self) -> Option<String> {
        None
    }


    /// Shift one page object by a distance, in page points.
    ///
    /// Text, image or path alike — a reader dragging something on a page does
    /// not distinguish, and neither does this.
    ///
    /// **This re-emits the page's content stream**, which is the one thing this
    /// crate avoids wherever it can: PDFium's object model is the only way to
    /// move an object, and committing a change to it needs
    /// `FPDFPage_GenerateContent`. Measured before it was offered — see the
    /// tests — because a move that quietly reflowed the paragraph beside it
    /// would be a poor trade.
    /// Put one thing at the front or the back of the page's drawing order.
    ///
    /// **The only way a PDF stacks anything is the order it draws it**, so this
    /// moves the operators that draw the object to the start or the end of the
    /// content stream — leaving every other byte alone — and re-establishes the
    /// state they were drawn under, because that state is set by operators they
    /// have now been moved away from.
    ///
    /// Words, pictures and shapes. Refuses rather than guess: a clipping path
    /// in force where the object was drawn cannot be reproduced at the other
    /// end of the stream without carrying the clip too, and an object drawn
    /// behind one that is no longer there is a worse answer than declining. A
    /// shape that *sets* a clip is refused for the mirror of that reason —
    /// `q`/`Q` restores the clipping path, so wrapping one would close its clip
    /// at the wrong moment.
    fn restack(
        &mut self,
        _page_index: usize,
        _object: usize,
        _where_to: Stacking,
    ) -> Result<()> {
        Err(PdfError::Unsupported("changing the drawing order"))
    }

    fn move_object(&mut self, _page_index: usize, _object: usize, _by: Point) -> Result<()> {
        Err(PdfError::Unsupported("moving an object on this page"))
    }

    /// The area one page object covers.
    ///
    /// **For the case where a drawn word is part of something bigger.** A
    /// heading converted to outlines is often a single path holding every
    /// letter of it, and a rectangle around one word merely crosses that path
    /// rather than containing it — which is the difference between a shape that
    /// can be taken off the page and one that cannot. A caller told *which*
    /// object is in the way needs to know how far it reaches.
    /// Everything drawn on a page, bottom first.
    ///
    /// The order is the page's own drawing order, which is the only stacking a
    /// PDF has — see [`DrawnObject`].
    fn drawn_objects(&self, _page_index: usize) -> Result<Vec<DrawnObject>> {
        Err(PdfError::Unsupported("listing what a page draws"))
    }

    fn object_bounds(&self, _page_index: usize, _object: usize) -> Result<Rect> {
        Err(PdfError::Unsupported("measuring an object on this page"))
    }

    /// The signatures placed on a page, as opposed to any other ink on it.
    ///
    /// Empty for a document that has none, and for one this program did not
    /// place them in — see [`SignatureMark`] for how the two are told apart.
    fn signature_marks(&self, _page_index: usize) -> Result<Vec<SignatureMark>> {
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

    /// This document's handle in whatever backend opened it, as an opaque
    /// number. `None` for a backend that has no such thing.
    ///
    /// The one place a backend detail reaches the trait, and it is here
    /// because moving pages needs *two* documents at once — the trait's
    /// methods each borrow one. Kept as a `usize` so the trait itself stays
    /// free of PDFium types; only the implementation knows what it means, and
    /// only that implementation may interpret it.
    fn backend_handle(&self) -> Option<usize> {
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

    /// A copy of a page as it stands now, in the form [`DocumentMut::insert_page`]
    /// takes back.
    ///
    /// Exists for redaction's undo. Every other command reverses by describing
    /// the change — put the crop back, write the old words — and redaction
    /// cannot, because what it removes is gone from the object model. The only
    /// honest inverse is a copy of the page from before.
    ///
    /// The cost is the page's own bytes sitting in the undo stack, images and
    /// all, until the entry ages out. `ImportPages` already carries whole PDFs
    /// for the same reason, and the alternative — the one destructive operation
    /// in the app being the only one with no undo — is worse.
    fn snapshot_page(&self, _index: usize) -> Result<RemovedPage> {
        Err(PdfError::Unsupported("copying a page"))
    }

    fn insert_blank_page(
        &mut self,
        at: usize,
        size: PageSize,
        fill: Option<Color>,
        ruling: Ruling,
    ) -> Result<()>;

    /// Persisted rotation, unlike the view rotation the reader applies at render
    /// time — this one survives a save.
    fn set_page_rotation(&mut self, index: usize, quarter_turns: u8) -> Result<()>;

    /// The rotation a page is currently at, in quarter turns.
    ///
    /// Read *before* a change so undo can restore it. Without it an undo record
    /// could only ever put back zero, which is correct exactly when the page was
    /// unrotated to begin with and silently wrong otherwise.
    fn page_rotation(&self, index: usize) -> Result<u8>;

    /// The page's crop box, in page points with the origin at the **top left**
    /// — the same convention every other rectangle in this crate uses.
    ///
    /// The crop box is what a reader sees and what this crate reports as the
    /// page size. The media box is the sheet it was laid out on, and the two
    /// differ on any document that has been trimmed.
    fn page_crop(&self, index: usize) -> Result<Rect>;


    /// Set the crop box. Clamped to the media box by the engine — a crop larger
    /// than the sheet is not a page, it is a mistake.
    fn set_page_crop(&mut self, index: usize, crop: Rect) -> Result<()>;

    /// Replace the words in one run, leaving where it sits and how it looks
    /// alone.
    ///
    /// Returns **the words and the appearance it replaced**. Undo needs both,
    /// and reading them here rather than beforehand means they cannot be taken
    /// from a different state than the one the change is applied to.
    fn set_text_run(&mut self, page_index: usize, object: usize, text: &str) -> Result<String> {
        self.set_text_run_styled(page_index, object, text, &TextStyle::default())
            .map(|(words, _)| words)
    }

    /// As [`DocumentMut::set_text_run`], also changing size, colour or position.
    fn set_text_run_styled(
        &mut self,
        page_index: usize,
        object: usize,
        text: &str,
        style: &TextStyle,
    ) -> Result<(String, TextStyle)>;

    /// Destroy every piece of content inside a rectangle.
    ///
    /// **Not a black rectangle drawn on top.** The text objects are deleted, the
    /// runs that straddle the edge are rebuilt without the covered characters,
    /// and what is returned says what was destroyed and what — an image crossing
    /// the boundary, a drawing under the mark — survived it.
    ///
    /// The document is left needing a full-copy save afterwards; see
    /// [`DocumentMut::must_save_full_copy`].
    ///
    /// `catalogue` is what turns a page whose words are curves from wholly
    /// refused into partly redactable. **Without one**, a page classified
    /// [`PageTextKind::Outlined`] still refuses outright, unchanged from
    /// before this parameter existed — there being no font is exactly the
    /// case that refusal exists for. **With one**, a path shaped like a
    /// letter is matched against it; matched paths already wholly inside the
    /// rectangle were always removable regardless of shape, and this is what
    /// lets a matched path merely *crossing* the rectangle's edge join them,
    /// rather than sitting forever as [`crate::document::Uncleared::OutlinedText`].
    /// A path that cannot be matched is unaffected either way — still
    /// reported, never guessed at.
    fn redact(
        &mut self,
        _request: &Redaction,
        _catalogue: Option<&glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        Err(PdfError::Unsupported("redacting this document"))
    }

    /// What a redaction **would** do, without doing any of it.
    ///
    /// The same survey [`DocumentMut::redact`] runs, stopped before it acts. A
    /// caller shows the report, and where the only thing in the way is an image
    /// beside the text rather than the text itself, asks the one person who can
    /// answer whether it matters — instead of refusing on their behalf.
    fn preview_redaction(
        &mut self,
        _request: &Redaction,
        _catalogue: Option<&glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        Err(PdfError::Unsupported("redacting this document"))
    }

    /// Replace a page with a copy of one.
    ///
    /// Delete-then-insert at the same index, so nothing after it moves. Used by
    /// undo and by unlocking, which are the same operation seen from two sides.
    fn replace_page(&mut self, _index: usize, _pdf: &[u8]) -> Result<()> {
        Err(PdfError::Unsupported("replacing a page"))
    }

    // ------------------------------------------------------------------ lock --

    /// Hide an area, keeping a sealed copy so a passcode can bring it back.
    ///
    /// **Lock hides; Redact destroys.** The content comes off the page exactly
    /// as a redaction takes it, and the page as it was goes into the document
    /// sealed under `passcode`. Everything that refuses a redaction refuses this
    /// too — a page whose words are curves cannot have them taken off it, and
    /// keeping a copy does not change that.
    ///
    /// The sealed copy is written **before** the removal. If the write fails,
    /// nothing has been taken off the page; if the removal then fails, the
    /// document carries a way back to a page that was never changed, which is
    /// harmless. The other order has a window where content is gone and nothing
    /// can return it.
    fn lock_area(
        &mut self,
        _request: &Redaction,
        _passcode: &[u8],
        _catalogue: Option<&glyphs::Catalogue>,
    ) -> Result<RedactionReport> {
        Err(PdfError::Unsupported("locking this document"))
    }

    /// Lock whole pages: seal each one, then leave it blank.
    ///
    /// **Deliberately not [`DocumentMut::lock_area`] over the whole page.**
    /// That routes through redaction, which has to identify and remove text
    /// objects precisely — and therefore refuses a page whose words are curves
    /// or a scan, exactly as `an_outlined_page_refuses_to_lock` pins down.
    /// Hiding an *entire* page needs no precision at all: the page is sealed
    /// and replaced by a blank one of the same size. So this works on every
    /// page, including the outlined and scanned ones area-locking must turn
    /// away, which is what "lock the whole document" has to mean for it to be
    /// worth offering.
    ///
    /// Every page is sealed and the vault written **before any page is
    /// blanked**, for the reason [`DocumentMut::lock_area`] gives: a failure
    /// part-way through then leaves a document carrying copies of pages that
    /// were never changed, rather than blank pages with no way back.
    ///
    /// Returns how many pages were newly locked. A page already locked keeps
    /// the way back it has — see [`crate::crypto::Vault::seal_page`] — and is
    /// not counted again.
    fn lock_pages(&mut self, _pages: &[usize], _passcode: &[u8]) -> Result<usize> {
        Err(PdfError::Unsupported("locking this document"))
    }

    /// Take one image off its page and seal it under a passcode.
    ///
    /// **The image itself is sealed, not the page it sat on.** An image is
    /// self-contained bytes and a matrix, with none of the font dependency that
    /// makes a sealed text run unreliable — so this one object can come back on
    /// its own, which is what a lock badge that unlocks what it sits on has to
    /// mean.
    ///
    /// Returns the id naming that seal from now on.
    fn lock_image(
        &mut self,
        _page_index: usize,
        _object: usize,
        _passcode: &[u8],
    ) -> Result<String> {
        Err(PdfError::Unsupported("locking an image in this document"))
    }

    /// Put one sealed object back where it came from.
    ///
    /// Unlike [`DocumentMut::open_lock`], this restores a single item and
    /// leaves every other seal alone — clicking one badge must not reveal the
    /// rest of the page.
    fn unlock_item(&mut self, _id: &str, _passcode: &[u8]) -> Result<()> {
        Err(PdfError::Unsupported("unlocking an item in this document"))
    }

    /// Everything sealed on one page, for drawing its badges.
    /// Finish any lock that recorded its badge but never took its picture off
    /// the page — and drop the badge where that still cannot be done.
    ///
    /// Returns how many were completed and how many were dropped. Nothing is
    /// touched on a document with no such lock, so this is safe to call on
    /// every open.
    fn repair_locks(&mut self) -> Result<(usize, usize)> {
        Ok((0, 0))
    }

    fn locked_items_on(&self, _page_index: usize) -> Result<Vec<LockedItem>> {
        Ok(Vec::new())
    }

    /// Every locked page's original, decrypted and verified.
    ///
    /// **Verifies before it returns anything.** Each page's tag is checked, and
    /// a single failure fails the call — so a document with one damaged seal
    /// cannot half restore and leave nobody able to say which pages are which.
    /// Nothing is written; the caller puts the pages back through the command
    /// stack so that undo works.
    fn open_lock(&mut self, _passcode: &[u8]) -> Result<Vec<(usize, Vec<u8>)>> {
        Err(PdfError::Unsupported("unlocking this document"))
    }

    /// Which pages have a sealed original in this document.
    fn locked_pages(&self) -> Result<Vec<usize>> {
        Ok(Vec::new())
    }

    /// Whether an incremental save would put back what a redaction removed.
    ///
    /// **The trap this exists for.** `FPDF_INCREMENTAL` keeps the original bytes
    /// verbatim and appends a delta, which is exactly right for preserving a
    /// signature and exactly wrong here: the words are still in the file, at
    /// their original offsets, and any reader that walks the earlier
    /// cross-reference section finds them. A redaction saved incrementally is a
    /// redaction that did not happen.
    fn must_save_full_copy(&self) -> bool {
        false
    }

    /// Transform everything on a page by `[a, b, c, d, e, f]`.
    ///
    /// PDF's own matrix order. Used to scale content when a page changes size —
    /// without it, resizing a sheet only reveals or hides margin, which is not
    /// what anybody means by *resize*.
    fn transform_page(&mut self, index: usize, matrix: [f32; 6]) -> Result<()>;

    /// Set the sheet size, in points. Sets the crop box to match.
    fn set_page_media(&mut self, index: usize, width_pt: f32, height_pt: f32) -> Result<()>;

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

    /// The app's own description of the text mark [`id`] on this page.
    ///
    /// Stored beside the words when they were written, and handed back untouched.
    /// It is what lets a caption be rebuilt as an editable mark after a save, and
    /// what lets erasing one be undone.
    fn text_mark_restore(&mut self, page_index: usize, id: i32) -> Result<String>;

    /// Take the words of one text mark off the page.
    ///
    /// Every object the write put there carries the id, so this finds all of them
    /// however the page has been edited since — and leaves everything else,
    /// including the document's own text, exactly where it was.
    fn remove_text(&mut self, page_index: usize, id: i32) -> Result<()>;

    /// Write recognised words onto a page as invisible, selectable text.
    ///
    /// The page must look **byte-for-byte the same** afterwards. What changes is
    /// that it becomes selectable and searchable — and, because the words are
    /// real text objects, the result then flows back through ordinary
    /// extraction like any other native text. One extraction path, not two.
    ///
    /// Every word is written under [`TEXT_LAYER_ID`], so the whole layer comes
    /// off again in one call and undo needs nothing new.
    fn add_text_layer(&mut self, page_index: usize, words: &[RecognisedWord]) -> Result<usize> {
        let _ = (page_index, words);
        Err(PdfError::Unsupported("writing a text layer to this document"))
    }

    /// A new document holding copies of the given pages. Does not mutate self.
    fn extract_pages(&self, range: &[usize]) -> Result<Box<dyn Document>>;

    /// Copy pages out of another open document into this one, at `at`.
    ///
    /// `indices` is taken in the order given, not sorted: "pages 3, 1 and 2"
    /// is a thing somebody can ask for, and quietly sorting the list hands
    /// them a different document without saying so. An empty list means every
    /// page of the source.
    ///
    /// Returns how many pages arrived, which is what an undo needs in order to
    /// take them back out.
    fn import_pages(
        &mut self,
        source: &dyn Document,
        indices: &[usize],
        at: usize,
    ) -> Result<usize>;

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

    /// Put a password on the document — one another reader will ask for.
    ///
    /// **Not the same promise as Lock.** Lock takes content off the page and
    /// keeps a sealed copy inside the file, so the words are gone for anyone
    /// without the passcode but the document itself opens for everybody. This
    /// encrypts the whole file: nothing at all is readable without the
    /// password, in Pagify or anywhere else.
    ///
    /// `owner`, when given, opens the document with the restrictions lifted;
    /// without it the one password does both, which is what "put a password on
    /// this" usually means.
    ///
    /// Applied when the document is next saved, because a password is a
    /// property of the file rather than of what is on the pages — and because
    /// the encryption has to be the last thing that touches the bytes.
    fn secure_document(
        &mut self,
        _user: &[u8],
        _owner: Option<&[u8]>,
        _permissions: crate::pdf::encrypt::Permissions,
    ) -> Result<()> {
        Err(PdfError::Unsupported("this document cannot be secured"))
    }

    /// Put Pagify's own password on the file instead of PDF's.
    ///
    /// **Nothing else will read the result.** See [`crate::pdf::secure_plus`]:
    /// Argon2id and authenticated SM4-GCM, behind a security handler no other
    /// program implements. Measured, PDFium refuses such a file outright and
    /// macOS renders a blank page — no content escapes either way, but neither
    /// can open it.
    ///
    /// That is the trade, and a caller must put it to the person before they
    /// choose it, not afterwards.
    fn secure_document_plus(&mut self, _user: &[u8]) -> Result<()> {
        Err(PdfError::Unsupported("this document cannot be secured"))
    }

    /// Whether the password on this document is Pagify's own.
    fn is_secure_plus(&self) -> bool {
        false
    }

    /// Take the password back off, before it has been saved with one.
    fn unsecure_document(&mut self) -> Result<()> {
        Err(PdfError::Unsupported("this document carries no password"))
    }

    /// Whether a password is waiting to be written on the next save.
    fn is_secured(&self) -> bool {
        false
    }

    /// Whether the file this came from **already** has a password.
    ///
    /// Distinct from [`DocumentMut::is_secured`], which asks whether one is
    /// waiting to be written. This asks whether one is already there — and it
    /// is what says a second password cannot be added, because the content
    /// would be encrypted twice and the file would open for nobody.
    ///
    /// Asked so a caller can refuse before somebody types a password rather
    /// than after.
    fn already_has_password(&self) -> bool {
        false
    }

    /// Whether the file this came from had a password when it was opened.
    ///
    /// Unlike [`DocumentMut::already_has_password`], this does **not** change
    /// when one is on its way off. It answers a different question — "was this
    /// file already encrypted when we got it?" — which is what says whether
    /// writing a password over it in place loses anything.
    fn had_password_on_open(&self) -> bool {
        false
    }

    /// Whether this is the password the document was opened with.
    ///
    /// For changing one: the current password is asked for before a new one is
    /// chosen, so that an unattended open document cannot have its password
    /// changed by whoever walks past it.
    fn password_matches(&self, _typed: &[u8]) -> bool {
        false
    }

    /// What this document carries that is not on its pages.
    ///
    /// Reports; changes nothing. See [`crate::pdf::hidden`] for what counts and
    /// why the two are separate calls.
    fn hidden_data(&mut self) -> Result<crate::pdf::hidden::Hidden> {
        Err(PdfError::Unsupported("this document cannot be surveyed"))
    }

    /// Sign this document with a certificate.
    ///
    /// **A real signature**, not a picture of one: a detached CMS blob over the
    /// bytes of the file, which any reader can check. See
    /// [`crate::pdf::sign`].
    ///
    /// Applied at once rather than at save time, because the signature is over
    /// the file *as written* — anything done afterwards would break it, and it
    /// is better for that to be obvious than for a save to silently invalidate
    /// what somebody just signed.
    fn sign_document(
        &mut self,
        _pkcs12: &[u8],
        _password: &str,
        _about: &crate::pdf::sign::Reason,
    ) -> Result<String> {
        Err(PdfError::Unsupported("signing this document"))
    }

    /// How many signatures this document carries.
    fn signature_count(&self) -> usize {
        0
    }

    /// Check the signatures this document carries.
    ///
    /// Answers whether the file has changed since it was signed — which is
    /// arithmetic — and **not** whether the signer is who they claim to be,
    /// which needs a chain of trust nothing here supplies. See
    /// [`crate::pdf::validate`]; every verdict carries the distinction.
    fn validate_signatures(&mut self) -> Result<Vec<crate::pdf::validate::Signature>> {
        Err(PdfError::Unsupported("checking this document's signatures"))
    }

    /// Ask a time authority to attest that this document existed now.
    ///
    /// **The one thing in this program that uses the network.** A time written
    /// here would prove nothing — see [`crate::pdf::timestamp`] for exactly
    /// what leaves the machine, which is a digest and nothing else.
    ///
    /// `authority` has no default and no fallback: nothing is contacted unless
    /// a caller names where.
    fn timestamp_document(&mut self, _authority: &str) -> Result<()> {
        Err(PdfError::Unsupported("timestamping this document"))
    }

    /// Put a tick, a cross or a dot on a page.
    ///
    /// # Why these are drawn rather than typed
    ///
    /// A tick is `\u{2713}` and a cross is `\u{2717}`, and neither is in the
    /// fonts a PDF can rely on having — Helvetica has no tick, so typing one
    /// gets a blank box or a substitution nobody chose. Drawn as strokes, they
    /// look the same in every reader and need no font at all.
    ///
    /// They go into the page's own content rather than into an annotation, so
    /// they print and cannot be switched off — which is the whole point of
    /// filling a form in.
    fn stamp_mark(&mut self, _page_index: usize, _mark: FillMark, _at: Point, _size: f32)
        -> Result<()> {
        Err(PdfError::Unsupported("filling in this document"))
    }

    /// Offer fonts for typing characters this document's own fonts cannot
    /// spell.
    ///
    /// A subset font carries only the glyphs the file already uses, so editing
    /// text inside one is limited to the letters already on the page. These are
    /// the fonts a caller is willing to have written into the document instead
    /// — the reader's own, since a licensed typeface is theirs to supply. See
    /// [`crate::pdf::embed`].
    fn set_typing_fonts(&mut self, _fonts: Vec<Vec<u8>>) {}

    /// Rule a line, the way a form is filled in by hand.
    ///
    /// A pen stroke: for striking something out, or for the rule somebody draws
    /// to sign above. In the same ink as the tick and the box, and written into
    /// the page rather than laid over it — see [`DocumentMut::stamp_box`] for
    /// why that is kept apart from the drawing tools.
    fn stamp_line(&mut self, _page_index: usize, _from: Point, _to: Point) -> Result<()> {
        Err(PdfError::Unsupported("ruling a line on this document"))
    }

    /// Draw a box around something, the way a form is filled in by hand.
    ///
    /// The fill-and-sign rectangle: an outline in the same ink as the tick and
    /// the cross, written into the page rather than laid over it as a mark
    /// somebody can pick up and move. Kept apart from the drawing tools for
    /// that reason — a box drawn here is part of the filled-in form.
    fn stamp_box(&mut self, _page_index: usize, _area: Rect) -> Result<()> {
        Err(PdfError::Unsupported("drawing a box on this document"))
    }

    /// Record that an ink annotation is a signature rather than a drawing.
    ///
    /// Written onto the annotation in the file, so the distinction survives
    /// being closed and reopened — see [`SignatureMark`].
    fn mark_as_signature(&mut self, _page_index: usize, _index: usize, _name: &str) -> Result<()> {
        Err(PdfError::Unsupported("marking a signature in this document"))
    }

    /// Burn this page's placed signatures into the page itself.
    ///
    /// Afterwards they are page content — the same as anything printed on the
    /// page — rather than annotations a reader can select and delete. **This is
    /// the point of it, and it does not undo**: what is written into a content
    /// stream cannot be lifted back out, any more than a whiteout can.
    ///
    /// Returns how many were applied.
    fn apply_signatures(&mut self, _page_index: usize) -> Result<usize> {
        Err(PdfError::Unsupported("applying signatures to this document"))
    }

    /// Mark how far this document may travel.
    ///
    /// Stamps every page and records the label in the document's information,
    /// so a person and a machine both find it. See
    /// [`crate::document::sensitivity`] — this marks; it does not protect, and
    /// a caller must not let the two be confused.
    ///
    /// Replaces any label already there, rather than stacking a second one.
    fn set_sensitivity(&mut self, _level: sensitivity::Sensitivity) -> Result<()> {
        Err(PdfError::Unsupported("marking this document"))
    }

    /// Take the marking off — every stamp, and the record.
    fn clear_sensitivity(&mut self) -> Result<()> {
        Err(PdfError::Unsupported("marking this document"))
    }

    /// What this document is marked as, if anything.
    fn sensitivity(&mut self) -> Option<sensitivity::Sensitivity> {
        None
    }

    /// Paint over an area without removing what is under it.
    ///
    /// # This is not redaction, and the difference is the whole point
    ///
    /// A whiteout is **paint**. The words underneath are still in the file,
    /// still selectable, still found by search, and still there for anyone who
    /// opens the document in a text editor. It covers a blemish on a scan, a
    /// pencil note in a margin, a stray mark — things somebody wants gone from
    /// the *look* of a page.
    ///
    /// It is the wrong tool for anything anyone must not read, and the right
    /// one for tidying. Every caller is expected to say so; the app does.
    ///
    /// Kept apart from [`DocumentMut::redact`] rather than offered as a flag on
    /// it, because a flag that turns "destroy" into "cover" is precisely the
    /// kind of thing that gets set by accident.
    fn whiteout(&mut self, _page_index: usize, _area: Rect, _colour: Color) -> Result<()> {
        Err(PdfError::Unsupported("painting over this document"))
    }

    /// Take that data out, and say what went.
    ///
    /// Applied at once rather than at save time, so the document in front of
    /// the person is the sanitised one — a survey that still reported the old
    /// findings afterwards would be worse than not offering this at all.
    fn remove_hidden_data(&mut self) -> Result<crate::pdf::hidden::Hidden> {
        Err(PdfError::Unsupported("this document cannot be sanitised"))
    }

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

impl Annotation {
    /// Whether this mark puts no ink on the page.
    ///
    /// A fully transparent mark is bookkeeping, not something anyone can see —
    /// the drawing tools write one at the page origin to carry the data that
    /// lets a stroke be found and erased again. Treating it as a visible mark
    /// made every drawing on a page fail whenever a lock happened to cover that
    /// corner.
    pub fn draws_nothing(&self) -> bool {
        let colour = match self {
            Annotation::Highlight { color, .. }
            | Annotation::Underline { color, .. }
            | Annotation::StrikeOut { color, .. }
            | Annotation::Squiggly { color, .. }
            | Annotation::Ink { color, .. }
            | Annotation::Note { color, .. }
            | Annotation::Text { color, .. } => color,
        };
        colour.a == 0
    }

    /// The rectangle this mark covers, in the same top-left space as the rest of
    /// the API.
    ///
    /// Every variant has geometry; only `Note` states it outright, so the others
    /// are measured from the rects or points they are made of. `None` for a mark
    /// with no geometry at all, which cannot be said to overlap anything.
    ///
    /// **What this is for.** Locking an area has to take the marks over that
    /// area with it. Reported from use as "annotations don't get locked": ink
    /// drawn across a passage stayed exactly where it was, drawn on top of the
    /// black mark, because annotations are not page content — they live in the
    /// page's `/Annots` array and cutting the content stream never touched them.
    pub fn bounds(&self) -> Option<Rect> {
        fn around(points: impl Iterator<Item = (f32, f32)>) -> Option<Rect> {
            let mut found = None::<Rect>;
            for (x, y) in points {
                found = Some(match found {
                    None => Rect { left: x, top: y, right: x, bottom: y },
                    Some(r) => Rect {
                        left: r.left.min(x),
                        top: r.top.min(y),
                        right: r.right.max(x),
                        bottom: r.bottom.max(y),
                    },
                });
            }
            found
        }
        fn over(rects: &[Rect]) -> Option<Rect> {
            around(rects.iter().flat_map(|r| [(r.left, r.top), (r.right, r.bottom)]))
        }

        match self {
            Annotation::Highlight { rects, .. }
            | Annotation::Underline { rects, .. }
            | Annotation::StrikeOut { rects, .. }
            | Annotation::Squiggly { rects, .. } => over(rects),
            Annotation::Ink { strokes, width, .. } => {
                // A stroke is a path, and the pen has width — half of it either
                // side of the line, or thick ink over a boundary would read as
                // not touching it.
                around(strokes.iter().flatten().map(|p| (p.x, p.y))).map(|r| Rect {
                    left: r.left - width / 2.0,
                    top: r.top - width / 2.0,
                    right: r.right + width / 2.0,
                    bottom: r.bottom + width / 2.0,
                })
            }
            Annotation::Note { rect, .. } => Some(*rect),
            Annotation::Text { glyphs, .. } => {
                around(glyphs.iter().map(|g| (g.x, g.y)))
            }
        }
    }
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
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Annotation {
    /// Text picked out with the highlighter: one rect per line covered, which is
    /// why this is a list rather than a rect. A selection spanning three lines is
    /// one annotation, so erasing it takes one action rather than three.
    Highlight { rects: Vec<Rect>, color: Color },
    /// A line under the text.
    ///
    /// The same PDF construct as a highlight — *text markup*, a list of
    /// quadrilaterals over the words covered — differing only in subtype and in
    /// what a reader draws for it. Kept as separate variants rather than one
    /// with a `kind` field because that is how every caller thinks of them: an
    /// underline and a highlight are two tools on a toolbar, not one tool with a
    /// setting.
    Underline { rects: Vec<Rect>, color: Color },
    /// A line through the text.
    StrikeOut { rects: Vec<Rect>, color: Color },
    /// A wavy line under the text — conventionally "something is wrong here",
    /// where an underline means "look at this".
    Squiggly { rects: Vec<Rect>, color: Color },
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
    /// Words written onto the page, straight or along a curve.
    ///
    /// The one variant here that is **not** an annotation. The others are marks
    /// laid over a page; this becomes page content — real text objects — so a
    /// reader can select it, search it and copy it out. That is the whole reason
    /// for writing text rather than drawing letters.
    ///
    /// Every object written carries a marked-content tag naming this app and the
    /// mark's own id, and the first of them carries [`restore`] as well. That is
    /// what lets words be found again after any number of saves and taken back
    /// out — without it, text stopped being a mark the moment it was saved and
    /// the eraser could no longer touch it, which is exactly how a clouded
    /// caption came apart: the ring erased and the words stayed.
    ///
    /// The glyphs arrive already placed. The app walks the baseline with the
    /// font's own metrics and so does its preview; only one side can be the
    /// authority on where a letter sits, and it has to be the side the person was
    /// looking at when they put it there.
    Text {
        text: String,
        /// A standard-14 name, as `FPDFText_LoadStandardFont` expects.
        font: String,
        /// A font registered by the app, when the words need one.
        ///
        /// The standard-14 have no Arabic, no Devanagari, no CJK — nothing but
        /// Latin-1 — and they are not embedded, so there is nothing to add. A
        /// script they cannot draw needs a real font file in the file itself.
        #[serde(default)]
        font_asset: Option<String>,
        size: f32,
        color: Color,
        glyphs: Vec<Glyph>,
        /// The app's own id for this mark, tagged onto every object written.
        #[serde(default)]
        id: i32,
        /// Whatever the app needs in order to rebuild this mark when it reads the
        /// file again. Opaque here: the engine stores it and hands it back.
        #[serde(default)]
        restore: String,
        /// The ring drawn around the words, if any, as one closed polyline.
        ///
        /// Page content like the letters, and tagged with the same id, so the two
        /// are one thing in the file as well as on screen. Written as a separate
        /// ink annotation it was a separate mark the moment the file was reopened,
        /// and the eraser took the ring off a clouded caption and left the words.
        #[serde(default)]
        frame: Vec<Point>,
        /// How thick that ring is drawn, in points.
        #[serde(default)]
        frame_width: f32,
    },
}

/// One placed glyph: what it is, where it goes, and which way it leans.
///
/// In the app's space — page points from the top left, y downwards, angles
/// clockwise. The flip to PDF's bottom-left convention happens once, at the
/// PDFium boundary, exactly as it does for every other mark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Glyph {
    /// The character as a string: one glyph, but not always one `char`.
    ///
    /// Still what a standard-14 font is written from. For an embedded font it is
    /// carried anyway, because it is what the glyph *means* — the ToUnicode is
    /// built from it, and without that the words draw and cannot be copied.
    pub ch: String,
    /// The glyph's id in an embedded font, from the shaper.
    ///
    /// Zero where there is none, which is both "no embedded font" and glyph 0,
    /// the notdef box — a glyph nothing should ever deliberately draw.
    #[serde(default)]
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub radians: f32,
}

/// An annotation together with **PDFium's own index** for it on the page.
///
/// The index is not the annotation's position in the returned list, and the
/// difference is load-bearing: a page can hold annotations this engine does not
/// model, and those are skipped on read. Numbering our own results would address
/// the wrong annotation as soon as a page contained one — and a delete would take
/// out somebody's form field instead of their highlight.
/// A run of text on a page, as the file stores it.
///
/// **Not a line and not a word.** A PDF holds text in runs decided by whoever
/// produced the file: one may be a whole paragraph, or a single letter that
/// needed different spacing. Editing works on runs because that is the unit the
/// file has — anything finer means rewriting the content stream, and anything
/// coarser is a guess about which runs belong together.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// Position in the page's object list. Stable until objects are added or
    /// removed, which is why an edit reads the list again rather than
    /// remembering one.
    pub object: usize,
    pub text: String,
    /// Page points, top-left origin.
    pub rect: Rect,
    /// Where the text is drawn *from*: the left end of its baseline, in page
    /// points with a top-left origin.
    ///
    /// **Not the top-left of `rect`**, which sits above it by the font's
    /// ascent. This is the point [`TextStyle::at`] means, and passing the box's
    /// top instead moved every restyled run up by most of its own height — a
    /// line of a paragraph drawn over the line above it, reported from use
    /// with a screenshot. Carried here so a caller cannot have to guess.
    pub origin: Point,
    pub size: f32,
    /// The colour it is drawn in.
    pub color: Color,
}

/// Something sealed on a page, as the page needs to show it.
///
/// Deliberately carries no ciphertext: this is what draws a badge over the gap
/// an object left, and drawing a badge should not require holding the sealed
/// bytes in memory.
#[derive(Debug, Clone, PartialEq)]
pub struct LockedItem {
    /// What an unlock asks for.
    pub id: String,
    /// Where the thing sat, in page points with a top-left origin.
    pub rect: Rect,
    /// Whether this is a *region* of the page rather than one object.
    ///
    /// The two want different marks. A locked area was redacted, so the page
    /// already carries the black rectangle that says so and anything drawn over
    /// it would be a second answer to the same question. A locked image left a
    /// genuine hole, which needs the chequerboard every editor uses for
    /// "nothing here" — and that chequerboard must not be painted over an area,
    /// where it would hide the black mark rather than explain it.
    pub is_area: bool,
    /// **The picture this stands over is still on the page.**
    ///
    /// A lock is two things: a sealed copy and a badge, written first, and the
    /// picture taken off the page, done second. A failure in between — and
    /// there was one, on any page carrying two pictures of the same size —
    /// left the badge with nothing behind it to explain. Reported from use as
    /// a grey layer over a picture that could not be selected or sent back:
    /// the chequerboard the badge draws, over a picture that was never
    /// removed. See [`DocumentMut::repair_locks`].
    pub stale: bool,
}

/// An image on a page, as the file stores it.
///
/// Enough to show it as selectable, to decide how it could be sealed, and to
/// put it back afterwards — see [`crate::crypto::vault::SealedItem`].
#[derive(Debug, Clone, PartialEq)]
pub struct PageImage {
    /// Position in the page's object list, the same address
    /// [`TextRun::object`] uses.
    pub object: usize,
    /// Page points, top-left origin — where it is on screen.
    pub rect: Rect,
    /// The PDF matrix that placed it: `a b c d e f`.
    pub matrix: [f32; 6],
    /// Pixel dimensions of the image itself, which are not its size on the
    /// page: a 2000px photo may be drawn two inches wide.
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// How many bytes the PDF holds for it, already compressed.
    pub raw_bytes: usize,
    /// The stream filters, outermost first — `DCTDecode` for a JPEG,
    /// `FlateDecode` for a PNG-ish one. Empty when the pixels are stored raw.
    pub filters: Vec<String>,
}

/// What kind of thing is drawn on a page.
///
/// Deliberately coarse. The question a reader is asking of a layer list is
/// "which of these is the photograph and which is the caption", not which PDF
/// operator drew it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DrawnKind {
    Words,
    Picture,
    /// A path: a rule, a box, a background panel.
    Shape,
    /// A form XObject, a shading, or anything else with its own contents.
    Group,
}

impl DrawnKind {
    /// What to call it in a list somebody is reading.
    pub fn describe(&self) -> &'static str {
        match self {
            DrawnKind::Words => "words",
            DrawnKind::Picture => "picture",
            DrawnKind::Shape => "shape",
            DrawnKind::Group => "group",
        }
    }
}

/// One thing drawn on a page, in the order the page draws it.
///
/// **The order is the whole point.** A PDF has no z-index: what is drawn later
/// is on top, so the position in this list *is* the stacking. Index 0 is at the
/// bottom and the last entry is what covers everything else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrawnObject {
    /// Position in the page's object list — the address [`TextRun::object`] and
    /// [`PageImage::object`] use, and what [`DocumentMut::move_object`] and
    /// [`DocumentMut::restack`] take.
    pub object: usize,
    pub kind: DrawnKind,
    /// Page points, top-left origin.
    pub rect: Rect,
    /// Something to show in a list: the first few words, or the picture's size.
    pub label: String,
    /// How deep inside a group this sits. `0` is drawn by the page itself.
    ///
    /// **A page can put nearly everything inside one group.** A brochure laid
    /// out in a design program routinely draws a whole panel through a single
    /// form, and a list that stopped at the page's own objects said "group,
    /// 145 × 63 pt" and nothing else — reported from use as a page that looked
    /// like it had layers which could not be read.
    pub depth: usize,
    /// Whether the drawing order can be changed for this one.
    ///
    /// **False inside a group.** What a group draws is a stream of its own,
    /// shared by every place the page draws it — restacking something in there
    /// would move it everywhere the group appears, which is not what anybody
    /// clicking one of them means. They are listed so the page can be read, and
    /// the group itself is what moves.
    pub movable: bool,
}

/// Where to put something in the page's drawing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stacking {
    /// Drawn last, so it covers everything.
    Front,
    /// Drawn first, so everything covers it.
    Back,
    /// One step later in the order — over the thing that was just above it.
    Up,
    /// One step earlier — under the thing that was just below it.
    Down,
}

/// What an edit should change about a run, beyond its words.
///
/// Every field optional and every `None` meaning *leave it alone*. An edit that
/// silently reset what it was not asked to change is how white text came back
/// black: `FPDFText_SetText` writes the object afresh, and everything not
/// carried over is written as a default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub size: Option<f32>,
    pub color: Option<Color>,
    /// Where the run sits: the left end of its baseline, in page points with a
    /// top-left origin. Moves it; does not resize it.
    ///
    /// The same point [`TextRun::origin`] reports, and it must be taken from
    /// there rather than from the top of a run's box — see that field.
    pub at: Option<(f32, f32)>,
}

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
            font_size: 12.0,
            font_name: "Helvetica".into(),
            direction: layout::Direction::Ltr,
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

    // -- outlined_words_are_trustworthy -------------------------------------

    /// Only `glyph_paths` varies across these tests; everything else about
    /// `Page` is either unused by the default method under test or irrelevant
    /// to it, which is why the other three required methods are trivial stubs.
    struct FakePage {
        glyph_paths: usize,
    }

    impl Page for FakePage {
        fn size(&self) -> PageSize {
            PageSize { width_pt: 1.0, height_pt: 1.0 }
        }
        fn render_into(&self, _request: &RenderRequest, _target: &mut RenderTarget<'_>) -> Result<()> {
            unimplemented!("not used by outlined_words_are_trustworthy")
        }
        fn text(&self) -> Result<String> {
            unimplemented!("not used by outlined_words_are_trustworthy")
        }
        fn classify(&self) -> Result<PageClassification> {
            Ok(PageClassification {
                kind: PageTextKind::Outlined,
                chars: 0,
                unmappable: 0,
                image_coverage: 0.0,
                paths: self.glyph_paths,
                glyph_paths: self.glyph_paths,
                rotated_chars: 0,
            })
        }
    }

    fn word(text: &str) -> RecognisedWord {
        RecognisedWord {
            text: text.into(),
            rect: Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 },
            confidence: 1.0,
            char_confidence: Vec::new(),
        }
    }

    /// The page's own face, measured: 0.84–0.87 characters recognised per
    /// glyph-shaped path found. Comfortably above the threshold.
    #[test]
    fn a_same_face_match_is_trusted() {
        let page = FakePage { glyph_paths: 468 };
        // "matched_chars=407" from the real measurement this threshold is
        // calibrated against.
        let words: Vec<RecognisedWord> = vec![word(&"x".repeat(407))];
        assert!(page.outlined_words_are_trustworthy(&words).unwrap());
    }

    /// A plausible but wrong face, measured at a 0.27 ratio. Comfortably
    /// below the threshold, with the same page and glyph count as the
    /// trusted case above — only the match quality differs.
    #[test]
    fn a_wrong_face_match_is_not_trusted() {
        let page = FakePage { glyph_paths: 468 };
        let words: Vec<RecognisedWord> = vec![word(&"x".repeat(127))];
        assert!(!page.outlined_words_are_trustworthy(&words).unwrap());
    }

    #[test]
    fn no_words_at_all_is_not_trusted() {
        let page = FakePage { glyph_paths: 468 };
        assert!(!page.outlined_words_are_trustworthy(&[]).unwrap());
    }

    /// A page with no glyph-shaped paths at all cannot divide by zero its way
    /// into looking trustworthy.
    #[test]
    fn zero_glyph_paths_does_not_panic_or_falsely_pass() {
        let page = FakePage { glyph_paths: 0 };
        let words = vec![word("anything")];
        assert!(!page.outlined_words_are_trustworthy(&words).unwrap());
    }

    /// The threshold itself, exercised at its own boundary — half of
    /// `glyph_paths` recognised is the cutover point by construction.
    #[test]
    fn the_threshold_is_inclusive_at_exactly_half() {
        let page = FakePage { glyph_paths: 100 };
        assert!(page.outlined_words_are_trustworthy(&[word(&"x".repeat(50))]).unwrap());
        assert!(!page.outlined_words_are_trustworthy(&[word(&"x".repeat(49))]).unwrap());
    }
}

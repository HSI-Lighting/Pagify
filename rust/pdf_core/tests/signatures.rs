//! Placed signatures, and burning them into the page.
//!
//! # What is being checked
//!
//! A placed signature is an ink annotation — a mark laid over a page, which
//! anybody can select and delete. Applying it makes it part of the page.
//!
//! Three things have to hold, and the third is the one this project has been
//! caught by before:
//!
//! 1. A signature is told apart from a drawing. Flattening somebody's markup
//!    because it looked like a signature would be a poor way to lose it.
//! 2. Applying leaves the mark visible and the annotation gone.
//! 3. **The rest of the page is untouched.** Every path that goes through
//!    `FPDFPage_GenerateContent` re-emits the whole content stream and rewrites
//!    text nobody asked it to touch. This one must not.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test signatures
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, DocumentMut, Point};

const INK: Color = Color { r: 0x14, g: 0x2B, b: 0x63, a: 0xFF };

/// A scrawl sitting on the lower half of a page.
fn scrawl(at: f32) -> Vec<Vec<Point>> {
    vec![
        vec![
            Point { x: 100.0, y: at },
            Point { x: 130.0, y: at - 30.0 },
            Point { x: 160.0, y: at },
            Point { x: 200.0, y: at - 34.0 },
        ],
        vec![Point { x: 110.0, y: at - 6.0 }, Point { x: 210.0, y: at - 6.0 }],
    ]
}

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// Place ink and say it is a signature, the way the app does.
fn place(doc: &mut PdfiumDocument, page: usize, at: f32, name: &str) -> usize {
    let index = doc
        .add_annotation(
            page,
            &Annotation::Ink { strokes: scrawl(at), color: INK, width: 1.6 },
        )
        .expect("place");
    doc.mark_as_signature(page, index, name).expect("mark");
    index
}

/// What the page draws, as a fraction of a small render that is not white.
fn ink_on(doc: &dyn Document, page: usize) -> f32 {
    let size = doc.page_size(page).expect("size");
    let width = 200u32;
    let scale = width as f32 / size.width_pt;
    let height = (size.height_pt * scale).max(1.0) as u32;

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut target = pdf_core::render::RenderTarget {
        width,
        height,
        stride: (width * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba,
        pixels: &mut pixels,
    };
    doc.page(page)
        .expect("page")
        .render_into(
            &pdf_core::document::RenderRequest { scale, ..Default::default() },
            &mut target,
        )
        .expect("render");
    let inked = pixels
        .chunks_exact(4)
        .filter(|p| p[0] < 240 || p[1] < 240 || p[2] < 240)
        .count();
    inked as f32 / (width * height) as f32
}

fn text_on(doc: &dyn Document, page: usize) -> String {
    doc.page(page).expect("page").text().unwrap_or_default()
}

/// **A signature is told apart from a drawing.**
#[test]
fn only_ink_that_was_marked_reads_back_as_a_signature() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    // A drawing, left unmarked — somebody's markup.
    doc.add_annotation(
        0,
        &Annotation::Ink { strokes: scrawl(600.0), color: Color { r: 255, g: 0, b: 0, a: 255 }, width: 2.0 },
    )
    .expect("draw");
    place(&mut doc, 0, 400.0, "mine");

    let found = doc.signature_marks(0).expect("read");
    assert_eq!(found.len(), 1, "expected one signature, got {found:?}");
    assert_eq!(found[0].name, "mine");
    assert_eq!(found[0].strokes.len(), 2, "a stroke went missing");
    assert_eq!(doc.annotations(0).expect("read").len(), 2, "both marks should be on the page");
}

/// And it survives being written out and opened again — which is the reason it
/// is recorded in the file rather than remembered in the program.
#[test]
fn a_signature_is_still_a_signature_after_a_save() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place(&mut doc, 0, 400.0, "after the save");

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");

    let found = reopened.signature_marks(0).expect("read");
    assert_eq!(found.len(), 1, "the mark did not survive the save");
    assert_eq!(found[0].name, "after the save");
}

/// **Applying makes it part of the page: the annotation is gone and the mark
/// is still drawn.**
#[test]
fn applying_leaves_the_mark_and_removes_the_annotation() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let bare = ink_on(&doc, 0);
    place(&mut doc, 0, 400.0, "mine");
    let with_annotation = ink_on(&doc, 0);
    assert!(with_annotation > bare, "the signature was not drawn at all");

    assert_eq!(doc.apply_signatures(0).expect("apply"), 1);

    // Nothing left to select or delete.
    assert!(doc.signature_marks(0).expect("read").is_empty());
    assert!(
        doc.annotations(0).expect("read").is_empty(),
        "the annotation is still there: {:?}",
        doc.annotations(0).expect("read")
    );

    // And it is still on the page, because it is the page now.
    let applied = ink_on(&doc, 0);
    assert!(
        (applied - with_annotation).abs() < 0.004,
        "the mark changed when it was applied: {with_annotation} then {applied}"
    );
    assert!(applied > bare, "applying rubbed the signature out: {applied} vs {bare}");
}

/// **The check this project keeps having to make.**
///
/// Every route through `FPDFPage_GenerateContent` re-emits the whole content
/// stream and rewrites text nobody touched. Applying a signature appends to the
/// stream through this crate's own writer instead, and the page's words have to
/// come back byte for byte.
#[test]
fn applying_a_signature_does_not_touch_the_words_on_the_page() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_on(&doc, 0);
    assert!(!before.is_empty(), "the fixture has no text to protect");

    place(&mut doc, 0, 400.0, "mine");
    doc.apply_signatures(0).expect("apply");

    assert_eq!(text_on(&doc, 0), before, "applying a signature rewrote the page's text");

    // And after a save, which is where a re-emission would surface.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    assert_eq!(text_on(&reopened, 0), before, "the saved file's text differs");
    assert!(reopened.signature_marks(0).expect("read").is_empty());
}

/// A page with nothing to apply is not an error, and does not touch the page.
#[test]
fn applying_nothing_changes_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = text_on(&doc, 0);
    assert_eq!(doc.apply_signatures(0).expect("apply"), 0);
    assert_eq!(text_on(&doc, 0), before);
}

/// **Only ink can be a signature.** Marking a highlight would have it flattened
/// into the page by a tool the user believed was touching their own name.
#[test]
fn a_highlight_cannot_be_marked_as_a_signature() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let index = doc
        .add_annotation(
            0,
            &Annotation::Highlight {
                rects: vec![pdf_core::document::Rect {
                    left: 100.0,
                    top: 100.0,
                    right: 200.0,
                    bottom: 120.0,
                }],
                color: Color { r: 255, g: 214, b: 0, a: 255 },
            },
        )
        .expect("highlight");

    assert!(doc.mark_as_signature(0, index, "not mine").is_err());
    assert!(doc.signature_marks(0).expect("read").is_empty());
}

/// Several signatures on one page all go in, and every one of them survives.
#[test]
fn every_signature_on_a_page_is_applied() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place(&mut doc, 0, 300.0, "first");
    place(&mut doc, 0, 400.0, "second");
    place(&mut doc, 0, 500.0, "third");
    let before = ink_on(&doc, 0);

    assert_eq!(doc.apply_signatures(0).expect("apply"), 3);
    assert!(doc.annotations(0).expect("read").is_empty());
    let after = ink_on(&doc, 0);
    assert!(
        (after - before).abs() < 0.004,
        "not all three came through: {before} then {after}"
    );
}

/// **How far an object reaches, for the case where a word is part of it.**
///
/// A heading converted to outlines is often one path holding every letter. A
/// rectangle around one word crosses that path rather than containing it, and a
/// crossed path stays on the page — so a caller told which object is in the way
/// has to be able to ask how far it goes.
#[test]
fn an_objects_bounds_can_be_measured() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("two-column.pdf");
    let size = doc.page_size(0).expect("size");

    let bounds = doc.object_bounds(0, 0).expect("the first object has bounds");
    assert!(bounds.right > bounds.left, "an empty box: {bounds:?}");
    assert!(bounds.bottom > bounds.top, "an empty box: {bounds:?}");
    assert!(
        bounds.right <= size.width_pt + 1.0 && bounds.bottom <= size.height_pt + 1.0,
        "the object reaches outside its own page: {bounds:?}"
    );

    // And an object that is not there is said so, rather than measured as zero.
    assert!(doc.object_bounds(0, 100_000).is_err());
}

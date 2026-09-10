//! Moving one thing on a page without disturbing the rest of it.
//!
//! # Why this is measured rather than assumed
//!
//! Moving an object means PDFium's object model, and committing a change to it
//! means `FPDFPage_GenerateContent` — which re-emits the whole content stream.
//! This crate avoids that everywhere it can, because it has been caught
//! rewriting paragraphs nobody touched: a signature covering 1458 bytes of a
//! 17826-byte re-save, a footer drawn twice, a line of a paragraph landing on
//! the line above it. Every one of those was a re-emission.
//!
//! So before the app offered a move at all, this asked what a move costs. The
//! answer, on a page of two columns: the object goes exactly where it is sent,
//! every other run stays where it was, and the page keeps the same words.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test moving
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point};

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// **It goes where it is sent.**
#[test]
fn an_object_moves_by_exactly_the_distance_asked_for() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let was = doc.object_bounds(0, 0).expect("bounds");
    doc.move_object(0, 0, Point { x: 20.0, y: 12.0 }).expect("move");
    let now = doc.object_bounds(0, 0).expect("bounds");

    assert!((now.left - was.left - 20.0).abs() < 0.01, "across: {was:?} then {now:?}");
    // Page space counts downwards, so a positive move goes down the page.
    assert!((now.top - was.top - 12.0).abs() < 0.01, "down: {was:?} then {now:?}");
}

/// **And nothing else does.** The whole reason this is measured.
#[test]
fn moving_one_object_leaves_every_other_run_where_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    for object in [0usize, 3, 7] {
        let doc = open("two-column.pdf");
        let before = doc.text_runs(0).expect("runs");
        drop(doc);

        let mut doc = open("two-column.pdf");
        doc.move_object(0, object, Point { x: 20.0, y: 12.0 }).expect("move");
        let after = doc.text_runs(0).expect("runs");

        assert_eq!(after.len(), before.len(), "moving object {object} changed the run count");
        for (was, now) in before.iter().zip(after.iter()) {
            if was.object == object {
                continue;
            }
            assert!(
                (was.rect.left - now.rect.left).abs() < 0.1
                    && (was.rect.top - now.rect.top).abs() < 0.1,
                "moving object {object} shifted another run: {:?} then {:?}",
                was.rect,
                now.rect
            );
        }

        // Reading order follows position, so moving a *text* object reorders
        // the page — correctly. What must not change is which words are on it.
        let mut spoken: Vec<String> = before.iter().map(|r| r.text.trim().to_string()).collect();
        let mut now: Vec<String> = after.iter().map(|r| r.text.trim().to_string()).collect();
        spoken.sort();
        now.sort();
        assert_eq!(spoken, now, "moving object {object} changed the page's words");
    }
}

/// **A move that would rewrite the page is undone rather than kept.**
///
/// Committing a move needs `FPDFPage_GenerateContent`, which re-emits the whole
/// content stream — and on a real catalogue that came back scrambled:
/// `HSI Lighting I HUE . SATURATION` as `ABOThe Boer T UOur e xpiunpics`, the
/// page's vertically-set heading broken into pieces. Reported from use, with
/// screenshots, *after* the feature had been offered.
///
/// **No fixture here reproduces it** — every one of them is simple enough that
/// PDFium round-trips it cleanly, which is exactly why the first measurement
/// said moving was safe. So this checks the half that can be checked: a move
/// that goes through leaves the page's words alone. The other half is the guard
/// itself, and the evidence for it is a real document.
#[test]
fn a_move_that_goes_through_has_not_rewritten_anything() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    for name in ["two-column.pdf", "text-lines.pdf", "spread.pdf", "mixed-sizes.pdf"] {
        let doc = open(name);
        if doc.text_runs(0).map(|r| r.is_empty()).unwrap_or(true) {
            continue;
        }
        let mut before: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        drop(doc);

        let mut doc = open(name);
        if doc.move_object(0, 0, Point { x: 10.0, y: 6.0 }).is_err() {
            // The guard refused, which is the other honest answer.
            continue;
        }
        let mut after: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after, "{name}: a move that was allowed still rewrote the page");
    }
}

/// An object that is not there is said so, rather than moving something else.
#[test]
fn moving_an_object_that_is_not_there_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    assert!(doc.move_object(0, 100_000, Point { x: 1.0, y: 1.0 }).is_err());
    assert!(doc.move_object(99, 0, Point { x: 1.0, y: 1.0 }).is_err());
}

/// Moving marks the document as owing a save, like any other change.
#[test]
fn moving_something_counts_as_a_change() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    assert!(!doc.is_dirty(), "a freshly opened document already looks changed");
    doc.move_object(0, 0, Point { x: 5.0, y: 0.0 }).expect("move");
    assert!(doc.is_dirty(), "a move was not counted as work owing a save");
}

/// **A picture moves without the page being re-emitted at all.**
///
/// Reported from use, with screenshots: moving something rearranged the page
/// around it — a heading broken into pieces, blocks of text overlapping, a
/// black rectangle where an image had been. The cause was
/// `FPDFPage_GenerateContent`, which every move through PDFium's object model
/// has to end with.
///
/// A picture is drawn by one operator, so wrapping *that* operator in its own
/// `q … Q` moves it and can reach nothing else — `Q` puts the graphics state
/// back exactly as it found it, and every other byte is copied through.
///
/// The distance is carried back through the transform in force at the operator.
/// Written naively it lands inside the picture's own scaling: measured, twenty
/// points came out as three thousand nine hundred.
#[test]
fn a_picture_moves_exactly_and_the_page_around_it_does_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut tried = 0;
    for name in ["scan-300dpi.pdf", "scan-lowdpi.pdf", "two-column.pdf", "spread.pdf"] {
        let doc = open(name);
        let pictures = doc.images_on(0).unwrap_or_default();
        let Some(picture) = pictures.first().cloned() else { continue };
        let mut before: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        drop(doc);
        tried += 1;

        let mut doc = open(name);
        doc.move_object(0, picture.object, Point { x: 20.0, y: 12.0 })
            .unwrap_or_else(|e| panic!("{name}: a picture would not move — {e}"));

        let moved = doc
            .images_on(0)
            .expect("images")
            .into_iter()
            .find(|i| i.object == picture.object)
            .unwrap_or_else(|| panic!("{name}: the picture vanished"));
        assert!(
            (moved.rect.left - picture.rect.left - 20.0).abs() < 0.5,
            "{name}: went {:.2} across, not 20",
            moved.rect.left - picture.rect.left
        );
        assert!(
            (moved.rect.top - picture.rect.top - 12.0).abs() < 0.5,
            "{name}: went {:.2} down, not 12",
            moved.rect.top - picture.rect.top
        );

        let mut after: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after, "{name}: moving a picture rewrote the page's words");
    }
    assert!(tried > 0, "no fixture here has a picture on its first page");
}

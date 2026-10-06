//! Turning one thing on a page, about a point, clockwise as it is seen.
//!
//! Measured on the real thing: a picture, and a run of words, turned through the
//! content stream and read back through PDFium. What is asserted is where the
//! thing ends up — not that a matrix was written.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test rotating
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point, Rect};

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None).expect("open")
}

fn size(r: &Rect) -> (f32, f32) {
    ((r.right - r.left).abs(), (r.bottom - r.top).abs())
}

fn near(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < 0.6, "{what}: {a} is not {b}");
}

/// **Turned clockwise a quarter about its own top-left corner, a picture that
/// stretched right and down now stretches down and left.** Which way is the
/// whole question — page space counts upwards, the screen downwards, and a
/// sign slip turns it the wrong way and is invisible to a test that only
/// checks sizes.
#[test]
fn a_quarter_turn_clockwise_about_the_top_left_corner_swings_the_picture_down_and_left() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    let picture = doc.images_on(0).expect("images").remove(0);
    let (w, h) = size(&picture.rect);
    let pivot = Point { x: picture.rect.left, y: picture.rect.top };

    doc.rotate_object(0, picture.object, pivot, 90.0).expect("rotate");

    let now = doc.object_bounds(0, picture.object).expect("bounds");
    near(now.right, pivot.x, "its right edge is where the pivot was");
    near(now.left, pivot.x - h, "it reaches left by what was its height");
    near(now.top, pivot.y, "its top is still at the pivot");
    near(now.bottom, pivot.y + w, "it reaches down by what was its width");
}

#[test]
fn turning_about_the_middle_keeps_the_middle_and_swaps_the_sides() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    let picture = doc.images_on(0).expect("images").remove(0);
    let (w, h) = size(&picture.rect);
    let middle = Point {
        x: (picture.rect.left + picture.rect.right) / 2.0,
        y: (picture.rect.top + picture.rect.bottom) / 2.0,
    };

    doc.rotate_object(0, picture.object, middle, 90.0).expect("rotate");
    let now = doc.object_bounds(0, picture.object).expect("bounds");
    let (nw, nh) = size(&now);
    near(nw, h, "width");
    near(nh, w, "height");
    near((now.left + now.right) / 2.0, middle.x, "middle across");
    near((now.top + now.bottom) / 2.0, middle.y, "middle down");
}

/// Undo is the same turn the other way about the same point.
#[test]
fn turning_there_and_back_puts_it_where_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    let picture = doc.images_on(0).expect("images").remove(0);
    let before = doc.object_bounds(0, picture.object).expect("bounds");
    let middle = Point { x: (before.left + before.right) / 2.0, y: (before.top + before.bottom) / 2.0 };

    doc.rotate_object(0, picture.object, middle, 33.0).expect("rotate");
    let turned = doc.object_bounds(0, picture.object).expect("bounds");
    assert!(size(&turned).0 > size(&before).0 + 0.5 || size(&turned).1 > size(&before).1 + 0.5, "it did not turn: {before:?} then {turned:?}");
    doc.rotate_object(0, picture.object, middle, -33.0).expect("rotate back");

    let after = doc.object_bounds(0, picture.object).expect("bounds");
    near(after.left, before.left, "left");
    near(after.top, before.top, "top");
    near(after.right, before.right, "right");
    near(after.bottom, before.bottom, "bottom");
}

#[test]
fn a_whole_turn_or_none_changes_nothing_and_a_bad_angle_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("pictures.pdf");
    let picture = doc.images_on(0).expect("images").remove(0);
    let before = doc.object_bounds(0, picture.object).expect("bounds");
    let middle = Point { x: 100.0, y: 100.0 };
    doc.rotate_object(0, picture.object, middle, 0.0).expect("nothing");
    doc.rotate_object(0, picture.object, middle, 360.0).expect("a whole turn");
    assert_eq!(doc.object_bounds(0, picture.object).expect("bounds"), before);
    assert!(doc.rotate_object(0, picture.object, middle, f32::NAN).is_err());
    assert!(doc.rotate_object(0, 100_000, middle, 10.0).is_err());
}

/// Words turn too — or are refused with a reason, which is the honest answer for
/// a run that shares its text object with others. They are never turned wrongly.
#[test]
fn a_run_of_words_turns_about_its_own_middle_or_says_why_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let run = doc.text_runs(0).expect("runs").remove(0);
    let (w, h) = size(&run.rect);
    let middle = Point { x: (run.rect.left + run.rect.right) / 2.0, y: (run.rect.top + run.rect.bottom) / 2.0 };

    // Whichever route it takes: a matrix round it, or PDFium's own object.
    match doc.rotate_object(0, run.object, middle, 90.0) {
        Ok(()) => {
            let now = doc.object_bounds(0, run.object).expect("bounds");
            let (nw, nh) = size(&now);
            assert!((nw - h).abs() < 1.5 && (nh - w).abs() < 1.5, "a turned run keeps its extent swapped: {:?} then {now:?}", run.rect);
            near((now.left + now.right) / 2.0, middle.x, "middle across");
            near((now.top + now.bottom) / 2.0, middle.y, "middle down");
        }
        Err(e) => eprintln!("refused, which is allowed: {e}"),
    }
}

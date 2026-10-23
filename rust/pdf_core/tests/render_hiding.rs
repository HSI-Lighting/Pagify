//! A page rendered with some of its objects left out, and the document untouched.
//!
//! # What this guards
//!
//! Edit Text shows the page *without* the paragraph being retyped under the editor,
//! so the old words do not show through the new ones. That picture is made from a
//! page loaded afresh for the purpose, with the objects taken off it in memory:
//! the document, its undo history and its saved bytes must not move.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test render_hiding
//! ```

mod harness;
use harness::{helvetica_page, serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Rect, RenderRequest, TextRun};
use pdf_core::Bitmap;

const SCALE: f32 = 2.0;

/// Two words on one line, each its own text object, far apart.
const PAGE: &[u8] = b"BT /F1 24 Tf 72 700 Td (Hello) Tj ET BT /F1 24 Tf 300 700 Td (World) Tj ET";

fn open() -> PdfiumDocument {
    PdfiumDocument::open_bytes(helvetica_page(PAGE), None).expect("open")
}

fn request() -> RenderRequest {
    RenderRequest { scale: SCALE, ..Default::default() }
}

fn word(doc: &PdfiumDocument, text: &str) -> TextRun {
    doc.text_runs(0).expect("runs").into_iter().find(|r| r.text.contains(text)).expect("the word")
}

/// The darkest channel of the pixel at (`x`, `y`).
fn darkness(bitmap: &Bitmap, x: u32, y: u32) -> u8 {
    let at = y as usize * bitmap.stride + x as usize * 4;
    bitmap.data[at].min(bitmap.data[at + 1]).min(bitmap.data[at + 2])
}

/// Whether the pixel (`x`, `y`) lies in `rect` (page points) grown by `pad` points.
fn within(rect: &Rect, pad: f32, x: u32, y: u32) -> bool {
    let (px, py) = (x as f32 / SCALE, y as f32 / SCALE);
    px >= rect.left - pad && px <= rect.right + pad && py >= rect.top - pad && py <= rect.bottom + pad
}

/// A full copy of the document, without its trailer `/ID`: PDFium writes a fresh random
/// one with every save, so two saves of an untouched document differ by that alone.
fn full_copy(doc: &mut PdfiumDocument) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    if let Some(at) = bytes.windows(4).position(|w| w == b"/ID[") {
        if let Some(len) = bytes[at..].iter().position(|b| *b == b']') {
            bytes.drain(at..=at + len);
        }
    }
    bytes
}

#[test]
fn a_hidden_object_is_not_drawn_and_nothing_else_changes() {
    if skip_without_pdfium().is_none() {
        return;
    }
    let _lock = serial();
    let doc = open();
    let hello = word(&doc, "Hello");
    let plain = doc.render_page_to_bitmap(0, &request()).expect("plain render");
    let hidden = doc.render_page_hiding(0, &request(), &[hello.object]).expect("hiding render");
    assert_eq!((hidden.width, hidden.height), (plain.width, plain.height));

    let mut had_ink = false;
    let mut left_ink = 0;
    let mut changed_elsewhere = 0;
    for y in 0..plain.height {
        for x in 0..plain.width {
            if within(&hello.rect, 1.0, x, y) {
                had_ink |= darkness(&plain, x, y) < 128;
                if darkness(&hidden, x, y) < 250 {
                    left_ink += 1;
                }
            } else if !within(&hello.rect, 3.0, x, y) && darkness(&plain, x, y) != darkness(&hidden, x, y) {
                changed_elsewhere += 1;
            }
        }
    }
    assert!(had_ink, "the plain render must show the word, or this test proves nothing");
    assert_eq!(left_ink, 0, "pixels of the hidden word that are still inked");
    assert_eq!(changed_elsewhere, 0, "pixels outside the hidden word that changed");
}

#[test]
fn hiding_leaves_the_document_as_it_was() {
    if skip_without_pdfium().is_none() {
        return;
    }
    let _lock = serial();
    let mut doc = open();
    let hello = word(&doc, "Hello").object;
    let (bytes, plain, runs) = (full_copy(&mut doc), doc.render_page_to_bitmap(0, &request()).expect("plain"), doc.text_runs(0).expect("runs").len());

    doc.render_page_hiding(0, &request(), &[hello]).expect("hiding render");

    assert_eq!(doc.text_runs(0).expect("runs").len(), runs, "text objects on the page");
    assert_eq!(doc.render_page_to_bitmap(0, &request()).expect("plain").data, plain.data, "the page as drawn");
    assert_eq!(full_copy(&mut doc), bytes, "the saved bytes");
}

#[test]
fn an_index_past_the_last_object_is_refused_and_removes_nothing() {
    if skip_without_pdfium().is_none() {
        return;
    }
    let _lock = serial();
    let doc = open();
    let hello = word(&doc, "Hello").object;
    let plain = doc.render_page_to_bitmap(0, &request()).expect("plain");

    assert!(doc.render_page_hiding(0, &request(), &[999]).is_err());
    // A good index beside a bad one: the bad one must be found before anything is taken off.
    assert!(doc.render_page_hiding(0, &request(), &[hello, 999]).is_err());
    assert_eq!(doc.render_page_to_bitmap(0, &request()).expect("plain").data, plain.data);
}

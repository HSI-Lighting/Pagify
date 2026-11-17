//! **Flatten**: every visible mark made part of its page, as Foxit's Flatten
//! Pages does — for a file that must look the same where marks are not drawn at
//! all, which is Android's and WhatsApp's previews.
//!
//! Replaced Save for sharing on 2026-11-17. Foxit and Acrobat keep marks as
//! marks on Save and flatten only on request; the owner chose the same, with
//! Flatten acting on the open document and undoable until the file is saved.
//! A placed picture goes down as a plain image, because the engine moves and
//! removes only pictures, shapes and words — measured before Save for sharing
//! was written: a form there could not be moved once anything else had been,
//! nor removed at all.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test flatten_marks
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, DocumentMut, DrawnKind, Point, Rect, RenderRequest};
use pdf_core::pdf::encrypt::Permissions;
use pdf_core::render::bitmap::PixelOrder;

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None).expect("open")
}

const PICTURE: Rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };
const HIGHLIGHT: Rect = Rect { left: 300.0, top: 400.0, right: 420.0, bottom: 416.0 };

/// A solid red picture, so the one placed is never mistaken for the page's own.
fn place_picture(doc: &mut PdfiumDocument) {
    let rgba = [255u8, 0, 0, 255].repeat(16);
    doc.add_annotation(0, &Annotation::Image { rect: PICTURE, rgba, width: 4, height: 4 }).expect("place the picture");
}

fn highlight(doc: &mut PdfiumDocument) {
    let color = Color { r: 255, g: 235, b: 0, a: 255 };
    doc.add_annotation(0, &Annotation::Highlight { rects: vec![HIGHLIGHT], color }).expect("highlight");
}

/// The colour at the middle of `rect`, with marks drawn or not.
fn colour_at(doc: &PdfiumDocument, rect: Rect, annotations: bool) -> (u8, u8, u8) {
    let bitmap = doc
        .render_page_to_bitmap(0, &RenderRequest { scale: 1.0, render_annotations: annotations, ..Default::default() })
        .expect("render");
    let (x, y) = (((rect.left + rect.right) / 2.0) as usize, ((rect.top + rect.bottom) / 2.0) as usize);
    let at = y * bitmap.stride + x * 4;
    let p = &bitmap.data[at..at + 4];
    match bitmap.order {
        PixelOrder::Bgra => (p[2], p[1], p[0]),
        PixelOrder::Rgba => (p[0], p[1], p[2]),
    }
}

fn red((r, g, b): (u8, u8, u8)) -> bool {
    r > 200 && g < 60 && b < 60
}

fn yellow((r, g, b): (u8, u8, u8)) -> bool {
    r > 200 && g > 180 && b < 120
}

/// Where the placed picture is, within a point either way.
fn near(a: Rect, b: Rect) -> bool {
    [(a.left, b.left), (a.top, b.top), (a.right, b.right), (a.bottom, b.bottom)].iter().all(|(x, y)| (x - y).abs() < 1.0)
}

/// The document as a file, reopened — a fresh copy for each edit tried below.
fn reopened(doc: &mut PdfiumDocument) -> PdfiumDocument {
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    PdfiumDocument::open_bytes(bytes, None).expect("reopen")
}

/// **The reported break, end to end.** A preview draws the page and not what
/// is laid over it, which is `render_annotations: false`. Before Flatten the
/// picture and the highlight are gone there; after it they are drawn.
#[test]
fn after_flatten_marks_are_drawn_where_annotations_are_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place_picture(&mut doc);
    highlight(&mut doc);
    // **Checked on a copy.** Drawing a page with its marks makes PDFium write
    // an appearance into each mark that has none, in memory — and then the
    // flatten would never meet a highlight without one, the case Pagify's own
    // highlights are in when their page was not drawn by PDFium.
    let copy = reopened(&mut doc);
    assert!(red(colour_at(&copy, PICTURE, true)), "setup: the picture is not drawn even with marks on");
    assert!(!red(colour_at(&copy, PICTURE, false)), "setup: without marks the picture is still drawn");
    assert!(!yellow(colour_at(&copy, HIGHLIGHT, false)), "setup: without marks the highlight is still drawn");

    doc.flatten_marks().expect("flatten");

    assert!(red(colour_at(&doc, PICTURE, false)), "the picture is still only a mark");
    assert!(yellow(colour_at(&doc, HIGHLIGHT, false)), "the highlight is still only a mark");
}

/// **The acceptance test.** What Flatten leaves where the picture was placed is
/// a picture — not a group, not a mark — and the engine's own move and remove
/// take it, including after another move, the order that refused a form.
#[test]
fn a_flattened_picture_is_a_picture_that_moves_and_removes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let marks_before = doc.annotation_count(0).expect("count");
    place_picture(&mut doc);
    doc.flatten_marks().expect("flatten");
    assert_eq!(doc.annotation_count(0).expect("count"), marks_before, "the picture is still a mark");

    let drawn = doc.drawn_objects(0).expect("drawn");
    let object = drawn
        .iter()
        .find(|o| o.kind == DrawnKind::Picture && o.depth == 0 && near(o.rect, PICTURE))
        .unwrap_or_else(|| panic!("no picture where it was placed; the page draws {drawn:?}"))
        .object;

    let mut moved = reopened(&mut doc);
    moved.move_object(0, object, Point { x: 30.0, y: 20.0 }).expect("the picture moves");
    let after = moved.images_on(0).expect("images").into_iter().find(|i| i.object == object).expect("still there");
    assert!(near(after.rect, Rect { left: 130.0, top: 220.0, right: 210.0, bottom: 260.0 }), "moved to {:?}", after.rect);

    let mut removed = reopened(&mut doc);
    removed.remove_object(0, object).expect("the picture is removed");
    assert!(removed.images_on(0).expect("images").iter().all(|i| i.object != object), "it is still drawn");

    let mut second = reopened(&mut doc);
    let other = second.drawn_objects(0).expect("drawn").into_iter().find(|o| o.kind == DrawnKind::Shape && o.depth == 0);
    if let Some(other) = other {
        let _ = second.move_object(0, other.object, Point { x: 5.0, y: 5.0 });
    }
    second.move_object(0, object, Point { x: 30.0, y: 20.0 }).expect("the picture moves after another move");
}

/// **Undoable until saved.** Undo puts every mark back as a mark — the placed
/// picture with its handles — and redo flattens again.
#[test]
fn undo_puts_every_mark_back_and_redo_flattens_again() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let base = doc.annotation_count(0).expect("count");
    place_picture(&mut doc);
    highlight(&mut doc);
    let marks = doc.annotation_count(0).expect("count");
    let mut history = CommandHistory::default();

    history.execute(Command::FlattenMarks, &mut doc).expect("flatten");
    assert_eq!(doc.annotation_count(0).expect("count"), base, "marks are left after Flatten");

    history.undo(&mut doc).expect("undo");
    assert_eq!(doc.annotation_count(0).expect("count"), marks, "undo did not put the marks back");
    assert_eq!(doc.placed_image_marks(0).expect("placed").len(), 1, "the picture came back without its handles");
    assert!(!red(colour_at(&doc, PICTURE, false)), "after undo the picture is still part of the page");

    history.redo(&mut doc).expect("redo");
    assert_eq!(doc.annotation_count(0).expect("count"), base, "redo did not flatten again");
}

/// **The break:** flattening a document with a password writes it out without
/// one — the way the copy for sharing could have leaked it, and why that copy
/// refused. Flatten keeps the password.
#[test]
fn a_document_with_a_password_keeps_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place_picture(&mut doc);
    doc.secure_document(b"correct horse", None, Permissions::all()).expect("a password is set");
    doc.flatten_marks().expect("flatten");

    let mut saved = Vec::new();
    doc.save_full_copy(&mut saved).expect("save");
    assert!(PdfiumDocument::open_bytes(saved.clone(), None).is_err(), "the file has no password");
    let opened = PdfiumDocument::open_bytes(saved, Some("correct horse")).expect("it opens with its password");
    assert!(red(colour_at(&opened, PICTURE, false)), "the picture is not part of the page");
}

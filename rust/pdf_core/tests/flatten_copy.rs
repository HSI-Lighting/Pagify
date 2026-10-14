//! A copy of a document for sending: its placed pictures baked into the page.
//!
//! Reported from use: a picture placed in Pagify showed as the grey box under
//! it in WhatsApp's preview, because a placed picture is an annotation and a
//! preview draws the page. The copy writes it into the page — as a plain image,
//! because the engine moves and removes only pictures, shapes and words, and a
//! form (what `qpdf --flatten-annotations` writes) is none of those. Measured on
//! a real datasheet before this was written: a form there could not be moved
//! once anything else had been, nor removed at all; a plain image could be both.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test flatten_copy
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, DocumentMut, DrawnKind, Point, Rect, RenderRequest};
use pdf_core::pdf::encrypt::Permissions;
use pdf_core::render::bitmap::PixelOrder;

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None).expect("open")
}

/// A solid red picture, so the one placed is never mistaken for the page's own.
fn red(width: u32, height: u32) -> Vec<u8> {
    [255u8, 0, 0, 255].repeat((width * height) as usize)
}

fn place_picture(doc: &mut PdfiumDocument, rect: Rect) {
    doc.add_annotation(0, &Annotation::Image { rect, rgba: red(4, 4), width: 4, height: 4 })
        .expect("place the picture");
}

/// Whether the middle of `rect` is drawn red — the colour of the picture placed
/// — with annotations drawn or not.
fn middle_is_red(doc: &PdfiumDocument, rect: Rect, annotations: bool) -> bool {
    let bitmap = doc
        .render_page_to_bitmap(0, &RenderRequest { scale: 1.0, render_annotations: annotations, ..Default::default() })
        .expect("render");
    let (x, y) = (((rect.left + rect.right) / 2.0) as usize, ((rect.top + rect.bottom) / 2.0) as usize);
    let at = y * bitmap.stride + x * 4;
    let p = &bitmap.data[at..at + 4];
    let (r, g, b) = match bitmap.order {
        PixelOrder::Bgra => (p[2], p[1], p[0]),
        PixelOrder::Rgba => (p[0], p[1], p[2]),
    };
    r > 200 && g < 60 && b < 60
}

/// Where the placed picture is, within a point either way.
fn near(a: Rect, b: Rect) -> bool {
    [(a.left, b.left), (a.top, b.top), (a.right, b.right), (a.bottom, b.bottom)].iter().all(|(x, y)| (x - y).abs() < 1.0)
}

/// **The acceptance test.** What the copy holds where the picture was placed is
/// a picture — not a group, not an annotation — and the engine's own move and
/// remove take it. Each is tried on a fresh copy, then moved *after* another
/// move, the order that refused the form.
#[test]
fn a_placed_picture_is_a_picture_in_the_copy_that_moves_and_removes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };
    let annotations_before = doc.annotation_count(0).expect("count");
    place_picture(&mut doc, rect);

    let copy = doc.flattened_copy().expect("a copy for sharing");
    assert_eq!((copy.pictures, copy.forms), (1, 0), "left {}", copy.left);

    let fresh = || PdfiumDocument::open_bytes(copy.bytes.clone(), None).expect("the copy opens");
    let mut flat = fresh();
    assert_eq!(flat.annotation_count(0).expect("count"), annotations_before, "the picture is still an annotation");
    let drawn = flat.drawn_objects(0).expect("drawn");
    let picture = drawn
        .iter()
        .find(|o| o.kind == DrawnKind::Picture && o.depth == 0 && near(o.rect, rect))
        .unwrap_or_else(|| panic!("no picture where it was placed; the page draws {drawn:?}"));
    let object = picture.object;

    let mut moved = fresh();
    moved.move_object(0, object, Point { x: 30.0, y: 20.0 }).expect("the picture moves");
    let after = moved.images_on(0).expect("images").into_iter().find(|i| i.object == object).expect("still there");
    assert!(
        near(after.rect, Rect { left: 130.0, top: 220.0, right: 210.0, bottom: 260.0 }),
        "moved to {:?}",
        after.rect
    );

    let mut removed = fresh();
    removed.remove_object(0, object).expect("the picture is removed");
    assert!(removed.images_on(0).expect("images").iter().all(|i| i.object != object), "it is still drawn");

    // The order that refused a form: something else moved first.
    let mut second = fresh();
    let other = second.drawn_objects(0).expect("drawn").into_iter().find(|o| o.kind == DrawnKind::Shape && o.depth == 0);
    if let Some(other) = other {
        let _ = second.move_object(0, other.object, Point { x: 5.0, y: 5.0 });
    }
    second.move_object(0, object, Point { x: 30.0, y: 20.0 }).expect("the picture moves after another move");
}

/// **The reported break, end to end.** A preview draws the page and not what is
/// laid over it, which is `render_annotations: false`. Without it the placed
/// picture is gone — the grey box in WhatsApp — and in the copy it is not.
#[test]
fn the_picture_is_drawn_when_annotations_are_not_which_is_what_a_preview_does() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };
    place_picture(&mut doc, rect);
    assert!(middle_is_red(&doc, rect, true), "setup: the placed picture is not drawn even with annotations on");
    assert!(
        !middle_is_red(&doc, rect, false),
        "setup: with annotations off the document still draws the picture, so this proves nothing"
    );

    let copy = doc.flattened_copy().expect("a copy");
    let flat = PdfiumDocument::open_bytes(copy.bytes, None).expect("the copy opens");
    assert!(middle_is_red(&flat, rect, false), "the copy's picture is still only an annotation");
}

/// **The break this catches:** making the copy changing the document it was
/// made from — flattening in place, where the picture could no longer be
/// dragged by its handles.
#[test]
fn making_the_copy_leaves_the_document_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place_picture(&mut doc, Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 });
    let annotations = doc.annotation_count(0).expect("count");
    let drawn = doc.drawn_objects(0).expect("drawn").len();

    doc.flattened_copy().expect("a copy");

    assert_eq!(doc.annotation_count(0).expect("count"), annotations, "an annotation was taken");
    assert_eq!(doc.placed_image_marks(0).expect("marks").len(), 1, "the picture is no longer a placed picture");
    assert_eq!(doc.drawn_objects(0).expect("drawn").len(), drawn, "the page's own drawing changed");
}

/// **The known limit, held to its word.** The engine writes a highlight as a
/// colour and quad points with no appearance, so there is nothing to bake: it
/// stays an annotation in the copy — a preview will not show it — and is
/// *counted*, so the person is told rather than finding out from the recipient.
/// What it must never do is vanish from the copy.
#[test]
fn a_highlight_has_no_appearance_to_bake_so_it_is_kept_and_counted() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let before = doc.annotation_count(0).expect("count");
    doc.add_annotation(
        0,
        &Annotation::Highlight {
            rects: vec![Rect { left: 72.0, top: 100.0, right: 200.0, bottom: 114.0 }],
            color: Color { r: 255, g: 235, b: 59, a: 255 },
        },
    )
    .expect("highlight");

    let copy = doc.flattened_copy().expect("a copy");
    assert_eq!((copy.pictures, copy.forms, copy.bare), (0, 0, 1), "left {}", copy.left);
    let flat = PdfiumDocument::open_bytes(copy.bytes, None).expect("the copy opens");
    assert_eq!(flat.annotation_count(0).expect("count"), before + 1, "the highlight was lost from the copy");
}

/// **The break:** a copy of a password-protected document that comes out with
/// no password — the one way this could leak what the password protects.
#[test]
fn a_document_with_a_password_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    place_picture(&mut doc, Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 });
    assert!(doc.flattened_copy().is_ok(), "setup: the same document makes a copy before it has a password");
    doc.secure_document(b"correct horse", None, Permissions::all()).expect("a password is set");
    assert!(doc.flattened_copy().is_err(), "a copy was made with no password on it");
}

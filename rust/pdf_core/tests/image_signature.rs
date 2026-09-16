//! A signature that is a picture rather than ink.
//!
//! Placed through PDFium's own annotation object API — a `/Stamp` carrying
//! one image page object, so any PDF reader shows it, not only this engine —
//! and applied (burnt into the page) through the byte-safe writer, so that
//! applying one on a signed document appends a revision instead of rewriting
//! the file. See `Annotation::Image` and `PdfiumDocument::fill_image_annotation`
//! for why those are two different mechanisms and why that split is exactly
//! backwards from how it looks at first: placing is PDFium's; applying is
//! `crate::pdf`'s.
//!
//! **The alpha byte is not honoured.** Confirmed by `image_signature_probe`
//! before this test was written: `FPDFImageObj_SetBitmap` drops it even
//! in-session, before anything is saved. Every picture placed here is opaque,
//! and the tests below check exactly that rather than assume it.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test image_signature
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{
    Annotation, Color, Document, DocumentMut, DrawnKind, Rect, Redaction, RenderRequest,
};

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// A small, solid picture — no two pixels ambiguous with the page underneath.
fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    rgba
}

/// Place a picture, mark it as a signature named `name`, and return its
/// annotation index.
fn place(doc: &mut PdfiumDocument, page: usize, rect: Rect, rgba: Vec<u8>, w: u32, h: u32, name: &str) -> usize {
    let index = doc
        .add_annotation(page, &Annotation::Image { rect, rgba, width: w, height: h })
        .expect("add image annotation");
    doc.mark_as_signature(page, index, name).expect("mark as signature");
    index
}

/// **The acceptance test.** Placed, read back in the same session: the right
/// name, the right rect, the right pixels, opaque — and it is not mistaken
/// for an ink signature or counted as page content yet.
#[test]
fn a_placed_picture_reads_back_exactly_as_placed() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let rect = Rect { left: 100.0, top: 200.0, right: 180.0, bottom: 240.0 };
    let rgba = solid(4, 4, [220, 40, 40]);
    place(&mut doc, 0, rect, rgba.clone(), 4, 4, "Red Square");

    let ink = doc.signature_marks(0).expect("signature_marks");
    assert!(ink.is_empty(), "a picture was read back as ink: {ink:?}");

    let images = doc.image_signature_marks(0).expect("image_signature_marks");
    assert_eq!(images.len(), 1);
    let mark = &images[0];
    assert_eq!(mark.name, "Red Square");
    assert_eq!(mark.rect, rect);
    assert_eq!(mark.width, 4);
    assert_eq!(mark.height, 4);
    assert_eq!(mark.rgba, rgba, "the pixels do not match what was placed");

    // Not yet applied: still an annotation, not page content.
    assert_eq!(doc.annotation_count(0).expect("annotation count"), 1);
}

/// **Round trip.** Saved as a whole file and reopened with none of the
/// placing code in the way: same name, same rect, same pixel colours —
/// opaque, which is the one thing that does *not* survive (see the module
/// doc), checked here so a future PDFium that starts honouring alpha is
/// noticed rather than silently trusted.
#[test]
fn a_placed_picture_survives_a_save_and_reopen() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let rect = Rect { left: 50.0, top: 60.0, right: 122.0, bottom: 96.0 };
    // Half-transparent in the source — placed anyway, to prove the opacity
    // claim rather than dodge it by only testing already-opaque pixels.
    let mut rgba = solid(3, 3, [10, 200, 30]);
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 90;
    }
    place(&mut doc, 0, rect, rgba, 3, 3, "Soft Green");

    let mut saved = Vec::new();
    doc.save_full_copy(&mut saved).expect("save");
    let reopened = PdfiumDocument::open_bytes(saved, None).expect("reopen");

    let images = reopened.image_signature_marks(0).expect("image_signature_marks");
    assert_eq!(images.len(), 1);
    let mark = &images[0];
    assert_eq!(mark.name, "Soft Green");
    assert_eq!(mark.rect, rect);
    assert_eq!(mark.width, 3);
    assert_eq!(mark.height, 3);
    for pixel in mark.rgba.chunks_exact(4) {
        assert_eq!(&pixel[..3], &[10, 200, 30], "colour changed in the round trip");
        assert_eq!(pixel[3], 255, "alpha survived the round trip — the module doc is now wrong");
    }
}

/// **Nothing new goes over a lock**, the same rule ink already follows —
/// checked here because the rule lives in `add_annotation`'s own bounds
/// check, shared by every annotation kind, and a picture is the newest one
/// to rely on it rather than to have been written with it in mind.
#[test]
fn a_picture_over_a_locked_area_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let locked_area = Rect { left: 0.0, top: 0.0, right: 300.0, bottom: 300.0 };
    doc.lock_area(&Redaction::new(0, locked_area), b"pagify", None).expect("lock");

    let result = doc.add_annotation(
        0,
        &Annotation::Image {
            rect: Rect { left: 50.0, top: 50.0, right: 100.0, bottom: 100.0 },
            rgba: solid(2, 2, [0, 0, 0]),
            width: 2,
            height: 2,
        },
    );
    assert!(result.is_err(), "a picture was placed over a locked area");
    assert!(result.unwrap_err().to_string().contains("locked"));
}

/// **Only a drawn or uploaded signature can be marked as one** — the same
/// refusal ink already has, now naming both kinds it accepts. A highlight
/// cannot be flattened into the page by a tool somebody thought only touched
/// their own name or picture.
#[test]
fn only_ink_or_a_picture_can_be_marked_as_a_signature() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let index = doc
        .add_annotation(
            0,
            &Annotation::Highlight {
                rects: vec![Rect { left: 10.0, top: 10.0, right: 50.0, bottom: 20.0 }],
                color: Color { r: 255, g: 255, b: 0, a: 255 },
            },
        )
        .expect("add highlight");
    let refused = doc.mark_as_signature(0, index, "Not A Signature");
    assert!(refused.is_err());
    assert!(refused.unwrap_err().to_string().contains("drawn or uploaded"));
}

/// **Applying burns it in, and it renders where it was placed.** The
/// strongest check available: not just that the annotation is gone, but that
/// a fresh render of the page shows the placed colour at the placed spot.
#[test]
fn applying_a_picture_signature_paints_it_into_the_page() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let size = doc.page(0).expect("page").size();
    // A block in the page's lower-right quadrant, away from the body text,
    // sized generously so a modest render scale still samples well inside it.
    let rect = Rect {
        left: size.width_pt - 140.0,
        top: size.height_pt - 140.0,
        right: size.width_pt - 40.0,
        bottom: size.height_pt - 40.0,
    };
    place(&mut doc, 0, rect, solid(8, 8, [30, 120, 210]), 8, 8, "Blue Block");

    let before = doc.drawn_objects(0).expect("objects");
    let pictures_before = before.iter().filter(|d| d.kind == DrawnKind::Picture).count();

    let applied = doc.apply_signatures(0).expect("apply");
    assert_eq!(applied, 1);
    assert!(doc.image_signature_marks(0).expect("marks").is_empty(), "the annotation is still there");
    assert_eq!(doc.annotation_count(0).expect("count"), 0, "the annotation was not removed");

    let after = doc.drawn_objects(0).expect("objects");
    let pictures_after = after.iter().filter(|d| d.kind == DrawnKind::Picture).count();
    assert_eq!(pictures_after, pictures_before + 1, "no new picture on the page");

    // Rendered, and sampled in the middle of where it was placed.
    let scale = 2.0;
    let bitmap = doc
        .render_page_to_bitmap(0, &RenderRequest { scale, ..Default::default() })
        .expect("render");
    let mid_x = (((rect.left + rect.right) / 2.0) * scale) as usize;
    let mid_y = (((rect.top + rect.bottom) / 2.0) * scale) as usize;
    let at = mid_y * bitmap.stride + mid_x * 4;
    let pixel = &bitmap.data[at..at + 4];
    // Within a few units either way: the render path resamples an 8x8
    // source across a much larger area, so this is not expected exact.
    let close = |sample: u8, expected: u8| sample.abs_diff(expected) <= 12;
    assert!(
        close(pixel[0], 30) && close(pixel[1], 120) && close(pixel[2], 210),
        "the placed colour is not at its rect after applying: got {pixel:?}"
    );
}

/// **Ink and picture signatures together, applied in one call**, checking the
/// batching this exists for: one append for the whole page, not one per
/// signature — both kinds gone afterward, and both counted.
#[test]
fn ink_and_picture_signatures_apply_together_in_one_call() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    use pdf_core::document::Point;
    doc.add_annotation(
        0,
        &Annotation::Ink {
            strokes: vec![vec![Point { x: 30.0, y: 30.0 }, Point { x: 60.0, y: 45.0 }]],
            color: Color { r: 0, g: 0, b: 0, a: 255 },
            width: 1.5,
        },
    )
    .and_then(|i| doc.mark_as_signature(0, i, "Written"))
    .expect("place and mark the ink signature");
    place(&mut doc, 0, Rect { left: 200.0, top: 30.0, right: 240.0, bottom: 60.0 }, solid(4, 4, [90, 200, 90]), 4, 4, "Uploaded");

    let applied = doc.apply_signatures(0).expect("apply");
    assert_eq!(applied, 2);
    assert!(doc.signature_marks(0).expect("ink marks").is_empty());
    assert!(doc.image_signature_marks(0).expect("image marks").is_empty());
    assert_eq!(doc.annotation_count(0).expect("count"), 0);
}

/// **Applying a picture signature on a signed document appends — it does not
/// rewrite.** The whole reason `apply_signatures` builds through
/// `edit_base`/`write_edit` instead of PDFium's own re-serialisation: a
/// signature covers a byte range, and this must not move it. Same shape as
/// `tests/signing.rs`'s ink equivalent, for a picture.
#[test]
fn applying_a_picture_signature_on_a_signed_document_appends_not_rewrites() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    use pdf_core::pdf::{sign, validate, File};

    let p12 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer-sm2.p12");
    let Ok(pkcs12) = std::fs::read(&p12) else { return };

    let mut doc = open("two-column.pdf");
    doc.sign_document(&pkcs12, "pagify", &sign::Reason::default()).expect("sign");
    let mut signed = Vec::new();
    doc.save_incremental(&mut signed).expect("save the signed file");

    place(&mut doc, 0, Rect { left: 40.0, top: 40.0, right: 90.0, bottom: 70.0 }, solid(4, 4, [200, 200, 10]), 4, 4, "Later");
    doc.apply_signatures(0).expect("apply");

    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save again");
    assert!(saved.starts_with(&signed), "the signed bytes were not kept in place");

    let file = File::parse(&saved).expect("parse");
    let found = validate::check(&file, &saved).expect("check");
    assert_eq!(found.len(), 1);
    match found[0].verdict {
        validate::Verdict::Incomplete { covered, total } => {
            assert_eq!(covered, signed.len());
            assert!(covered < total);
        }
        ref other => panic!("expected Incomplete, got {other:?}"),
    }
    assert_eq!(found[0].signer.as_deref(), Some("CN=Pagify SM2 Test Signer,O=Pagify"));
}

/// **The fallback this all exists for.** An unrelated, unsaved mark — on the
/// signed page, placed after signing, and never applied — must not vanish
/// just because a picture signature elsewhere is being applied at the same
/// moment. If the exact/append base were trusted here regardless, reopening
/// onto it would silently drop this highlight: it was never in those bytes
/// to begin with. The engine must notice and fall back to a real rewrite —
/// correct, merely not appended — rather than lose it.
#[test]
fn an_unrelated_pending_mark_is_not_lost_when_a_picture_signature_is_applied() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    use pdf_core::pdf::sign;

    let p12 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer-sm2.p12");
    let Ok(pkcs12) = std::fs::read(&p12) else { return };

    let mut doc = open("two-column.pdf");
    doc.sign_document(&pkcs12, "pagify", &sign::Reason::default()).expect("sign");
    let mut signed = Vec::new();
    doc.save_incremental(&mut signed).expect("save the signed file");

    // An unrelated highlight, placed after signing and never applied or saved.
    doc.add_annotation(
        0,
        &Annotation::Highlight {
            rects: vec![Rect { left: 10.0, top: 10.0, right: 60.0, bottom: 22.0 }],
            color: Color { r: 255, g: 255, b: 0, a: 255 },
        },
    )
    .expect("add the unrelated highlight");

    place(&mut doc, 0, Rect { left: 40.0, top: 40.0, right: 90.0, bottom: 70.0 }, solid(4, 4, [200, 200, 10]), 4, 4, "Later");
    doc.apply_signatures(0).expect("apply");

    // The highlight is still there — not silently dropped by an append that
    // reopened onto bytes from before either mark existed.
    let annotations = doc.annotations(0).expect("annotations");
    assert!(
        annotations.iter().any(|a| matches!(a.annotation, Annotation::Highlight { .. })),
        "the unrelated highlight did not survive: {annotations:?}"
    );
}

//! What a signature check is allowed to describe.
//!
//! Found by audit (L-6): the check preferred the bytes kept in memory, then
//! the file on disk, without asking whether either still matched the open
//! document — so a mark added after opening had a verdict read out about the
//! file from before it. A check that cannot describe the document must say
//! so, not describe an earlier revision of it.

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, DocumentMut, Rect};

fn open_signed() -> PdfiumDocument {
    PdfiumDocument::open_path(
        harness::fixture_path("sm2-signed.pdf").to_str().expect("path"),
        None,
    )
    .expect("open the signed fixture")
}

/// The untouched case, which must keep working: the file on disk is the
/// document, so the check answers about it.
#[test]
fn an_opened_signed_document_is_checked_against_its_file() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open_signed();
    let found = doc.validate_signatures().expect("an untouched document is its file");
    assert_eq!(found.len(), 1, "expected one signature, got {found:?}");
}

/// **A change that is not in the signed bytes is not described by them.**
/// Adding an annotation leaves `written` and the file on disk behind, so the
/// check must say it cannot rather than read out the file from before the
/// mark. Once the change is saved the bytes are the document again, and the
/// answer is that the mark is a later revision the signature does not cover.
#[test]
fn an_unsaved_change_is_not_described_by_the_signed_bytes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open_signed();
    doc.add_annotation(
        0,
        &Annotation::Highlight {
            rects: vec![Rect { left: 10.0, top: 10.0, right: 60.0, bottom: 22.0 }],
            color: Color { r: 255, g: 255, b: 0, a: 255 },
        },
    )
    .expect("add a highlight the signature does not cover");

    assert!(
        doc.validate_signatures().is_err(),
        "a stale verdict was read out about a document that has changed"
    );

    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).expect("save");
    let found = doc.validate_signatures().expect("the saved document is its bytes");
    assert_eq!(found.len(), 1, "expected one signature, got {found:?}");
    assert!(
        !matches!(found[0].verdict, pdf_core::pdf::validate::Verdict::Unaltered),
        "an annotation made after signing was reported as unaltered: {:?}",
        found[0]
    );
}

//! A page imported from another document must not leave the destination
//! pointing at a source the process has closed.
//!
//! PDFium's page importer (`CPDF_PageOrganizer::UpdateReference`) clones a
//! page's references and deliberately skips the `Parent` key, so a `Popup`
//! annotation's `/Parent` stays bound to the *source* document. Closing the
//! source then leaves the destination with a reference into freed memory, and
//! the next full save walks it. Reported from use: pasting `LINEA 8X9.pdf`'s
//! page (a placed picture — a `/Stamp` with its `/Popup`) into another
//! document and pressing Save killed the app inside `pdfium.dll` at
//! `+0x7317e`.
//!
//! The page is written by hand so the source's annotation object numbers are
//! well above the one-page copy's own, which is what makes the carried page
//! keep a `/Parent` that names no object in it — exactly the file the crash
//! session left on disk.

mod harness;

use harness::{helvetica_page, page_with_fonts, skip_without_pdfium};
use pdf_core::command::history::CommandHistory;
use pdf_core::command::Command;
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut};

/// One page, a Stamp annotation and its Popup. The five empty objects in
/// between put the annotations at objects 11 and 12; the one-page copy that
/// `extract_pages` makes numbers its own objects from 1, so `/Parent 11 0 R`
/// cannot accidentally be right.
fn page_with_a_popup() -> Vec<u8> {
    let extras: Vec<Vec<u8>> = (0..5)
        .map(|_| b"<< >>".to_vec())
        .chain([
            b"<< /Type /Annot /Subtype /Stamp /Rect [100 100 300 200] \
               /P 3 0 R /Popup 12 0 R >>"
                .to_vec(),
            b"<< /Type /Annot /Subtype /Popup /Rect [100 100 300 200] \
               /P 3 0 R /Parent 11 0 R >>"
                .to_vec(),
        ])
        .collect();
    page_with_fonts(
        b"BT /F1 24 Tf 72 700 Td (popup page) Tj ET",
        &[("F1", "Helvetica")],
        "/Annots [11 0 R 12 0 R]",
        &extras,
    )
}

#[test]
fn a_page_with_a_popup_is_imported_and_the_destination_still_saves() {
    let Some(_) = skip_without_pdfium() else {
        return;
    };
    let _serial = harness::serial();

    let mut source = PdfiumDocument::open_bytes(page_with_a_popup(), None).expect("open source");
    let mut carried = source
        .as_document_mut()
        .expect("mutable")
        .extract_pages(&[0])
        .expect("extract the page");
    let mut carried_bytes = Vec::new();
    carried
        .as_document_mut()
        .expect("mutable")
        .save_full_copy(&mut carried_bytes)
        .expect("save the carried page");

    assert!(
        carried_bytes
            .windows(b"/Parent 11 0 R".len())
            .any(|window| window == b"/Parent 11 0 R"),
        "the fixture no longer reproduces the condition: the carried page has \
         no `/Parent` naming an object it does not hold",
    );

    let mut destination = PdfiumDocument::open_bytes(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (host page) Tj ET"),
        None,
    )
    .expect("open destination");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::ImportPages { at: 1, pdf: carried_bytes },
            destination.as_document_mut().expect("mutable"),
        )
        .expect("import the page");

    // The fix's invariant: the document the importer opened is still alive, so
    // the Popup's `/Parent` no longer points into freed memory.
    assert_eq!(
        destination.retained_import_sources(),
        1,
        "the imported-from document was dropped; a save could walk its freed object table",
    );

    // And the operation this whole bug was reported as: save.
    let mut bytes = Vec::new();
    destination
        .save_full_copy(&mut bytes)
        .expect("save after the import");
    assert!(!bytes.is_empty());
}

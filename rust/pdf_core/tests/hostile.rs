//! Documents built to hurt.
//!
//! Every fixture here was made from a finding in the security audit: an input
//! shaped to make the engine panic, overflow or exhaust memory. The tests ask
//! only that the engine **survives and answers** — an error is fine, a wrong
//! answer is not tested here, and a dead process is the failure.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test hostile
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn open(name: &str) -> PdfiumDocument {
    let path = harness::fixture_path(name);
    PdfiumDocument::open_path(path.to_str().expect("path"), None).expect("open fixture")
}

/// **A mark with a name longer than the buffer it is read into.** The app
/// walks every page's marks on open, to restore its own; a name that did not
/// fit reported a length past the buffer's end and the slice by it ended the
/// process. Found by audit.
#[test]
fn a_mark_with_an_overlong_name_is_not_ours_and_does_not_crash() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("long-mark.pdf");
    let marks = doc.text_marks(0).expect("walking the marks");
    assert!(marks.is_empty(), "somebody else's mark was taken for ours: {marks:?}");
    // And the page is otherwise an ordinary page.
    let text = doc.page(0).expect("page").text().expect("text");
    assert!(text.contains("Marked with a very long tag"), "{text}");
}

/// The same through a session — which is how the desktop reaches the
/// engine, and where a panic is now an error rather than the end.
#[test]
fn a_hostile_document_through_a_session_answers_or_errors_but_lives() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = harness::fixture_path("long-mark.pdf");
    let handle = pdf_core::registry::insert_with(|| {
        Ok(Box::new(PdfiumDocument::open_path(path.to_str().expect("path"), None)?)
            as Box<dyn Document>)
    })
    .expect("open");
    let outcome = pdf_core::registry::with_session(handle, |s| s.document.text_marks(0));
    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(pdf_core::registry::remove(handle));
}

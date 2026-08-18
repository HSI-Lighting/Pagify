//! Shared plumbing for the round-trip tests.
//!
//! Kept separate so the tests themselves read as statements about behaviour
//! rather than as PDFium setup.

use std::path::PathBuf;

use pdf_core::document::Document;
use pdf_core::document::pdfium_doc::PdfiumDocument;

/// The desktop PDFium, or `None` if it is not configured.
///
/// Skipping rather than failing is deliberate: these tests need a desktop build
/// of the *pinned* PDFium, which is fetched separately and is not on every
/// machine. A hard failure would train people to ignore a red suite; a skip that
/// says why does not. CI sets the variable and therefore runs them.
pub fn skip_without_pdfium() -> Option<()> {
    match std::env::var("PAGIFY_PDFIUM_LIB") {
        Ok(_) => Some(()),
        Err(_) => {
            eprintln!(
                "skipped: set PAGIFY_PDFIUM_LIB to a desktop PDFium of the pinned \
                 build (chromium/7881) to run the round-trip tests"
            );
            None
        }
    }
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

pub fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).expect("read fixture")
}

pub fn open_fixture(_pdfium: &(), name: &str) -> Box<dyn Document> {
    let path = fixture_path(name);
    Box::new(
        PdfiumDocument::open_path(path.to_str().expect("fixture path"), None)
            .expect("open fixture"),
    )
}

/// Widths in whole points, which is how these tests identify pages.
pub fn page_widths(doc: &Box<dyn Document>) -> Vec<i32> {
    (0..doc.page_count())
        .map(|i| doc.page_size(i).expect("page size").width_pt.round() as i32)
        .collect()
}

/// Save through the engine and reopen from those bytes.
///
/// The reopen is the whole point: asserting against the in-memory document after
/// a command proves only that the command ran, not that what it did was written.
pub fn save_and_reopen(_pdfium: &(), doc: &mut Box<dyn Document>) -> Box<dyn Document> {
    let mut bytes = Vec::new();
    doc.as_document_mut()
        .expect("the document must be mutable")
        .save_incremental(&mut bytes)
        .expect("save");

    Box::new(PdfiumDocument::open_bytes(bytes, None).expect("reopen what we just wrote"))
}

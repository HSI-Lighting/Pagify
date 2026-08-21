//! Where does a page-tree edit lose the page sizes?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example page_ops_probe -- rust/pdf_core/fixtures/pages-ladder.pdf
//! ```
//!
//! The round-trip test reported `[200, 612, 612, 612]` after deleting a page and
//! saving: the right number of pages, the first size correct, the rest US Letter.
//! Letter is what PDFium reports for a page with no usable MediaBox, so something
//! between the delete and the reopen is dropping page geometry.
//!
//! Three states, printed in order, so the step that loses it is obvious rather
//! than inferred: in memory before the save, after an incremental save and
//! reopen, and after a full-copy save and reopen.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn widths(doc: &dyn Document) -> Vec<i32> {
    (0..doc.page_count())
        .map(|i| {
            doc.page_size(i)
                .map(|s| s.width_pt.round() as i32)
                .unwrap_or(-1)
        })
        .collect()
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: page_ops_probe <pdf>");

    let mut doc = PdfiumDocument::open_path(&path, None).expect("open");
    println!("opened          : {:?}", widths(&doc));

    let removed = doc
        .as_document_mut()
        .expect("mutable")
        .delete_page(2)
        .expect("delete page 2");
    println!(
        "after delete    : {:?}   (removed page was {:.0} pt wide)",
        widths(&doc),
        removed.size.width_pt,
    );

    let mut incremental = Vec::new();
    doc.as_document_mut()
        .expect("mutable")
        .save_incremental(&mut incremental)
        .expect("incremental save");
    let reopened = PdfiumDocument::open_bytes(incremental.clone(), None).expect("reopen");
    println!(
        "incremental     : {:?}   ({} bytes)",
        widths(&reopened),
        incremental.len(),
    );

    let mut full = Vec::new();
    doc.as_document_mut()
        .expect("mutable")
        .save_full_copy(&mut full)
        .expect("full save");
    let reopened_full = PdfiumDocument::open_bytes(full.clone(), None).expect("reopen");
    println!(
        "full copy       : {:?}   ({} bytes)",
        widths(&reopened_full),
        full.len(),
    );
}

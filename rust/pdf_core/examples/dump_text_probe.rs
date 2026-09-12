//! Print every page's text, for reading a document from the terminal.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=… cargo run --release --example dump_text_probe -- <pdf>
//! ```
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, Page};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    for page in 0..doc.page_count() {
        println!("===== page {} =====", page + 1);
        match doc.page(page).and_then(|p| p.text()) {
            Ok(text) => println!("{text}"),
            Err(e) => println!("(no text: {e})"),
        }
    }
}

//! What the engine reports for the annotations already in a file.
//!
//! ```
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example annotations_probe -- some.pdf 0
//! ```
//!
//! Written to answer one question the app cannot: a shape drawn, saved and
//! reopened came back in the wrong colour, and the app's parser defaults to
//! yellow when the colour is missing. Either the engine is not reporting one or
//! the app is not reading it, and only one of those can be seen from here.

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: annotations_probe <pdf> [page]");
    let page: usize = args
        .next()
        .unwrap_or_else(|| "0".into())
        .parse()
        .expect("page");

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    println!("pdfium: {lib}");

    let doc = PdfiumDocument::open_path(&path, None).expect("open the document");
    println!("pages: {}", doc.page_count());

    let marks = doc.annotations(page).expect("read the annotations");
    println!("annotations on page {page}: {}", marks.len());

    for mark in &marks {
        println!(
            "  {}",
            serde_json::to_string(mark).unwrap_or_else(|e| format!("<unserialisable: {e}>")),
        );
    }
}

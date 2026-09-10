//! What does the scanner find in a real document?
//!
//! A tool that reports a wattage as a telephone number teaches people to ignore
//! it. So this is run against a real catalogue — full of part numbers, ratings
//! and years — and every find is printed for a person to judge.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example smart_redact_probe -- <file.pdf> [pages]
//! ```

use pdf_core::document::{sensitive, Document};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(200);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let pages = doc.page_count().min(limit);

    let mut tally: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut shown = 0usize;
    for page in 0..pages {
        let Ok(text) = doc.page(page).and_then(|p| p.characters()) else { continue };
        for found in sensitive::scan(&text.text) {
            *tally.entry(found.kind.describe()).or_default() += 1;
            if shown < 24 {
                println!("  page {:>3}  {:<22} {:?}", page + 1, found.kind.describe(), found.text);
                shown += 1;
            }
        }
    }

    println!("\n{pages} pages scanned");
    if tally.is_empty() {
        println!("  nothing found");
    }
    for (kind, count) in &tally {
        println!("  {kind:<22} {count}");
    }
}

//! What does reading the lock cost, once a whole document is in it?
//!
//! Reported from use: locking every page left the app lagging badly. The badges
//! on a page are re-read while drawing, and `locked_items_on` goes through
//! `read_vault` — which pulls the whole attachment out of the file, parses it as
//! JSON and base64-decodes every sealed page inside.
//!
//! With one area locked that is a few kilobytes. With every page locked the
//! vault holds the entire document. This measures the difference, because "it
//! feels slow" and "it is doing forty megabytes of work per frame" are different
//! problems with different fixes.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example vault_cost_probe -- <file.pdf> [pages]
//! ```

use std::time::Instant;

use pdf_core::document::{Document, DocumentMut};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let wanted: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(8);

    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let pages: Vec<usize> = (0..doc.page_count().min(wanted)).collect();
    println!("{}: locking {} of {} pages", path, pages.len(), doc.page_count());

    let started = Instant::now();
    doc.lock_pages(&pages, b"a good passcode").expect("lock");
    println!("locking took {:?}", started.elapsed());

    // What the page drawing does, once per visible page per frame.
    for round in 0..3 {
        let started = Instant::now();
        let items = doc.locked_items_on(0).expect("locked items");
        println!(
            "  locked_items_on(0) #{round}: {:?}  ({} badge(s))",
            started.elapsed(),
            items.len()
        );
    }

    let started = Instant::now();
    let locked = doc.locked_pages().expect("locked pages");
    println!("locked_pages(): {:?} ({} pages)", started.elapsed(), locked.len());

    // And the size of the thing being parsed each time.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    println!("document with the lock in it: {} bytes", bytes.len());
}

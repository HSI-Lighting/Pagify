//! What does the classifier actually say about real documents?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example classify_probe -- <pdf>…
//! ```
//!
//! The house rule is *measure, do not infer*. Every threshold in
//! `PageClassification` is a guess until a corpus disagrees with it, so this
//! prints the counts beside the verdict rather than the verdict alone — a page
//! called `Empty` with 4,000 unmappable characters is a threshold problem, and
//! only the numbers show it.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, PageTextKind};

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: classify_probe <pdf>…");
        std::process::exit(2);
    }

    for path in &files {
        let document = match PdfiumDocument::open_path(path, None) {
            Ok(document) => document,
            Err(e) => {
                println!("{path}\n  could not open: {e}\n");
                continue;
            }
        };

        let name = path.rsplit('/').next().unwrap_or(path);
        println!("{name}  ({} pages)", document.page_count());
        println!(
            "  {:>4}  {:<11} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
            "page", "verdict", "chars", "unmap", "img%", "paths", "glyphs", "rot"
        );

        let mut tally = std::collections::BTreeMap::<String, usize>::new();

        for index in 0..document.page_count() {
            let page = match document.page(index) {
                Ok(page) => page,
                Err(_) => continue,
            };
            let c = match page.classify() {
                Ok(c) => c,
                Err(e) => {
                    println!("  {:>4}  failed: {e}", index + 1);
                    continue;
                }
            };

            *tally.entry(format!("{:?}", c.kind)).or_default() += 1;

            // Only the interesting pages, or the first few, so a 500-page
            // catalogue does not scroll past.
            let interesting = !matches!(c.kind, PageTextKind::Native) || index < 3;
            if interesting {
                println!(
                    "  {:>4}  {:<11} {:>7} {:>7} {:>6.1}% {:>7} {:>7} {:>7}",
                    index + 1,
                    format!("{:?}", c.kind),
                    c.chars,
                    c.unmappable,
                    c.image_coverage * 100.0,
                    c.paths,
                    c.glyph_paths,
                    c.rotated_chars,
                );
            }
        }

        let summary: Vec<String> = tally.iter().map(|(k, n)| format!("{n} {k}")).collect();
        println!("  → {}\n", summary.join(", "));
    }
}

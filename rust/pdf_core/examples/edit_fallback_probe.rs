//! When editing a word, does it go through the content stream or fall back?
//!
//! `set_run_in_stream` changes the codes in place and touches nothing else. It
//! refuses where it cannot be certain, and the caller then falls back to
//! PDFium — which re-emits the whole page and is exactly what rewrites the
//! text around the edit. So the question is how often that happens, and why.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example edit_fallback_probe -- <file.pdf> [pages]
//! ```

use pdf_core::document::{Document, DocumentMut};

type Doc = pdf_core::document::pdfium_doc::PdfiumDocument;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(20);

    let pages = Doc::open_path(&path, None).expect("open").page_count().min(limit);
    let (mut clean, mut fell_back, mut skipped) = (0usize, 0usize, 0usize);
    let mut why: std::collections::BTreeMap<String, usize> = Default::default();

    for page in 0..pages {
        let mut doc = Doc::open_path(&path, None).expect("open");
        let Ok(runs) = doc.text_runs(page) else { skipped += 1; continue };
        let Some(run) = runs.iter().find(|r| r.text.chars().count() > 12) else {
            skipped += 1;
            continue;
        };
        let object = run.object;
        let before = doc.page(page).and_then(|p| p.characters()).map(|c| c.text).unwrap_or_default();

        // The engine's own path, asked directly so the reason comes back.
        match doc.try_set_run_in_stream(page, object, "REPLACED") {
            Ok(()) => {
                let after = doc.page(page).and_then(|p| p.characters()).map(|c| c.text).unwrap_or_default();
                let _ = (before, after);
                clean += 1;
            }
            Err(e) => {
                fell_back += 1;
                *why.entry(e.to_string()).or_default() += 1;
            }
        }
    }

    println!("{pages} pages tried");
    println!("  edited in the stream : {clean}");
    println!("  fell back to PDFium  : {fell_back}");
    println!("  skipped              : {skipped}");
    for (reason, count) in &why {
        println!("    {count:>3}  {reason}");
    }
}

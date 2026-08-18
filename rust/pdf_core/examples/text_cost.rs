//! What does asking a page for its text actually cost?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --release --example text_cost -- <pdf> <page> [repeats]
//! ```
//!
//! The highlighter cannot draw anything until `text_segments` returns, while the
//! marker needs nothing from the engine at all. So "highlighting does not work on
//! the big file, but the marker does" is first of all a question about this call's
//! cost, and it is worth measuring off-device where nothing else is competing for
//! the same lock.
//!
//! Timed in three parts, because they have very different causes: loading the page
//! is per-page parsing work that scales with how much is *on* the page, building
//! the text page is PDFium's own extraction, and walking the runs is ours.
use pdfium_render::prelude::*;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: text_cost <pdf> <page> [repeats]");
    let index: i32 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(0);
    let repeats: usize = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(3);

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));

    let opened = Instant::now();
    let doc = pdfium.load_pdf_from_file(path, None).expect("open");
    println!(
        "{}\n  open        : {:>8} ms   ({} pages)",
        path,
        opened.elapsed().as_millis(),
        doc.pages().len(),
    );

    for round in 1..=repeats {
        let t0 = Instant::now();
        let page = doc.pages().get(index).expect("page");
        let load_ms = t0.elapsed().as_millis();

        let t1 = Instant::now();
        let text = page.text().expect("text");
        let text_ms = t1.elapsed().as_millis();

        let t2 = Instant::now();
        let runs = text
            .segments()
            .iter()
            .filter(|s| !s.text().trim().is_empty())
            .count();
        let walk_ms = t2.elapsed().as_millis();

        println!(
            "  round {round}     : load {load_ms:>6} ms | text() {text_ms:>6} ms | walk {walk_ms:>6} ms | total {:>6} ms | {runs} runs, {} chars",
            t0.elapsed().as_millis(),
            text.len(),
        );
    }
}

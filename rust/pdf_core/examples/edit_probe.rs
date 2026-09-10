//! What happens to a page when one run's words are changed in the stream.
//!
//! Reported from use: editing small text at the bottom of a page duplicated it.
//! This does the edit the app does, on every run whose text matches a pattern,
//! and prints the page's words before and after — so the duplication can be
//! seen rather than guessed at.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example edit_probe -- <file.pdf> <page> <needle>
//! ```

use pdf_core::document::{Document, DocumentMut};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let needle = std::env::args().nth(3).unwrap_or_else(|| "SATURATION".into());
    let into = std::env::args().nth(4).unwrap_or_else(|| "CHANGED".into());

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");

    // `find` rather than a page number: the footer this is about is on many
    // pages and nobody knows which.
    let page = if std::env::args().nth(2).as_deref() == Some("find") {
        let mut found = 0;
        for candidate in 0..doc.page_count() {
            let text = doc.page(candidate).and_then(|p| p.text()).unwrap_or_default();
            if text.contains(&needle) {
                found = candidate;
                println!("found {needle:?} on page {}", candidate + 1);
                break;
            }
        }
        found
    } else {
        page
    };

    let runs = doc.text_runs(page).expect("runs");
    println!("page {} has {} run(s)", page + 1, runs.len());

    let hits: Vec<_> = runs
        .iter()
        .filter(|r| r.text.contains(&needle))
        .cloned()
        .collect();
    println!("{} run(s) contain {needle:?}\n", hits.len());

    for run in &hits {
        println!(
            "object {:>4}  size {:>5.2}  rect {:?}\n            origin {:?}\n            text {:?}",
            run.object, run.size, run.rect, run.origin, run.text
        );
    }

    let which: usize = std::env::args().nth(5).and_then(|n| n.parse().ok()).unwrap_or(0);
    let Some(target) = hits.get(which) else { return };
    println!("\n--- editing object {} to {into:?} ---", target.object);

    let before = doc.page(page).expect("page").text().unwrap_or_default();
    drop(doc);

    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    match doc.try_set_run_in_stream(page, target.object, &into) {
        Ok(()) => println!("the stream edit took"),
        Err(e) => {
            println!("refused: {e}");
            return;
        }
    }
    let after = doc.page(page).expect("page").text().unwrap_or_default();

    // What the line looks like now, next to what it looked like.
    let line_of = |text: &str| -> String {
        text.lines()
            .find(|l| l.contains(&needle) || l.contains(&into))
            .unwrap_or("")
            .to_string()
    };
    println!("\nbefore: {:?}", line_of(&before));
    println!("after : {:?}", line_of(&after));

    println!("\nwhole page before {} chars, after {} chars", before.len(), after.len());
    if after.len() > before.len() + 32 {
        println!("GREW by {} — something was added, not replaced", after.len() - before.len());
    }

    // And how many times the needle appears, which is the reported symptom.
    println!(
        "{needle:?} appears {} time(s) before, {} time(s) after",
        before.matches(&needle).count(),
        after.matches(&needle).count()
    );
}

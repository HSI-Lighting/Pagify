//! Can a closed document poison the next one, with no concurrency at all?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example handle_reuse_probe
//! ```
//!
//! A parallel test run reported `[200, 612, 612, 612]` after a delete-and-reopen:
//! the right page count, the first width correct, the rest US Letter — which is
//! what PDFium reports for a page whose geometry it cannot find. Serialising the
//! tests made it go away, and that is exactly the kind of fix that hides a bug
//! rather than removing it.
//!
//! The suspicion is `PdfPageIndexCache`, a process-global map keyed on the raw
//! `(FPDF_DOCUMENT, FPDF_PAGE)` pointer values. It has no purge-on-close: entries
//! leave only when a `PdfPage` drops. PDFium reuses pointer values after a close,
//! so if anything survives a document's lifetime, the *next* document to be handed
//! that address inherits it — and that needs no threads whatsoever.
//!
//! If that is the mechanism, the app hits it every time a user closes one document
//! and opens another, and serialising the write path would not help at all. So the
//! question has to be answered sequentially, before anything is wired up.
use pdfium_render::prelude::*;

/// Widths as PDFium reports them, which is the number that went wrong.
fn widths(doc: &PdfDocument) -> Vec<i32> {
    (0..doc.pages().len())
        .filter_map(|i| doc.pages().get(i).ok())
        .map(|p| p.width().value.round() as i32)
        .collect()
}

fn main() {
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));

    // Resolved against the crate root so the probe runs from anywhere.
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let ladder = format!("{fixtures}/pages-ladder.pdf");
    let mixed = format!("{fixtures}/mixed-sizes.pdf");

    // The truth, established once while nothing else has ever been opened.
    let (truth_ladder, truth_mixed) = {
        let a = pdfium.load_pdf_from_file(&ladder, None).expect("ladder");
        let b = pdfium.load_pdf_from_file(&mixed, None).expect("mixed");
        (widths(&a), widths(&b))
    };
    println!("ladder truth : {truth_ladder:?}");
    println!("mixed  truth : {truth_mixed:?}\n");

    let mut seen_addresses: Vec<usize> = Vec::new();
    let mut failures = 0;

    // Alternate the two documents so handle values are recycled between different
    // files. Identical files would still reuse addresses, but a stale entry would
    // then be indistinguishable from a correct one.
    for round in 0..8 {
        let (path, truth, label) = if round % 2 == 0 {
            (&ladder, &truth_ladder, "ladder")
        } else {
            (&mixed, &truth_mixed, "mixed ")
        };

        let doc = pdfium.load_pdf_from_file(path, None).expect("open");
        let address = doc.handle() as usize;
        let reused = seen_addresses.contains(&address);
        seen_addresses.push(address);

        // Touch every page, which is what populates the cache in the first place.
        let observed = widths(&doc);
        let ok = observed == *truth;
        if !ok {
            failures += 1;
        }

        println!(
            "round {round}  {label}  handle {address:#x}{}  {observed:?}  {}",
            if reused { " (REUSED)" } else { "" },
            if ok { "ok" } else { "<-- WRONG" },
        );

        // Dropped at the end of the iteration, closing the document and freeing
        // the address for the next round.
    }

    println!(
        "\n{} of 8 rounds reported the wrong page geometry",
        failures
    );
    if failures == 0 {
        println!(
            "sequential open/close does NOT poison a later document — whatever broke \
             the parallel run needs concurrency to happen"
        );
    }
}

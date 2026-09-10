//! How many of a page's runs can be edited, and why the rest cannot.
//!
//! Reported from use: "editing text in this is nearly impossible." The edit
//! path refuses rather than falling back — falling back re-emits the page and
//! rewrites paragraphs nobody touched — so a refusal is the honest answer and
//! also the whole user experience. This counts them, by reason.
//!
//! Each run is tried twice: once with its **own words**, which isolates whether
//! the machinery can find and line up the run at all, and once with a plain
//! replacement, which additionally asks whether the font can spell new letters.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example edit_coverage_probe -- <file.pdf> [page|all]
//! ```

use std::collections::BTreeMap;

use pdf_core::document::{Document, DocumentMut};
use pdf_core::document::pdfium_doc::PdfiumDocument;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let which = std::env::args().nth(2).unwrap_or_else(|| "0".into());
    let into = std::env::args().nth(3).unwrap_or_else(|| "Sample text".into());

    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    let pages: Vec<usize> = if which == "all" {
        (0..doc.page_count()).collect()
    } else {
        vec![which.parse().unwrap_or(0)]
    };
    drop(doc);

    let mut same_ok = 0usize;
    let mut new_ok = 0usize;
    let mut total = 0usize;
    let mut why_same: BTreeMap<String, usize> = BTreeMap::new();
    let mut why_new: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();
    let mut swapped_to: BTreeMap<String, usize> = BTreeMap::new();

    for page in &pages {
        let doc = PdfiumDocument::open_path(&path, None).expect("open");
        let runs = doc.text_runs(*page).expect("runs");
        drop(doc);

        for run in &runs {
            if run.text.trim().is_empty() {
                continue;
            }
            total += 1;

            // Its own words: can the run be found and lined up at all?
            let mut doc = PdfiumDocument::open_path(&path, None).expect("open");
            match doc.try_set_run_in_stream(*page, run.object, &run.text) {
                Ok(()) => same_ok += 1,
                Err(e) => {
                    let reason = short(&e.to_string());
                    *why_same.entry(reason.clone()).or_default() += 1;
                    if examples.len() < 8 {
                        examples.push(format!(
                            "  p{} obj {:>4} {:?} — {reason}",
                            page + 1,
                            run.object,
                            run.text.chars().take(40).collect::<String>()
                        ));
                    }
                }
            }

            // And ordinary new words, which most editing actually is.
            let mut doc = PdfiumDocument::open_path(&path, None).expect("open");
            doc.set_typing_fonts(typing_fonts());
            match doc.try_set_run_in_stream(*page, run.object, &into) {
                Ok(()) => {
                    new_ok += 1;
                    if let Some(face) = doc.substituted_face() {
                        *swapped_to.entry(face.to_string()).or_default() += 1;
                    }
                }
                Err(e) => *why_new.entry(short(&e.to_string())).or_default() += 1,
            }
        }
    }

    println!("{} run(s) with words, over {} page(s)\n", total, pages.len());
    let pct = |n: usize| if total == 0 { 0.0 } else { 100.0 * n as f32 / total as f32 };
    println!("editable with their own words : {same_ok} ({:.0}%)", pct(same_ok));
    println!("editable with {into:?}       : {new_ok} ({:.0}%)", pct(new_ok));

    println!("\nwhy the machinery could not place a run:");
    for (reason, count) in &why_same {
        println!("  {count:>4}  {reason}");
    }
    if !swapped_to.is_empty() {
        println!("\nwritten in a face that was not the run's own:");
        for (face, count) in &swapped_to {
            println!("  {count:>4}  {face}");
        }
    }
    println!("\nwhy new words were refused:");
    for (reason, count) in &why_new {
        println!("  {count:>4}  {reason}");
    }
    if !examples.is_empty() {
        println!("\nexamples:");
        for line in &examples {
            println!("{line}");
        }
    }
}

/// The fonts this probe offers for typing, as the app does.
fn typing_fonts() -> Vec<Vec<u8>> {
    ["Montserrat-Regular.ttf", "Montserrat-Bold.ttf"]
        .iter()
        .filter_map(|name| {
            std::fs::read(format!("/Users/hsilighting/pagify/third_party/fonts/{name}")).ok()
        })
        .collect()
}

/// The reason, without the wrapper the error type adds.
fn short(said: &str) -> String {
    said.replace(" is not implemented yet", "").trim().to_string()
}

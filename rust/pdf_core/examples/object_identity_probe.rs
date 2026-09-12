//! After a byte-safe move, does every other object still have the same index?
//!
//! The app keeps a run's object index across a move — the editor nudges its box
//! and goes on editing the same run. If inserting the two positioning operators
//! makes PDFium group the page's text differently, that index now names a
//! different run, and the next edit writes onto the wrong words.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(3);
    let by = Point { x: 20.0, y: 12.0 };
    let password = std::env::var("PDF_PASSWORD").ok();
    let open = |p: &str| PdfiumDocument::open_path(p, password.as_deref());

    let pages = open(&path).expect("open").page_count().min(limit);
    let (mut kept, mut renumbered) = (0usize, 0usize);
    let mut examples: Vec<String> = Vec::new();

    for page in 0..pages {
        let doc = open(&path).expect("open");
        let runs = doc.text_runs(page).unwrap_or_default();
        drop(doc);

        for run in runs.iter().take(10) {
            let doc = open(&path).expect("open");
            let before: Vec<(usize, String)> = doc
                .text_runs(page)
                .unwrap_or_default()
                .iter()
                .map(|r| (r.object, r.text.trim().to_string()))
                .collect();
            drop(doc);

            let mut doc = open(&path).expect("open");
            if doc.try_move_run_in_stream(page, run.object, by).is_err() {
                continue;
            }
            let after: Vec<(usize, String)> = doc
                .text_runs(page)
                .unwrap_or_default()
                .iter()
                .map(|r| (r.object, r.text.trim().to_string()))
                .collect();

            // Every object index that existed before must still name the same
            // words afterwards.
            let mut moved_wrong = None;
            for (object, words) in &before {
                let now = after.iter().find(|(o, _)| o == object).map(|(_, w)| w.clone());
                if now.as_deref() != Some(words.as_str()) {
                    moved_wrong = Some(format!(
                        "moving object {} ({:?}): object {object} was {:?}, is now {:?}",
                        run.object,
                        run.text.trim().chars().take(20).collect::<String>(),
                        words.chars().take(20).collect::<String>(),
                        now.map(|w| w.chars().take(20).collect::<String>()),
                    ));
                    break;
                }
            }
            match moved_wrong {
                None => kept += 1,
                Some(why) => {
                    renumbered += 1;
                    if examples.len() < 4 {
                        examples.push(why);
                    }
                }
            }
        }
    }

    println!("{path}");
    println!("  indices held    : {kept}");
    println!("  indices shifted : {renumbered}");
    for e in &examples {
        println!("     — {e}");
    }
}

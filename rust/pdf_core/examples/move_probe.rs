//! Does moving one object disturb the rest of the page?
//!
//! Moving needs PDFium's object model, and committing a change to it needs
//! `FPDFPage_GenerateContent` — which re-emits the whole content stream. This
//! crate avoids that everywhere it can, because it has been caught rewriting
//! paragraphs nobody touched. So: measure, before offering the feature.
use pdf_core::document::{Document, DocumentMut, Point};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let object: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let before_text = doc.page(page).expect("page").text().unwrap_or_default();
    let before_runs = doc.text_runs(page).expect("runs");
    println!("before: {} run(s), {} characters", before_runs.len(), before_text.len());
    drop(doc);

    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let was = doc.object_bounds(page, object).expect("bounds");
    doc.move_object(page, object, Point { x: 20.0, y: 12.0 }).expect("move");
    let now = doc.object_bounds(page, object).expect("bounds");
    println!(
        "object {object} moved by ({:+.2}, {:+.2}) — asked for (+20.00, +12.00)",
        now.left - was.left,
        now.top - was.top
    );

    let after_text = doc.page(page).expect("page").text().unwrap_or_default();
    let after_runs = doc.text_runs(page).expect("runs");
    println!("after:  {} run(s), {} characters", after_runs.len(), after_text.len());
    println!(
        "\nthe page's words are {}",
        if after_text == before_text { "unchanged" } else { "DIFFERENT" }
    );
    if after_text != before_text {
        // Where they first differ, with a little either side.
        let (a, b): (Vec<char>, Vec<char>) =
            (before_text.chars().collect(), after_text.chars().collect());
        let at = a.iter().zip(b.iter()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        let from = at.saturating_sub(30);
        println!(
            "   first differ at character {at}:\n   before: {:?}\n   after:  {:?}",
            a[from..(at + 30).min(a.len())].iter().collect::<String>(),
            b[from..(at + 30).min(b.len())].iter().collect::<String>()
        );
    }

    // The words themselves, as a set — reading order follows position, so
    // moving a text object *should* reorder the page. What must not change is
    // which words are on it.
    let mut was: Vec<String> = before_runs.iter().map(|r| r.text.trim().to_string()).collect();
    let mut now: Vec<String> = after_runs.iter().map(|r| r.text.trim().to_string()).collect();
    was.sort();
    now.sort();
    println!(
        "the page's runs are {}",
        if was == now { "the same words" } else { "DIFFERENT WORDS" }
    );
    if was != now {
        for (a, b) in was.iter().zip(now.iter()).filter(|(a, b)| a != b).take(3) {
            println!("   was {a:?}\n   now {b:?}");
        }
    }

    // And where every *other* run sits, which is the part a re-emission moves.
    let mut moved = 0;
    let mut worst = 0.0f32;
    for (a, b) in before_runs.iter().zip(after_runs.iter()) {
        if a.object == object {
            continue;
        }
        let shift = (a.rect.left - b.rect.left).abs().max((a.rect.top - b.rect.top).abs());
        if shift > 0.1 {
            moved += 1;
            worst = worst.max(shift);
        }
    }
    println!("{moved} other run(s) moved; the worst by {worst:.2}pt");

    // Through a save, which is where a re-emission really lands.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    let saved = reopened.page(page).expect("page").text().unwrap_or_default();
    println!(
        "after saving, the words are {}",
        if saved == before_text { "unchanged" } else { "DIFFERENT" }
    );
}

//! Does re-emitting a page's content stream, changing nothing, damage it?
use pdf_core::document::Document;
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let before: Vec<String> = doc.text_runs(page).expect("runs")
        .iter().map(|r| r.text.trim().to_string()).collect();
    drop(doc);

    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    doc.regenerate_only(page).expect("regenerate");
    let after: Vec<String> = doc.text_runs(page).expect("runs")
        .iter().map(|r| r.text.trim().to_string()).collect();

    println!("page {}: {} run(s) before, {} after", page + 1, before.len(), after.len());
    let changed = before.iter().zip(after.iter()).filter(|(a, b)| a != b).count();
    println!("{changed} run(s) came back saying something different");
    for (a, b) in before.iter().zip(after.iter()).filter(|(a, b)| a != b).take(4) {
        println!("   was {a:?}\n   now {b:?}");
    }
    println!(
        "\nre-emitting alone {} this page",
        if changed == 0 { "leaves" } else { "DAMAGES" }
    );
}

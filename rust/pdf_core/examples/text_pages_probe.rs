//! How many of a document's pages have text a pointer can click on.
use pdf_core::document::Document;
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let upto: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(20);
    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let pages = doc.page_count().min(upto);
    let (mut with, mut without) = (0usize, 0usize);
    for page in 0..pages {
        let runs = doc.text_runs(page).map(|r| r.len()).unwrap_or(0);
        if runs == 0 { without += 1 } else { with += 1 }
        if page < 12 {
            println!("  page {:>3}: {runs} run(s)", page + 1);
        }
    }
    println!("\nof the first {pages} pages: {with} have runs, {without} have none");
}

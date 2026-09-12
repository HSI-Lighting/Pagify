//! The app's own marks, as written into a page's stream.
use pdf_core::document::pdfium_doc::PdfiumDocument;
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let password = std::env::var("PDF_PASSWORD").ok();
    let doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    let stream = doc.page_stream(page).expect("stream");
    let text = String::from_utf8_lossy(&stream);
    for (i, hit) in text.match_indices("/Pagify").take(5) {
        let end = text[i..].find("BDC").map(|e| i + e + 3).unwrap_or(text.len().min(i + 2000));
        println!("--- at byte {i} ---\n{}\n", &text[i..end]);
    }
}

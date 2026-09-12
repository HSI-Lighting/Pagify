//! Print a window of a page's content-stream operations.
use pdf_core::document::pdfium_doc::PdfiumDocument;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let from: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(0);
    let count: usize = std::env::args().nth(4).and_then(|p| p.parse().ok()).unwrap_or(20);

    let password = std::env::var("PDF_PASSWORD").ok();
    let doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    let stream = doc.page_stream(page).expect("stream");
    let operations = pdf_core::pdf::content::parse(&stream).expect("parse");
    let states = pdf_core::pdf::content::states(&operations);
    for (index, operation) in operations.iter().enumerate().skip(from).take(count) {
        let s = &states[index];
        println!(
            "{index:>6}  {:<64}  text[{:.1},{:.1}] line[{:.1},{:.1}]",
            String::from_utf8_lossy(&stream[operation.span.clone()])
                .replace('\n', " ").chars().take(64).collect::<String>(),
            s.text[4], s.text[5], s.line[4], s.line[5],
        );
    }
}

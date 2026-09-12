//! Re-stack one object and print the stream around where it landed.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Stacking};
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let object: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(0);
    let how = std::env::args().nth(4).unwrap_or("down".into());
    let needle = std::env::args().nth(5).unwrap_or("/Im5 Do".into());
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    let to = match how.as_str() { "back" => Stacking::Back, "up" => Stacking::Up, "front" => Stacking::Front, _ => Stacking::Down };
    doc.restack(page, object, to).expect("restack");
    let stream = doc.page_stream(page).expect("stream");
    let ops = pdf_core::pdf::content::parse(&stream).expect("parse");
    let at = ops.iter().position(|o| String::from_utf8_lossy(&stream[o.span.clone()]) == needle).expect("needle");
    for (i, op) in ops.iter().enumerate().skip(at.saturating_sub(22)).take(30) {
        println!("{i:>5}  {}", String::from_utf8_lossy(&stream[op.span.clone()]).replace('\n', " ").chars().take(70).collect::<String>());
    }
}

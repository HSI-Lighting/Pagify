//! What the survey makes of words drawn through a form.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=… cargo run --release --example form_cut_probe -- <pdf> [hit]
//! ```
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Redaction};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let hit: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(0);
    let mut doc = PdfiumDocument::open_path(&path, None).expect("open");
    let found = doc.sensitive_on(0).expect("scan");
    for (i, f) in found.iter().enumerate() {
        println!("hit {i}: {:?} at {:?}", f.text, f.area);
    }
    let area = found[hit].area;
    let report = doc.preview_redaction(&Redaction::new(0, area), None).expect("survey");
    println!("survey: {report:#?}");
    let done = doc.redact(&Redaction::new(0, area), None);
    println!("apply: {done:#?}");
    let text = doc.page(0).expect("page").text().expect("text");
    println!("after: {text}");
}

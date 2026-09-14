//! Apply a redaction over the first words drawn through a form on a page,
//! save a copy beside the input, and read both pages back.
//!
//! ```text
//! PDF_PASSWORD=… cargo run --release --example form_apply_probe -- <pdf> <page> <out.pdf>
//! ```
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Redaction};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).expect("page");
    let out = std::env::args().nth(3).expect("out");
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    let drawn = doc.drawn_objects(page).expect("drawn");
    let first = drawn
        .iter()
        .find(|d| d.depth > 0 && d.kind == pdf_core::document::DrawnKind::Words)
        .expect("a nested run");
    let r = first.rect;
    println!("target: {:?} {:?}", first.label, r);
    let area = pdf_core::document::Rect { left: r.left - 1.0, top: r.top - 1.0, right: r.right + 1.0, bottom: r.bottom + 1.0 };
    let before = doc.page(page).expect("page").text().expect("text");
    let report = doc.redact(&Redaction::new(page, area), None).expect("redact");
    println!("report: {report:#?}");
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    std::fs::write(&out, &bytes).expect("write");
    drop(doc);
    let reopened = PdfiumDocument::open_path(&out, password.as_deref()).expect("reopen");
    let after = reopened.page(page).expect("page").text().expect("text");
    println!("label still on the page: {}", after.contains(first.label.trim()));
    println!("before {} chars, after {} chars", before.chars().count(), after.chars().count());
    if page + 1 < reopened.page_count() {
        let next = reopened.page(page + 1).expect("page").text().expect("text");
        println!("next page still has the label: {}", next.contains(first.label.trim()));
    }
}

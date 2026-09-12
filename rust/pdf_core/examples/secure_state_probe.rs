//! What the save guard sees after an edit on a password-protected file.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point};
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    let report = |doc: &PdfiumDocument, when: &str| {
        println!("{when:<28} is_secured={:<5} had_password_on_open={}", doc.is_secured(), doc.had_password_on_open());
    };
    report(&doc, "after open");
    let (completed, dropped) = doc.repair_locks().expect("repair");
    println!("   repair_locks -> ({completed}, {dropped})");
    report(&doc, "after repair_locks");
    let picture = doc.images_on(2).expect("images")[0].object;
    doc.move_object(2, picture, Point { x: 5.0, y: 5.0 }).expect("move");
    report(&doc, "after moving a picture");
}

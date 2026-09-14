//! What PDFium reports for objects inside a form: their bounds and matrix,
//! against the form object's own matrix.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    for d in doc.drawn_objects(0).expect("drawn") {
        println!("depth {} {:?} {:?} rect {:?}", d.depth, d.kind, d.label, d.rect);
    }
}

//! What does a Pagify save actually write?
//!
//! The question that decides how much of a PDF reader Lock needs. Blanking an
//! image without letting `FPDFPage_GenerateContent` re-emit the page means
//! editing the saved file ourselves — and how hard that is depends entirely on
//! what PDFium emits: a classic cross-reference table is a few hundred lines to
//! read and rewrite, a cross-reference *stream* needs Flate and a second
//! format, and object streams need a third.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example save_shape_probe -- <file.pdf>
//! ```

use pdf_core::document::{Document, DocumentMut};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    println!("saved {} bytes", bytes.len());

    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(16)]);
    println!("header: {:?}", head.lines().next().unwrap_or(""));

    // The tail is where the shape shows: `xref` for a classic table, or an
    // object number followed by `obj` for a cross-reference stream.
    let tail_at = bytes.len().saturating_sub(400);
    let tail = String::from_utf8_lossy(&bytes[tail_at..]);
    println!("\n--- last 400 bytes ---\n{tail}");

    let classic = bytes.windows(5).filter(|w| *w == b"xref\n" || *w == b"xref\r").count();
    let streams = bytes.windows(12).filter(|w| w == b"/ObjStm").count();
    let xref_streams = bytes.windows(9).filter(|w| w == b"/XRef").count();
    println!("\nclassic `xref` sections: {classic}");
    println!("/XRef  (cross-reference streams): {xref_streams}");
    println!("/ObjStm (objects packed in streams): {streams}");
}

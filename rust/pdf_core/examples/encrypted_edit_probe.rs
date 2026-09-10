//! Can text be edited in a document that carries a password?
//!
//! The content-stream editor works on the bytes PDFium would write, and PDFium
//! keeps a document's encryption when it saves one it opened encrypted. So the
//! reader is handed ciphertext and cannot inflate it — which surfaces as "a
//! content stream filter this build cannot read".
use pdf_core::document::{Document, DocumentMut};
use pdf_core::document::pdfium_doc::PdfiumDocument;

fn main() {
    let path = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(
        "fixtures/two-column.pdf",
        None,
    );
    let mut plain = path.expect("open");
    let run = plain.text_runs(0).expect("runs")
        .into_iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .expect("a run");
    println!("editing {:?} on a plain document:", run.text.trim());
    match plain.try_set_run_in_stream(0, run.object, "Replaced") {
        Ok(()) => println!("   ok"),
        Err(e) => println!("   refused: {e}"),
    }

    // The same document, with a password on it.
    let mut secured = PdfiumDocument::open_path("fixtures/two-column.pdf", None).expect("open");
    secured
        .secure_document(b"Correct-Horse-99", None, pdf_core::pdf::encrypt::Permissions::all())
        .expect("secure");
    let mut bytes = Vec::new();
    secured.save_full_copy(&mut bytes).expect("save");
    println!("saved {} bytes with a password", bytes.len());

    let mut reopened =
        PdfiumDocument::open_bytes(bytes, Some("Correct-Horse-99")).expect("reopen");
    let run = reopened.text_runs(0).expect("runs")
        .into_iter()
        .find(|r| r.text.trim().chars().count() > 5)
        .expect("a run");
    println!("editing {:?} on the secured one:", run.text.trim());
    match reopened.try_set_run_in_stream(0, run.object, "Replaced") {
        Ok(()) => println!("   ok"),
        Err(e) => println!("   refused: {e}"),
    }

    // And the password must still be there afterwards.
    println!("still secured in memory: {}", reopened.is_secured());
    let mut out = Vec::new();
    reopened.save_full_copy(&mut out).expect("save");
    let plain = PdfiumDocument::open_bytes(out.clone(), None);
    println!(
        "the saved file {} a password",
        if plain.is_err() { "still needs" } else { "NO LONGER NEEDS" }
    );
    let with = PdfiumDocument::open_bytes(out, Some("Correct-Horse-99")).expect("reopen");
    let text = with.page(0).expect("page").text().unwrap_or_default();
    println!("and it says {:?}", text.chars().take(40).collect::<String>());
}

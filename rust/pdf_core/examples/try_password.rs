//! Does this password open this file?
//!
//! For answering that question without a window, and without the guessing that
//! a prompt encourages. Each candidate is tried once and the answer is plain.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example try_password -- <file.pdf> <password>...
//! ```

use pdf_core::document::Document;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a pdf path");
    let candidates: Vec<String> = args.collect();

    if candidates.is_empty() {
        println!("give one or more passwords to try");
        return;
    }
    for password in &candidates {
        match pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, Some(password)) {
            Ok(doc) => {
                let text = doc
                    .page(0)
                    .and_then(|p| p.characters())
                    .map(|c| c.text)
                    .unwrap_or_default();
                println!(
                    "OPENS: {password:?} — {} page(s), first words {:?}",
                    doc.page_count(),
                    text.chars().take(40).collect::<String>()
                );
                return;
            }
            Err(e) => println!("  no: {password:?} ({e})"),
        }
    }
    println!("\nnone of those opened it");
}

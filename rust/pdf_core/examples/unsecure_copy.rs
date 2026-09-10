//! A plain copy of a secured document, given its password.
//!
//! Not a way past a password — it needs the password. It is the way back for
//! somebody who has one and wants an ordinary file again.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example unsecure_copy -- <in.pdf> <password> <out.pdf>
//! ```

use pdf_core::document::{Document, DocumentMut};

fn main() {
    let mut args = std::env::args().skip(1);
    let from = args.next().expect("a pdf path");
    let password = args.next().expect("its password");
    let to = args.next().expect("where the plain copy goes");

    if std::path::Path::new(&to).exists() {
        println!("{to} already exists — refusing to write over it");
        return;
    }

    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&from, Some(&password))
            .expect("the password did not open it");
    println!("opened {} page(s)", doc.page_count());

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    std::fs::write(&to, &bytes).expect("write");
    println!("wrote {to} ({} bytes)", bytes.len());

    // Checked rather than assumed: the point is a file that opens with nothing.
    match pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&to, None) {
        Ok(again) => {
            let text = again
                .page(0)
                .and_then(|p| p.characters())
                .map(|c| c.text)
                .unwrap_or_default();
            println!(
                "opens with no password: {} page(s), first words {:?}",
                again.page_count(),
                text.chars().take(44).collect::<String>()
            );
        }
        Err(e) => println!("but it still will not open plainly: {e}"),
    }
}

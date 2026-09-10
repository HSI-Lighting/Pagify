//! Does `unsecure` then `saveas` actually produce a plain copy?
//!
//! It is what the refusal message tells somebody to do when they want to change
//! a password, so it had better be true.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example unsecure_route_probe -- <file.pdf>
//! ```

use pdf_core::document::{Document, DocumentMut};
use pdf_core::pdf::encrypt::Permissions;

type Doc = pdf_core::document::pdfium_doc::PdfiumDocument;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let first = "Correct-Horse-99-Battery";

    // Make one with a password.
    let mut doc = Doc::open_path(&path, None).expect("open");
    doc.secure_document(first.as_bytes(), None, Permissions::all()).expect("secure");
    let mut secured = Vec::new();
    doc.save_full_copy(&mut secured).expect("save");
    println!("secured: opens with nothing = {}", Doc::open_bytes(secured.clone(), None).is_ok());

    // Which FPDF_REMOVE_SECURITY value does this build use? It is 3 in older
    // PDFium and 4 in newer, and guessing would silently save an encrypted
    // file while believing otherwise.
    for flag in [3u32, 4u32] {
        let again = Doc::open_bytes(secured.clone(), Some(first)).expect("reopen");
        match again.save_flagged(flag) {
            Ok(bytes) => println!(
                "flag {flag}: {} bytes, opens with nothing = {}",
                bytes.len(),
                Doc::open_bytes(bytes, None).is_ok()
            ),
            Err(e) => println!("flag {flag}: refused ({e})"),
        }
    }

    // Now the route the message recommends.
    let mut again = Doc::open_bytes(secured, Some(first)).expect("reopen with the password");
    match again.unsecure_document() {
        Ok(()) => println!("`unsecure` was accepted"),
        Err(e) => println!("`unsecure` said: {e}"),
    }
    let mut plain = Vec::new();
    again.save_full_copy(&mut plain).expect("saveas");

    let opens_plainly = Doc::open_bytes(plain.clone(), None).is_ok();
    let still_wants_it = Doc::open_bytes(plain, Some(first)).is_ok();
    println!("the copy opens with nothing     : {opens_plainly}");
    println!("the copy opens with the password: {still_wants_it}");
    println!(
        "\n{}",
        if opens_plainly {
            "the route works — the copy is plain"
        } else {
            "THE ROUTE DOES NOT WORK — the copy is still encrypted"
        }
    );
}

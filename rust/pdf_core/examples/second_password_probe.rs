//! What happens when a document that already has a password is given another?
//!
//! There are three possible answers and only one of them is acceptable: the new
//! password works, the old one still works, or **neither does** — a file nobody
//! can open. The last is the one worth finding before somebody else does.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example second_password_probe -- <file.pdf>
//! ```

use pdf_core::document::{Document, DocumentMut};
use pdf_core::pdf::encrypt::Permissions;

type Doc = pdf_core::document::pdfium_doc::PdfiumDocument;

fn opens(bytes: &[u8], password: Option<&str>) -> bool {
    Doc::open_bytes(bytes.to_vec(), password).is_ok()
}

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let (first, second) = ("first password", "second password");

    let mut doc = Doc::open_path(&path, None).expect("open");
    doc.secure_document(first.as_bytes(), None, Permissions::all()).expect("secure once");
    let mut once = Vec::new();
    doc.save_full_copy(&mut once).expect("save");
    println!("after one password:");
    println!("  opens with nothing     : {}", opens(&once, None));
    println!("  opens with the first   : {}", opens(&once, Some(first)));

    // Now the case in question: reopen it and set a different password.
    let mut again = Doc::open_bytes(once.clone(), Some(first)).expect("reopen");
    match again.secure_document(second.as_bytes(), None, Permissions::all()) {
        Ok(()) => println!("\nsetting a second password was accepted"),
        Err(e) => {
            println!("\nsetting a second password was refused: {e}");
            return;
        }
    }

    let mut twice = Vec::new();
    match again.save_full_copy(&mut twice) {
        Ok(()) => {
            println!("saving it was accepted — {} bytes", twice.len());
            let nothing = opens(&twice, None);
            let old = opens(&twice, Some(first));
            let new = opens(&twice, Some(second));
            println!("  opens with nothing     : {nothing}");
            println!("  opens with the first   : {old}");
            println!("  opens with the second  : {new}");
            println!(
                "\n{}",
                if !nothing && !old && !new {
                    "RUINED — the document opens for nobody at all"
                } else if new {
                    "fine — the new password opens it"
                } else if old {
                    "the old password still opens it; the new one did nothing"
                } else {
                    "it is readable by anyone — the password came off"
                }
            );
        }
        Err(e) => println!("saving it was refused: {e}\n\nfine — nothing was written"),
    }
}

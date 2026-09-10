//! Does this file actually need a password, or only a permissions one?
//!
//! A great many real PDFs — catalogues, datasheets, drawings — carry an
//! **owner** password that restricts printing or copying while leaving the
//! *user* password empty. Those open perfectly well with an empty string, and
//! refusing them as "password protected" locks the reader out of a document
//! anyone can read.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example password_probe -- <pdf>
//! ```

use pdfium_render::prelude::*;

fn main() {
    let path = std::env::args().nth(1).expect("usage: password_probe <pdf>");
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind"));

    let given = std::env::args().nth(2);
    for (label, password) in [
        ("no password", None),
        ("empty password", Some("")),
        ("given", given.as_deref()),
    ] {
        if label == "given" && password.is_none() { continue; }
        match pdfium.load_pdf_from_file(&path, password) {
            Ok(doc) => println!("  {label:<16} opens — {} pages", doc.pages().len()),
            Err(e) => println!("  {label:<16} {e:?}"),
        }
    }
}

//! Can anyone else open the file we secured?
//!
//! The only test that matters for encryption. Our own code agreeing with itself
//! proves nothing — the reader on the other side is somebody else's, and the
//! bytes have to be exactly what the specification says. PDFium is that reader
//! here: it must open the file **with** the password and refuse it **without**.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example secure_probe -- <file.pdf>
//! ```

use pdf_core::document::Document;
use pdf_core::pdf::encrypt::{secure, Permissions, Security};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let password = "correct horse battery staple";

    let bytes = std::fs::read(&path).expect("read");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");

    let random = |buffer: &mut [u8]| {
        use rand_core::RngCore;
        rand_core::OsRng.fill_bytes(buffer);
    };
    // `secure_probe <file> [out] [readonly]`
    let permissions = if std::env::args().any(|a| a == "readonly") {
        Permissions::read_only()
    } else {
        Permissions::all()
    };
    let security = Security::new(password.as_bytes(), None, permissions, random)
        .expect("set up security");
    let sealed = secure(&file, &security, random).expect("secure the document");

    println!("{} bytes in, {} bytes out", bytes.len(), sealed.len());
    if let Some(out) = std::env::args().nth(2) {
        std::fs::write(&out, &sealed).expect("write");
        println!("written to {out}");
    }
    println!(
        "carries an /Encrypt dictionary: {}",
        String::from_utf8_lossy(&sealed[sealed.len().saturating_sub(600)..]).contains("/Encrypt")
    );

    // Without the password.
    let refused =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(sealed.clone(), None);
    println!("\nopening with no password : {}", match &refused {
        Ok(_) => "OPENED — the document is not secured".to_string(),
        Err(e) => format!("refused ({e})"),
    });

    // With it.
    let opened =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(sealed, Some(password));
    match opened {
        Ok(doc) => {
            println!("opening with the password: opened, {} page(s)", doc.page_count());
            let text = doc
                .page(0)
                .and_then(|p| p.characters())
                .map(|c| c.text)
                .unwrap_or_default();
            println!(
                "  first words back: {:?}",
                text.chars().take(48).collect::<String>()
            );
            println!(
                "\n{}",
                if refused.is_err() && !text.is_empty() {
                    "SECURED — another reader needs the password, and gets the document with it"
                } else {
                    "NOT RIGHT — see above"
                }
            );
        }
        Err(e) => println!("opening with the password: FAILED ({e})"),
    }
}

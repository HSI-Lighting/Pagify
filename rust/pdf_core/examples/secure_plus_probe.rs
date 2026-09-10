//! Does Secure Plus seal a document, shut every other reader out, and open again?
//!
//! Three questions, and the middle one is the whole point of the feature:
//! `/Filter /Pagify` is a security handler nobody else implements, so a
//! conforming reader must refuse the file rather than ask for a password.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example secure_plus_probe -- <file.pdf> [out.pdf]
//! ```

use pdf_core::crypto::kdf::KdfParams;
use pdf_core::document::Document;
use pdf_core::pdf::{secure_plus, File};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let password = b"Correct-Horse-99-Battery";

    let bytes = std::fs::read(&path).expect("read");
    let file = File::parse(&bytes).expect("parse");

    let random = |buffer: &mut [u8]| {
        use rand_core::RngCore;
        rand_core::OsRng.fill_bytes(buffer);
    };
    let plus = secure_plus::SecurePlus::new(password, KdfParams::default(), random)
        .expect("derive a key");
    let sealed = secure_plus::secure(&file, &plus).expect("seal");
    println!("{} bytes in, {} bytes out", bytes.len(), sealed.len());
    if let Some(out) = std::env::args().nth(2) {
        std::fs::write(&out, &sealed).expect("write");
        println!("written to {out}");
    }

    // 1. Nobody else opens it — not even with the password.
    for password in [None, Some("Correct-Horse-99-Battery")] {
        let outcome =
            pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(sealed.clone(), password);
        println!(
            "  PDFium with {:<28}: {}",
            match password { None => "no password".into(), Some(p) => format!("{p:?}") },
            match outcome { Ok(_) => "OPENED — that is wrong".into(), Err(e) => format!("refused ({e})") }
        );
    }

    // 2. It is recognisably ours.
    let again = File::parse(&sealed).expect("parse the sealed file");
    println!("  recognised as Secure Plus     : {}", secure_plus::dictionary_of(&again).is_some());

    // 3. And the right password brings it all back.
    let dict = secure_plus::dictionary_of(&again).expect("its own dictionary");
    assert!(
        secure_plus::SecurePlus::open(b"the wrong one", &dict).is_err(),
        "a wrong password derived a working key"
    );
    let opened = secure_plus::SecurePlus::open(password, &dict).expect("the password failed");
    let plain = secure_plus::unseal(&again, &opened).expect("unseal");

    match pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(plain, None) {
        Ok(doc) => {
            let text = doc.page(0).and_then(|p| p.characters()).map(|c| c.text).unwrap_or_default();
            println!(
                "  unsealed                      : {} page(s), first words {:?}",
                doc.page_count(),
                text.chars().take(44).collect::<String>()
            );
        }
        Err(e) => println!("  unsealed                      : FAILED ({e})"),
    }
}

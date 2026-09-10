//! Set a password, change it, save, reopen. Which password opens it?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example change_password_probe -- <file.pdf>
//! ```

use pdf_core::document::{Document, DocumentMut};
use pdf_core::pdf::encrypt::Permissions;

type Doc = pdf_core::document::pdfium_doc::PdfiumDocument;

fn opens(path: &str, password: Option<&str>) -> bool {
    Doc::open_path(path, password).is_ok()
}

fn main() {
    let source = std::env::args().nth(1).expect("a pdf path");
    let (old, new) = ("Old-Password-99!", "New-Password-99!");
    let out = std::env::temp_dir().join("pagify-change-password.pdf");
    let out = out.to_str().expect("path");

    // Secured with the first password, written out.
    {
        let mut doc = Doc::open_path(&source, None).expect("open");
        doc.secure_document(old.as_bytes(), None, Permissions::all()).expect("secure");
        let mut bytes = Vec::new();
        doc.save_full_copy(&mut bytes).expect("save");
        std::fs::write(out, &bytes).expect("write");
    }
    println!("after the first password:");
    println!("  opens with the old : {}", opens(out, Some(old)));
    println!("  opens with the new : {}", opens(out, Some(new)));

    // Reopened with it, and changed — exactly as the app does it.
    {
        let mut doc = Doc::open_path(out, Some(old)).expect("reopen with the old password");
        assert!(doc.password_matches(old.as_bytes()), "it did not recognise its own password");
        doc.unsecure_document().expect("take the old one off");
        doc.secure_document(new.as_bytes(), None, Permissions::all()).expect("put the new one on");
        let mut bytes = Vec::new();
        doc.save_full_copy(&mut bytes).expect("save");
        std::fs::write(out, &bytes).expect("write");
        println!("\nwrote {} bytes", bytes.len());
    }

    println!("\nafter changing it:");
    let old_works = opens(out, Some(old));
    let new_works = opens(out, Some(new));
    let none_works = opens(out, None);
    println!("  opens with nothing : {none_works}");
    println!("  opens with the old : {old_works}");
    println!("  opens with the new : {new_works}");
    println!(
        "\n{}",
        if new_works && !old_works && !none_works {
            "right — the new password, and only the new password"
        } else if old_works {
            "WRONG — the old password still opens it"
        } else {
            "WRONG — see above"
        }
    );
}

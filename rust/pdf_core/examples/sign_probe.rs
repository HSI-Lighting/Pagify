//! Sign a document, then check the signature the hard way.
//!
//! Four questions, and only the last two are interesting:
//!   1. does the file still open?
//!   2. does a reader see a signature in it?
//!   3. does the range it declares cover the whole file?
//!   4. does the engine's own check read it back as unaltered, under the
//!      certificate — and does the digest it committed to match the file?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example sign_probe -- <file.pdf> [out.pdf]
//! ```
//!
//! Signs with `fixtures/test-signer-sm2.p12` unless `P12=<path>` names another
//! identity, with its password in `P12_PASSWORD` (default `pagify`, the test
//! identity's own). The identity must hold an SM2 key; the RSA one in
//! `fixtures/test-signer.p12` is refused, which is the point of keeping it.

use pdf_core::document::Document;
use pdf_core::pdf::{sign, validate, File};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let p12 = std::env::var("P12").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/test-signer-sm2.p12")
    });
    let password = std::env::var("P12_PASSWORD").unwrap_or_else(|_| "pagify".into());

    let identity = match sign::Identity::from_pkcs12(&std::fs::read(&p12).expect("read"), &password) {
        Ok(identity) => identity,
        Err(e) => {
            println!("{}: {e}", p12.display());
            std::process::exit(1);
        }
    };
    println!("signing as: {} (SM2 over SM3)", identity.subject().unwrap_or_default());

    let bytes = std::fs::read(&path).expect("read");
    let file = File::parse(&bytes).expect("parse");
    let signed = sign::sign(
        &file,
        &identity,
        &sign::Reason { reason: "Approved".into(), location: "London".into(), ..Default::default() },
    )
    .expect("sign");
    println!("{} bytes in, {} bytes out", bytes.len(), signed.len());
    if let Some(out) = std::env::args().nth(2) {
        std::fs::write(&out, &signed).expect("write");
        println!("written to {out}");
    }

    // 1. Still a document.
    match pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(signed.clone(), None) {
        Ok(doc) => {
            let text = doc.page(0).and_then(|p| p.characters()).map(|c| c.text).unwrap_or_default();
            println!("  opens              : {} page(s), {:?}", doc.page_count(),
                text.chars().take(32).collect::<String>());
        }
        Err(e) => println!("  opens              : FAILED ({e})"),
    }

    // 2. A reader sees the signature.
    println!("  PDFium sees        : {} signature(s)", count_signatures(&signed));

    // 3. The range it declares covers the whole file but the hole.
    //
    // Read from `/ByteRange` in the finished file, which is what a reader does
    // — `find_placeholder` only works before the hole is filled.
    let range = range_from_file(&signed).expect("no /ByteRange in the signed file");
    println!("  range covers all   : {}", range.covers_everything());

    // 4. The engine's own check, which is what a Pagify reader runs.
    let file = File::parse(&signed).expect("parse");
    for found in validate::check(&file, &signed).expect("check") {
        println!(
            "  engine verdict     : {} — signer {:?}",
            found.verdict.describe(),
            found.signer
        );
    }
}

fn count_signatures(bytes: &[u8]) -> i32 {
    use pdf_core::document::pdfium_doc::PdfiumDocument;
    let Ok(doc) = PdfiumDocument::open_bytes(bytes.to_vec(), None) else { return -1 };
    doc.signature_count()
}

fn range_from_file(bytes: &[u8]) -> Option<sign::ByteRange> {
    let at = bytes.windows(10).position(|w| w == b"/ByteRange")?;
    let open = bytes[at..].iter().position(|b| *b == b'[').map(|n| at + n)?;
    let close = bytes[open..].iter().position(|b| *b == b']').map(|n| open + n)?;

    let numbers: Vec<usize> = String::from_utf8_lossy(&bytes[open + 1..close])
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    let [_start, first, second_at, _second_len] = numbers[..] else { return None };
    Some(sign::ByteRange {
        hole_at: first,
        hole_len: second_at - first,
        total: bytes.len(),
    })
}

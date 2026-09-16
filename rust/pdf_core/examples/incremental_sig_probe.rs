//! Certify, edit, save: are the signed bytes still the first bytes of the
//! file, and does the signature still verify over its range?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example incremental_sig_probe
//! ```
//!
//! The probe that found a whiteout after signing re-serialising the whole
//! file: "starts with the signed bytes: false", and the check reading
//! CHANGED once it looked at the digest before calling anything incomplete.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Color, DocumentMut, Rect};
use pdf_core::pdf::{sign, validate, File};

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut doc = PdfiumDocument::open_path(root.join("fixtures/two-column.pdf").to_str().unwrap(), None).unwrap();
    let p12 = std::fs::read(root.join("fixtures/test-signer-sm2.p12")).unwrap();
    doc.sign_document(&p12, "pagify", &sign::Reason::default()).unwrap();
    let mut signed = Vec::new();
    doc.save_incremental(&mut signed).unwrap();
    println!("signed: {} bytes", signed.len());
    let file = File::parse(&signed).unwrap();
    for s in validate::check(&file, &signed).unwrap() { println!("  after signing: {}", s.verdict.describe()); }

    doc.whiteout(0, Rect { left: 100.0, top: 100.0, right: 200.0, bottom: 120.0 }, Color { r: 255, g: 255, b: 255, a: 255 }).unwrap();
    let mut saved = Vec::new();
    doc.save_incremental(&mut saved).unwrap();
    println!("saved: {} bytes; starts with the signed bytes: {}", saved.len(), saved.starts_with(&signed));
    if !saved.starts_with(&signed) {
        let first = signed.iter().zip(saved.iter()).position(|(a, b)| a != b);
        println!("  first difference at byte {:?} of {}", first, signed.len());
        if let Some(at) = first {
            let lo = at.saturating_sub(40); let hi = (at + 40).min(signed.len()).min(saved.len());
            println!("  signed: {:?}", String::from_utf8_lossy(&signed[lo..hi]));
            println!("  saved : {:?}", String::from_utf8_lossy(&saved[lo..hi]));
        }
    }
    let file = File::parse(&saved).unwrap();
    for s in validate::check(&file, &saved).unwrap() { println!("  after edit+save: {} (signer {:?})", s.verdict.describe(), s.signer); }
    // The ByteRange the saved file declares.
    let at = saved.windows(10).position(|w| w == b"/ByteRange").unwrap();
    println!("  {}", String::from_utf8_lossy(&saved[at..at + 60]).lines().next().unwrap());
    let count = saved.windows(10).filter(|w| w == b"/ByteRange").count();
    println!("  /ByteRange occurrences: {count}");
}

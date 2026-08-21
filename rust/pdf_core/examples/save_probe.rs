//! Does the binding's save path append, or rewrite?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example save_probe -- <pdf>
//! ```
//!
//! Roadmap §4.1 commits to incremental save, because a digital signature covers a
//! byte range and a full rewrite relocates every object in the file. That decision
//! is only as good as what the binding actually emits, and `save_to_writer` in
//! pdfium-render 0.9.3 hardcodes `flags = 0` behind a TODO open since 2022.
//!
//! This settles it without needing a signed fixture. An incremental update is an
//! **append**: the original bytes stay byte-for-byte at the front of the file and a
//! delta plus a new cross-reference section follows. A full rewrite renumbers and
//! relocates, so the original bytes do not survive as a prefix. Comparing the
//! output against the input is therefore a direct read of which path ran.
use pdfium_render::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: save_probe <pdf>");

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));

    let original = std::fs::read(path).expect("read input");
    let doc = pdfium.load_pdf_from_file(path, None).expect("open");
    println!(
        "input   : {} bytes, {} pages",
        original.len(),
        doc.pages().len()
    );
    println!("signatures on input: {}", doc.signatures().len());

    // Save with no modification at all. Anything the save path does to the bytes
    // here is the save path's doing, not an edit's.
    let mut out: Vec<u8> = Vec::new();
    doc.save_to_writer(&mut out).expect("save");

    println!("output  : {} bytes", out.len());

    let shared = original
        .iter()
        .zip(out.iter())
        .take_while(|(a, b)| a == b)
        .count();

    println!("identical leading bytes: {shared}");
    println!(
        "original survives as an exact prefix: {}",
        out.len() >= original.len() && out[..original.len()] == original[..]
    );

    let verdict = if out.len() >= original.len() && out[..original.len()] == original[..] {
        "INCREMENTAL — original bytes preserved, delta appended"
    } else {
        "FULL REWRITE — original bytes did not survive"
    };
    println!("\nverdict : {verdict}");

    // A rewrite is also visible in the trailer: an incremental save leaves the
    // first %%EOF where it was and adds another after the appended section.
    let eofs = original.windows(5).filter(|w| w == b"%%EOF").count();
    let eofs_out = out.windows(5).filter(|w| w == b"%%EOF").count();
    println!("%%EOF markers: input {eofs}, output {eofs_out}");
}

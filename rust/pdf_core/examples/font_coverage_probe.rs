//! Can the font a text run is actually drawn in be asked what it can draw?
//!
//! The question Obfuscate turns on. `crypto::tweak::coverage` answers for fonts
//! *the app registered*, and returns `Unknown` for a document's own — which is
//! honest but, taken literally, refuses every obfuscation, since `Unknown` is
//! never safe.
//!
//! FF1 preserves the alphabet but not the *characters*: `4821000734` uses seven
//! distinct digits, and its replacement may use all ten. An embedded font is
//! usually subset to the glyphs the document already needed, so the three it
//! never used may simply not be there — and `pdfium_doc.rs`'s own text-splitting
//! code names this exact failure ("a subsetted font asked for a character the
//! document never contained"), sidestepping it by only ever writing a subset of
//! what was already drawn. Obfuscate cannot do that.
//!
//! So: `FPDFFont_GetFontData` hands back the embedded font's bytes, and
//! `ttf_parser` — already used by `document::glyphs` — says which characters it
//! has. This measures whether that route works on real files, and what it says
//! about a genuinely subset font.
//!
//! ```text
//! cargo run --example font_coverage_probe -- <file.pdf> [page]
//! ```

use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page_index: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let runs = doc.text_runs(page_index).expect("text runs");
    println!("page {} has {} text run(s)\n", page_index + 1, runs.len());

    // The self-check that decides whether a font's verdict can be believed at
    // all: a font that reports it cannot draw the text it is visibly drawing is
    // being read through the wrong mapping, and its answer about *other*
    // characters is worthless — "unknown", not "no".
    let (mut trustworthy, mut misread, mut unreadable, mut absent) = (0, 0, 0, 0);

    for run in &runs {
        if run.text.trim().is_empty() {
            continue;
        }
        match doc.run_font_data(page_index, run.object) {
            Ok(Some(data)) => match ttf_parser::Face::parse(&data, 0) {
                Ok(face) => {
                    let own = run.text.chars().filter(|c| !c.is_whitespace());
                    let total = own.clone().count();
                    let drawable = own.filter(|c| face.glyph_index(*c).is_some()).count();
                    if total > 0 && drawable == total {
                        trustworthy += 1;
                    } else {
                        misread += 1;
                        if misread <= 3 {
                            let shown: String = run.text.chars().take(40).collect();
                            println!(
                                "  misread  object {:>4}  {drawable}/{total}  {shown:?}",
                                run.object
                            );
                        }
                    }
                }
                Err(_) => unreadable += 1,
            },
            _ => absent += 1,
        }
    }

    let inspectable = trustworthy + misread;
    println!(
        "\n  font can draw its own text (verdict usable):  {trustworthy}\n  \
           font cannot (wrong mapping — must be Unknown): {misread}\n  \
           ttf_parser could not read the bytes:           {unreadable}\n  \
           no font data at all:                           {absent}"
    );
    if inspectable > 0 {
        println!(
            "\n  usable on {:.0}% of the runs that have an inspectable font",
            100.0 * trustworthy as f32 / inspectable as f32
        );
    }
}

//! What shape are a page's path objects?
//!
//! The classifier decides "text drawn as outlines" by counting glyph-sized
//! paths — small in both directions. That assumes **one path per glyph**. A
//! producer that emits one path per word or per line defeats it, and the page
//! is then called `Native` while most of its text is invisible to extraction.
//! This prints the actual shapes so the assumption can be checked against a
//! real file instead of trusted.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example path_shapes -- <pdf> <page>
//! ```

use pdfium_render::prelude::*;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let index: i32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("PAGIFY_PDFIUM_LIB");

    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));
    let doc = pdfium.load_pdf_from_file(&a[0], None).expect("open");
    let page = doc.pages().get(index).expect("page");

    let (pw, ph) = (page.width().value, page.height().value);
    println!("page {} — {pw:.0} x {ph:.0} pt\n", index + 1);
    println!("  kind   segs      width x height     at          glyph-sized?");

    let (mut paths, mut small) = (0, 0);
    for object in page.objects().iter() {
        let Ok(b) = object.bounds() else { continue };
        let w = (b.right().value - b.left().value).abs();
        let h = (b.top().value - b.bottom().value).abs();

        let kind = match object.object_type() {
            PdfPageObjectType::Text => "text",
            PdfPageObjectType::Path => "path",
            PdfPageObjectType::Image => "image",
            _ => "other",
        };
        if object.object_type() != PdfPageObjectType::Path {
            continue;
        }
        paths += 1;

        // The classifier's own test, reproduced.
        let is_small = w > 0.5 && h > 0.5 && w < pw * 0.10 && h < ph * 0.10;
        if is_small {
            small += 1;
        }

        let segments = object.as_path_object().map(|p| p.segments().len()).unwrap_or(0);
        println!(
            "  {kind:<6} {segments:>5}   {w:>7.1} x {h:>6.1}   ({:>6.1},{:>6.1})   {}",
            b.left().value,
            b.top().value,
            if is_small { "yes" } else { "no" }
        );
    }
    println!("\n  {paths} paths, {small} counted as glyph-sized");
}

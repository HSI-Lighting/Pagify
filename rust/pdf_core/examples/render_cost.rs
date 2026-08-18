//! Is bounding PDFium's image cache costing us a re-decode every render?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --release --example render_cost -- <pdf> <page>
//! ```
//!
//! `limit_render_image_cache_size(true)` was set to stop a catalogue with tens of
//! megabytes of imagery per page accumulating decoded bitmaps. On a *scanned*
//! document, though, every page is essentially one enormous image — so a bounded
//! cache may mean decoding it again on every pass, and this file renders three
//! times slower than the vector catalogue.
//!
//! Renders each page twice per configuration: the first pass pays for the decode,
//! the second shows whether anything was kept.
use pdfium_render::prelude::*;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: render_cost <pdf> <page>");
    let index: i32 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(0);

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));
    let doc = pdfium.load_pdf_from_file(path, None).expect("open");

    for limited in [true, false] {
        println!(
            "\nlimit_render_image_cache_size({limited})"
        );
        for pass in 1..=3 {
            let t = Instant::now();
            let page = doc.pages().get(index).expect("page");
            let config = PdfRenderConfig::new()
                .set_target_size(1620, 1088)
                .set_format(PdfBitmapFormat::BGRA)
                .clear_before_rendering(true)
                .set_clear_color(PdfColor::WHITE)
                .limit_render_image_cache_size(limited);
            let bitmap = page.render_with_config(&config).expect("render");
            println!(
                "  pass {pass}: {:>6} ms   ({}x{})",
                t.elapsed().as_millis(),
                bitmap.width(),
                bitmap.height(),
            );
        }
    }
}

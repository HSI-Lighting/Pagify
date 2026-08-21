//! Host-side dump of a page's text runs.
//!
//! ```text
//! # once: a desktop build of the same PDFium the app is pinned to
//! curl -sSL -o pdfium-win.tgz https://github.com/bblanchon/pdfium-binaries/\
//! releases/download/chromium%2F7881/pdfium-win-x64.tgz
//! tar -xzf pdfium-win.tgz -C /tmp/pdfw
//!
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example inspect_text -- <pdf> <page-index> [tsv]
//! ```
//!
//! The tag must match `$PdfiumTag` in `tools/fetch_pdfium.ps1`, for the same
//! reason the Android ABIs do — the bindings are generated against one API
//! surface.
//!
//! This exists because the granularity and ordering of a PDFium text run are
//! what decide whether a highlight can follow a paragraph, and neither is
//! documented. Reading a real page settled both questions in a minute after
//! reasoning about them had produced a confident wrong answer. It is also how
//! the selection fixture in `app/src/test/resources/` is regenerated from a real
//! document, rather than hand-written to match whatever the code happens to do.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: inspect_text <pdf> <page-index> [tsv]");
    let index: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(0);
    let tsv = args.get(3).map(|s| s == "tsv").unwrap_or(false);

    // Through the engine, not around it. This example used to repeat the
    // top-left conversion itself, which meant it agreed with a bug in that
    // conversion instead of exposing it: on a page whose CropBox is inset, both
    // the engine and this dump reported runs 90 pt too high, and the dump was
    // the tool being used to check the engine.
    let doc = PdfiumDocument::open_path(path, None).expect("open");
    let size = doc.page_size(index).expect("page size");
    let page = doc.page(index).expect("page");
    let segments = page.text_segments().expect("text runs");

    if !tsv {
        println!(
            "page {index}  {:.3} x {:.3} pts",
            size.width_pt, size.height_pt,
        );
        println!("runs: {}", segments.len());
    }

    for (i, seg) in segments.iter().enumerate() {
        let content = seg.text.replace(['\t', '\n', '\r'], " ");
        let (left, top, right, bottom) = (seg.left, seg.top, seg.right, seg.bottom);

        if tsv {
            println!("{left:.2}\t{top:.2}\t{right:.2}\t{bottom:.2}\t{content}");
        } else {
            println!("{i:4}  L {left:7.2} T {top:7.2} R {right:7.2} B {bottom:7.2}  {content:?}",);
        }
    }

    if !tsv {
        // A run outside the page is the signature of a conversion that used the
        // wrong reference — it is how the CropBox bug was found, so the dump says
        // so rather than leaving it to be noticed.
        let stray = segments
            .iter()
            .filter(|s| s.top < 0.0 || s.left < 0.0 || s.bottom > size.height_pt)
            .count();
        if stray > 0 {
            println!(
                "\n  {stray} of {} runs fall outside the page — the coordinate \
                 conversion is wrong, not the document",
                segments.len(),
            );
        }
    }
}

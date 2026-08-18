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
use pdfium_render::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: inspect_text <pdf> <page-index> [tsv]");
    let index: i32 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(0);
    let tsv = args.get(3).map(|s| s == "tsv").unwrap_or(false);

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let bindings = Pdfium::bind_to_library(&lib).expect("bind pdfium");
    let pdfium = Pdfium::new(bindings);

    let doc = pdfium.load_pdf_from_file(path, None).expect("open");
    let page = doc.pages().get(index).expect("page");
    let height = page.height().value;
    let text = page.text().expect("text");

    if !tsv {
        println!("page {index}  {} x {} pts", page.width().value, height);
        println!("chars: {}", text.len());
    }

    for (i, seg) in text.segments().iter().enumerate() {
        let b = seg.bounds();
        // Same top-left flip the engine applies, so the dump is in the space the
        // UI actually works in.
        let (left, top) = (b.left().value, height - b.top().value);
        let (right, bottom) = (b.right().value, height - b.bottom().value);
        let content = seg.text().replace(['\t', '\n', '\r'], " ");
        // The engine drops whitespace-only runs — they carry no glyphs to select
        // — so a dump that kept them would not be the list the app works from.
        if content.trim().is_empty() {
            continue;
        }

        if tsv {
            println!("{left:.2}\t{top:.2}\t{right:.2}\t{bottom:.2}\t{content}");
        } else {
            let preview = content.chars().take(60).collect::<String>();
            println!(
                "{i:4}  L{left:8.2} T{top:8.2} R{right:8.2} B{bottom:8.2}  {preview:?}"
            );
        }
    }
}

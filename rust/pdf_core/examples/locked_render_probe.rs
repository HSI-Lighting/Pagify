//! Does a locked page still *draw* anything?
//!
//! Clearing the caches stops a stale thumbnail being shown, but that is only
//! half the question. The other half is whether the page, re-rendered from
//! scratch, still has the content on it — because a cache fix would hide a
//! lock that never really removed anything.
//!
//! So this renders the page before and after locking, at thumbnail size, and
//! reports how much ink is left.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example locked_render_probe -- <file.pdf> [page]
//! ```

use pdf_core::document::{Document, DocumentMut};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");

    let before = ink(&doc, page);
    println!("before locking : {:.2}% of the thumbnail has ink", before * 100.0);

    doc.lock_pages(&[page], b"a good passcode").expect("lock the page");

    // Re-rendered from the document as it now stands, not from a cache.
    let after = ink(&doc, page);
    println!("after locking  : {:.2}%", after * 100.0);

    // And from the saved file, reopened — what anyone else would see.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    let saved = ink(&reopened, page);
    println!("saved and reopened: {:.2}%", saved * 100.0);

    println!(
        "\n{}",
        if after < 0.0005 && saved < 0.0005 {
            "BLANK — nothing of the page is drawn any more"
        } else {
            "STILL DRAWING — the lock did not remove the content"
        }
    );
}

/// How much of a thumbnail-sized render is not white.
fn ink(doc: &dyn Document, page: usize) -> f32 {
    let size = doc.page_size(page).expect("size");
    // The width a thumbnail is drawn at in the strip.
    let width = 140u32;
    let scale = width as f32 / size.width_pt;
    let height = (size.height_pt * scale).max(1.0) as u32;

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut target = pdf_core::render::RenderTarget {
        width,
        height,
        stride: (width * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba,
        pixels: &mut pixels,
    };
    doc.page(page)
        .expect("page")
        .render_into(
            &pdf_core::document::RenderRequest { scale, ..Default::default() },
            &mut target,
        )
        .expect("render");

    let inked = pixels
        .chunks_exact(4)
        .filter(|p| {
            // Anything meaningfully darker than paper.
            p[0] < 240 || p[1] < 240 || p[2] < 240
        })
        .count();
    inked as f32 / (width * height) as f32
}

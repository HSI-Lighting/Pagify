//! What encodings do real images on a page actually use?
//!
//! The question that decides how Lock stores a sealed image. Two routes exist
//! and they trade against each other:
//!
//! - **The bytes the PDF already holds** (`FPDFImageObj_GetImageDataRaw`) are
//!   small — a catalogue photo is a JPEG measured in tens of kilobytes — but
//!   putting one back needs a loader for its filter, and PDFium only offers
//!   `FPDFImageObj_LoadJpegFileInline`.
//! - **The decoded pixels** (`FPDFImageObj_GetBitmap` / `SetBitmap`) always go
//!   back, whatever the filter was, but cost width × height × 4 bytes: a
//!   2000×1500 photo is 12 MB sealed instead of 200 kB.
//!
//! So the choice turns on how much of a real document is DCTDecode. Measured
//! rather than assumed, because guessing wrong means either a feature that
//! refuses half the images on a page or a file that grows by a hundred
//! megabytes.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example image_filter_probe -- <file.pdf> [first] [last]
//! ```

use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let first: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let last: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(first);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");

    let mut by_filter: std::collections::BTreeMap<String, (usize, u64, u64)> = Default::default();
    let mut images = 0;

    for page in first..=last.min(doc.page_count().saturating_sub(1)) {
        for found in doc.images_on(page).expect("images") {
            images += 1;
            let filter = if found.filters.is_empty() {
                "(none)".to_string()
            } else {
                found.filters.join("+")
            };
            let decoded = u64::from(found.pixel_width) * u64::from(found.pixel_height) * 4;
            let entry = by_filter.entry(filter).or_insert((0, 0, 0));
            entry.0 += 1;
            entry.1 += found.raw_bytes as u64;
            entry.2 += decoded;
        }
    }

    println!("pages {}..={last}, {images} image(s)\n", first + 1);
    println!("{:<28} {:>6} {:>14} {:>14}", "filter", "count", "raw total", "as pixels");
    for (filter, (count, raw, decoded)) in &by_filter {
        println!(
            "{filter:<28} {count:>6} {:>12} kB {:>12} kB",
            raw / 1024,
            decoded / 1024
        );
    }

    let total_raw: u64 = by_filter.values().map(|v| v.1).sum();
    let total_decoded: u64 = by_filter.values().map(|v| v.2).sum();
    if total_raw > 0 {
        println!(
            "\nstoring pixels instead of the stored bytes would cost {:.1}× more",
            total_decoded as f64 / total_raw as f64
        );
    }
}

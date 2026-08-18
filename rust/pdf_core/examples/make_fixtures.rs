//! Generates the committed round-trip fixtures.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example make_fixtures -- rust/pdf_core/fixtures
//! ```
//!
//! Each page is given a **distinct size**, which is the whole trick: it makes a
//! reorder, a deletion or an insertion verifiable from the page tree alone, with
//! no text to extract and no pixels to compare. A round-trip test can assert the
//! exact sequence of widths and know precisely which page ended up where.
//!
//! Generated rather than collected so the corpus is reproducible and small enough
//! to commit. The fixtures this cannot produce — scanned, signed, encrypted,
//! malformed, CJK — are listed as gaps in `fixtures/README.md`; they have to be
//! sourced, and pretending otherwise would leave holes that look covered.
use pdfium_render::prelude::*;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "fixtures".into());
    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));

    // Width in points identifies the page; the heights vary too so a rotation is
    // visible as a swap of the two rather than as no change at all.
    let ladder = [
        (200.0f32, 400.0f32),
        (250.0, 400.0),
        (300.0, 400.0),
        (350.0, 400.0),
        (400.0, 400.0),
    ];

    write(&pdfium, &format!("{out}/pages-ladder.pdf"), &ladder);
    write(&pdfium, &format!("{out}/single-page.pdf"), &ladder[..1]);
    write(
        &pdfium,
        &format!("{out}/mixed-sizes.pdf"),
        &[(595.0, 842.0), (420.0, 595.0), (612.0, 792.0), (842.0, 1191.0)],
    );
}

fn write(pdfium: &Pdfium, path: &str, sizes: &[(f32, f32)]) {
    let mut doc = pdfium.create_new_pdf().expect("create");
    for (w, h) in sizes {
        doc.pages_mut()
            .create_page_at_end(PdfPagePaperSize::from_points(
                PdfPoints::new(*w),
                PdfPoints::new(*h),
            ))
            .expect("add page");
    }
    doc.save_to_file(path).expect("save");
    println!(
        "{path}: {} pages, widths {:?}",
        sizes.len(),
        sizes.iter().map(|(w, _)| *w).collect::<Vec<_>>()
    );
}

//! Does a document made out of nothing come back as a document?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example blank_document_probe -- <scratch-dir>
//! ```
//!
//! Asked before the UI is built on top, because three separate things have to be
//! true and none of them is visible from the call site:
//!
//!   1. `FPDFPage_New` at index N actually appends rather than replacing,
//!   2. the ruling survives `save_to_writer` and a reopen — content written to a
//!      page that is closed before the save is exactly the shape of bug that
//!      leaves a file of blank sheets,
//!   3. the sheets come back the size that was asked for, in the orientation
//!      that was asked for.
//!
//! The object counts are the point. A file with the right number of pages and
//! nothing on them is what a silent failure looks like here.

use pdf_core::document::blank::{blank_document, Ruling};
use pdf_core::document::{Color, PageSize};
use pdfium_render::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scratch = args.get(1).expect("usage: blank_document_probe <scratch-dir>");
    std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");

    let a4 = PageSize { width_pt: 595.0, height_pt: 842.0 };
    let landscape = PageSize { width_pt: 842.0, height_pt: 595.0 };
    let cream = Color { r: 255, g: 246, b: 224, a: 255 };
    let black = Color { r: 16, g: 18, b: 20, a: 255 };

    let cases: Vec<(&str, usize, PageSize, Option<Color>, Ruling)> = vec![
        ("plain-white", 1, a4, None, Ruling::None),
        ("three-cream-lined", 3, a4, Some(cream), Ruling::Lined),
        ("grid-landscape", 2, landscape, None, Ruling::Grid),
        // Dark paper: the ruling has to be mixed the other way to exist at all.
        ("dots-on-black", 1, a4, Some(black), Ruling::Dots),
    ];

    for (name, pages, size, fill, ruling) in cases {
        let bytes = blank_document(pages, size, fill, ruling).expect("build");
        let path = format!("{scratch}/blank-{name}.pdf");
        std::fs::write(&path, &bytes).expect("write");

        let (count, first_objects, first_size) = inspect(&path);
        println!(
            "{name}: {} bytes, {count} page(s), {first_objects} object(s) on page 1, \
             {:.0}x{:.0}pt",
            bytes.len(),
            first_size.0,
            first_size.1,
        );

        assert_eq!(count, pages, "{name}: wrong number of sheets");
        assert!(
            (first_size.0 - size.width_pt).abs() < 1.0
                && (first_size.1 - size.height_pt).abs() < 1.0,
            "{name}: the sheet is not the size that was asked for",
        );

        // What must be on the page: one rectangle for coloured paper, plus one
        // object per rule or dot. Plain white unruled paper is the only case
        // that is legitimately empty.
        let expected_empty = fill.is_none() && ruling == Ruling::None;
        if expected_empty {
            assert_eq!(first_objects, 0, "{name}: plain paper is not plain");
        } else {
            assert!(first_objects > 0, "{name}: the sheet came back blank");
        }
        if fill.is_some() && ruling != Ruling::None {
            assert!(
                first_objects > 1,
                "{name}: the paper is there but the ruling is not",
            );
        }
    }

    println!("VERDICT: blank documents come back with their sheets, size and ruling intact");
}

/// Page count, objects on page 1, and page 1's size — read back off disk with
/// none of the builder's code in the way.
fn inspect(path: &str) -> (usize, i32, (f32, f32)) {
    // The engine binds PDFium once per process, and the builder has already
    // done it. Binding again is an error, not a second handle.
    let pdfium = pdf_core::document::pdfium_doc::pdfium().expect("pdfium");
    let bindings = pdfium.bindings();
    let doc = pdfium.load_pdf_from_file(path, None).expect("reopen");
    let handle = doc.handle();

    // Safety: the page is live for the block and closed before it ends.
    unsafe {
        let count = bindings.FPDF_GetPageCount(handle) as usize;
        let page = bindings.FPDF_LoadPage(handle, 0);
        assert!(!page.is_null(), "page 1 would not load");
        let objects = bindings.FPDFPage_CountObjects(page);
        let size = (
            bindings.FPDF_GetPageWidthF(page),
            bindings.FPDF_GetPageHeightF(page),
        );
        bindings.FPDF_ClosePage(page);
        (count, objects, size)
    }
}

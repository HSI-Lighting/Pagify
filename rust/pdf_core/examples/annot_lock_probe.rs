//! Does locking a page take its annotations with it — and give them back?
//!
//! Annotations are not page content. They live in the page's `/Annots` array
//! and the viewer draws them on top, so cutting the content stream — which is
//! all Lock did — leaves the ink exactly where it was. Reported from use as
//! "annotations don't get locked", with a locked page still showing pen
//! strokes over it.
//!
//! Two things have to be true before that can be fixed properly:
//!
//! 1. the page snapshot the vault keeps must **carry** the annotations, or
//!    removing them would destroy them for good;
//! 2. removing them must leave the rendered page blank.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example annot_lock_probe -- <file.pdf>
//! ```

use pdf_core::document::{Annotation, Document, DocumentMut, Point};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");

    // Ink on the page, as the Draw tab makes it.
    let size = doc.page_size(0).expect("size");
    let at = |fx: f32, fy: f32| Point { x: size.width_pt * fx, y: size.height_pt * fy };
    let ink = Annotation::Ink {
        strokes: vec![vec![at(0.2, 0.4), at(0.5, 0.5), at(0.7, 0.35)]],
        color: pdf_core::document::Color { r: 255, g: 20, b: 147, a: 255 },
        width: 3.0,
    };
    doc.add_annotation(0, &ink).expect("add ink");
    println!("annotations on the page  : {}", doc.annotations(0).map(|a| a.len()).unwrap_or(0));
    println!("ink with the annotation  : {:.2}%", ink_percent(&doc, 0) * 100.0);

    // Locking an *area* over the ink — the other half of the feature.
    let area = pdf_core::document::Rect {
        left: size.width_pt * 0.15,
        top: size.height_pt * 0.45,
        right: size.width_pt * 0.75,
        bottom: size.height_pt * 0.68,
    };
    let mut area_doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    area_doc.add_annotation(0, &ink).expect("add ink");
    let outcome = area_doc.lock_area(
        &pdf_core::document::Redaction {
            require_complete: false,
            ..pdf_core::document::Redaction::new(0, area)
        },
        b"a good passcode",
        None,
    );
    println!("\nlocking an area over the ink: {}", match &outcome {
        Ok(_) => "ok".to_string(),
        Err(e) => format!("refused — {e}"),
    });
    if outcome.is_ok() {
        println!(
            "  annotations left : {}",
            area_doc.annotations(0).map(|a| a.len()).unwrap_or(0)
        );
        println!("  ink on the page  : {:.2}%", ink_percent(&area_doc, 0) * 100.0);
        // The black mark accounts for most of that. What matters is the *pink*
        // — the annotation's own colour, which no redaction mark can produce.
        println!("  pink pixels      : {}", pink(&area_doc, 0));

        // And it comes back.
        let sealed = area_doc.open_lock(b"a good passcode").expect("unlock");
        for (index, pdf) in &sealed {
            area_doc.replace_page(*index, pdf).expect("restore");
        }
        println!(
            "  after unlocking  : {} annotation(s), {} pink pixel(s)",
            area_doc.annotations(0).map(|a| a.len()).unwrap_or(0),
            pink(&area_doc, 0)
        );
    }

    // What locking a whole page does today.
    doc.lock_pages(&[0], b"a good passcode").expect("lock");
    let left = doc.annotations(0).map(|a| a.len()).unwrap_or(0);
    println!("\nafter locking the page:");
    println!("  annotations left : {left}");
    println!("  ink on the page  : {:.2}%", ink_percent(&doc, 0) * 100.0);

    // And whether the sealed copy carries them, which decides whether they
    // could be removed and given back rather than destroyed.
    let sealed = doc.open_lock(b"a good passcode").expect("unlock");
    let kept = sealed
        .first()
        .and_then(|(_, pdf)| {
            pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(pdf.clone(), None).ok()
        })
        .and_then(|d| d.annotations(0).ok())
        .map(|a| a.len())
        .unwrap_or(0);
    println!("\nthe sealed copy keeps {kept} annotation(s)");
    println!(
        "  -> {}",
        if kept > 0 {
            "RECOVERABLE — they can be removed on locking and given back on unlocking"
        } else {
            "NOT RECOVERABLE — removing them would destroy them"
        }
    );
}

/// How much of a render is not paper, annotations included.
fn ink_percent(doc: &dyn Document, page: usize) -> f32 {
    let size = doc.page_size(page).expect("size");
    let width = 200u32;
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
        .filter(|p| p[0] < 240 || p[1] < 240 || p[2] < 240)
        .count();
    inked as f32 / (width * height) as f32
}

/// How many pixels are the ink's own colour — a pink no redaction mark draws.
fn pink(doc: &dyn Document, page: usize) -> usize {
    let size = doc.page_size(page).expect("size");
    let width = 200u32;
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

    pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 180 && p[1] < 120 && p[2] > 100 && p[2] < 200)
        .count()
}

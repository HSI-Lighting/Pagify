//! What is actually drawn on a page, when extraction finds no text?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --release --example text_objects -- <pdf> <page>
//! ```
//!
//! `text_cost` reports what `FPDFText_*` extraction yields, and on the big
//! catalogue that is nothing. But "extraction finds no text" has three quite
//! different causes, and they call for different answers:
//!
//! - **a scan** — one image covering the page, nothing else;
//! - **text converted to outlines** — the words are drawn as vector paths, which
//!   reads perfectly and cannot be selected in any viewer;
//! - **text objects PDFium cannot map** — a subset font with no `ToUnicode`, where
//!   another viewer might still select what PDFium reports as nothing.
//!
//! Counting objects by type and measuring how much of the page each image covers
//! separates them, which arguing from a single extraction number cannot.
use pdfium_render::prelude::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: text_objects <pdf> <page>");
    let arg = args.get(2).map(String::as_str).unwrap_or("0");

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));
    let doc = pdfium.load_pdf_from_file(path, None).expect("open");

    if arg == "all" {
        sweep(&doc);
        return;
    }
    let index: i32 = arg.parse().expect("page index");
    let page = doc.pages().get(index).expect("page");
    let page_area = page.width().value * page.height().value;
    println!(
        "page {index} of {} — {:.0} x {:.0} pt",
        doc.pages().len(),
        page.width().value,
        page.height().value,
    );

    let objects = page.objects();
    let (mut text, mut image, mut path_obj, mut form, mut other) = (0, 0, 0, 0, 0);
    let mut image_coverage = 0.0f32;
    let mut largest_image = 0.0f32;

    for object in objects.iter() {
        match object.object_type() {
            PdfPageObjectType::Text => text += 1,
            PdfPageObjectType::Path => path_obj += 1,
            PdfPageObjectType::XObjectForm => form += 1,
            PdfPageObjectType::Image => {
                image += 1;
                if let Ok(b) = object.bounds() {
                    let share = (b.width().value * b.height().value) / page_area * 100.0;
                    image_coverage += share;
                    largest_image = largest_image.max(share);
                }
            }
            _ => other += 1,
        }
    }

    println!(
        "  {} objects — text {text}, image {image}, path {path_obj}, form {form}, other {other}",
        objects.len(),
    );
    println!(
        "  images cover {image_coverage:.1}% of the page; largest single image {largest_image:.1}%",
    );

    let extracted = page.text().map(|t| t.len()).unwrap_or(-1);
    println!("  FPDFText characters extracted: {extracted}");

    let verdict = match (text, extracted) {
        (t, 0) if t > 0 => {
            "TEXT OBJECTS PRESENT, EXTRACTION RETURNS NOTHING — a font mapping problem, \
             and another viewer may well select what we cannot"
        }
        (0, 0) if largest_image > 90.0 => {
            "A SCAN — one image covers the page; nothing is selectable without OCR"
        }
        (0, 0) if path_obj > 100 => {
            "TEXT CONVERTED TO OUTLINES — the words are vector paths, not glyphs. \
             Reads perfectly, selects in no viewer at all without OCR"
        }
        (0, 0) => "no text and little artwork — probably a blank or near-blank page",
        _ => "text objects present and extraction works",
    };
    println!("\n  verdict: {verdict}");
}

/// Every page at once, so a claim about "the document" rests on the document
/// rather than on a sample of it.
fn sweep(doc: &PdfDocument) {
    let count = doc.pages().len();
    let mut with_text_objects = Vec::new();
    let mut with_extractable = Vec::new();

    for index in 0..count {
        let Ok(page) = doc.pages().get(index) else {
            continue;
        };
        let texts = page
            .objects()
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Text)
            .count();
        let chars = page.text().map(|t| t.len()).unwrap_or(0);

        if texts > 0 {
            with_text_objects.push((index, texts));
        }
        if chars > 0 {
            with_extractable.push((index, chars));
        }
    }

    println!("\n{count} pages");
    println!(
        "  pages carrying text objects : {} {:?}",
        with_text_objects.len(),
        with_text_objects.iter().take(12).collect::<Vec<_>>(),
    );
    println!(
        "  pages with extractable text : {} {:?}",
        with_extractable.len(),
        with_extractable.iter().take(12).collect::<Vec<_>>(),
    );
}

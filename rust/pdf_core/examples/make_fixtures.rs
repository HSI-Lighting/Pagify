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
    write_quadrants(&pdfium, &format!("{out}/quadrants.pdf"));
    write_spread(&pdfium, &format!("{out}/spread.pdf"));
    write_text_lines(&pdfium, &format!("{out}/text-lines.pdf"));
}

/// Three lines of known text at known places.
///
/// Every other fixture here is deliberately blank — page geometry was all the
/// write path needed. Selection needs the opposite: text whose exact content and
/// order are known in advance, so a test can say "these characters, in this
/// order, at these positions" rather than "some characters came back".
pub const TEXT_LINES: [&str; 3] = [
    "The quick brown fox",
    "jumps over the lazy dog",
    "Pack my box with five dozen jugs",
];

fn write_text_lines(pdfium: &Pdfium, path: &str) {
    let mut doc = pdfium.create_new_pdf().expect("create");
    let font = doc.fonts_mut().helvetica();

    let mut page = doc
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::from_points(
            PdfPoints::new(400.0),
            PdfPoints::new(300.0),
        ))
        .expect("add page");

    // Well apart vertically, so a test can aim at one line without ambiguity
    // about which it hit.
    for (index, line) in TEXT_LINES.iter().enumerate() {
        let mut object = PdfPageTextObject::new(&doc, line, font, PdfPoints::new(14.0))
            .expect("build the line");
        object
            .translate(PdfPoints::new(40.0), PdfPoints::new(240.0 - index as f32 * 60.0))
            .expect("place the line");
        page.objects_mut().add_text_object(object).expect("add the line");
    }

    page.regenerate_content().expect("regenerate");
    drop(page);
    doc.save_to_file(path).expect("save");
    println!("{path}: 1 page, 400x300, three lines of known text");
}

/// Two pages, each a single flat colour.
///
/// For the viewport capture, which draws parts of several pages into one picture.
/// The question it has to answer is *which page each pixel came from*, and one
/// colour per page answers it off a single pixel — where page geometry cannot,
/// since the bottom of one page and the top of the next are the same shape.
fn write_spread(pdfium: &Pdfium, path: &str) {
    const SIDE: f32 = 400.0;
    let colours = [
        PdfColor::new(255, 0, 0, 255),
        PdfColor::new(0, 0, 255, 255),
    ];

    let mut doc = pdfium.create_new_pdf().expect("create");
    for colour in colours {
        let mut page = doc
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::from_points(
                PdfPoints::new(SIDE),
                PdfPoints::new(SIDE),
            ))
            .expect("add page");

        let object = PdfPagePathObject::new_rect(
            &doc,
            PdfRect::new_from_values(0.0, 0.0, SIDE, SIDE),
            None,
            None,
            Some(colour),
        )
        .expect("build page fill");
        page.objects_mut().add_path_object(object).expect("add fill");
        page.regenerate_content().expect("regenerate");
    }

    doc.save_to_file(path).expect("save");
    println!("{path}: 2 pages, 400x400, red then blue");
}

/// A page in four solid colours, one per quadrant.
///
/// For the region export, where the thing under test is *where* the pixels came
/// from. Page geometry cannot answer that — a crop of the wrong part of the page
/// is the same size as a crop of the right part — so this fixture makes the
/// answer readable straight off a pixel: a correct crop of the top-left quadrant
/// contains red and nothing else, and any of the other three colours appearing
/// anywhere in it is content leaking in from outside the crop.
fn write_quadrants(pdfium: &Pdfium, path: &str) {
    const SIDE: f32 = 400.0;
    let half = SIDE / 2.0;

    let mut doc = pdfium.create_new_pdf().expect("create");
    let mut page = doc
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::from_points(
            PdfPoints::new(SIDE),
            PdfPoints::new(SIDE),
        ))
        .expect("add page");

    // PDF's own bottom-left origin here, since this talks to PDFium directly
    // rather than through the engine's top-left space. `new_from_values` takes
    // (bottom, left, top, right) — an order worth writing out rather than
    // trusting to memory.
    let quadrants = [
        // bottom, left, top,  right, colour
        (half, 0.0, SIDE, half, PdfColor::new(255, 0, 0, 255)), // top-left: red
        (half, half, SIDE, SIDE, PdfColor::new(0, 255, 0, 255)), // top-right: green
        (0.0, 0.0, half, half, PdfColor::new(0, 0, 255, 255)),  // bottom-left: blue
        (0.0, half, half, SIDE, PdfColor::new(255, 255, 0, 255)), // bottom-right: yellow
    ];

    for (bottom, left, top, right, colour) in quadrants {
        let rect = PdfRect::new_from_values(bottom, left, top, right);
        let object = PdfPagePathObject::new_rect(&doc, rect, None, None, Some(colour))
            .expect("build quadrant");
        page.objects_mut().add_path_object(object).expect("add quadrant");
    }

    page.regenerate_content().expect("regenerate");
    drop(page);
    doc.save_to_file(path).expect("save");
    println!("{path}: 1 page, 400x400, four solid quadrants");
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

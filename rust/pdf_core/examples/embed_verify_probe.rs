//! Does an edit made with an embedded font survive a save and draw?
//!
//! The edit succeeding only means the bytes were written. This reopens the
//! saved file the way any other reader would and asks two questions: are the
//! words there, and is there ink where they should be?

use pdf_core::document::{Document, DocumentMut};
use pdf_core::document::pdfium_doc::PdfiumDocument;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let into = std::env::args().nth(3).unwrap_or_else(|| "Sample text".into());
    let fonts: Vec<Vec<u8>> = std::env::args()
        .skip(4)
        .filter_map(|p| std::fs::read(p).ok())
        .collect();
    println!("{} typing font(s) offered", fonts.len());

    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    let runs = doc.text_runs(page).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 6)
        .cloned()
        .expect("a run with words");
    println!("editing object {} — {:?}", target.object, target.text);
    drop(doc);

    let mut doc = PdfiumDocument::open_path(&path, None).expect("open");
    doc.set_typing_fonts(fonts);
    doc.try_set_run_in_stream(page, target.object, &into).expect("edit");
    if let Some(face) = doc.substituted_face() {
        println!("written in {face}");
    }

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    println!("saved {} bytes", bytes.len());
    let reopened = PdfiumDocument::open_bytes(bytes.clone(), None).expect("reopen");

    let text = reopened.page(page).expect("page").text().unwrap_or_default();
    println!(
        "the words are {}in the saved file",
        if text.contains(&into) { "" } else { "NOT " }
    );

    // And there is ink where the run was.
    let size = reopened.page_size(page).expect("size");
    let scale = 900.0 / size.width_pt;
    let (w, h) = ((size.width_pt * scale) as u32, (size.height_pt * scale) as u32);
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let mut target_buf = pdf_core::render::RenderTarget {
        width: w,
        height: h,
        stride: (w * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba,
        pixels: &mut pixels,
    };
    reopened
        .page(page)
        .expect("page")
        .render_into(
            &pdf_core::document::RenderRequest { scale, ..Default::default() },
            &mut target_buf,
        )
        .expect("render");

    // Only the band the run sits in.
    let top = ((target.rect.top - 2.0) * scale).max(0.0) as u32;
    let bottom = (((target.rect.bottom + 2.0) * scale) as u32).min(h);
    let left = ((target.rect.left - 2.0) * scale).max(0.0) as u32;
    let right = (((target.rect.left + 200.0) * scale) as u32).min(w);
    let mut inked = 0usize;
    let mut looked = 0usize;
    for y in top..bottom {
        for x in left..right {
            let at = ((y * w + x) * 4) as usize;
            looked += 1;
            if pixels[at] < 240 || pixels[at + 1] < 240 || pixels[at + 2] < 240 {
                inked += 1;
            }
        }
    }
    println!(
        "ink where the words are: {inked} of {looked} pixels ({:.1}%)",
        100.0 * inked as f32 / looked.max(1) as f32
    );
    println!("{}", if inked > 8 { "DRAWN" } else { "NOTHING IS DRAWN THERE" });

    std::fs::write("/tmp/embed-verify.pdf", &bytes).ok();
    println!("saved a copy to /tmp/embed-verify.pdf");
}

//! Re-stack one object on a page and render, so what changed is what a reader
//! would see — and say whether anything visibly changed at all.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Stacking};

fn render(doc: &PdfiumDocument, page: usize) -> (u32, u32, Vec<u8>) {
    let size = doc.page_size(page).expect("size");
    let width = 1400u32;
    let scale = width as f32 / size.width_pt;
    let height = (size.height_pt * scale) as u32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut target = pdf_core::render::RenderTarget {
        width, height, stride: (width * 4) as usize,
        order: pdf_core::render::PixelOrder::Rgba, pixels: &mut pixels,
    };
    doc.page(page).expect("page")
        .render_into(&pdf_core::document::RenderRequest { scale, ..Default::default() }, &mut target)
        .expect("render");
    (width, height, pixels)
}

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let object: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(0);
    let how = std::env::args().nth(4).unwrap_or("front".into());
    let out = std::env::args().nth(5);
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");

    let before = render(&doc, page);
    let order_before: Vec<usize> = doc.drawn_objects(page).unwrap().iter().filter(|d| d.depth == 0).map(|d| d.object).collect();
    let target = doc.drawn_objects(page).unwrap().into_iter().find(|d| d.object == object).expect("the object");
    println!("target: {} {:?} at index {} of {}", target.kind.describe(), target.label, order_before.iter().position(|o| *o == object).unwrap(), order_before.len());

    let to = match how.as_str() { "back" => Stacking::Back, "up" => Stacking::Up, "down" => Stacking::Down, _ => Stacking::Front };
    match doc.restack(page, object, to) {
        Ok(()) => println!("restack {how}: ok"),
        Err(e) => { println!("restack {how}: REFUSED — {e}"); return; }
    }
    let after = render(&doc, page);
    let changed = before.2.chunks_exact(4).zip(after.2.chunks_exact(4)).filter(|(a, b)| a != b).count();
    println!("pixels that changed: {changed} of {}", before.0 * before.1);
    let now: Vec<String> = doc.drawn_objects(page).unwrap().iter().filter(|d| d.depth == 0).map(|d| format!("{}:{}", d.kind.describe(), d.label.chars().take(12).collect::<String>())).collect();
    println!("now at: index {:?} — last five: {:?}", now.iter().position(|n| n.contains(&target.label.chars().take(12).collect::<String>())), &now[now.len().saturating_sub(5)..]);
    if let Some(out) = out {
        let mut ppm = format!("P6\n{} {}\n255\n", after.0, after.1).into_bytes();
        for px in after.2.chunks_exact(4) { ppm.extend_from_slice(&px[..3]); }
        std::fs::write(&out, ppm).expect("write");
    }
}

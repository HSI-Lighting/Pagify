//! Move one picture on a page and render before and after, so what a reader
//! would see is what gets judged — not what the object list says.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point};

fn render(doc: &PdfiumDocument, page: usize, out: &str) {
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
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for px in pixels.chunks_exact(4) { ppm.extend_from_slice(&px[..3]); }
    std::fs::write(out, ppm).expect("write");
}

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let object: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(0);
    let out = std::env::args().nth(4).expect("output .ppm");
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");

    let before = doc.images_on(page).expect("images").into_iter().find(|i| i.object == object).expect("the picture");
    println!("before: {:?}", before.rect);
    doc.move_object(page, object, Point { x: 40.0, y: 190.0 }).expect("move");
    let after = doc.images_on(page).expect("images").into_iter().find(|i| i.object == object).expect("still there");
    println!("after : {:?}  (moved {:+.1}, {:+.1})", after.rect, after.rect.left - before.rect.left, after.rect.top - before.rect.top);
    render(&doc, page, &out);
    println!("wrote {out}");
}

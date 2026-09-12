//! Move a picture onto something, then re-stack it, rendering each step.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point, Stacking};
fn render(doc: &PdfiumDocument, page: usize) -> Vec<u8> {
    let size = doc.page_size(page).expect("size");
    let (width, scale) = (1400u32, 1400.0 / size.width_pt);
    let height = (size.height_pt * scale) as u32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut t = pdf_core::render::RenderTarget { width, height, stride: (width * 4) as usize, order: pdf_core::render::PixelOrder::Rgba, pixels: &mut pixels };
    doc.page(page).unwrap().render_into(&pdf_core::document::RenderRequest { scale, ..Default::default() }, &mut t).unwrap();
    pixels
}
fn diff(a: &[u8], b: &[u8]) -> usize { a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(x, y)| x != y).count() }
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let page: usize = std::env::args().nth(2).unwrap().parse().unwrap();
    let object: usize = std::env::args().nth(3).unwrap().parse().unwrap();
    let out = std::env::args().nth(4);
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).unwrap();
    let a = render(&doc, page);
    doc.move_object(page, object, Point { x: 40.0, y: 190.0 }).expect("move");
    let b = render(&doc, page);
    println!("after moving onto the caption : {} pixels changed", diff(&a, &b));
    // Indices move when the stream does — find the picture again by where it is.
    let find = |doc: &PdfiumDocument, near: pdf_core::document::Rect| {
        doc.images_on(page).unwrap().into_iter()
            .find(|i| (i.rect.left - near.left).abs() < 1.0 && (i.rect.top - near.top).abs() < 1.0)
            .map(|i| i.object).expect("the picture, by position")
    };
    let here = doc.images_on(page).unwrap().into_iter().find(|i| i.object == object).unwrap().rect;
    doc.restack(page, find(&doc, here), Stacking::Front).expect("front");
    let c = render(&doc, page);
    println!("after bringing it to the front: {} pixels changed (caption should now be hidden)", diff(&b, &c));
    doc.restack(page, find(&doc, here), Stacking::Back).expect("back");
    let d = render(&doc, page);
    println!("after sending it to the back  : {} pixels changed (caption back on top, picture still visible)", diff(&c, &d));
    if let Some(out) = out {
        let mut ppm = format!("P6\n1400 {}\n255\n", d.len() / 4 / 1400).into_bytes();
        for px in d.chunks_exact(4) { ppm.extend_from_slice(&px[..3]); }
        std::fs::write(out, ppm).unwrap();
    }
}

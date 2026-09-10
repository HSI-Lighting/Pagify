//! How much of each page is inked — is there anything on it at all?
use pdf_core::document::Document;
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    for page in 0..doc.page_count().min(10) {
        let size = doc.page_size(page).expect("size");
        let scale = 1.5f32;
        let (w, h) = ((size.width_pt * scale) as u32, (size.height_pt * scale) as u32);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let mut target = pdf_core::render::RenderTarget {
            width: w, height: h, stride: (w * 4) as usize,
            order: pdf_core::render::PixelOrder::Rgba, pixels: &mut pixels,
        };
        doc.page(page).expect("page").render_into(
            &pdf_core::document::RenderRequest { scale, ..Default::default() }, &mut target,
        ).expect("render");
        let inked = pixels.chunks_exact(4).filter(|p| p[0] < 240 || p[1] < 240 || p[2] < 240).count();
        println!("page {:>2}: {:.2}% inked", page + 1, 100.0 * inked as f32 / (w * h) as f32);
    }
}

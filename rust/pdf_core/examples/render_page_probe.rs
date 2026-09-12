//! Render one page to a PNG, exactly as PDFium draws it — no app overlays.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let out = std::env::args().nth(3).expect("an output .ppm path");
    let password = std::env::var("PDF_PASSWORD").ok();
    let doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
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
    // PPM is trivial to write and macOS can convert it.
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for px in pixels.chunks_exact(4) { ppm.extend_from_slice(&px[..3]); }
    std::fs::write(&out, ppm).expect("write");
    println!("wrote {out} ({width}x{height})");
}

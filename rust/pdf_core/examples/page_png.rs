//! Render a page to PNG, for looking at.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example page_png -- <pdf> <page> <out.png> [dpi]
//! ```

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, RenderRequest};
use pdf_core::render::{bitmap::PixelOrder, RenderTarget};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (path, index, out) = (&a[0], a[1].parse::<usize>().unwrap(), &a[2]);
    let dpi: f32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(150.0);

    let doc: Box<dyn Document> = Box::new(PdfiumDocument::open_path(path, None).expect("open"));
    let page = doc.page(index).expect("page");

    let scale = dpi / 72.0;
    let (w, h) = page.size().pixel_size(scale);
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    {
        let mut target =
            RenderTarget::new(w, h, w as usize * 4, PixelOrder::Rgba, &mut pixels).expect("target");
        page.render_into(
            &RenderRequest { scale, render_annotations: true, render_form_data: true, ..Default::default() },
            &mut target,
        )
        .expect("render");
    }

    image::save_buffer(out, &pixels, w, h, image::ColorType::Rgba8).expect("write");
    println!("{out}  {w}x{h}");
}

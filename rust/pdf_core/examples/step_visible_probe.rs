use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point, Stacking};
fn render(doc: &PdfiumDocument, page: usize) -> Vec<u8> {
    let size = doc.page_size(page).unwrap();
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
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).unwrap();
    doc.move_object(page, object, Point { x: 40.0, y: 190.0 }).expect("move");
    let here = doc.images_on(page).unwrap().into_iter().find(|i| i.object == object).unwrap().rect;
    let find = |doc: &PdfiumDocument| doc.images_on(page).unwrap().into_iter()
        .find(|i| (i.rect.left - here.left).abs() < 1.0 && (i.rect.top - here.top).abs() < 1.0).map(|i| i.object).unwrap();
    for (label, up) in [("up", true), ("up", true), ("down", false), ("down", false), ("down", false)] {
        let me = find(&doc);
        let over = doc.stacking_neighbour(page, me, up).unwrap();
        let before = render(&doc, page);
        let done = doc.restack(page, me, if up { Stacking::Up } else { Stacking::Down });
        let changed = diff(&before, &render(&doc, page));
        println!("{label:<5} passes {:<40} -> {}  ({changed} pixels changed)",
            over.map(|d| format!("{} {:?}", d.kind.describe(), d.label.chars().take(24).collect::<String>())).unwrap_or("nothing".into()),
            match done { Ok(()) => "ok".to_string(), Err(e) => format!("refused: {e}") });
    }
}

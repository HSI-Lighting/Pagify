//! How long matching a page of outlined type takes, and how well it does.
use pdf_core::document::Document;
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let fonts: Vec<Vec<u8>> = std::env::args()
        .skip(3)
        .filter_map(|p| std::fs::read(p).ok())
        .collect();

    let mut catalogue = pdf_core::document::glyphs::Catalogue::default();
    for font in &fonts {
        catalogue.extend_from_font_common(font);
    }
    println!("{} font(s), {} catalogue entries", fonts.len(), catalogue.len());

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let started = std::time::Instant::now();
    let words = doc
        .page(page)
        .expect("page")
        .recognise_outlined_words(&catalogue)
        .expect("recognise");
    println!(
        "{} word(s) in {:.2}s",
        words.len(),
        started.elapsed().as_secs_f32()
    );
    for word in words.iter().take(6) {
        println!("  {:?}", word.text);
    }
}

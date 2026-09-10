//! What a page is actually made of: text, pictures, or pictures of text.
use pdf_core::document::Document;
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    for page in 0..doc.page_count().min(10) {
        let size = doc.page_size(page).expect("size");
        let runs = doc.text_runs(page).map(|r| r.len()).unwrap_or(0);
        let words: usize = doc
            .text_runs(page)
            .map(|r| r.iter().map(|x| x.text.split_whitespace().count()).sum())
            .unwrap_or(0);
        let images = doc.images_on(page).unwrap_or_default();
        let biggest = images
            .iter()
            .map(|i| ((i.rect.right - i.rect.left) * (i.rect.bottom - i.rect.top)).abs())
            .fold(0.0f32, f32::max);
        let page_area = size.width_pt * size.height_pt;
        println!(
            "page {page:>2}: {runs:>3} run(s) / {words:>4} word(s), {} image(s), largest covers {:.0}% of the page",
            images.len(),
            100.0 * biggest / page_area.max(1.0)
        );
    }
}

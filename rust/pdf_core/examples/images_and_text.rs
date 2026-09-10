//! Which documents have both text and an image on one page?
use pdf_core::document::{Document, DocumentMut};
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(mut doc) = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None)
        else { continue };
        for page in 0..doc.page_count().min(60) {
            let images = doc.images_on(page).map(|i| i.len()).unwrap_or(0);
            let chars = doc.page(page).and_then(|p| p.characters()).map(|c| c.text.chars().count()).unwrap_or(0);
            if images > 0 && chars > 40 {
                println!("{}: page {} — {images} image(s), {chars} characters",
                    path.rsplit('/').next().unwrap_or(&path), page + 1);
                break;
            }
        }
    }
}

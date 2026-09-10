//! A page's boxes and rotation — the things that make where a word *is*
//! disagree with where it is *drawn*.
use pdf_core::document::{Document, DocumentMut};
fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    for page in 0..doc.page_count().min(3) {
        let size = doc.page_size(page).expect("size");
        let crop = doc.page_crop(page);
        let rotation = doc.page_rotation(page);
        println!(
            "page {}: size {:.2} x {:.2}  rotation {:?}",
            page + 1,
            size.width_pt,
            size.height_pt,
            rotation
        );
        match crop {
            Ok(c) => println!(
                "   crop: left {:.2} top {:.2} right {:.2} bottom {:.2}  ({:.2} x {:.2})",
                c.left, c.top, c.right, c.bottom,
                (c.right - c.left).abs(), (c.bottom - c.top).abs()
            ),
            Err(e) => println!("   crop: {e}"),
        }
        // Where the first few runs sit, to compare against the page box.
        if let Ok(runs) = doc.text_runs(page) {
            for run in runs.iter().take(3) {
                println!(
                    "   run {:>3}: rect l {:.1} t {:.1} r {:.1} b {:.1}  {:?}",
                    run.object, run.rect.left, run.rect.top, run.rect.right, run.rect.bottom,
                    run.text.chars().take(20).collect::<String>()
                );
            }
        }
    }
}

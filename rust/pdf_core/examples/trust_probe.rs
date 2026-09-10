//! How far apart are a well-authored page and a paint-ordered one, really?
//!
//! A threshold set from one synthetic example is a threshold set from nothing.
//! This prints the measurement for each fixture so the gap between the two
//! populations is visible and the cut can be put where the gap is.
use pdf_core::document::layout;
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    println!(
        "{:<44} {:>8}  {:<5} {:>9}",
        "document", "worst", "page", "flagged/judged"
    );
    for name in std::env::args().skip(1) {
        let Ok(doc) = PdfiumDocument::open_path(&name, None) else { continue };
        // Per page, not per document. A document's worst page is what decides
        // whether anything gets reconstructed, and averaging hides it.
        let mut worst = 0.0f32;
        let mut worst_page = 0usize;
        let mut rebuilt = 0usize;
        let mut judged = 0usize;

        for i in 0..doc.page_count() {
            let Ok(page) = doc.page(i) else { continue };
            let Ok(glyphs) = page.glyphs() else { continue };
            if glyphs.len() < 40 {
                continue;
            }
            let t = layout::trust(&glyphs);
            judged += 1;
            if t.disorder() > worst {
                worst = t.disorder();
                worst_page = i + 1;
            }
            if t.is_noteworthy() {
                rebuilt += 1;
            }
        }

        if judged == 0 {
            continue;
        }
        let flag = if rebuilt > 0 { "  <-- flagged" } else { "" };
        println!(
            "{:<44} {:>8.3}  p{:<4} {:>4}/{:<4}{}",
            name.rsplit('/').next().unwrap_or(&name).chars().take(44).collect::<String>(),
            worst,
            worst_page,
            rebuilt,
            judged,
            flag
        );
    }
}

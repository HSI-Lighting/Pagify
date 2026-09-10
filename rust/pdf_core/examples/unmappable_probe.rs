//! Which words in a document did not survive extraction?
//!
//! Prints every character that came back as U+FFFD, in context, so the damage
//! can be read as words rather than as a count. A page can be 99.7% correct and
//! still have turned `config` into `con?g`, which no ratio makes visible.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example unmappable_probe -- <pdf>
//! ```

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("usage: unmappable_probe <pdf>");
    let doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(&path, None).expect("open"));

    let mut total = 0usize;
    for index in 0..doc.page_count() {
        let Ok(page) = doc.page(index) else { continue };
        let Ok(chars) = page.characters() else { continue };

        let text: Vec<char> = chars.text.chars().collect();
        let bad: Vec<usize> =
            text.iter().enumerate().filter(|(_, c)| **c == '\u{FFFD}').map(|(i, _)| i).collect();
        if bad.is_empty() {
            continue;
        }
        total += bad.len();

        println!("page {}: {} of {} characters unreadable", index + 1, bad.len(), text.len());
        for i in bad.iter().take(20) {
            let from = i.saturating_sub(12);
            let to = (i + 13).min(text.len());
            let around: String =
                text[from..to].iter().collect::<String>().replace(['\n', '\r'], " ");
            println!("  at {i:>6}  …{around}…");
        }
    }

    println!("\n{total} unreadable characters in {} pages", doc.page_count());
}

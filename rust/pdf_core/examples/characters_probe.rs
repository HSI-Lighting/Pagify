//! Why does a page with text produce no character boxes?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --release --example characters_probe -- <pdf> [page]
//! ```
//!
//! On the device, selecting on the catalogue's 145/146 spread produced nothing:
//! the long press fired, the engine came back with no characters, and the page
//! plainly has text — `text_segments` reports hundreds of runs on it.
//!
//! `characters()` drops any character whose bounds it cannot read, which is a
//! safe default and a silent one: a page where every call fails is
//! indistinguishable from a page with no text at all. This separates the two —
//! how many characters PDFium counts, how many map to Unicode, and how many have
//! usable bounds — so the answer is a number rather than an inference.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: characters_probe <pdf> [page]");
    let page_index: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(0);

    let doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(path, None).expect("open"));
    if args.get(2).map(String::as_str) == Some("all") {
        // A sweep, because "which pages have text" is a question about the
        // document rather than about one page — and on a 2.9 GB file, opening it
        // once and walking is the difference between seconds and minutes.
        let mut with_text = 0;
        for index in 0..doc.page_count() {
            let page = doc.page(index).expect("page");
            let runs = page.text_segments().map(|s| s.len()).unwrap_or(0);
            let characters = page
                .characters()
                .map(|c| c.text.chars().count())
                .unwrap_or(0);
            if runs > 0 || characters > 0 {
                with_text += 1;
                println!("page {index}: runs={runs} characters={characters}");
            }
        }
        println!("{with_text} of {} pages carry text", doc.page_count());
        return;
    }

    let page = doc.page(page_index).expect("page");

    let runs = page.text_segments().map(|s| s.len()).unwrap_or(0);
    let flat = page.text().map(|t| t.chars().count()).unwrap_or(0);
    let characters = page.characters().expect("characters");

    println!("page {page_index}");
    println!("  runs from text_segments : {runs}");
    println!("  chars from text()       : {flat}");
    println!("  chars from characters() : {}", characters.text.chars().count());
    println!("  boxes                   : {}", characters.boxes.len() / 4);

    if characters.text.is_empty() && flat > 0 {
        println!("\n  Every character was dropped. The text is there and the boxes are not.");
    }

    let sample: String = characters.text.chars().take(60).collect();
    println!("  first characters        : {sample:?}");
}

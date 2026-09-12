//! Do the paths PDFium reports line up with the paths the stream draws?
//!
//! A path in a content stream is a run of construction operators — `m`, `l`,
//! `c`, `v`, `y`, `re`, `h` — closed by a painting one: `S`, `f`, `B`, `n` and
//! their variants. Whether the *n*th of those is the *n*th path object PDFium
//! reports is the question everything else depends on, and it is a question
//! about real files rather than about the specification.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DrawnKind};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(6);
    let password = std::env::var("PDF_PASSWORD").ok();

    let doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    let pages = doc.page_count().min(limit);
    let (mut agree, mut differ) = (0usize, 0usize);

    for page in 0..pages {
        let reported = doc
            .drawn_objects(page)
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.depth == 0 && d.kind == DrawnKind::Shape)
            .count();

        let Ok(stream) = doc.page_stream(page) else { continue };
        let Ok(operations) = pdf_core::pdf::content::parse(&stream) else { continue };

        let (mut drawn, mut clips, mut open) = (0usize, 0usize, false);
        let mut clipping = false;
        for op in &operations {
            match op.operator.as_slice() {
                b"m" | b"re" => open = true,
                b"l" | b"c" | b"v" | b"y" | b"h" => {}
                b"W" | b"W*" => clipping = true,
                b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                    if open {
                        if op.operator.as_slice() == b"n" {
                            clips += 1;
                        } else {
                            drawn += 1;
                        }
                        open = false;
                        clipping = false;
                    }
                }
                _ => {}
            }
        }
        let _ = clipping;

        let ok = drawn == reported;
        if ok { agree += 1 } else { differ += 1 }
        println!(
            "  page {:>3}: PDFium says {reported:>4} path(s); the stream paints {drawn:>4} \
             (+{clips} clip-only)  {}",
            page + 1,
            if ok { "agree" } else { "*** DIFFER ***" }
        );
    }
    println!("\n{path}\n  pages agreeing: {agree}, differing: {differ}");
}
